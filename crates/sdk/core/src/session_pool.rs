//! Client-wide account task groups and real removal-stop integration; no cache is opened here.

use crate::{
    AccountId, ServerId,
    removal::{RemovalFuture, RemovalHooks},
    session::SessionInner,
};
use std::sync::atomic::Ordering;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, atomic::AtomicU64},
};
use tokio_util::sync::CancellationToken;

pub(crate) struct SessionPool {
    pub root: CancellationToken,
    pub next: Arc<AtomicU64>,
    sessions: Mutex<HashMap<AccountId, Arc<SessionInner>>>,
    caches: Mutex<Option<crate::AccountCaches>>,
    media: Mutex<Option<crate::media::MediaStore>>,
    retired: Mutex<HashMap<AccountId, u64>>,
    retired_servers: Mutex<HashMap<ServerId, u64>>,
    lifecycle: Mutex<HashMap<AccountId, Arc<tokio::sync::RwLock<()>>>>,
    attempts: Mutex<
        Vec<(
            ServerId,
            std::sync::Weak<crate::observable::StateCell<crate::AuthState>>,
        )>,
    >,
}
impl SessionPool {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            root: CancellationToken::new(),
            next: Arc::new(AtomicU64::new(1)),
            sessions: Mutex::new(HashMap::new()),
            caches: Mutex::new(None),
            media: Mutex::new(None),
            retired: Mutex::new(HashMap::new()),
            retired_servers: Mutex::new(HashMap::new()),
            lifecycle: Mutex::new(HashMap::new()),
            attempts: Mutex::new(Vec::new()),
        })
    }
    pub fn set_caches(&self, caches: crate::AccountCaches) {
        *self
            .caches
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(caches);
    }
    pub fn set_media(&self, media: crate::media::MediaStore) {
        *self
            .media
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(media);
    }
    pub fn get(&self, account: AccountId) -> Option<Arc<SessionInner>> {
        self.sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&account)
            .cloned()
    }
    pub(crate) fn lifecycle(&self, account: AccountId) -> Arc<tokio::sync::RwLock<()>> {
        Arc::clone(
            self.lifecycle
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .entry(account)
                .or_default(),
        )
    }
    pub async fn insert(&self, session: Arc<SessionInner>) {
        let _lifecycle = self.lifecycle(session.account).write_owned().await;
        self.sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(session.account, session);
    }
    pub fn register_auth(
        &self,
        server: ServerId,
        state: &Arc<crate::observable::StateCell<crate::AuthState>>,
    ) {
        let mut attempts = self
            .attempts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.root.is_cancelled() {
            state.close();
            return;
        }
        attempts.retain(|(_, state)| state.strong_count() != 0);
        attempts.push((server, Arc::downgrade(state)));
    }
    fn close_auth(&self, server: Option<ServerId>) {
        let mut attempts = self
            .attempts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        attempts.retain(|(id, state)| {
            let Some(state) = state.upgrade() else {
                return false;
            };
            if server.is_none_or(|server| server == *id) {
                state.send_replace(crate::AuthState::SignedOut);
                state.close();
                false
            } else {
                true
            }
        });
    }
    pub fn admit_login(&self) -> u64 {
        self.next.fetch_add(1, Ordering::Relaxed)
    }
    pub fn retire_server(&self, server: ServerId) {
        self.close_auth(Some(server));
        self.retired_servers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(server, self.admit_login());
    }
    pub fn login_is_current(&self, account: AccountId, admitted: u64) -> bool {
        let account_retired = self
            .retired
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&account)
            .copied()
            .unwrap_or(0);
        let server_retired = self
            .retired_servers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&account.server())
            .copied()
            .unwrap_or(0);
        !self.root.is_cancelled() && account_retired < admitted && server_retired < admitted
    }
    pub async fn stop(&self, account: AccountId) {
        let _lifecycle = self.lifecycle(account).write_owned().await;
        self.stop_inner(account).await;
    }
    async fn stop_inner(&self, account: AccountId) {
        self.retired
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(account, self.admit_login());
        let session = self
            .sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&account);
        if let Some(session) = session {
            session.stop().await;
        }
    }
    pub async fn stop_if_current(&self, expected: &Arc<SessionInner>) {
        let _lifecycle = self.lifecycle(expected.account).write_owned().await;
        let removed = {
            let mut sessions = self
                .sessions
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if sessions
                .get(&expected.account)
                .is_some_and(|current| Arc::ptr_eq(current, expected))
            {
                sessions.remove(&expected.account)
            } else {
                None
            }
        };
        if let Some(session) = removed {
            self.retired
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(session.account, self.admit_login());
            session.stop().await;
        }
    }
    pub async fn stop_all(&self) {
        self.root.cancel();
        self.close_auth(None);
        let sessions = std::mem::take(
            &mut *self
                .sessions
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        for session in sessions.into_values() {
            session.stop().await;
        }
    }
}
impl RemovalHooks for SessionPool {
    fn stop_session(&self, account: AccountId) -> RemovalFuture<'_> {
        Box::pin(async move {
            let _lifecycle = self.lifecycle(account).write_owned().await;
            self.stop_inner(account).await;
            let caches = self
                .caches
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            if let Some(caches) = caches {
                caches
                    .close(account)
                    .await
                    .map_err(|_| crate::removal::RemovalHookError::Incomplete)?;
            }
            Ok(())
        })
    }
    fn delete_media_refs(&self, account: AccountId) -> RemovalFuture<'_> {
        Box::pin(async move {
            let media = self
                .media
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            let media = media.ok_or(crate::removal::RemovalHookError::Incomplete)?;
            media
                .remove_account(account)
                .await
                .map_err(|_| crate::removal::RemovalHookError::Incomplete)
        })
    }
}
