//! Cache-based ordered unread counts from history's coverage queries; an unknown range keeps
//! the count a lower bound, never an inferred exact total.

use super::{ReadState, ReadingError, UnreadCount};
use crate::notifications::classification::Tally;
use crate::{HistorySnapshot, MessageKey};

#[cfg(test)]
pub(super) fn update(
    state: &mut ReadState,
    snapshot: &HistorySnapshot,
    server_head: Option<MessageKey>,
) -> Result<(), ReadingError> {
    update_classified(state, snapshot, server_head, |_| Tally::EMPTY)
}

pub(super) async fn update_for(
    session: &crate::Session,
    channel: crate::Uuid,
    state: &mut ReadState,
    snapshot: &HistorySnapshot,
    server_head: Option<MessageKey>,
) -> Result<(), ReadingError> {
    Policy::load(session)
        .await?
        .update(session, channel, state, snapshot, server_head)
}

pub(super) struct Policy {
    settings: crate::NotificationSettings,
    resources: std::sync::Arc<crate::ResourceSnapshot>,
    backend: papo_api::wire::Notifications,
}
impl Policy {
    pub async fn load(session: &crate::Session) -> Result<Self, ReadingError> {
        let settings = crate::notifications::ledger::settings(session)
            .await
            .map_err(|_| crate::ClientError::ResourceData)?;
        let resources = std::sync::Arc::clone(&session.inner.resources.borrow());
        let backend = crate::notifications::backend::current(&session.inner, session.generation());
        Ok(Self {
            settings,
            resources,
            backend,
        })
    }

    pub fn with_resources(mut self, resources: &crate::ResourceSnapshot) -> Self {
        self.resources = std::sync::Arc::new(resources.clone());
        self
    }

    pub fn eligible(&self, channel: crate::Uuid) -> bool {
        crate::notifications::policy::allows(
            &self.resources,
            &self.settings,
            &self.backend,
            channel,
            true,
        )
    }

    pub fn update(
        &self,
        session: &crate::Session,
        channel: crate::Uuid,
        state: &mut ReadState,
        snapshot: &HistorySnapshot,
        server_head: Option<MessageKey>,
    ) -> Result<(), ReadingError> {
        if self.eligible(channel) {
            update_classified(state, snapshot, server_head, |range| {
                crate::notifications::classification::ClassificationCache::project(
                    &session.inner.notifications.classification,
                    channel,
                    snapshot,
                    session.account().user(),
                    &self.resources,
                    range,
                )
            })
        } else {
            crate::notifications::classification::ClassificationCache::evict(
                &session.inner.notifications.classification,
                channel,
            );
            update_classified(state, snapshot, server_head, |_| Tally::EMPTY)?;
            state.mention_count = UnreadCount::Exact(0);
            Ok(())
        }
    }
}

fn update_classified(
    state: &mut ReadState,
    snapshot: &HistorySnapshot,
    server_head: Option<MessageKey>,
    classify: impl FnOnce(std::ops::Range<usize>) -> Tally,
) -> Result<(), ReadingError> {
    let coverage = snapshot.coverage()?;
    let point = state
        .point()
        .map(|point| coverage.resolve(point).map(|resolved| resolved.key))
        .transpose()?;
    let unread = coverage.range(point, None)?;
    let mut count = u64::try_from(unread.messages.len()).map_err(|_| ReadingError::InvalidPoint)?;
    let first = snapshot
        .messages
        .get(unread.messages.start)
        .map(|message| MessageKey::from_message(message))
        .transpose()?
        .map(|point| point.message_id);
    let cached_head = snapshot
        .messages
        .last()
        .map(|message| MessageKey::from_message(message))
        .transpose()?;
    // Metadata names a real additional message, but never proves the unknown middle complete.
    if server_head.is_some_and(|head| Some(head) > point && Some(head) > cached_head) {
        count += 1;
    }
    let exact =
        unread.complete && server_head.is_none_or(|head| Some(head) <= cached_head.max(point));
    state.unread_count = if exact {
        UnreadCount::Exact(count)
    } else {
        UnreadCount::AtLeast(count)
    };
    // Called for an empty range too: the classifier releases what a fully read channel pinned.
    let Tally {
        triggered: mentions,
        complete: classified_complete,
    } = classify(unread.messages);
    state.mention_count = if exact && classified_complete {
        UnreadCount::Exact(mentions)
    } else {
        UnreadCount::AtLeast(mentions)
    };
    state.first_unread = first;
    Ok(())
}

pub(super) fn unresolved(state: &mut ReadState) {
    // An unknown remote mark may be newer than every cached row; no positive lower
    // bound or first-unread claim is justified until its ordering key is resolved.
    state.unread_count = UnreadCount::AtLeast(0);
    if state.mention_count != UnreadCount::Exact(0) {
        state.mention_count = UnreadCount::AtLeast(0);
    }
    state.first_unread = None;
}
