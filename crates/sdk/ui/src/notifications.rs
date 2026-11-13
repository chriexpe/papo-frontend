//! Scoped notification inbox and delivery projections; policy/claims/background work live exclusively in core.

use futures_util::{StreamExt, stream};
use papo_sdk::{
    ClientHandle, InboxCursor, InboxPage, NotificationError, NotificationRecord,
    NotificationTarget, Session,
};
use std::sync::Arc;

/// Stable inbox row identity, independent of list position or current active account.
#[derive(Clone)]
pub struct NotificationItem {
    /// Original backend row with account/generation provenance.
    pub record: NotificationRecord,
}
/// Account-isolated inbox success or retry state. One failed server does not erase other accounts.
#[derive(Clone)]
#[non_exhaustive]
pub enum AccountInbox {
    /// Complete accepted backend page.
    Ready {
        /// Original account.
        account: papo_sdk::AccountId,
        /// Bounded page in backend order.
        page: Arc<InboxPage>,
    },
    /// Account failed and may be retried independently.
    Retry(papo_sdk::AccountId),
}
/// Shared notification UI entry point; contains no mute/mention/delivery policy.
#[derive(Clone)]
pub struct NotificationClient {
    client: ClientHandle,
}
impl NotificationClient {
    /// Binds to the SDK client, independent of native lifecycle and widgets.
    #[must_use]
    pub fn new(client: ClientHandle) -> Self {
        Self { client }
    }
    /// Gets one account's authoritative inbox, without claiming or acknowledging reads.
    /// # Errors
    /// Returns missing/revoked account, malformed records or backend failure.
    pub async fn inbox(
        &self,
        account: papo_sdk::AccountId,
        cursor: Option<InboxCursor>,
    ) -> Result<InboxPage, NotificationError> {
        self.client
            .session(account)?
            .notifications()
            .inbox(cursor)
            .await
    }
    /// Loads signed-in accounts independently; failures remain per-account retry rows.
    pub async fn inboxes(&self) -> Vec<AccountInbox> {
        let accounts = self
            .client
            .subscribe_registry()
            .borrow()
            .accounts
            .iter()
            .map(|account| account.id)
            .collect::<Vec<_>>();
        stream::iter(accounts)
            .map(|account| async move {
                match self.inbox(account, None).await {
                    Ok(page) => AccountInbox::Ready {
                        account,
                        page: Arc::new(page),
                    },
                    Err(_) => AccountInbox::Retry(account),
                }
            })
            .buffer_unordered(4)
            .collect()
            .await
    }

    /// Resolves a scoped inbox item for client navigation; does not mark unseen targets read.
    /// # Errors
    /// Rejects account replacement, stale generation or missing channel.
    pub fn target(&self, item: &NotificationItem) -> Result<NotificationTarget, NotificationError> {
        self.session(item.record.account)?
            .notifications()
            .target(&item.record)
    }
    /// Resolves a push hint only when the supplied account or server-independent message identity is unique.
    /// Push payload is never display authority; callers subsequently fetch the SDK inbox.
    /// # Errors
    /// Returns unavailable/ambiguous account. Never falls back to current active user.
    pub fn push_account(
        &self,
        account: Option<papo_sdk::AccountId>,
    ) -> Result<Session, NotificationError> {
        if let Some(account) = account {
            return self.session(account);
        }
        let accounts = self
            .client
            .subscribe_registry()
            .borrow()
            .accounts
            .iter()
            .map(|a| a.id)
            .collect::<Vec<_>>();
        if accounts.len() != 1 {
            return Err(NotificationError::InvalidData);
        }
        self.session(accounts[0])
    }
    fn session(&self, account: papo_sdk::AccountId) -> Result<Session, NotificationError> {
        self.client.session(account).map_err(Into::into)
    }
}
