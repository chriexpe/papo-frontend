//! Observable server/account/removal lists; validation, probing and durable mutations belong to SDK.

use std::pin::Pin;

use eyeball_im::VectorDiff;
use futures_util::Stream;
use imbl::Vector;
use papo_sdk::removal::RemovalJob;
use papo_sdk::{
    AccountId, ClientError, ClientHandle, SavedAccount, SavedServer, ServerId, UnixMicros,
};

use crate::registry_projection;

/// Immutable lists projected from one fully committed SDK registry version.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerListSnapshot {
    /// Visible servers in durable registry order.
    pub servers: Vector<SavedServer>,
    /// Visible accounts, including their active flags.
    pub accounts: Vector<SavedAccount>,
    /// Recoverable jobs, including typed failures; removed entries stay hidden.
    pub removals: Vector<RemovalJob>,
}

impl ServerListSnapshot {
    /// Returns visible accounts belonging only to `server`, preserving registry order.
    #[must_use]
    pub fn accounts_for_server(&self, server: ServerId) -> Vector<SavedAccount> {
        self.accounts
            .iter()
            .filter(|account| account.id.server() == server)
            .cloned()
            .collect()
    }

    /// Returns the active visible account for `server`, or `None` when none is selected.
    #[must_use]
    pub fn active_account(&self, server: ServerId) -> Option<AccountId> {
        self.accounts
            .iter()
            .find(|account| account.id.server() == server && account.active)
            .map(|account| account.id)
    }
}

/// One committed registry change. Apply all three diff lists as a single unit.
///
/// Consecutive source versions use identity-aware edits. A skipped source version
/// resets every list, even an unchanged list. Stable IDs survive resets; indices do not.
#[derive(Clone, Debug)]
pub struct ServerListUpdate {
    /// SDK registry version after applying this entire batch.
    pub version: u64,
    /// Ordered edits to the visible server list.
    pub servers: Vec<VectorDiff<SavedServer>>,
    /// Ordered edits to visible accounts, including active selection changes.
    pub accounts: Vec<VectorDiff<SavedAccount>>,
    /// Ordered edits to recoverable removal progress and typed failures.
    pub removals: Vec<VectorDiff<RemovalJob>>,
}

/// Pull-driven committed registry changes; ends when the SDK closes its watch.
///
/// No background task or runtime is created. A slow consumer retains only its last
/// projection and the watch's newest source snapshot, not an unbounded edit queue.
pub type ServerListStream = Pin<Box<dyn Stream<Item = ServerListUpdate> + Send>>;

/// Atomic baseline and subsequent stream, with no snapshot/subscription race.
pub struct ServerListSubscription {
    /// SDK version represented by `snapshot`.
    pub version: u64,
    /// Baseline for applying every subsequent stream batch.
    pub snapshot: ServerListSnapshot,
    /// Changes strictly after the atomically captured baseline.
    pub stream: ServerListStream,
}

/// Pull-driven edits to the accounts of one server, including active flags.
pub type AccountListStream = Pin<Box<dyn Stream<Item = AccountListUpdate> + Send>>;

/// One source registry version projected onto a server's account list.
#[derive(Clone, Debug)]
pub struct AccountListUpdate {
    /// SDK registry version represented after applying `diffs`.
    pub version: u64,
    /// Apply in order. A source version gap always produces one `Reset`.
    pub diffs: Vec<VectorDiff<SavedAccount>>,
}

/// Atomic account-list baseline and pull-driven changes for one server.
pub struct AccountListSubscription {
    /// SDK version represented by `snapshot`.
    pub version: u64,
    /// Visible accounts for the requested server only, in registry order.
    pub snapshot: Vector<SavedAccount>,
    /// Subsequent changes, including empty batches for unrelated registry commits.
    pub stream: AccountListStream,
}

/// Public SDK-backed list projection and mutation facade, without platform state.
#[derive(Clone)]
pub struct ServerListService {
    /// Shared SDK capability; does not create or own a runtime.
    handle: ClientHandle,
}

impl ServerListService {
    /// Uses the existing client's runtime, durable registry and shutdown lifecycle.
    #[must_use]
    pub fn new(handle: ClientHandle) -> Self {
        Self { handle }
    }

    /// Atomically captures the committed baseline and subscribes to future changes.
    #[must_use]
    pub fn subscribe(&self) -> ServerListSubscription {
        registry_projection::subscribe(self.handle.subscribe_registry())
    }

    /// Subscribes atomically to one server's visible accounts and active flags.
    ///
    /// Unknown/removed servers have an empty list. Source version gaps still reset
    /// the list even when the skipped commits affected other servers only.
    #[must_use]
    pub fn subscribe_accounts(&self, server: ServerId) -> AccountListSubscription {
        registry_projection::subscribe_accounts(self.handle.subscribe_registry(), server)
    }

    /// Reports whether the SDK client has closed; streams finish on watch closure.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.handle.is_closed()
    }

    /// Delegates address validation, `/server` probing and durable registration to SDK.
    /// `added_at` is caller-supplied Unix microseconds; this service reads no clock.
    ///
    /// # Errors
    /// Forwards SDK validation, probe, persistence and closed-client errors unchanged.
    pub async fn add_server(
        &self,
        address: &str,
        added_at: UnixMicros,
    ) -> Result<ServerId, ClientError> {
        self.handle.add_server(address, added_at).await
    }

    /// Commits the requested complete server order through SDK.
    ///
    /// # Errors
    /// Forwards SDK invalid-order, persistence and closed-client errors unchanged.
    pub async fn reorder_servers(&self, servers: Vec<ServerId>) -> Result<(), ClientError> {
        self.handle.reorder_servers(servers).await
    }

    /// Starts the SDK's recoverable server removal, hiding entries after commit.
    ///
    /// # Errors
    /// Forwards SDK removal, persistence and closed-client errors unchanged.
    pub async fn remove_server(&self, server: ServerId) -> Result<(), ClientError> {
        self.handle.remove_server(server).await
    }

    /// Starts recoverable removal for one account, never another server's account.
    ///
    /// # Errors
    /// Forwards SDK removal, persistence and closed-client errors unchanged.
    pub async fn remove_account(&self, account: AccountId) -> Result<(), ClientError> {
        self.handle.remove_account(account).await
    }

    /// Requests SDK retry of unfinished removal jobs; progress is observable.
    ///
    /// # Errors
    /// Forwards SDK retry, persistence and closed-client errors unchanged.
    pub async fn retry_removals(&self) -> Result<(), ClientError> {
        self.handle.retry_removals().await
    }

    /// Selects an active account for `server`, or clears its selection with `None`.
    ///
    /// # Errors
    /// Forwards SDK account-scope, persistence and closed-client errors unchanged.
    pub async fn set_active_account(
        &self,
        server: ServerId,
        account: Option<AccountId>,
    ) -> Result<(), ClientError> {
        self.handle.set_active_account(server, account).await
    }
}
