//! Counts held unread triggers using history's range proof and reading's batched evidence.
//! No hydration, role inference, delivery decision or retained per-message annotation lives here.

use crate::{
    HistorySnapshot, MessageKey, UnreadCount, Uuid, authored_by_other,
    notifications::policy::{self, ChannelPolicy},
    reading::evidence::{ChannelReadEvidence, ReadEvidence},
};
use std::collections::HashMap;

pub(super) fn mentions(
    snapshot: &HistorySnapshot,
    read: ChannelReadEvidence,
    policy: ChannelPolicy,
    user: Uuid,
    dm: bool,
) -> Result<UnreadCount, crate::InvalidMessageKey> {
    if policy.badges_disabled() {
        return Ok(UnreadCount::Exact(0));
    }
    if policy.badges_uncertain() || !read.boundary_known() {
        return Ok(UnreadCount::AtLeast(0));
    }
    let coverage = snapshot.coverage()?;
    let point = read
        .point()
        .map(|point| coverage.resolve(point).map(|resolved| resolved.key))
        .transpose()?;
    let range = coverage.range(point, None)?;
    let cached_head = snapshot
        .messages
        .last()
        .map(|message| MessageKey::from_message(message))
        .transpose()?;
    let mut exact = range.complete
        && read
            .head()
            .is_none_or(|head| Some(head) <= cached_head.max(point));
    let read = read.with_point(point);
    let mut parents = HashMap::new();
    // Index only requested parents for this recount, not another message/history store.
    for message in snapshot
        .messages
        .iter()
        .skip(range.messages.start)
        .take(range.messages.len())
    {
        if let Some(parent) = message
            .reply_to
            .as_deref()
            .and_then(|id| id.parse::<Uuid>().ok())
        {
            parents.insert(parent, None);
        }
    }
    if !parents.is_empty() {
        for message in &snapshot.messages {
            if let Some(author) = message
                .id
                .as_deref()
                .and_then(|id| id.parse::<Uuid>().ok())
                .and_then(|id| parents.get_mut(&id))
            {
                *author = message.author_id.as_deref().and_then(|id| id.parse().ok());
            }
        }
    }
    let mut count = 0;
    for message in snapshot
        .messages
        .iter()
        .skip(range.messages.start)
        .take(range.messages.len())
    {
        if !authored_by_other(message, user) {
            continue;
        }
        let key = MessageKey::from_message(message)?;
        match read.badge_message(key.message_id, Some(key)) {
            ReadEvidence::Read => continue,
            ReadEvidence::NotReady => {
                exact = false;
                continue;
            }
            ReadEvidence::Unread => {}
        }
        let parent_author = message
            .reply_to
            .as_deref()
            .and_then(|id| id.parse::<Uuid>().ok())
            .and_then(|id| parents.get(&id))
            .copied()
            .flatten();
        match policy.badge_eligible(policy::classify(message, user, dm, parent_author)) {
            Some(true) => count += 1,
            Some(false) => {}
            None => exact = false,
        }
    }
    Ok(if exact {
        UnreadCount::Exact(count)
    } else {
        UnreadCount::AtLeast(count)
    })
}

pub(super) const fn has_unread(count: UnreadCount) -> Option<bool> {
    match count {
        UnreadCount::Exact(0) => Some(false),
        UnreadCount::AtLeast(0) => None,
        UnreadCount::Exact(_) | UnreadCount::AtLeast(_) => Some(true),
    }
}
