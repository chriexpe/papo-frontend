//! Seals uncertain access without changing durable revocations, using R1 decisions and fences.

use super::{super::channel::Channel, Admissions};
use crate::{
    ClientError, ResourceSnapshot, Uuid,
    permissions::read_access::{ReadAccess, ReadDecisions},
    session::SessionInner,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

/// Rejection suspends only the channels missing evidence. Publication under R1's lock
/// prevents a recount from restoring an obsolete exact census between seal and publish.
pub(crate) async fn quarantine_rejected(
    session: &SessionInner,
    snapshot: ResourceSnapshot,
    changed: bool,
) -> Result<(), ClientError> {
    let ids = ReadDecisions::evaluate(&snapshot, session.account.user()).with(ReadAccess::Unknown);
    let channels = session.history.channels.lock().await;
    let mut admissions = session.history.admission.write().await;
    session.epochs.apply(snapshot.generation, || {
        quarantine(session, &mut admissions, &channels, &ids);
        if !snapshot.channel_census_complete {
            crate::badges::invalidate_census(session);
        }
        if changed {
            session.publish_current_resources(snapshot)?;
        }
        Ok(())
    })?
}

/// The failed writer still owns `resource_serial`. Even if its epoch ended during the
/// switch, no newer resource grant can have been installed; retain stronger backend denials.
pub(crate) async fn quarantine_failed_install(session: &SessionInner) -> Result<(), ClientError> {
    let channels = session.history.channels.lock().await;
    let mut admissions = session.history.admission.write().await;
    let allowed = admissions.allowed();
    quarantine(session, &mut admissions, &channels, &allowed);
    crate::badges::invalidate_census(session);

    // A recount may already hold the last complete image, even when it contains no
    // channels to quarantine. Wake it under the same admission fence with the lost proof.
    let previous = session.resources.borrow().clone();
    if previous.channel_census_complete {
        let mut snapshot = (*previous).clone();
        snapshot.channel_census_complete = false;
        session.publish_resources(snapshot)?;
    }
    Ok(())
}

fn quarantine(
    session: &SessionInner,
    admissions: &mut Admissions,
    channels: &BTreeMap<Uuid, Arc<Channel>>,
    ids: &BTreeSet<Uuid>,
) {
    let mut changed = Vec::new();
    for &id in ids {
        if admissions.quarantine(id) {
            if let Some(channel) = channels.get(&id) {
                channel.quarantine();
            }
            changed.push(id);
        }
    }
    session.reading.positions.quarantine(&changed);
    crate::badges::restrict(session, &changed, std::iter::empty());
}
