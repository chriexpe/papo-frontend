//! Android supplies lifecycle and rendering. Core owns policy, deduplication,
//! authenticated requests and bounded background reconciliation.
use super::*;
use crate::bindings::{Backend, CoreError, Credentials, ForeignSecrets, NativeClient};
use papo_core::api::net::BackgroundRunResult;
use papo_core::notification::{NotificationCoordinator, NotificationEnvelope, NotificationSink};
use papo_core::runtime::RuntimeNotificationView;
use std::sync::Condvar;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Debug, uniffi::Record)]
pub struct MessageAlert {
    pub server_url: String,
    pub owner_user_id: String,
    pub channel_id: String,
    pub message_id: String,
    pub notification_id: String,
    pub title: String,
    pub body: String,
}
impl From<NotificationEnvelope> for MessageAlert {
    fn from(e: NotificationEnvelope) -> Self {
        Self {
            server_url: e.navigation_server,
            owner_user_id: e.owner_user_id,
            channel_id: e.channel_id,
            message_id: e.message_id,
            notification_id: e.notification_id.unwrap_or_default(),
            title: e.title,
            body: e.body,
        }
    }
}
#[uniffi::export(with_foreign)]
pub trait NotificationListener: Send + Sync {
    fn available(&self) -> bool;
    fn is_foreground(&self) -> bool;
    fn show(&self, alert: MessageAlert);
}

#[derive(Default)]
struct Owners {
    interactive: usize,
    background: Option<Arc<AtomicBool>>,
}
static OWNERS: OnceLock<(Mutex<HashMap<String, Owners>>, Condvar)> = OnceLock::new();
fn owners() -> &'static (Mutex<HashMap<String, Owners>>, Condvar) {
    OWNERS.get_or_init(Default::default)
}
fn lease_key(url: &str, path: &std::path::Path) -> String {
    format!("{}\n{url}", path.display())
}
/// Interactive startup cancels and joins an existing headless owner before
/// either transport can rotate the same refresh token or write session state.
pub(super) struct InteractiveLease(String);
impl InteractiveLease {
    pub(super) fn acquire(url: &str, path: &std::path::Path) -> Self {
        let key = lease_key(url, path);
        let (lock, changed) = owners();
        let mut all = lock.lock().unwrap_or_else(|e| e.into_inner());
        all.entry(key.clone()).or_default().interactive += 1;
        while let Some(cancel) = all.get(&key).and_then(|entry| entry.background.as_ref()) {
            cancel.store(true, Ordering::Release);
            all = changed.wait(all).unwrap_or_else(|e| e.into_inner());
        }
        Self(key)
    }
}
impl Drop for InteractiveLease {
    fn drop(&mut self) {
        let mut all = owners().0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = all.get_mut(&self.0) {
            entry.interactive = entry.interactive.saturating_sub(1);
            if entry.interactive == 0 && entry.background.is_none() {
                all.remove(&self.0);
            }
        }
    }
}
struct BackgroundLease(String);
impl BackgroundLease {
    fn acquire(key: String, cancelled: Arc<AtomicBool>) -> Option<Self> {
        let mut all = owners().0.lock().unwrap_or_else(|e| e.into_inner());
        let entry = all.entry(key.clone()).or_default();
        if entry.interactive > 0 || entry.background.is_some() {
            return None;
        }
        entry.background = Some(cancelled);
        Some(Self(key))
    }
}
impl Drop for BackgroundLease {
    fn drop(&mut self) {
        let (lock, changed) = owners();
        let mut all = lock.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = all.get_mut(&self.0) {
            entry.background = None;
            if entry.interactive == 0 {
                all.remove(&self.0);
            }
        }
        changed.notify_all();
    }
}

