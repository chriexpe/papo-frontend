//! Pin acknowledgments and exact grouped reactor pagination; durable human edits/reactions stay in outgoing.

use super::{OperationError, OperationReceipt, Operations, Refresh};
use crate::{AccountId, Generated, Generation, HistoryOrder, Uuid};
use papo_api::Cursor;
pub use papo_api::wire::{
    MessageReactionGroup, MessageReactionSummary, MessageReactionUser, MessageUserReaction,
    PinnedMessage,
};

/// Exact continuation derived from all raw grouped reaction users, not the last displayed group.
#[derive(Clone)]
pub struct ReactorCursor {
    account: AccountId,
    generation: Generation,
    channel: Uuid,
    message: Uuid,
    order: HistoryOrder,
    created_at: String,
    id: Uuid,
}
/// One backend-authoritative grouped page, with counts scoped to that page.
#[derive(Clone)]
pub struct ReactorPage {
    /// Groups retain backend identity and user membership; counts are not whole-message totals.
    pub groups: Vec<MessageReactionGroup>,
    /// Exact tuple boundary in the page's order, absent only when the backend reports exhaustion.
    pub next: Option<ReactorCursor>,
}
impl ReactorPage {
    /// A page for isolated projection tests: `more` stands in for a continuation that no
    /// request can use.
    #[cfg(feature = "test-support")]
    #[must_use]
    pub fn for_test(groups: Vec<MessageReactionGroup>, more: bool) -> Self {
        Self {
            groups,
            next: more.then(|| ReactorCursor {
                account: AccountId::new(crate::ServerId::for_test(), Uuid::nil()),
                generation: Generation::for_test(0),
                channel: Uuid::nil(),
                message: Uuid::nil(),
                order: HistoryOrder::Asc,
                created_at: String::new(),
                id: Uuid::nil(),
            }),
        }
    }
}

impl Operations {
    /// Pins a known backend message. Backend pin and channel gates remain authoritative.
    /// # Errors
    /// Returns definite denial or unknown dispatch; post-ack pin fetch failure is only a warning.
    pub async fn pin_message(
        &self,
        channel: Uuid,
        message: Uuid,
    ) -> Result<OperationReceipt<PinnedMessage>, OperationError> {
        self.mutate(
            move |rest| async move { rest.pin_message(channel, message).await },
            Refresh::Pins(channel),
        )
        .await
    }
    /// Unpins a known message, without marking it read or moving any timeline window.
    /// # Errors
    /// Returns definite denial or unknown dispatch; post-ack pin fetch failure is only a warning.
    pub async fn unpin_message(
        &self,
        channel: Uuid,
        message: Uuid,
    ) -> Result<OperationReceipt<()>, OperationError> {
        self.mutate(
            move |rest| async move { rest.unpin_message(channel, message).await },
            Refresh::Pins(channel),
        )
        .await
    }
    /// Lists reaction users in backend `(created_at, id)` tuple `order` (ascending: the order
    /// people reacted), all emoji together, at most 100 reactions a page, grouped only after
    /// raw pagination. A continuation keeps the order its first page was asked in.
    /// Human reaction mutations use `OutgoingService::enqueue(OperationKind::Reaction)`.
    /// # Errors
    /// Rejects foreign/stale cursor identities (or another order), malformed boundaries or
    /// typed backend failure.
    pub async fn reaction_users(
        &self,
        channel: Uuid,
        message: Uuid,
        order: HistoryOrder,
        cursor: Option<ReactorCursor>,
    ) -> Result<Generated<ReactorPage>, OperationError> {
        let account = self.session.account();
        let generation = self.session.generation();
        if cursor.as_ref().is_some_and(|cursor| {
            cursor.account != account
                || cursor.generation != generation
                || cursor.channel != channel
                || cursor.message != message
                || cursor.order != order
        }) {
            return Err(OperationError::InvalidInput);
        }
        let boundary = cursor.clone();
        let result = self
            .read(move |rest| async move {
                // The order goes as `message_reactions`' own parameter, first page included.
                let wire = boundary.as_ref().map(|cursor| Cursor {
                    since: &cursor.created_at,
                    last_id: None,
                    order: None,
                });
                let id = boundary.as_ref().map(|cursor| cursor.id.to_string());
                let wire = wire.map(|mut cursor| {
                    cursor.last_id = id.as_deref();
                    cursor
                });
                rest.message_reactions(channel, message, wire.as_ref(), order)
                    .await
            })
            .await?;
        if result.generation != generation {
            return Err(crate::ClientError::Superseded.into());
        }
        if result
            .value
            .message_id
            .as_deref()
            .and_then(|id| id.parse().ok())
            != Some(message)
        {
            return Err(OperationError::InvalidData);
        }
        let groups = result.value.reactions.ok_or(OperationError::InvalidData)?;
        let more = result.value.has_more.ok_or(OperationError::InvalidData)?;
        let last = last_boundary(&groups, order, cursor.as_ref())?;
        let next = if more {
            let (created_at, id) = last.ok_or(OperationError::InvalidData)?;
            Some(ReactorCursor {
                account,
                generation,
                channel,
                message,
                order,
                created_at,
                id,
            })
        } else {
            None
        };
        self.session.inner.epochs.validate(generation)?;
        Ok(Generated {
            generation,
            value: ReactorPage { groups, next },
        })
    }
}

