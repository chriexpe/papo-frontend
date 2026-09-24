//! Android WorkManager bridge.
//!
//! Scheduling belongs to the foreground app; execution belongs to a cold-safe
//! JNI entry point that constructs only papo-core runtime/cache/storage pieces.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use jni::objects::{JClass, JString};
use jni::sys::{jboolean, jstring};
use jni::JNIEnv;
use papo_core::api::net::BackgroundRunResult;
use papo_core::notification::{NotificationCoordinator, NotificationSink};
use papo_core::runtime::ServerRuntime;
use serde::Serialize;

use super::android_message::NativeNotification;

const BACKGROUND_DEADLINE: Duration = Duration::from_secs(25);
const RECEIVE_SLICE: Duration = Duration::from_millis(100);

#[derive(Serialize)]
struct WorkServer {
    server_key: String,
    server_url: String,
}

#[derive(Serialize)]
struct WorkSync {
    notifications_enabled: bool,
    servers: Vec<WorkServer>,
}

static LAST_SCHEDULE: Mutex<Option<String>> = Mutex::new(None);

/// Mirrors the actual configured workspaces into WorkManager. Repeated frame
/// calls are cheap: JNI is only crossed when the serialized desired set changes.
pub fn sync_periodic<'a>(
    servers: impl IntoIterator<Item = (&'a str, &'a str)>,
    notifications_enabled: bool,
) {
    let payload = WorkSync {
        notifications_enabled,
        servers: servers
            .into_iter()
            .map(|(server_key, server_url)| WorkServer {
                server_key: server_key.to_owned(),
                server_url: server_url.to_owned(),
            })
            .collect(),
    };
    let Ok(payload) = serde_json::to_string(&payload) else {
        return;
    };
    if LAST_SCHEDULE
        .lock()
        .ok()
        .as_ref()
        .and_then(|value| value.as_ref())
        .is_some_and(|last| last == &payload)
    {
        return;
    }
    if super::jvm::call_activity(
        "syncBackgroundReconcile",
        "(Ljava/lang/String;)V",
        Some(&payload),
    ) && let Ok(mut last) = LAST_SCHEDULE.lock()
    {
        *last = Some(payload);
    }
}

fn cancelled() -> &'static Mutex<HashSet<String>> {
    static CANCELLED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    CANCELLED.get_or_init(|| Mutex::new(HashSet::new()))
}

fn clear_cancelled(server_key: &str) {
    if let Ok(mut values) = cancelled().lock() {
        values.remove(server_key);
    }
}

fn is_cancelled(server_key: &str) -> bool {
    cancelled()
        .lock()
        .map(|values| values.contains(server_key))
        .unwrap_or(true)
}

fn mark_cancelled(server_key: String) {
    if let Ok(mut values) = cancelled().lock() {
        values.insert(server_key);
    }
}

fn read_string(env: &mut JNIEnv<'_>, value: JString<'_>) -> Option<String> {
    env.get_string(&value).ok().map(Into::into)
}

fn result_json(status: &'static str, retryable: bool) -> String {
    serde_json::json!({
        "status": status,
        "retryable": retryable,
        "notifications": [],
        "diagnostics": {
            "duration_ms": 0,
            "updates_processed": 0,
        }
    })
    .to_string()
}

