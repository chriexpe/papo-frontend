//! Trigger rules for one candidate row: what direct, reply and `@everyone` evidence means for the
//! account under the current resources. Caching and ranges stay with the classifier.

use super::{Classification, Facts, Parent, ParentEvidence};
use crate::{ResourceSnapshot, Uuid};
use papo_api::wire::Message;

/// Classifies a row authored by someone else, outside a direct conversation, that carries a
/// mention marker or a reply.
pub(super) fn classify_candidate(
    message: &Message,
    facts: &Facts,
    parents: &ParentEvidence,
    user: Uuid,
    resources: &ResourceSnapshot,
) -> Classification {
    if facts.direct {
        return Classification::TRIGGER;
    }
    let mut result = Classification::NONE;
    if let Some(reply) = message.reply_to.as_deref().and_then(|id| id.parse().ok()) {
        match parents.get(&reply) {
            Some(Parent::Author(author)) if author == user => {
                return Classification::TRIGGER;
            }
            Some(Parent::Author(_)) => {}
            Some(Parent::UnknownAuthor | Parent::Absent) | None => {
                result.complete = false;
            }
        }
    }
    if facts.everyone
        && message.webhook_id.is_none()
        && let Some(author) = message.author_id.as_deref()
    {
        let owner = resources
            .server
            .as_ref()
            .and_then(|server| server.owner_id.as_deref());
        if owner == Some(author) {
            return Classification::TRIGGER;
        }
        if let Some(member) = resources
            .members
            .iter()
            .find(|member| member.id.as_deref() == Some(author))
        {
            let permitted = member.roles.iter().flatten().any(|summary| {
                resources.roles.iter().any(|role| {
                    role.id.is_some()
                        && role.id == summary.id
                        && role
                            .permissions
                            .as_ref()
                            .is_some_and(|permissions| permissions.everyone_message == Some(true))
                })
            });
            if permitted {
                return Classification::TRIGGER;
            }
            result.complete &= author.parse().is_ok_and(|author| {
                crate::permissions::global_permissions(resources, author).is_ok()
            });
        } else {
            result.complete = false;
        }
    }
    result
}
