//! A partially fetched batch can invalidate a prior grant before its remaining REST calls.
//! Only missing proofs are published at this point. Effective access still comes from R1.

use super::{EvidenceGaps, FetchStamp, Reservation, ScopedFailure, reserve};
use crate::{
    CacheResource, ClientError, history::access, resources::AdmittedMutation, session::SessionInner,
};
use std::sync::{Arc, atomic::Ordering};

/// The pump holds the writer gate across its fetch. Optimistic operation refresh takes
/// it only when it has an observed loss to seal, then releases it before more network I/O.
pub(crate) struct Observation<'a> {
    session: &'a Arc<SessionInner>,
    pub(crate) stamp: &'a mut FetchStamp,
    held: Option<&'a Reservation>,
}

impl FetchStamp {
    pub(crate) fn observing<'a>(
        &'a mut self,
        session: &'a Arc<SessionInner>,
        held: Option<&'a Reservation>,
    ) -> Observation<'a> {
        Observation {
            session,
            stamp: self,
            held,
        }
    }
}

impl Observation<'_> {
    pub(crate) async fn before_rest(
        &mut self,
        group: CacheResource,
        facts: &AdmittedMutation,
    ) -> Result<(), ScopedFailure> {
        let base = &self.stamp.base;
        let own = self.session.account.user();
        let mut gaps = base.gaps.clone();
        gaps.observe(group, base, facts, own);
        if gaps == base.gaps {
            return Ok(());
        }
        let inner = Arc::clone(self.session);
        let reservation = self.held.cloned();
        let stamp = self.stamp.clone();
        // Cancellation after observing a possible loss must not discard its seal. The
        // task rechecks the original stamp, even when it must wait for another writer.
        *self.stamp = self
            .session
            .tasks
            .spawn(async move {
                let _guard = match reservation {
                    Some(guard) => guard,
                    None => reserve(&inner).await,
                };
                install_gaps(&inner, stamp, gaps).await
            })
            .await
            .map_err(|_| ClientError::Executor)??;
        Ok(())
    }
}

async fn install_gaps(
    session: &SessionInner,
    mut stamp: FetchStamp,
    gaps: EvidenceGaps,
) -> Result<FetchStamp, ClientError> {
    session.epochs.validate(stamp.generation)?;
    if !stamp.current(session) {
        return Err(ClientError::Superseded);
    }
    let mut snapshot = (*stamp.base).clone();
    snapshot.channel_census_complete &= gaps.census_known();
    snapshot.gaps = gaps;
    stamp.advance(session)?;
    access::quarantine_rejected(session, snapshot, true).await?;
    stamp.observation = session.resource_observation.load(Ordering::Acquire);
    stamp.base = session.resources.borrow().clone();
    Ok(stamp)
}
