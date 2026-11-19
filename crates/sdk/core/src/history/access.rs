//! Per-channel history admission for one session: applies accepted read decisions to channel
//! incarnations, the durable cache revoke marker and the reading fence. Decisions come from
//! `permissions::read_access`; cache SQL, reading storage and resource fetching live elsewhere.
//!
//! Every resource producer runs `seal` before writing accepted resources and `grant` after,
//! while holding `resource_serial`, so producers never interleave. A 403/404 or deletion
//! (`deny`) can land at any time; `grant` never admits over its unconfirmed marker.

use std::sync::Arc;

use papo_api::Uuid;

use crate::{
    AccountCache, CacheError, CacheResource, ClientError, ClientHandle, Generation,
    ResourceSnapshot,
    permissions::read_access::{ReadAccess, ReadDecisions},
    reconcile::ResourceConnection,
    session::SessionInner,
};

use super::HistoryClosedReason;

mod admissions;
use admissions::Admission;
pub(crate) use admissions::{Admissions, ChannelAdmission, Evidence, Refetch};

/// Whether accepted changes to these resources can change a read decision: everything the
/// evaluator reads (server owner, channels and overwrites, role assignments, DMs, blocks).
pub(crate) fn affects_read_access(changed: &[CacheResource]) -> bool {
    changed
        .iter()
        .any(|resource| !matches!(resource, CacheResource::Profiles))
}

/// One accepted snapshot's access transition, split around the producer's resource write.
/// `begin` evaluates and seals before the write; `finish` grants and installs the cache
/// connection after it. Both run under `resource_serial`.
#[must_use]
pub(crate) struct Transition {
    decisions: ReadDecisions,
    generation: Generation,
    evidence: Evidence,
    sealed: bool,
}

/// Capture before a producer starts fetching the resources it will admit.
pub(crate) async fn evidence(session: &SessionInner, refetch: Refetch) -> Evidence {
    session.history.admission.read().await.evidence(refetch)
}

impl Transition {
    /// # Errors
    /// Returns `ResourceData` for an unattributable channel ID, generation changes or the
    /// durable revoke failure; on error every seal already applied stays in place.
    pub(crate) async fn begin(
        session: &SessionInner,
        connection: &ResourceConnection,
        snapshot: &ResourceSnapshot,
        evidence: Evidence,
    ) -> Result<Self, ClientError> {
        session.epochs.validate(snapshot.generation)?;
        let decisions = ReadDecisions::evaluate(snapshot, session.account.user())?;
        let sealed = seal(session, connection, &decisions, snapshot.generation).await?;
        Ok(Self {
            decisions,
            generation: snapshot.generation,
            evidence,
            sealed,
        })
    }

    /// Returns whether any channel's admission changed, so reading must reconcile.
    /// # Errors
    /// Returns generation changes or a failed revoke of a previously unrevoked channel.
    pub(crate) async fn finish(
        self,
        session: &SessionInner,
        connection: &ResourceConnection,
    ) -> Result<bool, ClientError> {
        let granted = grant(
            session,
            connection,
            &self.decisions,
            self.evidence,
            self.generation,
        )
        .await?;
        *session.history.connection.write().await = Some(super::channel::CacheConnection {
            cache: std::sync::Arc::clone(&connection.cache),
            rest: connection.rest.clone(),
            generation: self.generation,
        });
        Ok(self.sealed || granted)
    }
}

