//! Serializes admitted observations, R1 transitions and installation of one resource image.
//! Fetching and acknowledgement ordering belong to the individual producers.

mod evidence;
mod interim;
pub(crate) use evidence::{EvidenceGaps, ScopedFailure, unresolved_overwrites};
pub(crate) use interim::Observation;

use crate::{
    CacheResource, ClientError, Generation, ResourceSnapshot,
    history::access::{self, Evidence, Refetch, Transition},
    reconcile::ResourceConnection,
    resources::AdmittedMutation,
    session::SessionInner,
};
use std::sync::{Arc, atomic::Ordering};
use tokio::sync::OwnedMutexGuard;

/// The public version can stay constant across equal observations. Capture both fences
/// before REST, along with R1's independent backend denial clock.
#[derive(Clone)]
pub(crate) struct FetchStamp {
    generation: Generation,
    observation: u64,
    pub(crate) base: Arc<ResourceSnapshot>,
    evidence: Evidence,
}

impl FetchStamp {
    pub(crate) async fn capture(
        session: &SessionInner,
        generation: Generation,
        scope: Refetch,
    ) -> Result<Self, ClientError> {
        session.epochs.validate(generation)?;
        let observation = session.resource_observation.load(Ordering::Acquire);
        let base = session.resources.borrow().clone();
        if base.generation != generation {
            return Err(ClientError::Superseded);
        }
        Ok(Self {
            generation,
            observation,
            base,
            evidence: access::evidence(session, scope).await,
        })
    }

    fn current(&self, session: &SessionInner) -> bool {
        self.observation == session.resource_observation.load(Ordering::Acquire)
            && self.base.version == session.resources.borrow().version
    }

