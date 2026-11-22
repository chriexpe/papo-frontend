//! Server registry, account lists, server badges, discovery, reordering, and recoverable removals.
//! Uses `papo_sdk_ui::ServerListService` directly with all 11 `VectorDiff` variants preserved.

use std::sync::Arc;

use papo_sdk::UnixMicros;
use papo_sdk_ui::ServerListService;

use crate::{
    client::FfiClient,
    diffs::translate_diffs,
    errors::{FfiClientError, map_discovery_error},
    ids::{FfiAccountId, parse_account_list, parse_server_id},
    server_badges_records::{FfiServerBadgesSnapshot, map_badges_snapshot},
    server_records::{
        FfiAccountDiff, FfiAccountListSnapshot, FfiAccountListUpdate, FfiAddedServer,
        FfiRemovalDiff, FfiServerDiff, FfiServerDiscovery, FfiServerListSnapshot,
        FfiServerListUpdate, map_registry_snapshot,
    },
    subscription::{FfiSubscriptionHandle, spawn_stream_subscription},
};

/// Listener receiving atomic server list baseline and ordered updates.
#[uniffi::export(with_foreign)]
pub trait FfiServerListListener: Send + Sync {
    /// Initial atomic baseline.
    fn on_snapshot(&self, snapshot: FfiServerListSnapshot);
    /// Subsequent coherent diff batch.
    fn on_update(&self, update: FfiServerListUpdate);
    /// Terminal callback emitted when registry closes.
    fn on_closed(&self);
}

/// Listener for per-server visible account changes.
#[uniffi::export(with_foreign)]
pub trait FfiAccountListListener: Send + Sync {
    /// Initial per-server baseline.
    fn on_snapshot(&self, snapshot: FfiAccountListSnapshot);
    /// Subsequent per-server diffs.
    fn on_update(&self, update: FfiAccountListUpdate);
    /// Terminal notification on watch close.
    fn on_closed(&self);
}

/// Listener for coherent badge aggregate updates.
#[uniffi::export(with_foreign)]
pub trait FfiServerBadgesListener: Send + Sync {
    /// Initial atomic badge totals.
    fn on_snapshot(&self, snapshot: FfiServerBadgesSnapshot);
    /// Subsequent changed aggregate totals.
    fn on_update(&self, snapshot: FfiServerBadgesSnapshot);
    /// Terminal notification on watch close.
    fn on_closed(&self);
}

#[uniffi::export]
impl FfiClient {
    /// Validates and probes `/server` without saving anything into the durable registry.
    ///
    /// # Errors
    /// Classifies probe errors using `DiscoveryFailure`.
    pub async fn discover(&self, address: String) -> Result<FfiServerDiscovery, FfiClientError> {
        let svc = ServerListService::new(self.handle.clone());
        self.run_scoped(|| FfiClientError::Closed, async move {
            svc.discover(&address)
                .await
                .map(Into::into)
                .map_err(|e| map_discovery_error(&e))
        })
        .await
    }

    /// Probes `/server`, saves the normalized server entry with FULL commit, and returns discovery.
    ///
    /// # Errors
    /// Classifies probe failures or returns duplicate/validation/storage errors.
    pub async fn add_server(
        &self,
        address: String,
        added_at_us: i64,
    ) -> Result<FfiAddedServer, FfiClientError> {
        let svc = ServerListService::new(self.handle.clone());
        self.run_scoped(|| FfiClientError::Closed, async move {
            svc.add_server(&address, UnixMicros(added_at_us))
                .await
                .map(Into::into)
                .map_err(|e| map_discovery_error(&e))
        })
        .await
    }

    /// Persists complete server order atomically.
    ///
    /// # Errors
    /// Returns [`FfiClientError::InvalidOrder`], invalid argument, or storage errors.
    pub async fn reorder_servers(&self, server_ids: Vec<String>) -> Result<(), FfiClientError> {
        let parsed: Vec<_> = server_ids
            .iter()
            .map(|id| parse_server_id(id))
            .collect::<Result<_, _>>()?;
        let svc = ServerListService::new(self.handle.clone());
        self.run_scoped(|| FfiClientError::Closed, async move {
            svc.reorder_servers(parsed)
                .await
                .map_err(FfiClientError::from)
        })
        .await
    }

    /// Persists complete account order within `server_id` atomically.
    ///
    /// # Errors
    /// Returns [`FfiClientError::InvalidOrder`], not found, or storage errors.
    pub async fn reorder_accounts(
        &self,
        server_id: String,
        accounts: Vec<FfiAccountId>,
    ) -> Result<(), FfiClientError> {
        let server = parse_server_id(&server_id)?;
        let accounts = parse_account_list(accounts)?;
        let svc = ServerListService::new(self.handle.clone());
        self.run_scoped(|| FfiClientError::Closed, async move {
            svc.reorder_accounts(server, accounts)
                .await
                .map_err(FfiClientError::from)
        })
        .await
    }

    /// Selects active account for `server_id`, or clears active selection with `None`.
    ///
    /// # Errors
    /// Returns scope, not found, or storage errors.
    pub async fn set_active_account(
        &self,
        server_id: String,
        account: Option<FfiAccountId>,
    ) -> Result<(), FfiClientError> {
        let server = parse_server_id(&server_id)?;
        let account = account.map(FfiAccountId::into_sdk).transpose()?;
        let svc = ServerListService::new(self.handle.clone());
        self.run_scoped(|| FfiClientError::Closed, async move {
            svc.set_active_account(server, account)
                .await
                .map_err(FfiClientError::from)
        })
        .await
    }