pub(super) fn coordinator(
    cache: Arc<ClientDb>,
    listener: Option<Arc<dyn NotificationListener>>,
) -> Arc<NotificationCoordinator> {
    match listener {
        Some(listener) => {
            let sink = Arc::clone(&listener);
            Arc::new(NotificationCoordinator::with_foreground_probe(
                cache,
                Some(Arc::new(move |alert| sink.show(alert.into()))),
                Arc::new(move || listener.is_foreground()),
            ))
        }
        None => Arc::new(NotificationCoordinator::new(cache, None)),
    }
}
impl Client {
    pub(super) fn sync_notification_context(&self) {
        self.runtime
            .sync_notification_context(RuntimeNotificationView {
                server_label: self
                    .runtime
                    .store
                    .server
                    .as_ref()
                    .map(|s| s.name.clone())
                    .unwrap_or_else(|| self.runtime.url.clone()),
                visible_server: self.notification_visible,
                notifications_enabled: self
                    .notification_listener
                    .as_ref()
                    .is_some_and(|l| l.available()),
                push_registered: self
                    .push_registration
                    .as_ref()
                    .is_some_and(|(owner, _)| owner == &self.runtime.store.me),
            });
    }
}
#[uniffi::export]
impl NativeClient {
    #[uniffi::constructor]
    pub fn open_notifying(
        url: String,
        database: String,
        credentials: Arc<dyn Credentials>,
        listener: Arc<dyn crate::bindings::WakeListener>,
        notifications: Arc<dyn NotificationListener>,
    ) -> Result<Arc<Self>, CoreError> {
        let client = Client::open_notifying(
            &url,
            PathBuf::from(database),
            Arc::new(ForeignSecrets(credentials)),
            Wake::new(move || listener.on_wake()),
            Some(notifications),
        )?;
        Ok(Arc::new(Self {
            backend: Mutex::new(Some(Backend::Live(Box::new(client)))),
            previews: Default::default(),
        }))
    }
    pub fn notification_visibility(&self, visible: bool) {
        if let Ok(mut backend) = self.backend.lock()
            && let Some(Backend::Live(c)) = backend.as_mut()
        {
            c.notification_visible = visible;
            c.sync_notification_context();
        }
    }
    /// A foreground FCM wake is verified against this account's backend inbox;
    /// its payload is never trusted as the identity or contents of an alert.
    pub fn reconcile_alerts(&self, owner: String, message_id: String) -> Result<(), CoreError> {
        let request = self.scoped_api(&owner)?;
        let notices = account::run(request.api.notifications(&owner))?;
        self.finish_request(&request, vec![Update::Notifications(notices.clone())])?;
        let mut backend = self
            .backend
            .lock()
            .map_err(|_| "Client lock failed".to_owned())?;
        if let Some(Backend::Live(c)) = backend.as_mut()
            && c.runtime.store.me == owner
        {
            c.sync_notification_context();
            let candidates = notices
                .into_iter()
                .filter(|n| message_id.is_empty() || n.message_id.as_deref() == Some(&message_id))
                .collect::<Vec<_>>();
            c.runtime.deliver_notifications(&candidates);
        }
        Ok(())
    }

    /// Uses frozen, owner-scoped credentials. A response for an earlier account
    /// cannot enable push suppression for the account that replaced it.
    pub fn push_device(
        &self,
        owner: String,
        token: String,
        device_name: String,
        enabled: bool,
    ) -> Result<(), CoreError> {
        let request = self.scoped_api(&owner)?;
        if enabled {
            account::run(request.api.register_push_device(
                &papo_core::api::models::PushDeviceRequest {
                    token: token.clone(),
                    platform: Some("android".into()),
                    device_name: Some(device_name.chars().take(32).collect()),
                },
            ))?;
        } else {
            account::run(request.api.unregister_push_device(&token))?;
        }
        self.finish_request(&request, vec![])?;
        if let Ok(mut backend) = self.backend.lock()
            && let Some(Backend::Live(c)) = backend.as_mut()
            && c.runtime.store.me == owner
        {
            if enabled {
                c.push_registration = Some((owner, token));
            } else if c
                .push_registration
                .as_ref()
                .is_some_and(|(_, known)| known == &token)
            {
                c.push_registration = None;
            }
            c.sync_notification_context();
        }
        Ok(())
    }
}

