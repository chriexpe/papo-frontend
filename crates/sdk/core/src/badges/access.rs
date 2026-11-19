//! Immediate badge redaction at history's existing admission write boundary. This consumes
//! decisions, never evaluates permissions; the recount worker restores eligible current values.

use super::{ChannelBadges, recount};
use crate::{Uuid, session::SessionInner};
use std::sync::Arc;

/// Caller holds history's admission write lock, serializing this redaction with recount publication.
pub(crate) fn restrict(
    session: &SessionInner,
    quarantined: &[Uuid],
    denied: impl Iterator<Item = Uuid>,
) {
    let mut denied = denied.peekable();
    if quarantined.is_empty() && denied.peek().is_none() {
        return;
    }
    let old = session.badges.image.borrow().clone();
    let mut channels = old.channels.clone();
    for id in quarantined {
        if let Some(value) = channels.get_mut(id) {
            let unknown = ChannelBadges::unknown(value.direct_message);
            if value.as_ref() != &unknown {
                *value = Arc::new(unknown);
            }
        }
    }
    for id in denied {
        channels.remove(&id);
    }
    if channels != old.channels {
        recount::publish(session, old.generation, old.channels_complete, channels);
    }
}