fn to_jstring(env: &mut JNIEnv<'_>, value: String) -> jstring {
    env.new_string(value)
        .map(JString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}

fn status(result: BackgroundRunResult) -> (&'static str, bool) {
    match result {
        BackgroundRunResult::Completed => ("completed", false),
        BackgroundRunResult::NoSession => ("no_session", false),
        BackgroundRunResult::PermanentAuthFailure => ("permanent_auth_failure", false),
        BackgroundRunResult::ServerLocked => ("server_locked", false),
        BackgroundRunResult::TransientFailure => ("transient_failure", true),
        BackgroundRunResult::Deadline => ("deadline", true),
        BackgroundRunResult::Cancelled => ("cancelled", false),
    }
}

#[derive(Serialize)]
struct Diagnostics {
    duration_ms: u128,
    updates_processed: usize,
    delivered: u64,
    duplicate: u64,
}

#[derive(Serialize)]
struct NativeResult {
    status: &'static str,
    retryable: bool,
    notifications: Vec<NativeNotification>,
    diagnostics: Diagnostics,
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chriexpe_papo_PapoReconcileWorker_nativeRunBackgroundReconcile(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    app_private_root: JString<'_>,
    server_key: JString<'_>,
    server_url: JString<'_>,
    notifications_enabled: jboolean,
    platform_notifications_available: jboolean,
) -> jstring {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Debug)
            .with_tag("papo"),
    );

    let Some(root) = read_string(&mut env, app_private_root) else {
        return to_jstring(&mut env, result_json("transient_failure", true));
    };
    let Some(server_key) = read_string(&mut env, server_key) else {
        return to_jstring(&mut env, result_json("transient_failure", true));
    };
    let Some(server_url) = read_string(&mut env, server_url) else {
        return to_jstring(&mut env, result_json("transient_failure", true));
    };

    if papo_core::server_key(&server_url) != server_key {
        log::warn!("background {server_key}: stale/mismatched server work ignored");
        return to_jstring(&mut env, result_json("permanent_auth_failure", false));
    }

    let Some(_lease) = super::runtime_lease::try_acquire_headless(&server_key) else {
        log::info!("background {server_key}: skipped interactive runtime active");
        return to_jstring(
            &mut env,
            result_json("skipped_foreground_runtime", false),
        );
    };

    clear_cancelled(&server_key);
    super::dirs::set_root(PathBuf::from(root));
    if let Some(data_dir) = super::dirs::data_dir()
        && let Err(error) = std::fs::create_dir_all(&data_dir)
    {
        log::warn!("background {server_key}: app-private data dir unavailable: {error}");
        return to_jstring(&mut env, result_json("transient_failure", true));
    }

    let started = Instant::now();
    log::info!("background {server_key}: started");

    let cache = super::client_db::get();
    let envelopes = Arc::new(Mutex::new(Vec::<NativeNotification>::new()));
    let platform_available = platform_notifications_available != 0;
    let sink: Option<NotificationSink> = if platform_available {
        let envelopes = Arc::clone(&envelopes);
        Some(Arc::new(move |envelope| {
            if let Ok(mut list) = envelopes.lock() {
                list.push(NativeNotification::from_envelope(envelope));
            }
        }))
    } else {
        None
    };
    let notification = Arc::new(NotificationCoordinator::new(Arc::clone(&cache), sink));
    notification.set_foreground(false);

    let mut runtime = ServerRuntime::open_background(
        server_url,
        Arc::new(crate::storage::FileSecretStore::new()),
        Arc::clone(&cache),
        Arc::clone(&notification),
        notifications_enabled != 0,
        BACKGROUND_DEADLINE,
    );

    let mut updates_processed = 0usize;
    while runtime.background_result().is_none() {
        if is_cancelled(&server_key) || super::runtime_lease::foreground_requested() {
            runtime.cancel_background();
        }
        if runtime.recv_timeout(RECEIVE_SLICE).is_some() {
            updates_processed = updates_processed.saturating_add(1);
        }
        // The core owns the real deadline. This outer ceiling only prevents a
        // broken terminal marker from pinning the synchronous Worker forever.
        if started.elapsed() > BACKGROUND_DEADLINE + Duration::from_secs(5) {
            runtime.cancel_background();
            break;
        }
    }

    // Drain any update that was queued immediately before the terminal marker,
    // then make accepted cache writes durable before Java returns success.
    while runtime.try_drain().is_some() {
        updates_processed = updates_processed.saturating_add(1);
    }
    cache.flush();

    let run_result = runtime
        .background_result()
        .unwrap_or(BackgroundRunResult::Deadline);
    let (status, retryable) = status(run_result);
    let notification_stats = notification.diagnostics(&server_key);
    let notifications = envelopes
        .lock()
        .map(|mut list| std::mem::take(&mut *list))
        .unwrap_or_default();
    let result = NativeResult {
        status,
        retryable,
        notifications,
        diagnostics: Diagnostics {
            duration_ms: started.elapsed().as_millis(),
            updates_processed,
            delivered: notification_stats.delivered,
            duplicate: notification_stats.duplicate_claims,
        },
    };
    clear_cancelled(&server_key);
    log::info!(
        "background {server_key}: {status} duration_ms={} updates={} delivered={} duplicate={}",
        result.diagnostics.duration_ms,
        result.diagnostics.updates_processed,
        result.diagnostics.delivered,
        result.diagnostics.duplicate,
    );
    to_jstring(
        &mut env,
        serde_json::to_string(&result)
            .unwrap_or_else(|_| result_json("transient_failure", true)),
    )
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chriexpe_papo_PapoReconcileWorker_nativeCancelBackgroundReconcile(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    server_key: JString<'_>,
) {
    if let Some(server_key) = read_string(&mut env, server_key) {
        mark_cancelled(server_key);
    }
}