    fn advance(&self, session: &SessionInner) -> Result<(), ClientError> {
        let next = self.observation.checked_add(1).ok_or(ClientError::Closed)?;
        session.resource_observation.store(next, Ordering::Release);
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Outcome {
    Accepted,
    ConcurrentChange,
}

impl Outcome {
    pub(crate) fn require_current(self) -> Result<(), ClientError> {
        match self {
            Self::Accepted => Ok(()),
            Self::ConcurrentChange => Err(ClientError::Superseded),
        }
    }
}

pub(crate) type FetchResult = Result<AdmittedMutation, ScopedFailure>;

pub(crate) type Reservation = Arc<OwnedMutexGuard<()>>;

pub(crate) async fn reserve(session: &SessionInner) -> Reservation {
    Arc::new(Arc::clone(&session.resource_serial).lock_owned().await)
}

/// Once a cache write is submitted it can outlive its caller. Transfer its serial guard
/// to a session-owned task so cancellation cannot release the gate between disk and memory.
pub(crate) async fn commit(
    session: &Arc<SessionInner>,
    connection: &ResourceConnection,
    guard: Reservation,
    stamp: FetchStamp,
    fetched: FetchResult,
) -> Result<Outcome, ClientError> {
    let inner = Arc::clone(session);
    let connection = connection.clone();
    session
        .tasks
        .spawn(async move { apply(&inner, &connection, guard, stamp, fetched).await })
        .await
        .map_err(|_| ClientError::Executor)?
}

async fn apply(
    session: &SessionInner,
    connection: &ResourceConnection,
    _guard: Reservation,
    stamp: FetchStamp,
    fetched: FetchResult,
) -> Result<Outcome, ClientError> {
    session.epochs.validate(stamp.generation)?;
    if !stamp.current(session) {
        return Ok(Outcome::ConcurrentChange);
    }
    let mutation = match fetched {
        Ok(value) => value,
        Err(error) => return reject(session, &stamp, error).await,
    };
    let complete = mutation.complete;
    let admitted = access::accepted(session, stamp.generation).await;
    let affects_authority = mutation.server.is_some()
        || mutation.channels.is_some()
        || mutation.roles.is_some()
        || mutation.roster.is_some()
        || !mutation.summaries.is_empty()
        || mutation.dms.is_some()
        || mutation.blocks.is_some();
    if !complete
        && (session.resource_recovery.borrow().is_some() || (affects_authority && !admitted))
    {
        return Ok(Outcome::ConcurrentChange);
    }
    let mut gaps = stamp.base.gaps.clone();
    gaps.recover(&mutation, session.account.user());
    gaps.observe_missing_peers(&stamp.base, &mutation);
    for id in unresolved_overwrites(&stamp.base, &mutation) {
        gaps.await_channel(id);
    }
    let mut snapshot = match mutation.apply(&stamp.base, session.account.user()) {
        Ok(snapshot) => snapshot,
        Err(error) => return reject(session, &stamp, error.into()).await,
    };
    snapshot.gaps = gaps;
    snapshot.channel_census_complete = (admitted || complete) && snapshot.gaps.census_known();
    let changed = snapshot.changed_groups(&stamp.base);
    let visibility_changed = snapshot.channel_census_complete != stamp.base.channel_census_complete
        || snapshot.gaps != stamp.base.gaps;

    // Reserve the observation before the first fallible effect. Even a failed cache write
    // can leave a seal; an older success must not override that seal.
    stamp.advance(session)?;
    let revalidate = complete
        || (admitted
            && (access::affects_read_access(&changed)
                || visibility_changed
                || session.history.admission.read().await.needs_revalidation()));
    // A failed install after a seal or a revision switch leaves no base for another
    // partial update. The next complete reconcile reloads the committed revision.
    let result = async {
        let transition = if revalidate {
            Some(Transition::begin(session, connection, &snapshot, stamp.evidence).await?)
        } else {
            None
        };

        let groups = if complete {
            &crate::account_cache::ALL_RESOURCES[..]
        } else {
            &changed
        };
        let needs_new_writer = persist(connection, &snapshot, groups).await?;
        install(
            session,
            connection,
            snapshot,
            transition,
            complete,
            visibility_changed,
            &changed,
        )
        .await?;
        Ok(needs_new_writer)
    }
    .await;
    match result {
        Ok(needs_new_writer) => {
            if needs_new_writer {
                // Cleanup can fail after a valid switch and a successful in-memory install.
                // The next connection needs a fresh writer before accepting partial updates.
                session
                    .resource_recovery
                    .send_replace(Some(stamp.generation));
            } else if complete {
                session.resource_recovery.send_replace(None);
            }
            Ok(Outcome::Accepted)
        }
        Err(error) => {
            let quarantine = access::quarantine_failed_install(session).await;
            session
                .resource_recovery
                .send_replace(Some(stamp.generation));
            quarantine?;
            Err(error)
        }
    }
}

/// Report a cleanup fault after a successful switch and return whether the writer needs renewal.
async fn persist(
    connection: &ResourceConnection,
    snapshot: &ResourceSnapshot,
    groups: &[CacheResource],
) -> Result<bool, ClientError> {
    if groups.is_empty() {
        return Ok(false);
    }
    if connection
        .cache
        .update_resources(snapshot, groups)
        .await?
        .is_some()
    {
        connection
            .client
            .inner
            .disk_status
            .send_replace(crate::DiskStatus::Fault(
                crate::DiskFault::ResourceMaintenance,
            ));
        return Ok(connection.cache.writer_failed());
    }
    Ok(false)
}

async fn install(
    session: &SessionInner,
    connection: &ResourceConnection,
    snapshot: ResourceSnapshot,
    transition: Option<Transition>,
    complete: bool,
    visibility_changed: bool,
    changed: &[CacheResource],
) -> Result<(), ClientError> {
    let admission_changed = match transition {
        Some(transition) => transition.finish(session, connection).await?,
        None => false,
    };
    if complete
        || admission_changed
        || changed
            .iter()
            .any(|group| matches!(group, CacheResource::Channels | CacheResource::Dms))
    {
        crate::reading::reconcile::resources(session, connection, &snapshot).await?;
    }
    if complete {
        crate::history::reconnect::refresh(session, connection, snapshot.generation).await?;
    }
    if visibility_changed || !changed.is_empty() {
        session.publish_resources(snapshot)?;
    }
    Ok(())
}

async fn reject(
    session: &SessionInner,
    stamp: &FetchStamp,
    failure: ScopedFailure,
) -> Result<Outcome, ClientError> {
    if failure.invalidates() {
        let mut snapshot = (*stamp.base).clone();
        let changed = snapshot.gaps.invalidate(&failure);
        snapshot.channel_census_complete &= snapshot.gaps.census_known();
        stamp.advance(session)?;
        access::quarantine_rejected(session, snapshot, changed).await?;
    }
    Err(failure.error)
}

#[cfg(test)]
mod tests;