/// The page's last reaction in `order`: every reaction must lie strictly past `cursor`.
fn last_boundary(
    groups: &[MessageReactionGroup],
    order: HistoryOrder,
    cursor: Option<&ReactorCursor>,
) -> Result<Option<(String, Uuid)>, OperationError> {
    let boundary = cursor
        .map(|cursor| tuple(&cursor.created_at, cursor.id))
        .transpose()?;
    let mut ids = std::collections::HashSet::new();
    let mut last: Option<(String, Uuid)> = None;
    for group in groups {
        for user in group.users.as_deref().ok_or(OperationError::InvalidData)? {
            let id = user
                .id
                .as_deref()
                .and_then(|id| id.parse().ok())
                .ok_or(OperationError::InvalidData)?;
            let time = user
                .created_at
                .as_ref()
                .ok_or(OperationError::InvalidData)?;
            let point = tuple(time, id)?;
            // Past `a` in `order`: after it ascending, before it descending.
            let past = |a: &_| match order {
                HistoryOrder::Asc => point > *a,
                HistoryOrder::Desc => point < *a,
            };
            if !ids.insert(id) || ids.len() > 100 || boundary.as_ref().is_some_and(|b| !past(b)) {
                return Err(OperationError::InvalidData);
            }
            if last
                .as_ref()
                .map(|(time, id)| tuple(time, *id))
                .transpose()?
                .is_none_or(|current| past(&current))
            {
                last = Some((time.clone(), id));
            }
        }
    }
    Ok(last)
}
fn tuple(
    time: &str,
    id: Uuid,
) -> Result<(chrono::DateTime<chrono::FixedOffset>, Uuid), OperationError> {
    Ok((
        chrono::DateTime::parse_from_rfc3339(time).map_err(|_| OperationError::InvalidData)?,
        id,
    ))
}
#[cfg(test)]
mod tests {
    //! Interleaved grouped users must produce the final raw boundary, independent of group order.
    use super::*;
    #[test]
    fn interleaved_groups_keep_raw_cursor() {
        let user = |index| papo_api::wire::MessageReactionUser {
            id: Some(Uuid::from_u128(index).to_string()),
            created_at: Some("2030-01-01T00:00:00.123456+03:00".into()),
            ..Default::default()
        };
        let groups = vec![
            MessageReactionGroup {
                users: Some(vec![user(5), user(1)]),
                ..Default::default()
            },
            MessageReactionGroup {
                users: Some(vec![user(4), user(2)]),
                ..Default::default()
            },
        ];
        let (time, id) = last_boundary(&groups, HistoryOrder::Desc, None)
            .unwrap()
            .unwrap();
        assert_eq!(id, Uuid::from_u128(1));
        assert_eq!(time, "2030-01-01T00:00:00.123456+03:00");
    }
}