    /// Starts recoverable server removal, immediately hiding its rows after FULL commit.
    ///
    /// # Errors
    /// Returns removal or storage errors.
    pub async fn remove_server(&self, server_id: String) -> Result<(), FfiClientError> {
        let server = parse_server_id(&server_id)?;
        let svc = ServerListService::new(self.handle.clone());
        self.run_scoped(|| FfiClientError::Closed, async move {
            svc.remove_server(server)
                .await
                .map_err(FfiClientError::from)
        })
        .await
    }

    /// Starts recoverable single-account removal without disturbing sibling accounts on that server.
    ///
    /// # Errors
    /// Returns removal or storage errors.
    pub async fn remove_account(&self, account: FfiAccountId) -> Result<(), FfiClientError> {
        let account = account.into_sdk()?;
        let svc = ServerListService::new(self.handle.clone());
        self.run_scoped(|| FfiClientError::Closed, async move {
            svc.remove_account(account)
                .await
                .map_err(FfiClientError::from)
        })
        .await
    }

    /// Retries any incomplete recoverable removal jobs.
    ///
    /// # Errors
    /// Returns storage or closure errors.
    pub async fn retry_removals(&self) -> Result<(), FfiClientError> {
        let svc = ServerListService::new(self.handle.clone());
        self.run_scoped(|| FfiClientError::Closed, async move {
            svc.retry_removals().await.map_err(FfiClientError::from)
        })
        .await
    }

    /// Subscribes to atomic server, account, and removal diffs across the entire profile.
    ///
    /// # Errors
    /// Returns closure or worker start errors.
    pub fn subscribe_servers(
        &self,
        listener: Arc<dyn FfiServerListListener>,
    ) -> Result<Arc<FfiSubscriptionHandle>, FfiClientError> {
        let sub = ServerListService::new(self.handle.clone()).subscribe();
        let baseline = map_registry_snapshot(sub.version, sub.snapshot);
        let l_snap = Arc::clone(&listener);
        let l_upd = Arc::clone(&listener);
        let l_end = listener;
        spawn_stream_subscription(
            &self.runtime,
            &self.tasks,
            &self.shutdown,
            self.callbacks.clone(),
            baseline,
            sub.stream,
            move |snap| {
                l_snap.on_snapshot(snap);
                true
            },
            move |upd| {
                let Ok(server_diffs) =
                    translate_diffs(upd.servers, |d| FfiServerDiff::from_sdk(d, Into::into))
                else {
                    return false;
                };
                let Ok(account_diffs) =
                    translate_diffs(upd.accounts, |d| FfiAccountDiff::from_sdk(d, Into::into))
                else {
                    return false;
                };
                let Ok(removal_diffs) =
                    translate_diffs(upd.removals, |d| FfiRemovalDiff::from_sdk(d, Into::into))
                else {
                    return false;
                };
                l_upd.on_update(FfiServerListUpdate {
                    version: upd.version,
                    server_diffs,
                    account_diffs,
                    removal_diffs,
                });
                true
            },
            move || l_end.on_closed(),
        )
    }

    /// Subscribes atomically to one server's visible account list diffs.
    ///
    /// # Errors
    /// Returns invalid argument, closure, or worker failures.
    pub fn subscribe_server_accounts(
        &self,
        server_id: String,
        listener: Arc<dyn FfiAccountListListener>,
    ) -> Result<Arc<FfiSubscriptionHandle>, FfiClientError> {
        let server = parse_server_id(&server_id)?;
        let sub = ServerListService::new(self.handle.clone()).subscribe_accounts(server);
        let baseline = FfiAccountListSnapshot {
            version: sub.version,
            accounts: sub.snapshot.into_iter().map(Into::into).collect(),
        };
        let l_snap = Arc::clone(&listener);
        let l_upd = Arc::clone(&listener);
        let l_end = listener;
        spawn_stream_subscription(
            &self.runtime,
            &self.tasks,
            &self.shutdown,
            self.callbacks.clone(),
            baseline,
            sub.stream,
            move |snap| {
                l_snap.on_snapshot(snap);
                true
            },
            move |upd| {
                let Ok(diffs) =
                    translate_diffs(upd.diffs, |d| FfiAccountDiff::from_sdk(d, Into::into))
                else {
                    return false;
                };
                l_upd.on_update(FfiAccountListUpdate {
                    version: upd.version,
                    diffs,
                });
                true
            },
            move || l_end.on_closed(),
        )
    }

    /// Subscribes to coherent badge aggregates across all accounts and active-account rails.
    ///
    /// # Errors
    /// Returns closure or worker start failures.
    pub fn subscribe_server_badges(
        &self,
        listener: Arc<dyn FfiServerBadgesListener>,
    ) -> Result<Arc<FfiSubscriptionHandle>, FfiClientError> {
        let sub = ServerListService::new(self.handle.clone()).subscribe_badges();
        let baseline = map_badges_snapshot(sub.version, sub.snapshot);
        let l_snap = Arc::clone(&listener);
        let l_upd = Arc::clone(&listener);
        let l_end = listener;
        spawn_stream_subscription(
            &self.runtime,
            &self.tasks,
            &self.shutdown,
            self.callbacks.clone(),
            baseline,
            sub.stream,
            move |snap| {
                l_snap.on_snapshot(snap);
                true
            },
            move |upd| {
                l_upd.on_update(map_badges_snapshot(upd.version, upd.snapshot));
                true
            },
            move || l_end.on_closed(),
        )
    }
}
