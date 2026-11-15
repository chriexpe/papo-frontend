//! The one rule for whether a message was written by someone other than the account.

use papo_api::{Uuid, wire::Message};

/// True unless `message` names `account_user` as its author. A missing or malformed author
/// (webhooks, system rows) counts as someone else, so it is never silently treated as read.
#[must_use]
pub fn authored_by_other(message: &Message, account_user: Uuid) -> bool {
    message
        .author_id
        .as_deref()
        .and_then(|id| Uuid::parse_str(id).ok())
        != Some(account_user)
}