/// Seal every channel whose accepted decision is no longer Allowed, before the producer
/// writes the accepted resources, then make each denial durable. A failed or cancelled
/// durable step returns its error with the seal still in place.
/// Returns whether any channel changed admission.
async fn seal(
    session: &SessionInner,
    connection: &ResourceConnection,
    decisions: &ReadDecisions,
    generation: Generation,
) -> Result<bool, ClientError> {
    session.epochs.validate(generation)?;
    let store = &session.history;
    let sealed = {
        let mut channels = store.channels.lock().await;
        let mut admissions = store.admission.write().await;
        let sealed = admissions.seal(decisions, &channels);
        // A quarantined incarnation stays so the same handle resumes on a grant.
        channels.retain(|_, channel| !channel.is_closed());
        session.reading.positions.quarantine(&sealed.quarantined);
        for (id, reason) in &sealed.denied {
            session
                .reading
                .positions
                .revoke(&[*id], generation, *reason);
        }
        for (id, reason) in &sealed.retained_denied {
            session
                .reading
                .positions
                .retain_revoked(*id, generation, *reason);
        }
        crate::badges::restrict(
            session,
            &sealed.quarantined,
            sealed
                .denied
                .iter()
                .chain(&sealed.retained_denied)
                .map(|(id, _)| *id),
        );
        sealed
    };
    let changed = !sealed.quarantined.is_empty() || !sealed.denied.is_empty();
    revoke(
        session,
        &connection.client,
        &connection.cache,
        &sealed.denied,
        |id| decisions.get(id).is_some(),
        generation,
    )
    .await?;
    Ok(changed)
}

/// Admit every Allowed channel after the producer wrote the accepted resources. A quarantined
/// channel resumes on its existing incarnation; a channel whose revoke never reached disk
/// is revoked first, so the regrant gets a fresh cache set. Returns whether anything changed.
async fn grant(
    session: &SessionInner,
    connection: &ResourceConnection,
    decisions: &ReadDecisions,
    evidence: Evidence,
    generation: Generation,
) -> Result<bool, ClientError> {
    session.epochs.validate(generation)?;
    let store = &session.history;
    let allowed = decisions.with(ReadAccess::Allowed);
    let unrevoked: Vec<_> = {
        let admissions = store.admission.read().await;
        allowed
            .iter()
            .filter(|id| admissions.may_grant(**id, evidence))
            .filter_map(|id| match admissions.get(*id) {
                Some(Admission::Denied {
                    reason,
                    durable: false,
                }) => Some((*id, reason)),
                _ => None,
            })
            .collect()
    };
    revoke(
        session,
        &connection.client,
        &connection.cache,
        &unrevoked,
        |_| true,
        generation,
    )
    .await?;
    session.epochs.validate(generation)?;
    let mut resumed = Vec::new();
    let mut changed = false;
    {
        let mut channels = store.channels.lock().await;
        let mut admissions = store.admission.write().await;
        for id in allowed {
            // Evidence fetched before a later denial (for example a 403 that completed while
            // this producer was fetching or writing) cannot reopen the channel.
            if !admissions.may_grant(id, evidence) {
                continue;
            }
            match admissions.get(id) {
                // Already allowed; or a 403/404 or deletion that landed while this grant was
                // revoking, with no confirmed marker yet: it stays denied until a later grant
                // revokes it.
                Some(Admission::Allowed | Admission::Denied { durable: false, .. }) => continue,
                Some(Admission::Quarantined) => resumed.push(id),
                Some(Admission::Denied { durable: true, .. }) | None => {
                    // Nothing admitted may already hold state for this channel: an Arc made
                    // outside admission could carry the old set, so it never becomes the grant.
                    if let Some(stray) = channels.remove(&id) {
                        stray.close(HistoryClosedReason::ChannelRemoved);
                    }
                }
            }
            changed = true;
            admissions.grant(id);
        }
        session.reading.positions.release(&resumed);
        for id in &resumed {
            if let Some(channel) = channels.get(id) {
                channel.release();
            }
        }
    }
    for id in resumed {
        resume(session, id, generation).await?;
    }
    Ok(changed)
}

/// Deliver the resumed incarnation's current state. Live events kept updating its model and
/// cache while it was quarantined, so a full publication replaces the stale last-good one.
async fn resume(
    session: &SessionInner,
    channel: Uuid,
    generation: Generation,
) -> Result<(), ClientError> {
    let Some(channel) = session.history.channels.lock().await.get(&channel).cloned() else {
        return Ok(());
    };
    let state = channel.state.lock().await;
    if state.set_id.is_none() {
        return Ok(());
    }
    match session
        .epochs
        .apply(generation, || channel.publish(&state, generation))?
    {
        Err(error) if !error.is_sealed() => Err(error.into()),
        _ => Ok(()),
    }
}

