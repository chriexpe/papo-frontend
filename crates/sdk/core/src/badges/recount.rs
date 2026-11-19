//! Coalesced per-channel badge recounts; immutable inputs and small fingerprints avoid rescanning
//! unrelated history. This worker never pins history, hydrates evidence or owns read/access facts.

#[cfg(test)]
#[path = "recount_tests.rs"]
mod tests;

use super::{BadgesSnapshot, ChannelBadges, counts};
use crate::{
    Generation, HistorySnapshot, ReadState, ReadingSnapshot, ResourceSnapshot, Session,
    UnreadCount, Uuid,
    history::access::ChannelAdmission,
    notifications::{ledger, policy::ChannelPolicy},
    reading::evidence::{self, ChannelBoundary, ChannelGate},
    session::SessionInner,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, atomic::Ordering},
};

#[derive(Clone, PartialEq, Eq)]
struct Input {
    history: Option<(Generation, u64)>,
    reading: Option<ReadState>,
    boundary: ChannelBoundary,
    gate: ChannelGate,
    policy: ChannelPolicy,
    alternative_policy: Option<ChannelPolicy>,
    backend_ready: bool,
    direct_message: bool,
}

struct Computed {
    input: Input,
    value: Option<Arc<ChannelBadges>>,
}

pub(super) fn start(session: &Session) {
    if session.inner.badges.started.swap(true, Ordering::AcqRel) {
        return;
    }
    let sources = Sources {
        resources: session.subscribe_resources(),
        reading: session.subscribe_reading(),
        history: session.inner.history.subscribe_changes(),
        policy: session.inner.notifications.policy_changes.subscribe(),
        connection: session.subscribe_connection(),
    };
    let session = session.clone();
    let token = session.inner.tasks.token();
    let runtime = session.client.inner.runtime.clone();
    runtime.spawn(async move {
        let _token = token;
        run(&session, sources).await;
    });
}

struct Sources {
    resources: tokio::sync::watch::Receiver<Arc<ResourceSnapshot>>,
    reading: tokio::sync::watch::Receiver<Arc<ReadingSnapshot>>,
    history: tokio::sync::watch::Receiver<()>,
    policy: tokio::sync::watch::Receiver<()>,
    connection: tokio::sync::watch::Receiver<crate::ConnectionStatus>,
}
impl Sources {
    fn changed(&self) -> bool {
        self.resources.has_changed().unwrap_or(true)
            || self.reading.has_changed().unwrap_or(true)
            || self.history.has_changed().unwrap_or(true)
            || self.policy.has_changed().unwrap_or(true)
            || self.connection.has_changed().unwrap_or(true)
    }
    async fn wait(&mut self, session: &Session) -> bool {
        tokio::select! {
            biased;
            () = session.inner.root.cancelled() => false,
            result = self.resources.changed() => result.is_ok(),
            result = self.reading.changed() => result.is_ok(),
            result = self.history.changed() => result.is_ok(),
            result = self.policy.changed() => result.is_ok(),
            result = self.connection.changed() => result.is_ok(),
        }
    }
}

async fn run(session: &Session, mut sources: Sources) {
    let mut previous = BTreeMap::new();
    loop {
        if session.is_closed() {
            return;
        }
        let resources = sources.resources.borrow_and_update().clone();
        let reading = sources.reading.borrow_and_update().clone();
        sources.history.borrow_and_update();
        sources.policy.borrow_and_update();
        sources.connection.borrow_and_update();
        let generation = session.generation();
        if resources.version > 0 && resources.generation == generation {
            let (computed, channels) = build(session, &resources, &reading, &previous).await;
            // Queries may yield to settings commits, read reports, history updates or revocation.
            // Serialize final publication with the same admission boundary that redacts images.
            let admissions = session.inner.history.admission.read().await;
            if sources.changed() || session.generation() != generation {
                continue;
            }
            if computed.iter().any(|(id, computed)| {
                let current = match admissions.channel(*id) {
                    Some(ChannelAdmission::Allowed) => {
                        evidence::channel_gate_sync(&session.inner, *id, generation)
                    }
                    Some(ChannelAdmission::Denied(reason)) => ChannelGate::Denied(reason),
                    Some(ChannelAdmission::Quarantined) | None => ChannelGate::Quarantined,
                };
                current != computed.input.gate
            }) {
                continue;
            }
            if session
                .inner
                .epochs
                .apply(generation, || {
                    publish(
                        &session.inner,
                        generation,
                        resources.channel_census_complete,
                        channels,
                    );
                })
                .is_err()
            {
                continue;
            }
            previous = computed;
        }
        if !sources.wait(session).await {
            return;
        }
    }
}

fn gate(session: &Session, channel: Uuid, generation: Generation) -> ChannelGate {
    // A never-admitted channel is uncertain, not an allowed cold cache.
    if session
        .inner
        .history
        .admission
        .try_read()
        .ok()
        .and_then(|a| a.channel(channel))
        .is_none()
    {
        return ChannelGate::Quarantined;
    }
    evidence::channel_gate_sync(&session.inner, channel, generation)
}

