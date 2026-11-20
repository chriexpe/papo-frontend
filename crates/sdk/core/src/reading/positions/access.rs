//! Channel fences for reading and its FULL writes. Position batching lives in the parent.
use super::{ChannelFence, Generation, HistoryClosedReason, PositionCoordinator, State, Uuid};

impl State {
    pub(super) fn fence(&self, channel: Uuid, generation: Generation) -> Option<ChannelFence> {
        if let Some(entry) = self.revoked.get(&channel)
            && entry.generation == generation
        {
            return Some(ChannelFence::Closed(entry.reason));
        }
        self.quarantined
            .contains(&channel)
            .then_some(ChannelFence::Unknown)
    }
}

impl PositionCoordinator {
    /// Captured under R1 for a server mark. A later denial increments the epoch
    /// before queuing its durable delete, even if a subsequent grant clears the fence.
    pub(crate) fn revocation_epoch(&self, channel: Uuid) -> u64 {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .revocation_epochs
            .get(&channel)
            .map_or(0, |entry| entry.epoch)
    }

    pub(in crate::reading) fn restrict_reading(
        &self,
        image: &mut crate::ReadingSnapshot,
        last_good: &crate::ReadingSnapshot,
        admitted: impl Fn(Uuid) -> bool,
    ) {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (id, revocation) in &state.revoked {
            if revocation.generation == image.generation || !admitted(*id) {
                image.channels.remove(id);
            }
        }
        for id in &state.quarantined {
            if let Some(candidate) = image.channels.get_mut(id) {
                if let Some(previous) = last_good.channels.get(id) {
                    candidate.clone_from(previous);
                } else {
                    image.channels.remove(id);
                }
            }
        }
    }

    pub fn check_writable(
        &self,
        channel: Uuid,
        generation: Generation,
        expected_epoch: u64,
    ) -> Option<ChannelFence> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.closed {
            return Some(ChannelFence::Closed(HistoryClosedReason::AccountClosed));
        }
        let current = state.revocation_epochs.get(&channel).copied();
        let current_epoch = current.map_or(0, |entry| entry.epoch);
        if current_epoch != expected_epoch {
            return Some(ChannelFence::Closed(
                state
                    .revoked
                    .get(&channel)
                    .map(|entry| entry.reason)
                    .or_else(|| current.map(|entry| entry.reason))
                    .unwrap_or(HistoryClosedReason::ChannelRemoved),
            ));
        }
        state.fence(channel, generation)
    }

    pub fn check_current_channel(
        &self,
        channel: Uuid,
        generation: Generation,
    ) -> Option<ChannelFence> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.closed {
            return Some(ChannelFence::Closed(HistoryClosedReason::AccountClosed));
        }
        state.fence(channel, generation)
    }
}
