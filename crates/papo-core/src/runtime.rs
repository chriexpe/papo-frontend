use std::sync::Arc;

use crate::api::models::IceServer;
use crate::api::net::{
    BackgroundRunResult, Command, Net, NetSender, RuntimeDiagnostics as NetDiagnostics,
    TransportMode, Update, Wake,
};
use crate::api::ws::{Connection, Event};
use crate::cache::ClientDb;
use crate::notification::{
    CandidateSource, NotificationContext, NotificationCoordinator,
};
use crate::state::{Phase, Screen, Store};
use crate::storage::{Secret, SecretStore};

pub const DEFAULT_DRAIN_LIMIT: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeMode {
    Interactive,
    BackgroundReconcile,
}

#[derive(Clone, Debug)]
pub enum CallRuntimeEffect {
    VoiceReady {
        channel_id: String,
        attempt: u64,
        servers: Vec<IceServer>,
    },
    Event {
        event: Box<Event>,
        me_before: String,
        phase_before: Phase,
        store_error_before: Option<String>,
        call_error_before: Option<String>,
    },
    ConnectionOffline,
    VoiceFailed {
        channel_id: String,
        attempt: u64,
        was_current: bool,
    },
}

#[derive(Clone, Debug)]
pub enum RuntimeEffect {
    ServerUnlocked,
    ServerLabelChanged(String),
    OutgoingRejected {
        content: String,
        reply_to: Option<String>,
        notify_reply: bool,
    },
    Call(Box<CallRuntimeEffect>),
}

#[derive(Clone, Debug, Default)]
pub struct RuntimeNotificationView {
    pub server_label: String,
    pub visible_server: bool,
    pub notifications_enabled: bool,
}

#[derive(Debug, Default)]
pub struct RuntimeDrain {
    pub processed: usize,
    pub effects: Vec<RuntimeEffect>,
    pub limit_reached: bool,
}

#[derive(Clone, Debug)]
pub struct ServerRuntimeDiagnostics {
    pub server_key: String,
    pub connection: Connection,
    pub generation: u64,
    pub authenticated: bool,
    pub reconnect_refreshes: u64,
    pub net: NetDiagnostics,
}

pub struct ServerRuntime {
    pub url: String,
    pub server_key: String,
    pub net: Net,
    pub store: Store,
    pub cache: Arc<ClientDb>,
    cached_owner: Option<String>,
    notification: Arc<NotificationCoordinator>,
    reconnect_refreshes: u64,
    mode: RuntimeMode,
    background_view: Option<RuntimeNotificationView>,
    background_result: Option<BackgroundRunResult>,
}

impl ServerRuntime {
    pub fn open(
        url: String,
        wake: Wake,
        storage: Arc<dyn SecretStore>,
        cache: Arc<ClientDb>,
        notification: Arc<NotificationCoordinator>,
    ) -> Self {
        Self::open_with_store(
            url,
            Store::default(),
            wake,
            storage,
            cache,
            notification,
        )
    }

    pub fn open_with_store(
        url: String,
        store: Store,
        wake: Wake,
        storage: Arc<dyn SecretStore>,
        cache: Arc<ClientDb>,
        notification: Arc<NotificationCoordinator>,
    ) -> Self {
        Self::open_with_mode(
            url,
            store,
            wake,
            storage,
            cache,
            notification,
            RuntimeMode::Interactive,
            None,
            std::time::Duration::from_secs(25),
        )
    }

