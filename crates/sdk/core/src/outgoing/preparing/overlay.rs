//! The queue's view of preparing messages: how they are shown, and the ✕, Retry and Discard
//! that apply to them before their files are ready. The rows are `preparing`'s.

use super::Preparing;
use crate::outgoing::{AccountQueue, Operation, OperationFailure, OperationState, Uuid};
use std::sync::Arc;

impl AccountQueue {
    pub(in crate::outgoing) fn preparing_map(
        &self,
    ) -> std::sync::MutexGuard<'_, std::collections::BTreeMap<Uuid, Preparing>> {
        self.preparing
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Shows each preparing message as this process knows it, over the row's crash-time form.
    pub(in crate::outgoing) fn present_preparing(&self, operations: &mut [Arc<Operation>]) {
        let map = self.preparing_map();
        if map.is_empty() {
            return;
        }
        for operation in operations.iter_mut() {
            let Some(entry) = map.get(&operation.id) else {
                continue;
            };
            if operation.state != OperationState::Failed
                || operation.failure != Some(OperationFailure::AttachmentLost)
            {
                continue;
            }
            let mut shown = (**operation).clone();
            if entry.cancelled {
                shown.failure = Some(OperationFailure::Cancelled);
            } else {
                shown.state = OperationState::Preparing;
                shown.failure = None;
            }
            *operation = Arc::new(shown);
        }
    }

    /// ✕ on a preparing message: Failed at once, the copy keeps going. True if it was preparing.
    pub(in crate::outgoing) fn cancel_preparing(&self, local: Uuid) -> bool {
        self.preparing_map()
            .get_mut(&local)
            .map(|entry| entry.cancelled = true)
            .is_some()
    }

    /// Retry on a preparing message ✕ cancelled: back to Preparing. True if it applied.
    pub(in crate::outgoing) fn retry_preparing(&self, local: Uuid) -> bool {
        match self.preparing_map().get_mut(&local) {
            Some(entry) if entry.cancelled => {
                entry.cancelled = false;
                true
            }
            _ => false,
        }
    }

    /// Discard: stop waiting for the copy; the row itself is discarded as any Failed one.
    pub(in crate::outgoing) fn abandon_preparing(&self, local: Uuid) {
        if let Some(entry) = self.preparing_map().remove(&local) {
            entry.abort.cancel();
        }
    }
}