#[derive(uniffi::Object)]
pub struct NotificationReconcile {
    cancelled: Arc<AtomicBool>,
}
#[uniffi::export]
impl NotificationReconcile {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            cancelled: Arc::new(AtomicBool::new(false)),
        })
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
    /// Runs on WorkManager's IO thread, never the UI or the interactive actor.
    pub fn run(
        &self,
        url: String,
        database_path: String,
        credentials: Arc<dyn Credentials>,
        listener: Arc<dyn NotificationListener>,
    ) -> Result<bool, CoreError> {
        let url = normalize_url(&url)?;
        let path = PathBuf::from(database_path);
        let Some(_lease) =
            BackgroundLease::acquire(lease_key(&url, &path), Arc::clone(&self.cancelled))
        else {
            return Ok(false);
        };
        if self.cancelled.load(Ordering::Acquire) || !listener.available() {
            return Ok(false);
        }
        let cache = database(path)?;
        let sink: NotificationSink = Arc::new(move |alert| listener.show(alert.into()));
        let notification = Arc::new(NotificationCoordinator::new(Arc::clone(&cache), Some(sink)));
        let mut runtime = ServerRuntime::open_background(
            url,
            Arc::new(ForeignSecrets(credentials)),
            Arc::clone(&cache),
            notification,
            true,
            Duration::from_secs(25),
        );
        let started = std::time::Instant::now();
        while runtime.background_result().is_none() {
            if started.elapsed() > Duration::from_secs(30) {
                runtime.cancel_background();
                break;
            }
            if self.cancelled.load(Ordering::Acquire) {
                runtime.cancel_background()
            }
            runtime.recv_timeout(Duration::from_millis(100));
        }
        runtime.wait_background_shutdown();
        while runtime.try_drain().is_some() {}
        cache.flush();
        Ok(matches!(
            runtime.background_result(),
            Some(BackgroundRunResult::TransientFailure | BackgroundRunResult::Deadline)
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use papo_core::notification::{CandidateSource, NotificationContext, NotificationOutcome};

    #[derive(Default)]
    struct Listener {
        foreground: AtomicBool,
        alerts: Mutex<Vec<MessageAlert>>,
    }
    impl NotificationListener for Listener {
        fn available(&self) -> bool {
            true
        }
        fn is_foreground(&self) -> bool {
            self.foreground.load(Ordering::Acquire)
        }
        fn show(&self, alert: MessageAlert) {
            self.alerts.lock().unwrap().push(alert)
        }
    }
    fn path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "papo-alert-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }
    #[test]
    fn native_alert_callback_preserves_owner_and_uses_core_foreground_and_dedupe_policy() {
        let root = path("bridge");
        std::fs::create_dir_all(&root).unwrap();
        let cache = Arc::new(ClientDb::open(Some(root.join("cache.db"))));
        let listener = Arc::new(Listener::default());
        let coordinator = coordinator(Arc::clone(&cache), Some(listener.clone()));
        coordinator.sync_context(NotificationContext {
            server_key: "server".into(),
            navigation_server: "https://papo.example".into(),
            owner_user_id: "owner".into(),
            selected_channel: "general".into(),
            visible_server: true,
            notifications_enabled: true,
            ..Default::default()
        });
        let notice = |id: &str| {
            serde_json::from_value(json!({"id":id,"channel_id":"general","message_id":id,"author_id":"other","read":false,"message_content":"Hello"})).unwrap()
        };
        assert_eq!(
            coordinator.handle_notification(
                "server",
                &notice("cached"),
                CandidateSource::CacheRestore
            ),
            NotificationOutcome::Ignored
        );
        listener.foreground.store(true, Ordering::Release);
        assert!(matches!(
            coordinator.handle_notification("server", &notice("visible"), CandidateSource::Live),
            NotificationOutcome::Suppressed(_)
        ));
        listener.foreground.store(false, Ordering::Release);
        assert_eq!(
            coordinator.handle_notification("server", &notice("live"), CandidateSource::Live),
            NotificationOutcome::Delivered
        );
        assert_eq!(
            coordinator.handle_notification("server", &notice("live"), CandidateSource::Background),
            NotificationOutcome::Duplicate
        );
        let alerts = listener.alerts.lock().unwrap();
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].owner_user_id, "owner");
        assert_eq!(alerts[0].server_url, "https://papo.example");
        drop(alerts);
        drop(coordinator);
        drop(cache);
        let _ = std::fs::remove_dir_all(root);
    }
    #[test]
    fn interactive_startup_preempts_and_waits_for_the_background_transport() {
        let path = path("lease");
        let url = "https://papo.example";
        let cancelled = Arc::new(AtomicBool::new(false));
        let background =
            BackgroundLease::acquire(lease_key(url, &path), Arc::clone(&cancelled)).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let other = path.clone();
        let foreground = std::thread::spawn(move || {
            let lease = InteractiveLease::acquire(url, &other);
            tx.send(()).unwrap();
            lease
        });
        let until = std::time::Instant::now() + Duration::from_secs(2);
        while !cancelled.load(Ordering::Acquire) {
            assert!(std::time::Instant::now() < until);
            std::thread::yield_now()
        }
        assert!(rx.try_recv().is_err());
        drop(background);
        rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let lease = foreground.join().unwrap();
        assert!(
            BackgroundLease::acquire(lease_key(url, &path), Arc::new(AtomicBool::new(false)))
                .is_none()
        );
        drop(lease);
        assert!(
            BackgroundLease::acquire(lease_key(url, &path), Arc::new(AtomicBool::new(false)))
                .is_some()
        );
    }
    #[test]
    fn member_projection_reuses_the_cached_main_role_color_without_fetching_a_profile() {
        let client = NativeClient::demo();
        let mut backend = client.backend.lock().unwrap();
        let Some(Backend::Demo(demo)) = backend.as_mut() else {
            panic!()
        };
        demo.store.profiles.clear();
        let view = demo.poll();
        assert_eq!(
            view["members"]
                .as_array()
                .unwrap()
                .iter()
                .find(|v| v["id"] == "ana")
                .unwrap()["roleColor"],
            "#67B989"
        );
        assert_eq!(
            view["members"]
                .as_array()
                .unwrap()
                .iter()
                .find(|v| v["id"] == "bruno")
                .unwrap()["roleColor"],
            ""
        );
    }
}