/// Seal one channel after a backend 403/404 or gateway deletion and make it durable.
/// The caller has already closed the incarnation.
pub(super) async fn deny(
    session: &SessionInner,
    client: &ClientHandle,
    cache: &Arc<AccountCache>,
    channel: Uuid,
    reason: HistoryClosedReason,
    generation: Generation,
) -> Result<(), ClientError> {
    {
        let mut channels = session.history.channels.lock().await;
        let mut admissions = session.history.admission.write().await;
        admissions.deny(channel, reason);
        channels.retain(|_, channel| !channel.is_closed());
        session
            .reading
            .positions
            .revoke(&[channel], generation, reason);
        crate::badges::restrict(session, &[], std::iter::once(channel));
    }
    let listed = reason != HistoryClosedReason::ChannelDeleted;
    revoke(
        session,
        client,
        cache,
        &[(channel, reason)],
        |_| listed,
        generation,
    )
    .await
}

/// Make denials durable. The cache revoke marker comes first because it is what stops a
/// regrant from reusing the old set: every channel's marker goes into one transaction that
/// is submitted before anything is awaited and that no cancellation or generation change can
/// stop. If that write fails, the account cache is discarded, which revokes every set. Then
/// reading state is deleted (FULL, one commit per reason), and only after both steps is the
/// denial recorded as durable; a failure leaves it unconfirmed, so the next transition
/// retries both. Nothing here runs under the channel-map lock: the seal already excludes
/// every admission for these channels.
///
/// The durable boundary is the marker commit. Before it, no accepted resource write,
/// publication or transition completion that depends on the denial has happened, so a
/// process that dies in that window leaves disk exactly as if the denial was never seen.
async fn revoke(
    session: &SessionInner,
    client: &ClientHandle,
    cache: &Arc<AccountCache>,
    denied: &[(Uuid, HistoryClosedReason)],
    listed: impl Fn(Uuid) -> bool,
    generation: Generation,
) -> Result<(), ClientError> {
    if denied.is_empty() {
        return Ok(());
    }
    let markers = denied.iter().map(|(id, _)| *id.as_bytes()).collect();
    let written = cache
        .history_revoke_detached(markers)
        .await
        .unwrap_or(Err(CacheError::TaskStopped));
    if let Err(error) = written {
        if matches!(
            error,
            CacheError::Storage(_) | CacheError::InvalidData | CacheError::TaskStopped
        ) {
            // A cache that cannot record the marker is discarded instead: no old set survives.
            client.inner.caches.invalidate(session.account).await?;
        }
        return Err(error.into());
    }
    let mut by_reason: Vec<(HistoryClosedReason, Vec<Uuid>)> = Vec::new();
    for (id, reason) in denied {
        if let Some((_, ids)) = by_reason.iter_mut().find(|(known, _)| known == reason) {
            ids.push(*id);
        } else {
            by_reason.push((*reason, vec![*id]));
        }
    }
    for (reason, channels) in by_reason {
        crate::reading::catch_up_sync::revoke_channels(
            client, session, &channels, generation, reason,
        )
        .await?;
        let mut admissions = session.history.admission.write().await;
        for channel in channels {
            admissions.revoked(channel, listed(channel));
        }
    }
    Ok(())
}

/// Whether this generation already admitted a fetched snapshot. Until then the published
/// resources are the cache-loaded image, which is display data and never access evidence.
pub(crate) async fn accepted(session: &SessionInner, generation: Generation) -> bool {
    session
        .history
        .connection
        .read()
        .await
        .as_ref()
        .is_some_and(|connection| connection.generation == generation)
}

#[cfg(test)]
mod badge_tests;
#[cfg(test)]
mod freshness_tests;
#[cfg(test)]
mod sequence_tests;
#[cfg(test)]
mod tests;