async fn build(
    session: &Session,
    resources: &ResourceSnapshot,
    reading: &ReadingSnapshot,
    previous: &BTreeMap<Uuid, Computed>,
) -> (BTreeMap<Uuid, Computed>, BTreeMap<Uuid, Arc<ChannelBadges>>) {
    let generation = resources.generation;
    let settings = ledger::settings(session).await.ok();
    let (backend, backend_ready) = {
        let backend = session
            .inner
            .notifications
            .backend
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let current = backend
            .as_ref()
            .filter(|policy| policy.generation == generation);
        (
            current.map_or_default(|policy| policy.config.clone()),
            current.is_some(),
        )
    };
    let settings_ready = settings.is_some();
    let settings = settings.unwrap_or_default();
    let snapshots: BTreeMap<_, _> = session
        .inner
        .history
        .snapshots()
        .await
        .into_iter()
        .collect();
    let boundaries = evidence::channel_boundaries(&session.inner).await;
    let metadata = channel_metadata(resources);
    let old = session.inner.badges.image.borrow().clone();
    let mut computed = BTreeMap::new();
    let mut channels = BTreeMap::new();
    for (id, metadata) in metadata {
        let ChannelMetadata {
            direct_message: dm,
            backend_mode: mode,
            ambiguous,
        } = metadata;
        let snapshot = snapshots
            .get(&id)
            .filter(|snapshot| snapshot.generation == generation);
        let input = Input {
            history: snapshot.map(|snapshot| (snapshot.generation, snapshot.version)),
            reading: (reading.generation == generation)
                .then(|| reading.channels.get(&id).cloned())
                .flatten(),
            boundary: boundaries.get(&id).copied().unwrap_or_default(),
            gate: gate(session, id, generation),
            policy: ChannelPolicy::new(
                &settings,
                &backend,
                id,
                dm,
                if ambiguous { None } else { mode },
            ),
            alternative_policy: ambiguous
                .then(|| ChannelPolicy::new(&settings, &backend, id, false, mode)),
            backend_ready: backend_ready && settings_ready,
            direct_message: dm,
        };
        if matches!(input.gate, ChannelGate::Denied(_)) {
            computed.insert(id, Computed { input, value: None });
            continue;
        }
        let value = if let Some(value) = previous
            .get(&id)
            .filter(|old| old.input == input)
            .and_then(|old| old.value.as_ref())
        {
            Arc::clone(value)
        } else {
            let value = evaluate(session, id, generation, &input, snapshot.map(Arc::as_ref)).await;
            old.channels
                .get(&id)
                .filter(|old| ***old == value)
                .cloned()
                .unwrap_or_else(|| Arc::new(value))
        };
        channels.insert(id, Arc::clone(&value));
        computed.insert(
            id,
            Computed {
                input,
                value: Some(value),
            },
        );
    }
    (computed, channels)
}

struct ChannelMetadata<'a> {
    direct_message: bool,
    backend_mode: Option<&'a str>,
    ambiguous: bool,
}

fn channel_metadata(resources: &ResourceSnapshot) -> BTreeMap<Uuid, ChannelMetadata<'_>> {
    let mut metadata = BTreeMap::new();
    for channel in resources.channels.iter() {
        if let Some(id) = channel.id.as_deref().and_then(|id| id.parse::<Uuid>().ok()) {
            metadata.insert(
                id,
                ChannelMetadata {
                    direct_message: false,
                    backend_mode: channel.notification_settings.as_deref(),
                    ambiguous: false,
                },
            );
        }
    }
    for dm in resources.dms.iter() {
        if let Ok(id) = dm.id.parse() {
            metadata
                .entry(id)
                .and_modify(|value| {
                    value.direct_message = true;
                    value.ambiguous = true;
                })
                .or_insert(ChannelMetadata {
                    direct_message: true,
                    backend_mode: None,
                    ambiguous: false,
                });
        }
    }
    metadata
}

async fn evaluate(
    session: &Session,
    id: Uuid,
    generation: Generation,
    input: &Input,
    snapshot: Option<&HistorySnapshot>,
) -> ChannelBadges {
    let mut result = ChannelBadges::unknown(input.direct_message);
    if input.gate == ChannelGate::Proceed {
        result.has_unread = input
            .reading
            .as_ref()
            .and_then(|state| counts::has_unread(state.unread_count));
    }
    if input.policy.badges_disabled()
        && input
            .alternative_policy
            .is_none_or(ChannelPolicy::badges_disabled)
    {
        result.mention_count = UnreadCount::Exact(0);
    } else if input.gate == ChannelGate::Proceed
        && input.backend_ready
        && let Some(snapshot) = snapshot
        && let Ok(read) = evidence::channel_read_evidence(session, generation, id).await
    {
        result.mention_count = counts::mentions(
            snapshot,
            read,
            input.policy,
            session.account().user(),
            input.direct_message,
        )
        .unwrap_or(UnreadCount::AtLeast(0));
    }
    result
}

pub(super) fn publish(
    session: &SessionInner,
    generation: Generation,
    channels_complete: bool,
    channels: BTreeMap<Uuid, Arc<ChannelBadges>>,
) {
    let old = session.badges.image.borrow().clone();
    if old.version > 0
        && old.generation == generation
        && old.channels_complete == channels_complete
        && old.channels == channels
    {
        return;
    }
    let Some(version) = old.version.checked_add(1) else {
        return;
    };
    session.badges.image.send_replace(Arc::new(BadgesSnapshot {
        generation,
        version,
        channels_complete,
        channels,
    }));
}