    pub fn open_background(
        url: String,
        storage: Arc<dyn SecretStore>,
        cache: Arc<ClientDb>,
        notification: Arc<NotificationCoordinator>,
        notifications_enabled: bool,
        deadline: std::time::Duration,
    ) -> Self {
        let view = RuntimeNotificationView {
            server_label: crate::server_key(&url),
            visible_server: false,
            notifications_enabled,
        };
        Self::open_with_mode(
            url,
            Store::default(),
            Wake::noop(),
            storage,
            cache,
            notification,
            RuntimeMode::BackgroundReconcile,
            Some(view),
            deadline,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn open_with_mode(
        url: String,
        mut store: Store,
        wake: Wake,
        storage: Arc<dyn SecretStore>,
        cache: Arc<ClientDb>,
        notification: Arc<NotificationCoordinator>,
        mode: RuntimeMode,
        background_view: Option<RuntimeNotificationView>,
        deadline: std::time::Duration,
    ) -> Self {
        let server_key = crate::server_key(&url);

        // Restore happens synchronously before the transport is allowed to
        // mutate the Store. Cached data is useful immediately, but Store keeps
        // it stale until current-runtime reconciliation makes it authoritative.
        let mut cached_owner = None;
        let has_session = storage
            .load(&server_key, Secret::SessionToken)
            .ok()
            .flatten()
            .is_some();
        if has_session
            && let Some(snapshot) = cache.load_snapshot(&server_key)
            && !snapshot.is_empty()
        {
            cached_owner = snapshot.owner_user_id.clone();
            store.restore_cached(snapshot);
            if let Some(owner) = cached_owner.as_deref() {
                match cache.load_outgoing(&server_key, owner) {
                    Ok(outgoing) => {
                        for item in outgoing {
                            store.project_outgoing(item);
                        }
                    }
                    Err(error) => {
                        log::warn!("outgoing {server_key}: restore inicial falhou: {error}");
                    }
                }
            }
        }

        let net = match mode {
            RuntimeMode::Interactive => Net::spawn(url.clone(), wake, storage, Arc::clone(&cache)),
            RuntimeMode::BackgroundReconcile => {
                Net::spawn_background(url.clone(), wake, storage, Arc::clone(&cache), deadline)
            }
        };

        // Live hooks only exist on the interactive transport. Background
        // notifications are sourced from the authoritative REST Notification
        // list after Store has consumed the bootstrap metadata.
        if mode == RuntimeMode::Interactive {
            let message_coordinator = Arc::clone(&notification);
            let message_server_key = server_key.clone();
            net.set_message_callback(Some(Arc::new(move |message| {
                let _ = message_coordinator.handle_message(
                    &message_server_key,
                    message,
                    CandidateSource::Live,
                );
            })));

            let notification_coordinator = Arc::clone(&notification);
            let notification_server_key = server_key.clone();
            net.set_notification_callback(Some(Arc::new(move |item| {
                let _ = notification_coordinator.handle_notification(
                    &notification_server_key,
                    item,
                    CandidateSource::Live,
                );
            })));
        }

        Self {
            url,
            server_key,
            net,
            store,
            cache,
            cached_owner,
            notification,
            reconnect_refreshes: 0,
            mode,
            background_view,
            background_result: None,
        }
    }

    pub fn sender(&self) -> NetSender {
        self.net.sender()
    }

    pub fn cached_owner(&self) -> Option<&str> {
        self.cached_owner.as_deref()
    }

    pub fn sync_notification_context(&self, view: RuntimeNotificationView) {
        self.notification.sync_context(NotificationContext {
            server_key: self.server_key.clone(),
            navigation_server: self.url.clone(),
            server_label: view.server_label,
            owner_user_id: self.store.me.clone(),
            owner_name: self.store.my_name.clone(),
            selected_channel: if self.mode == RuntimeMode::BackgroundReconcile {
                String::new()
            } else {
                self.store.selected_channel.clone()
            },
            visible_server: view.visible_server,
            notifications_enabled: view.notifications_enabled,
            channels: self
                .store
                .channels
                .iter()
                .map(|channel| (channel.id.clone(), channel.name.clone()))
                .collect(),
            members: self
                .store
                .members
                .iter()
                .map(|member| (member.id.clone(), member.name.clone()))
                .collect(),
        });
    }

    pub fn process_update(&mut self, update: Update) -> Vec<RuntimeEffect> {
        let background_notifications = if self.mode == RuntimeMode::BackgroundReconcile {
            match &update {
                Update::Notifications(items) => Some(items.clone()),
                _ => None,
            }
        } else {
            None
        };
        if let Update::BackgroundFinished(result) = &update {
            self.background_result = Some(*result);
        }

        // Ordering is intentional:
        // 1) inspect the still-unconsumed update for presentation-only effects;
        // 2) apply the authoritative runtime transition to Store;
        // 3) enforce verified-account cache isolation;
        // 4) drain Store cache effects into ClientDb;
        // 5) schedule the one reconnect Refresh, if this was a real transition.
        let mut effects = self.presentation_effects(&update);
        let session_me = match &update {
            Update::Session(Some(me)) => Some(me.id.clone()),
            _ => None,
        };
        let session_ended = matches!(&update, Update::Session(None));
        let became_online = matches!(&update, Update::Connection(Connection::Online))
            && self.store.connection != Connection::Online;
        let refresh_after_online = became_online && self.store.screen == Screen::Chat;

        self.store.apply(update);

        if let Some(me_id) = session_me {
            if self
                .cached_owner
                .as_deref()
                .is_some_and(|owner| owner != me_id)
            {
                // Only reconstructible state is cleared. Owner-partitioned
                // outgoing/notification durability lives outside this reset.
                self.cache.clear_cached_data(&self.server_key);
                self.store.clear_cached_state();
            }
            self.cached_owner = Some(me_id);
        }
        if session_ended {
            self.cached_owner = None;
        }

        let cache_ops = self.store.take_cache_ops();
        if !cache_ops.is_empty() {
            self.cache.submit(&self.server_key, cache_ops);
        }

        if refresh_after_online && self.mode == RuntimeMode::Interactive {
            self.net.send(Command::Refresh);
            self.reconnect_refreshes = self.reconnect_refreshes.saturating_add(1);
        }

        if self.mode == RuntimeMode::BackgroundReconcile {
            if let Some(view) = &mut self.background_view {
                if let Some(server) = &self.store.server {
                    view.server_label = server.name.clone();
                }
                let view = view.clone();
                self.sync_notification_context(view);
            }
            if let Some(items) = background_notifications {
                for item in &items {
                    let _ = self.notification.handle_notification(
                        &self.server_key,
                        item,
                        CandidateSource::Background,
                    );
                }
            }
        }

        effects.shrink_to_fit();
        effects
    }

    pub fn try_drain(&mut self) -> Option<Vec<RuntimeEffect>> {
        self.net
            .try_recv()
            .map(|update| self.process_update(update))
    }

    pub fn recv_timeout(
        &mut self,
        timeout: std::time::Duration,
    ) -> Option<Vec<RuntimeEffect>> {
        self.net
            .recv_timeout(timeout)
            .map(|update| self.process_update(update))
    }

    pub fn mode(&self) -> RuntimeMode {
        self.mode
    }

    pub fn transport_mode(&self) -> TransportMode {
        self.net.mode()
    }

    pub fn background_result(&self) -> Option<BackgroundRunResult> {
        self.background_result
    }

    pub fn cancel_background(&self) {
        self.net.cancel_background();
    }

    pub fn wait_background_shutdown(&mut self) {
        self.net.wait_background_shutdown();
    }

    pub fn drain_until_idle(&mut self, max_updates: usize) -> RuntimeDrain {
        let mut drain = RuntimeDrain::default();
        if max_updates == 0 {
            drain.limit_reached = true;
            return drain;
        }

        for _ in 0..max_updates {
            let Some(update) = self.net.try_recv() else {
                return drain;
            };
            drain.processed += 1;
            drain.effects.extend(self.process_update(update));
        }

        // We intentionally do not probe by consuming a (max+1)th update:
        // reaching the caller's hard bound is enough to ask it to drive again.
        drain.limit_reached = true;
        drain
    }

    pub fn diagnostics(&self) -> ServerRuntimeDiagnostics {
        ServerRuntimeDiagnostics {
            server_key: self.server_key.clone(),
            connection: self.store.connection,
            generation: self.store.sync_generation(),
            authenticated: !self.store.me.is_empty(),
            reconnect_refreshes: self.reconnect_refreshes,
            net: self.net.diagnostics(),
        }
    }

    /// Explicit destructive lifecycle. Ordinary Drop intentionally does not
    /// clear any durable namespace or credential.
    pub fn forget_server(&self) {
        self.notification.remove_context(&self.server_key);
        self.cache.clear_server(&self.server_key);
        self.net.forget_credentials();
    }

    fn presentation_effects(&self, update: &Update) -> Vec<RuntimeEffect> {
        let mut effects = Vec::new();
        match update {
            Update::ServerUnlocked => effects.push(RuntimeEffect::ServerUnlocked),
            Update::Server(Some(server)) if !server.name.is_empty() => {
                effects.push(RuntimeEffect::ServerLabelChanged(server.name.clone()));
            }
            Update::OutgoingRejected {
                content,
                reply_to,
                notify_reply,
                ..
            } => effects.push(RuntimeEffect::OutgoingRejected {
                content: content.clone(),
                reply_to: reply_to.clone(),
                notify_reply: *notify_reply,
            }),
            Update::VoiceReady {
                channel_id,
                attempt,
                servers,
            } => effects.push(RuntimeEffect::Call(Box::new(CallRuntimeEffect::VoiceReady {
                channel_id: channel_id.clone(),
                attempt: *attempt,
                servers: servers.clone(),
            }))),
            Update::Event(event)
                if matches!(
                    &**event,
                    Event::VoiceLeft { .. } | Event::Failure { .. }
                ) =>
            {
                effects.push(RuntimeEffect::Call(Box::new(CallRuntimeEffect::Event {
                    event: Box::new((**event).clone()),
                    me_before: self.store.me.clone(),
                    phase_before: self.store.call.phase,
                    store_error_before: self.store.error.clone(),
                    call_error_before: self.store.call.error.clone(),
                })));
            }
            Update::Connection(Connection::Offline) => {
                effects.push(RuntimeEffect::Call(Box::new(
                    CallRuntimeEffect::ConnectionOffline,
                )));
            }
            Update::VoiceFailed {
                channel_id,
                attempt,
                ..
            } => effects.push(RuntimeEffect::Call(Box::new(CallRuntimeEffect::VoiceFailed {
                channel_id: channel_id.clone(),
                attempt: *attempt,
                was_current: self.store.call.current(channel_id, *attempt),
            }))),
            _ => {}
        }
        effects
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::models::{Notification, Server, Whoami};
    use crate::storage::MemorySecretStore;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn disabled_runtime() -> (ServerRuntime, Arc<MemorySecretStore>) {
        let cache = Arc::new(ClientDb::open(None));
        let notification = Arc::new(NotificationCoordinator::new(
            Arc::clone(&cache),
            None,
        ));
        let secrets = Arc::new(MemorySecretStore::default());
        let runtime = ServerRuntime::open(
            "http://127.0.0.1:9".to_owned(),
            Wake::noop(),
            secrets.clone(),
            cache,
            notification,
        );
        (runtime, secrets)
    }

    fn wait_background(runtime: &mut ServerRuntime) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while runtime.background_result().is_none() && std::time::Instant::now() < deadline {
            let _ = runtime.recv_timeout(std::time::Duration::from_millis(100));
        }
    }

    fn whoami(id: &str) -> Whoami {
        Whoami {
            id: id.to_owned(),
            username: id.to_owned(),
            nickname: None,
            status: None,
            status_message: None,
            roles: Vec::new(),
        }
    }

    fn temp_db_path(name: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("relógio")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "papo-runtime-{name}-{}-{stamp}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("criar temporário");
        dir.join("papo-cache.db")
    }

    #[test]
    fn runtime_headless_aceita_wake_noop_sem_egui() {
        let (runtime, _) = disabled_runtime();
        assert!(!runtime.server_key.is_empty());
        assert_eq!(runtime.url, "http://127.0.0.1:9");
    }

    #[test]
    fn background_runtime_no_session_is_clean_bounded_noop() {
        let cache = Arc::new(ClientDb::open(None));
        let notification = Arc::new(NotificationCoordinator::new(
            Arc::clone(&cache),
            None,
        ));
        let secrets = Arc::new(MemorySecretStore::default());
        let mut runtime = ServerRuntime::open_background(
            "http://127.0.0.1:9".to_owned(),
            secrets,
            cache,
            notification,
            true,
            std::time::Duration::from_secs(1),
        );

        assert_eq!(runtime.mode(), RuntimeMode::BackgroundReconcile);
        assert_eq!(runtime.transport_mode(), TransportMode::BackgroundReconcile);
        assert!(!runtime.transport_mode().opens_websocket());
        wait_background(&mut runtime);
        assert_eq!(
            runtime.background_result(),
            Some(BackgroundRunResult::NoSession)
        );
    }

    #[test]
    fn interactive_runtime_keeps_interactive_transport() {
        let (runtime, _) = disabled_runtime();
        assert_eq!(runtime.mode(), RuntimeMode::Interactive);
        assert_eq!(runtime.transport_mode(), TransportMode::Interactive);
        assert!(runtime.transport_mode().opens_websocket());
    }

    #[test]
    fn background_runtime_honours_hard_deadline() {
        let cache = Arc::new(ClientDb::open(None));
        let notification = Arc::new(NotificationCoordinator::new(
            Arc::clone(&cache),
            None,
        ));
        let secrets = Arc::new(MemorySecretStore::default());
        let url = "http://127.0.0.1:9".to_owned();
        let key = crate::server_key(&url);
        secrets
            .store(&key, Secret::SessionToken, "saved-token")
            .expect("guardar token");
        let mut runtime = ServerRuntime::open_background(
            url,
            secrets,
            cache,
            notification,
            true,
            std::time::Duration::ZERO,
        );

        wait_background(&mut runtime);
        assert_eq!(
            runtime.background_result(),
            Some(BackgroundRunResult::Deadline)
        );
    }

    #[test]
    fn cancelling_background_runtime_does_not_clear_credentials_or_cache() {
        let path = temp_db_path("background-cancel");
        let cache = Arc::new(ClientDb::open(Some(path.clone())));
        let notification = Arc::new(NotificationCoordinator::new(
            Arc::clone(&cache),
            None,
        ));
        let secrets = Arc::new(MemorySecretStore::default());
        let url = "http://127.0.0.1:9".to_owned();
        let key = crate::server_key(&url);
        secrets
            .store(&key, Secret::SessionToken, "saved-token")
            .expect("guardar token");

        let mut runtime = ServerRuntime::open_background(
            url,
            secrets.clone(),
            Arc::clone(&cache),
            notification,
            true,
            std::time::Duration::from_secs(5),
        );
        runtime.process_update(Update::Server(Some(Box::new(Server {
            id: "srv".to_owned(),
            name: "Papo".to_owned(),
            owner_id: None,
            owner_username: None,
            public: false,
            member_count: 1,
            channel_count: 1,
        }))));
        cache.flush();
        assert!(
            cache
                .load_snapshot(&key)
                .is_some_and(|snapshot| !snapshot.is_empty())
        );

        runtime.cancel_background();
        runtime.wait_background_shutdown();
        drop(runtime);
        cache.flush();

        assert_eq!(
            secrets
                .load(&key, Secret::SessionToken)
                .expect("ler token")
                .as_deref(),
            Some("saved-token")
        );
        assert!(
            cache
                .load_snapshot(&key)
                .is_some_and(|snapshot| !snapshot.is_empty())
        );

        drop(cache);
        if let Some(dir) = path.parent() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn background_runtime_routes_only_backend_notifications_to_coordinator() {
        let path = temp_db_path("background-notification");
        let cache = Arc::new(ClientDb::open(Some(path.clone())));
        let deliveries = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&deliveries);
        let notification = Arc::new(NotificationCoordinator::new(
            Arc::clone(&cache),
            Some(Arc::new(move |_| {
                seen.fetch_add(1, Ordering::SeqCst);
            })),
        ));
        let secrets = Arc::new(MemorySecretStore::default());
        let mut runtime = ServerRuntime::open_background(
            "http://127.0.0.1:9".to_owned(),
            secrets,
            Arc::clone(&cache),
            notification,
            true,
            std::time::Duration::from_secs(1),
        );

        runtime.process_update(Update::Session(Some(Box::new(whoami("me")))));
        runtime.process_update(Update::Notifications(vec![Notification {
            id: "n1".to_owned(),
            message_id: Some("m1".to_owned()),
            channel_id: Some("general".to_owned()),
            author_id: Some("bia".to_owned()),
            message_content: Some("oi".to_owned()),
            read: false,
            created_at: None,
        }]));
        assert_eq!(deliveries.load(Ordering::SeqCst), 1);

        // Arbitrary reconciled Store changes do not pass through the
        // NotificationCoordinator; only Update::Notifications above does.
        runtime.process_update(Update::Server(None));
        assert_eq!(deliveries.load(Ordering::SeqCst), 1);

        drop(runtime);
        drop(cache);
        if let Some(dir) = path.parent() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn update_aplica_store_e_sinaliza_efeito_de_ui() {
        let (mut runtime, _) = disabled_runtime();
        let effects = runtime.process_update(Update::OutgoingRejected {
            content: "olá".to_owned(),
            reply_to: Some("m1".to_owned()),
            notify_reply: true,
            message: "falhou".to_owned(),
        });

        assert_eq!(runtime.store.error.as_deref(), Some("falhou"));
        assert!(matches!(
            effects.as_slice(),
            [RuntimeEffect::OutgoingRejected {
                content,
                reply_to: Some(reply_to),
                notify_reply: true,
            }] if content == "olá" && reply_to == "m1"
        ));
    }

    #[test]
    fn cache_ops_sao_drenados_pelo_runtime() {
        let path = temp_db_path("cache-ops");
        let cache = Arc::new(ClientDb::open(Some(path.clone())));
        assert!(cache.is_enabled());
        let notification = Arc::new(NotificationCoordinator::new(
            Arc::clone(&cache),
            None,
        ));
        let secrets = Arc::new(MemorySecretStore::default());
        let mut runtime = ServerRuntime::open(
            "http://127.0.0.1:9".to_owned(),
            Wake::noop(),
            secrets,
            Arc::clone(&cache),
            notification,
        );

        runtime.process_update(Update::Server(Some(Box::new(Server {
            id: "srv".to_owned(),
            name: "Papo".to_owned(),
            owner_id: None,
            owner_username: None,
            public: false,
            member_count: 1,
            channel_count: 1,
        }))));
        cache.flush();

        assert!(cache.stats().written > 0);
        drop(runtime);
        drop(cache);
        if let Some(dir) = path.parent() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn online_repetido_nao_duplica_refresh() {
        let (mut runtime, _) = disabled_runtime();
        runtime.store.screen = Screen::Chat;

        runtime.process_update(Update::Connection(Connection::Online));
        runtime.process_update(Update::Connection(Connection::Online));

        assert_eq!(runtime.diagnostics().reconnect_refreshes, 1);
    }

    #[test]
    fn troca_de_owner_isola_estado_reconstruivel() {
        let (mut runtime, _) = disabled_runtime();
        runtime.cached_owner = Some("antigo".to_owned());
        runtime.store.selected_channel = "segredo".to_owned();

        runtime.process_update(Update::Session(Some(Box::new(whoami("novo")))));

        assert_eq!(runtime.cached_owner(), Some("novo"));
        assert!(runtime.store.selected_channel.is_empty());
        assert_eq!(runtime.store.me, "novo");
    }

    #[test]
    fn session_none_limpa_estado_mas_nao_executa_forget() {
        let (mut runtime, secrets) = disabled_runtime();
        secrets
            .store(
                &runtime.server_key,
                Secret::SessionToken,
                "continua-durável",
            )
            .expect("guardar token");
        runtime.cached_owner = Some("ana".to_owned());
        runtime.store.me = "ana".to_owned();
        runtime.store.selected_channel = "geral".to_owned();

        runtime.process_update(Update::Session(None));

        assert_eq!(runtime.cached_owner(), None);
        assert_eq!(runtime.store.screen, Screen::Auth);
        assert_eq!(
            secrets
                .load(&runtime.server_key, Secret::SessionToken)
                .expect("ler token")
                .as_deref(),
            Some("continua-durável")
        );
    }

    #[test]
    fn drop_nao_esquece_credenciais_mas_forget_server_sim() {
        let (runtime, secrets) = disabled_runtime();
        let key = runtime.server_key.clone();
        secrets
            .store(&key, Secret::SessionToken, "token")
            .expect("guardar token");
        drop(runtime);
        assert_eq!(
            secrets
                .load(&key, Secret::SessionToken)
                .expect("ler token")
                .as_deref(),
            Some("token")
        );

        let cache = Arc::new(ClientDb::open(None));
        let notification = Arc::new(NotificationCoordinator::new(
            Arc::clone(&cache),
            None,
        ));
        let runtime = ServerRuntime::open(
            "http://127.0.0.1:9".to_owned(),
            Wake::noop(),
            secrets.clone(),
            cache,
            notification,
        );
        runtime.forget_server();
        assert!(
            secrets
                .load(&runtime.server_key, Secret::SessionToken)
                .expect("ler token")
                .is_none()
        );
    }

    #[test]
    fn runtimes_de_servidores_sao_isolados() {
        let (mut a, _) = disabled_runtime();

        let cache = Arc::new(ClientDb::open(None));
        let notification = Arc::new(NotificationCoordinator::new(
            Arc::clone(&cache),
            None,
        ));
        let secrets = Arc::new(MemorySecretStore::default());
        let b = ServerRuntime::open(
            "http://127.0.0.1:10".to_owned(),
            Wake::noop(),
            secrets,
            cache,
            notification,
        );

        a.process_update(Update::Session(Some(Box::new(whoami("conta-a")))));

        assert_eq!(a.store.me, "conta-a");
        assert!(b.store.me.is_empty());
        assert_eq!(b.cached_owner(), None);
        assert_ne!(a.server_key, b.server_key);
    }

    #[test]
    fn runtime_headless_nao_inicia_demanda_de_timeline_selecionada() {
        let (mut runtime, _) = disabled_runtime();
        runtime.store.screen = Screen::Chat;
        runtime.store.selected_channel = "geral".to_owned();

        runtime.process_update(Update::Connection(Connection::Online));

        // A demanda continua explícita para o consumidor foreground:
        // ServerRuntime não chama mark_loading nem LoadMessages sozinho.
        assert_eq!(
            runtime.store.channel_needing_messages().as_deref(),
            Some("geral")
        );
    }

    #[test]
    fn forget_server_limpa_namespace_duravel_e_credenciais() {
        let path = temp_db_path("forget");
        let cache = Arc::new(ClientDb::open(Some(path.clone())));
        let notification = Arc::new(NotificationCoordinator::new(
            Arc::clone(&cache),
            None,
        ));
        let secrets = Arc::new(MemorySecretStore::default());
        let mut runtime = ServerRuntime::open(
            "http://127.0.0.1:11".to_owned(),
            Wake::noop(),
            secrets.clone(),
            Arc::clone(&cache),
            notification,
        );
        secrets
            .store(&runtime.server_key, Secret::SessionToken, "token")
            .expect("guardar token");

        runtime.process_update(Update::Server(Some(Box::new(Server {
            id: "srv".to_owned(),
            name: "Papo".to_owned(),
            owner_id: None,
            owner_username: None,
            public: false,
            member_count: 1,
            channel_count: 1,
        }))));
        cache.flush();
        assert!(
            cache
                .load_snapshot(&runtime.server_key)
                .is_some_and(|snapshot| !snapshot.is_empty())
        );

        runtime.forget_server();
        cache.flush();

        assert!(
            cache
                .load_snapshot(&runtime.server_key)
                .is_some_and(|snapshot| snapshot.is_empty())
        );
        assert!(
            secrets
                .load(&runtime.server_key, Secret::SessionToken)
                .expect("ler token")
                .is_none()
        );

        drop(runtime);
        drop(cache);
        if let Some(dir) = path.parent() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

}
