//! Account-scoped in-memory catch-up state and revision snapshot publishing.
//! Async reporting, clear operations and SQLite commits live in `reading::catch_up_ops` and `catch_up_store`.

use std::{collections::BTreeMap, sync::Arc};

use papo_api::Uuid;

use crate::{
    CatchUpSnapshot, ClientError, Generation, catch_up::CatchUpState, session::SessionInner,
};

impl super::Reading {
    fn publish_catch_up_revisions(
        &self,
        session: &SessionInner,
        generation: Generation,
        channels: BTreeMap<Uuid, u64>,
    ) -> Result<(), ClientError> {
        session.epochs.apply(generation, || {
            let version = self
                .catch_up_image
                .borrow()
                .version
                .checked_add(1)
                .ok_or(ClientError::Closed)?;
            self.catch_up_image.send_replace(Arc::new(CatchUpSnapshot {
                generation,
                version,
                channels,
            }));
            Ok(())
        })?
    }

    pub(crate) fn hydrate_catch_up(
        &self,
        session: &SessionInner,
        generation: Generation,
        loaded: BTreeMap<Uuid, CatchUpState>,
    ) -> Result<(), ClientError> {
        let mut guard = self
            .catch_up_states
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let revisions = guard.hydrate(loaded);
        if revisions.is_empty() {
            return Ok(());
        }
        self.publish_catch_up_revisions(session, generation, revisions)
    }

    pub(crate) fn catch_up_state_for(&self, channel: Uuid) -> Option<CatchUpState> {
        self.catch_up_states
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(channel)
    }

    pub(crate) fn active_catch_up_intervals(&self, channels: &[Uuid]) -> Vec<(Uuid, CatchUpState)> {
        self.catch_up_states
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .interval_channels(channels)
    }

    pub(crate) fn apply_catch_up_updates(
        &self,
        session: &SessionInner,
        generation: Generation,
        updates: Vec<(Uuid, CatchUpState)>,
        force_publish: bool,
    ) -> Result<(), ClientError> {
        let mut guard = self
            .catch_up_states
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let filtered: Vec<_> = updates
            .into_iter()
            .filter(|(ch, _)| {
                self.positions
                    .check_current_channel(*ch, generation)
                    .is_none()
            })
            .collect();
        if filtered.is_empty() && !force_publish {
            return Ok(());
        }
        let revisions = guard.apply(filtered);
        self.publish_catch_up_revisions(session, generation, revisions)
    }

    pub(crate) fn remove_catch_up_channels(
        &self,
        session: &SessionInner,
        generation: Generation,
        channels: &[Uuid],
    ) {
        if channels.is_empty() {
            return;
        }
        {
            let mut keys = self
                .reconcile_keys
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            for channel in channels {
                keys.remove(channel);
            }
        }
        let mut guard = self
            .catch_up_states
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (changed, revisions) = guard.remove_channels(channels);
        if changed {
            let _ = self.publish_catch_up_revisions(session, generation, revisions);
        }
    }
}
