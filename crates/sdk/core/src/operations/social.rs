//! Owns direct-conversation and blocking mutations; history and permission proofs remain separate.

pub use papo_api::{
    Uuid,
    wire::{
        BlockListResponse, DirectConversation, DirectConversationList, OpenDirectMessageRequest,
    },
};

use super::{OperationError, OperationReceipt, Operations, Refresh};
use crate::{ClientError, Generated};

/// What this account may do in a direct conversation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DirectAccess {
    /// Open to messages.
    Open,
    /// This account blocks the other person; unblocking opens it again.
    YouBlocked,
    /// The backend refuses it: the other person blocked this account, or the server forbids
    /// it. The backend does not say which, and neither do clients.
    Refused,
    /// This account is not in the conversation, or it no longer exists.
    Gone,
}

impl Operations {
    /// Reads the authenticated account's visible DM list, without granting access to its history.
    /// # Errors
    /// Returns backend rejection or a stale/revoked account generation.
    pub async fn direct_conversations(
        &self,
    ) -> Result<Generated<DirectConversationList>, OperationError> {
        self.read(|rest| async move { rest.direct_conversations().await })
            .await
    }

    /// Verifies one conversation through the backend, including both directions of blocking.
    /// # Errors
    /// Returns backend rejection or a stale/revoked account generation.
    pub async fn direct_conversation(
        &self,
        conversation: Uuid,
    ) -> Result<Generated<DirectConversation>, OperationError> {
        self.read(move |rest| async move { rest.direct_conversation(conversation).await })
            .await
    }

    /// Whether `conversation` with `peer` is open to messages. Answers from the account's own
    /// block list and DM list when they settle it; only a conversation the list no longer
    /// holds (the backend leaves out hidden ones and ones blocked in either direction) asks
    /// the backend, which tells a refusal from a hidden conversation.
    /// # Errors
    /// Returns backend rejection other than a refusal or absence, or a stale/revoked account
    /// generation.
    pub async fn direct_access(
        &self,
        conversation: Uuid,
        peer: Uuid,
    ) -> Result<DirectAccess, OperationError> {
        let image = std::sync::Arc::clone(&self.session.subscribe_resources().borrow());
        if image.blocks.contains(peer) {
            return Ok(DirectAccess::YouBlocked);
        }
        if image.dms.get(conversation).is_some() {
            return Ok(DirectAccess::Open);
        }
        match self.direct_conversation(conversation).await {
            Ok(_) => Ok(DirectAccess::Open),
            Err(error) => closed_access(&error).ok_or(error),
        }
    }

    /// Opens or unhides a conversation with a real backend user ID.
    /// External webhook authors have no person ID and must not use a bridge account here.
    /// A DM result grants neither timeline coverage nor a read acknowledgement.
    /// # Errors
    /// Returns backend rejection (including reverse blocking), stale generation, or uncertainty.
    pub async fn open_direct_conversation(
        &self,
        user: Uuid,
    ) -> Result<OperationReceipt<DirectConversation>, OperationError> {
        let request = OpenDirectMessageRequest {
            user_id: user.to_string(),
        };
        self.mutate(
            move |rest| async move { rest.open_direct_conversation(&request).await },
            Refresh::Social,
        )
        .await
    }

    /// Hides a conversation without deleting canonical history or read state.
    /// # Errors
    /// Returns typed rejection, stale generation, or uncertain dispatch.
    pub async fn close_direct_conversation(
        &self,
        conversation: Uuid,
    ) -> Result<OperationReceipt<()>, OperationError> {
        self.mutate(
            move |rest| async move { rest.close_direct_conversation(conversation).await },
            Refresh::Social,
        )
        .await
    }

    /// Reads only this user's outbound blocks; absence does not prove reverse-block absence.
    /// # Errors
    /// Returns backend rejection or a stale/revoked account generation.
    pub async fn blocked_users(&self) -> Result<Generated<BlockListResponse>, OperationError> {
        self.read(|rest| async move { rest.blocked_users().await })
            .await
    }

    /// Blocks a real backend user. The server rejects self-blocking and unknown identities.
    /// # Errors
    /// Returns typed rejection, stale generation, or uncertain dispatch.
    pub async fn block_user(&self, user: Uuid) -> Result<OperationReceipt<()>, OperationError> {
        self.mutate(
            move |rest| async move { rest.block_user(user).await },
            Refresh::Social,
        )
        .await
    }

    /// Removes an outbound block without reopening the conversation.
    /// # Errors
    /// Returns typed rejection, stale generation, or uncertain dispatch.
    pub async fn unblock_user(&self, user: Uuid) -> Result<OperationReceipt<()>, OperationError> {
        self.mutate(
            move |rest| async move { rest.unblock_user(user).await },
            Refresh::Social,
        )
        .await
    }
}

/// The access a failed conversation read proves, when it proves one.
fn closed_access(error: &OperationError) -> Option<DirectAccess> {
    let problem = match error {
        OperationError::NotFound => return Some(DirectAccess::Gone),
        OperationError::Session(ClientError::Api(papo_api::Error::Problem { status, problem })) => {
            (*status, problem.kind())
        }
        _ => return None,
    };
    match problem {
        (404, _) => Some(DirectAccess::Gone),
        (_, papo_api::ProblemKind::DirectMessageBlocked) => Some(DirectAccess::Refused),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{DirectAccess, OperationError, closed_access};
    use crate::ClientError;
    use papo_api::wire::Problem;

    fn problem(status: u16, slug: &str) -> OperationError {
        OperationError::Session(ClientError::Api(papo_api::Error::Problem {
            status,
            problem: Box::new(Problem {
                r#type: format!("/errors/{slug}"),
                title: String::new(),
                status: i64::from(status),
                detail: None,
                instance: None,
            }),
        }))
    }

    #[test]
    fn a_blocked_conversation_is_refused_without_saying_who_blocked() {
        assert_eq!(
            closed_access(&problem(403, "dm-blocked")),
            Some(DirectAccess::Refused)
        );
    }

    #[test]
    fn a_conversation_the_account_is_not_in_is_gone() {
        assert_eq!(
            closed_access(&problem(404, "not-found")),
            Some(DirectAccess::Gone)
        );
        assert_eq!(
            closed_access(&OperationError::NotFound),
            Some(DirectAccess::Gone)
        );
    }

    #[test]
    fn other_failures_prove_nothing() {
        assert_eq!(closed_access(&problem(403, "forbidden")), None);
        assert_eq!(closed_access(&problem(500, "internal")), None);
        assert_eq!(closed_access(&OperationError::InvalidData), None);
    }
}
