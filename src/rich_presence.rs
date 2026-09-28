//! Desktop Rich Presence acquisition.
//!
//! Papo prefers an already-running arRPC-compatible bridge. That lets
//! Equibop/Vesktop/arRPC and Papo consume the same local activity without
//! duplicating process scanners or competing for Discord RPC IPC slots.
//! When no bridge exists, the built-in collector implements only the pieces
//! Papo needs: Discord RPC SET_ACTIVITY plus a lightweight process detector.
//! Android never starts a collector; it only renders activities received
//! from Papo servers.

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};

use crate::state::{Activity, ActivityKind};

const DEFAULT_HOST: &str = "127.0.0.1";
const DEFAULT_PORT: &str = "1337";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum OverrideKind {
    Listening,
    #[default]
    Playing,
    Working,
}

impl OverrideKind {
    pub fn activity_kind(self) -> ActivityKind {
        match self {
            Self::Listening => ActivityKind::Listening,
            Self::Playing => ActivityKind::Playing,
            Self::Working => ActivityKind::Working,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityOverride {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub kind: OverrideKind,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub details: String,
    #[serde(default)]
    pub state: String,
    /// Starts a timer when the override becomes active.
    #[serde(default = "yes")]
    pub elapsed: bool,
    /// Optional total duration. Empty/zero means no end timestamp.
    #[serde(default)]
    pub duration_minutes: String,
}

impl Default for ActivityOverride {
    fn default() -> Self {
        Self {
            enabled: false,
            kind: OverrideKind::Playing,
            name: String::new(),
            details: String::new(),
            state: String::new(),
            elapsed: true,
            duration_minutes: String::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Prefer the Papo collector when a shared local arRPC bridge is absent.
    #[serde(default = "yes")]
    pub built_in: bool,
    #[serde(default = "yes")]
    pub game_detection: bool,
    #[serde(default = "yes")]
    pub auto_reconnect: bool,
    #[serde(default)]
    pub debug: bool,
    #[serde(default = "default_host")]
    pub external_host: String,
    #[serde(default = "default_port")]
    pub external_port: String,
    #[serde(default)]
    pub override_activity: ActivityOverride,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: true,
            built_in: true,
            game_detection: true,
            auto_reconnect: true,
            debug: false,
            external_host: default_host(),
            external_port: default_port(),
            override_activity: ActivityOverride::default(),
        }
    }
}

fn yes() -> bool {
    true
}

fn default_host() -> String {
    DEFAULT_HOST.to_owned()
}

fn default_port() -> String {
    DEFAULT_PORT.to_owned()
}

impl Settings {
    pub fn external_port_number(&self) -> u16 {
        self.external_port.trim().parse().unwrap_or(1337)
    }

    fn endpoint(&self, shared_local: bool) -> String {
        let host = if shared_local {
            DEFAULT_HOST
        } else {
            self.external_host.trim()
        };
        let host = if host.parse::<std::net::Ipv6Addr>().is_ok() {
            format!("[{host}]")
        } else if host.is_empty() {
            DEFAULT_HOST.to_owned()
        } else {
            host.to_owned()
        };
        let port = if shared_local {
            1337
        } else {
            self.external_port_number()
        };
        format!("ws://{host}:{port}")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Disabled,
    Connecting,
    External,
    BuiltIn,
    Override,
    Unsupported,
    Error,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub source: Source,
    pub detail: String,
    pub activity: Option<Activity>,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            source: Source::Connecting,
            detail: String::new(),
            activity: None,
        }
    }
}

impl ActivityOverride {
    pub fn preview(&self) -> Option<Activity> {
        self.activity(Utc::now())
    }

    fn activity(&self, started: DateTime<Utc>) -> Option<Activity> {
        let name = self.name.trim();
        if !self.enabled || name.is_empty() {
            return None;
        }
        let started_at = self.elapsed.then_some(started);
        let duration = self
            .duration_minutes
            .trim()
            .parse::<i64>()
            .ok()
            .filter(|minutes| *minutes > 0);
        Some(Activity {
            kind: self.kind.activity_kind(),
            name: name.to_owned(),
            details: nonempty(&self.details),
            state: nonempty(&self.state),
            started_at,
            ends_at: started_at.zip(duration).map(|(start, minutes)| {
                start + ChronoDuration::minutes(minutes)
            }),
        })
    }
}

fn nonempty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

pub struct Manager {
    settings: Settings,
    snapshot: Snapshot,
    #[cfg(not(target_os = "android"))]
    control: tokio::sync::mpsc::UnboundedSender<Control>,
    #[cfg(not(target_os = "android"))]
    events: std::sync::mpsc::Receiver<Snapshot>,
}

impl Manager {
    pub fn new(settings: Settings, repaint: egui::Context) -> Self {
        #[cfg(target_os = "android")]
        {
            let _ = repaint;
            let snapshot = android_snapshot(&settings);
            return Self { settings, snapshot };
        }

        #[cfg(not(target_os = "android"))]
        {
            let (control_tx, control_rx) = tokio::sync::mpsc::unbounded_channel();
            let (event_tx, event_rx) = std::sync::mpsc::channel();
            let worker_settings = settings.clone();
            std::thread::Builder::new()
                .name("papo-rich-presence".to_owned())
                .spawn(move || {
                    let runtime = tokio::runtime::Builder::new_multi_thread()
                        .worker_threads(1)
                        .enable_all()
                        .build();
                    match runtime {
                        Ok(runtime) => runtime.block_on(worker(
                            worker_settings,
                            control_rx,
                            event_tx,
                            repaint,
                        )),
                        Err(error) => log::warn!(
                            "rich presence: não foi possível iniciar o runtime: {error}"
                        ),
                    }
                })
                .ok();
            Self {
                settings,
                snapshot: Snapshot::default(),
                control: control_tx,
                events: event_rx,
            }
        }
    }

    pub fn configure(&mut self, settings: Settings) {
        if self.settings == settings {
            return;
        }
        self.settings = settings.clone();

        #[cfg(target_os = "android")]
        {
            self.snapshot = android_snapshot(&settings);
        }
        #[cfg(not(target_os = "android"))]
        {
            let _ = self.control.send(Control::Configure(settings));
        }
    }

    pub fn restart(&mut self) {
        #[cfg(target_os = "android")]
        {
            self.snapshot = android_snapshot(&self.settings);
        }
        #[cfg(not(target_os = "android"))]
        {
            let _ = self.control.send(Control::Restart);
        }
    }

    /// Drain worker updates. Returns true when the visible state changed.
    pub fn pump(&mut self) -> bool {
        #[cfg(target_os = "android")]
        {
            false
        }
        #[cfg(not(target_os = "android"))]
        {
            let mut changed = false;
            while let Ok(next) = self.events.try_recv() {
                if self.snapshot != next {
                    self.snapshot = next;
                    changed = true;
                }
            }
            changed
        }
    }

    pub fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }
}

#[cfg(target_os = "android")]
fn android_snapshot(settings: &Settings) -> Snapshot {
    if !settings.enabled {
        Snapshot {
            source: Source::Disabled,
            detail: "Rich Presence is disabled".to_owned(),
            activity: None,
        }
    } else if settings.override_activity.enabled {
        Snapshot {
            source: Source::Override,
            detail: "Manual override".to_owned(),
            activity: settings.override_activity.activity(Utc::now()),
        }
    } else {
        Snapshot {
            source: Source::Unsupported,
            detail: "Desktop only".to_owned(),
            activity: None,
        }
    }
}

#[cfg(not(target_os = "android"))]
#[derive(Debug)]
enum Control {
    Configure(Settings),
    Restart,
}

#[cfg(not(target_os = "android"))]
enum RunExit {
    Control(Control),
    Disconnected(String),
    PreferExternal,
    Closed,
}

#[cfg(not(target_os = "android"))]
fn publish(
    tx: &std::sync::mpsc::Sender<Snapshot>,
    repaint: &egui::Context,
    source: Source,
    detail: impl Into<String>,
    activity: Option<Activity>,
) {
    let _ = tx.send(Snapshot {
        source,
        detail: detail.into(),
        activity,
    });
    repaint.request_repaint();
}

#[cfg(not(target_os = "android"))]
async fn worker(
    mut settings: Settings,
    mut controls: tokio::sync::mpsc::UnboundedReceiver<Control>,
    events: std::sync::mpsc::Sender<Snapshot>,
    repaint: egui::Context,
) {
    loop {
        if !settings.enabled {
            publish(
                &events,
                &repaint,
                Source::Disabled,
                "Rich Presence disabled",
                None,
            );
            match controls.recv().await {
                Some(Control::Configure(next)) => settings = next,
                Some(Control::Restart) => {}
                None => return,
            }
            continue;
        }

        if settings.override_activity.enabled {
            let activity = settings.override_activity.activity(Utc::now());
            publish(
                &events,
                &repaint,
                Source::Override,
                "Manual override",
                activity,
            );
            match controls.recv().await {
                Some(Control::Configure(next)) => settings = next,
                Some(Control::Restart) => {}
                None => return,
            }
            continue;
        }

        // With the built-in collector enabled, a shared local arRPC bridge
        // always wins. Equibop/Vesktop can therefore own collection while
        // Papo is merely another bridge subscriber.
        let shared_local = settings.built_in;
        let endpoint = settings.endpoint(shared_local);
        publish(
            &events,
            &repaint,
            Source::Connecting,
            endpoint.clone(),
            None,
        );
        match run_external(
            &settings,
            &endpoint,
            shared_local,
            &mut controls,
            &events,
            &repaint,
        )
        .await
        {
            RunExit::Control(Control::Configure(next)) => {
                settings = next;
                continue;
            }
            RunExit::Control(Control::Restart) => continue,
            RunExit::Closed => return,
            RunExit::Disconnected(error) if settings.built_in => {
                if settings.debug {
                    log::debug!("rich presence: shared arRPC unavailable: {error}");
                }
            }
            RunExit::Disconnected(error) => {
                publish(&events, &repaint, Source::Error, error, None);
                if !settings.auto_reconnect {
                    match controls.recv().await {
                        Some(Control::Configure(next)) => settings = next,
                        Some(Control::Restart) => {}
                        None => return,
                    }
                    continue;
                }
                match wait_or_control(&mut controls, std::time::Duration::from_secs(2)).await {
                    Some(Control::Configure(next)) => settings = next,
                    Some(Control::Restart) => {}
                    None if controls.is_closed() => return,
                    None => {}
                }
                continue;
            }
            RunExit::PreferExternal => continue,
        }

        // Shared bridge was absent: use Papo's own Discord-compatible
        // collector. It never opens a localhost bridge of its own.
        match run_builtin(
            &settings,
            &mut controls,
            &events,
            &repaint,
        )
        .await
        {
            RunExit::Control(Control::Configure(next)) => settings = next,
            RunExit::Control(Control::Restart) | RunExit::PreferExternal => {}
            RunExit::Disconnected(error) => {
                publish(&events, &repaint, Source::Error, error, None);
                match wait_or_control(&mut controls, std::time::Duration::from_secs(2)).await {
                    Some(Control::Configure(next)) => settings = next,
                    Some(Control::Restart) => {}
                    None if controls.is_closed() => return,
                    None => {}
                }
            }
            RunExit::Closed => return,
        }
    }
}

#[cfg(not(target_os = "android"))]
async fn wait_or_control(
    controls: &mut tokio::sync::mpsc::UnboundedReceiver<Control>,
    duration: std::time::Duration,
) -> Option<Control> {
    tokio::select! {
        control = controls.recv() => control,
        _ = tokio::time::sleep(duration) => None,
    }
}

#[cfg(not(target_os = "android"))]
async fn run_external(
    settings: &Settings,
    endpoint: &str,
    shared_local: bool,
    controls: &mut tokio::sync::mpsc::UnboundedReceiver<Control>,
    events: &std::sync::mpsc::Sender<Snapshot>,
    repaint: &egui::Context,
) -> RunExit {
    use futures_util::StreamExt;

    let timeout = if shared_local {
        std::time::Duration::from_millis(450)
    } else {
        std::time::Duration::from_secs(4)
    };
    let connection = tokio::time::timeout(timeout, tokio_tungstenite::connect_async(endpoint)).await;
    let (stream, _) = match connection {
        Ok(Ok(connection)) => connection,
        Ok(Err(error)) => {
            return RunExit::Disconnected(format!("arRPC {endpoint}: {error}"));
        }
        Err(_) => {
            return RunExit::Disconnected(format!("arRPC {endpoint}: connection timed out"));
        }
    };

    if settings.debug {
        log::debug!("rich presence: connected to shared arRPC bridge {endpoint}");
    }
    publish(events, repaint, Source::External, endpoint, None);

    let (_, mut reader) = stream.split();
    let mut activities: std::collections::HashMap<String, (u64, Activity)> =
        std::collections::HashMap::new();
    let mut revision = 0u64;

    loop {
        tokio::select! {
            control = controls.recv() => {
                return control.map(RunExit::Control).unwrap_or(RunExit::Closed);
            }
            message = reader.next() => {
                let Some(message) = message else {
                    return RunExit::Disconnected(format!("arRPC {endpoint}: connection closed"));
                };
                let message = match message {
                    Ok(message) => message,
                    Err(error) => return RunExit::Disconnected(format!("arRPC {endpoint}: {error}")),
                };
                let text = match message {
                    tokio_tungstenite::tungstenite::Message::Text(text) => text,
                    tokio_tungstenite::tungstenite::Message::Binary(bytes) => {
                        match String::from_utf8(bytes.to_vec()) {
                            Ok(text) => text.into(),
                            Err(_) => continue,
                        }
                    }
                    tokio_tungstenite::tungstenite::Message::Close(_) => {
                        return RunExit::Disconnected(format!("arRPC {endpoint}: connection closed"));
                    }
                    _ => continue,
                };
                if let Some((socket, activity)) = parse_bridge_message(text.as_ref()) {
                    revision = revision.wrapping_add(1);
                    match activity {
                        Some(activity) => {
                            activities.insert(socket, (revision, activity));
                        }
                        None => {
                            activities.remove(&socket);
                        }
                    }
                    let current = activities
                        .values()
                        .max_by_key(|(seen, _)| *seen)
                        .map(|(_, activity)| activity.clone());
                    publish(events, repaint, Source::External, endpoint, current);
                } else if settings.debug {
                    log::debug!("rich presence: ignored unrecognized arRPC bridge frame");
                }
            }
        }
    }
}

#[cfg(not(target_os = "android"))]
fn parse_bridge_message(text: &str) -> Option<(String, Option<Activity>)> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    if value.get("activity").is_none() {
        return None;
    }
    let socket = value
        .get("socketId")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .or_else(|| {
            value
                .get("socketId")
                .and_then(serde_json::Value::as_u64)
                .map(|id| id.to_string())
        })
        .or_else(|| {
            value
                .get("pid")
                .and_then(serde_json::Value::as_u64)
                .map(|pid| pid.to_string())
        })
        .unwrap_or_else(|| "activity".to_owned());
    let activity = value
        .get("activity")
        .filter(|activity| !activity.is_null())
        .and_then(activity_from_value);
    Some((socket, activity))
}

#[cfg(not(target_os = "android"))]
fn activity_from_value(value: &serde_json::Value) -> Option<Activity> {
    let kind = match value
        .get("type")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0)
    {
        2 => ActivityKind::Listening,
        _ => ActivityKind::Playing,
    };
    let application_id = value
        .get("application_id")
        .and_then(serde_json::Value::as_str);
    let name = value
        .get("name")
        .and_then(serde_json::Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .map(str::to_owned)
        .or_else(|| application_id.map(|id| format!("App {id}")))?;
    let timestamps = value.get("timestamps");
    Some(Activity {
        kind,
        name,
        details: value
            .get("details")
            .and_then(serde_json::Value::as_str)
            .and_then(nonempty),
        state: value
            .get("state")
            .and_then(serde_json::Value::as_str)
            .and_then(nonempty),
        started_at: timestamps
            .and_then(|value| value.get("start"))
            .and_then(timestamp),
        ends_at: timestamps
            .and_then(|value| value.get("end"))
            .and_then(timestamp),
    })
}

#[cfg(not(target_os = "android"))]
fn timestamp(value: &serde_json::Value) -> Option<DateTime<Utc>> {
    let raw = value
        .as_i64()
        .or_else(|| value.as_str()?.parse::<i64>().ok())?;
    let millis = if raw.abs() < 10_000_000_000 {
        raw.saturating_mul(1000)
    } else {
        raw
    };
    DateTime::<Utc>::from_timestamp_millis(millis)
}

// -------------------------------------------------------------------------
// Built-in collector
// -------------------------------------------------------------------------

#[cfg(not(target_os = "android"))]
#[derive(Clone, Debug, Deserialize)]
struct DetectableApplication {
    id: String,
    name: String,
    #[serde(default)]
    executables: Vec<DetectableExecutable>,
}

#[cfg(not(target_os = "android"))]
#[derive(Clone, Debug, Deserialize)]
struct DetectableExecutable {
    name: String,
    #[serde(default)]
    is_launcher: bool,
    os: Option<String>,
    arguments: Option<String>,
}

#[cfg(not(target_os = "android"))]
#[derive(Clone, Debug)]
struct DetectableCandidate {
    application_id: String,
    app_name: String,
    executable: String,
    exact: bool,
    arguments: Option<String>,
}

#[cfg(not(target_os = "android"))]
#[derive(Default)]
struct DetectableIndex {
    by_name: std::collections::HashMap<String, Vec<DetectableCandidate>>,
}

#[cfg(not(target_os = "android"))]
impl DetectableIndex {
    fn from_json(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        let applications: Vec<DetectableApplication> = serde_json::from_slice(bytes)?;
        let mut index = Self::default();
        for application in applications {
            for executable in application.executables {
                if executable.is_launcher || !executable_for_this_os(executable.os.as_deref()) {
                    continue;
                }
                let mut name = executable.name.replace('\\', "/").to_ascii_lowercase();
                let exact = name.starts_with('>');
                if exact {
                    name.remove(0);
                }
                while name.starts_with('/') {
                    name.remove(0);
                }
                let basename = name.rsplit('/').next().unwrap_or(&name).to_owned();
                if basename.is_empty() {
                    continue;
                }
                index
                    .by_name
                    .entry(basename)
                    .or_default()
                    .push(DetectableCandidate {
                        application_id: application.id.clone(),
                        app_name: application.name.clone(),
                        executable: name,
                        exact,
                        arguments: executable.arguments,
                    });
            }
        }
        Ok(index)
    }

    fn detect(&self, system: &mut sysinfo::System) -> (bool, Option<Activity>) {
        use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, UpdateKind};

        system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing()
                .with_exe(UpdateKind::Always)
                .with_cmd(UpdateKind::Always),
        );

        let mut discord = false;
        let mut found: Option<(String, String)> = None;
        for process in system.processes().values() {
            let process_name = process
                .name()
                .to_string_lossy()
                .to_ascii_lowercase();
            let clean_name = process_name.strip_suffix(".exe").unwrap_or(&process_name);
            if matches!(
                clean_name,
                "discord" | "discordcanary" | "discordptb" | "discorddevelopment"
            ) {
                discord = true;
            }

            let Some(path) = process.exe() else {
                continue;
            };
            let normalized = path
                .to_string_lossy()
                .replace('\\', "/")
                .to_ascii_lowercase();
            let basename = normalized.rsplit('/').next().unwrap_or(&normalized);
            let Some(candidates) = self.by_name.get(basename) else {
                continue;
            };
            let arguments = process
                .cmd()
                .iter()
                .map(|part| part.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" ");
            if let Some(candidate) = candidates.iter().find(|candidate| {
                let name_matches = if candidate.exact {
                    basename == candidate.executable
                } else {
                    normalized == candidate.executable
                        || normalized.ends_with(&format!("/{}", candidate.executable))
                };
                let arguments_match = candidate
                    .arguments
                    .as_deref()
                    .is_none_or(|required| arguments.contains(required));
                name_matches && arguments_match
            }) {
                found = Some((
                    candidate.application_id.clone(),
                    candidate.app_name.clone(),
                ));
                break;
            }
        }

        let activity = found.map(|(_, name)| Activity {
            kind: ActivityKind::Playing,
            name,
            details: None,
            state: None,
            started_at: Some(Utc::now()),
            ends_at: None,
        });
        (discord, activity)
    }
}

#[cfg(not(target_os = "android"))]
fn executable_for_this_os(os: Option<&str>) -> bool {
    match os {
        None => true,
        Some("win32") => cfg!(target_os = "windows"),
        Some("darwin") => cfg!(target_os = "macos"),
        Some("linux") => cfg!(target_os = "linux"),
        Some(_) => false,
    }
}

#[cfg(not(target_os = "android"))]
async fn load_cached_detectables() -> (DetectableIndex, bool) {
    let path = crate::platform::dirs::cache_dir().join("rich-presence-detectable.json");
    let stale = std::fs::metadata(&path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .is_none_or(|age| age > std::time::Duration::from_secs(7 * 24 * 60 * 60));
    let index = match tokio::fs::read(&path).await {
        Ok(bytes) => DetectableIndex::from_json(&bytes).unwrap_or_default(),
        Err(_) => DetectableIndex::default(),
    };
    (index, stale)
}

#[cfg(not(target_os = "android"))]
async fn fetch_detectables() -> Result<DetectableIndex, String> {
    const URL: &str = "https://discord.com/api/v9/applications/detectable";
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .user_agent(concat!("Papo/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| error.to_string())?;
    let response = client
        .get(URL)
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?;
    let bytes = response.bytes().await.map_err(|error| error.to_string())?;
    let index = DetectableIndex::from_json(&bytes).map_err(|error| error.to_string())?;
    let path = crate::platform::dirs::cache_dir().join("rich-presence-detectable.json");
    if let Some(parent) = path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    let _ = tokio::fs::write(path, &bytes).await;
    Ok(index)
}

#[cfg(not(target_os = "android"))]
enum BuiltinEvent {
    Rpc {
        socket: u64,
        activity: Option<Activity>,
    },
    ListenerReady(u32),
    ListenerError(String),
}

#[cfg(not(target_os = "android"))]
async fn run_builtin(
    settings: &Settings,
    controls: &mut tokio::sync::mpsc::UnboundedReceiver<Control>,
    events: &std::sync::mpsc::Sender<Snapshot>,
    repaint: &egui::Context,
) -> RunExit {
    let (mut detector, stale) = load_cached_detectables().await;
    let mut refresh = if settings.game_detection && (stale || detector.by_name.is_empty()) {
        Some(tokio::spawn(fetch_detectables()))
    } else {
        None
    };

    let (ipc_tx, mut ipc_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut ipc_task: Option<tokio::task::JoinHandle<()>> = None;
    let mut ipc_slot: Option<u32> = None;
    let mut system = sysinfo::System::new();
    let mut rpc_activities: std::collections::HashMap<u64, (u64, Activity)> =
        std::collections::HashMap::new();
    let mut revision = 0u64;
    let mut process_activity: Option<Activity> = None;
    let mut last_published: Option<Activity> = None;
    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(5));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut external_probe = tokio::time::interval(std::time::Duration::from_secs(5));
    external_probe.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    external_probe.tick().await;

    publish(
        events,
        repaint,
        Source::BuiltIn,
        "Built-in collector",
        None,
    );

    loop {
        tokio::select! {
            control = controls.recv() => {
                if let Some(task) = ipc_task.take() {
                    task.abort();
                }
                return control.map(RunExit::Control).unwrap_or(RunExit::Closed);
            }
            _ = ticker.tick() => {
                let (discord_running, detected) = if settings.game_detection && !detector.by_name.is_empty() {
                    detector.detect(&mut system)
                } else {
                    // Even with game detection disabled we still need to know
                    // whether native Discord owns RPC priority.
                    let empty = DetectableIndex::default();
                    empty.detect(&mut system)
                };

                if discord_running {
                    if let Some(task) = ipc_task.take() {
                        task.abort();
                        ipc_slot = None;
                        rpc_activities.clear();
                        if settings.debug {
                            log::debug!("rich presence: native Discord detected; releasing Papo IPC");
                        }
                    }
                } else if ipc_task.is_none() {
                    let sender = ipc_tx.clone();
                    ipc_task = Some(tokio::spawn(async move {
                        if let Err(error) = run_ipc(sender.clone()).await {
                            let _ = sender.send(BuiltinEvent::ListenerError(error.to_string()));
                        }
                    }));
                }

                process_activity = detected.map(|mut detected| {
                    if let Some(previous) = process_activity.as_ref()
                        && previous.name == detected.name
                    {
                        detected.started_at = previous.started_at;
                    }
                    detected
                });
                if rpc_activities.is_empty() {
                    if process_activity != last_published {
                        last_published = process_activity.clone();
                        let detail = if discord_running {
                            "Native Discord owns RPC · game detection"
                        } else if let Some(slot) = ipc_slot {
                            if settings.game_detection {
                                return_detail(slot, true)
                            } else {
                                return_detail(slot, false)
                            }
                        } else {
                            "Built-in collector".to_owned()
                        };
                        publish(events, repaint, Source::BuiltIn, detail, process_activity.clone());
                    }
                }
            }
            event = ipc_rx.recv() => {
                let Some(event) = event else {
                    return RunExit::Disconnected("built-in IPC stopped".to_owned());
                };
                match event {
                    BuiltinEvent::ListenerReady(slot) => {
                        ipc_slot = Some(slot);
                        let detail = return_detail(slot, settings.game_detection);
                        publish(events, repaint, Source::BuiltIn, detail, last_published.clone());
                    }
                    BuiltinEvent::ListenerError(error) => {
                        ipc_task = None;
                        ipc_slot = None;
                        if settings.debug {
                            log::debug!("rich presence: IPC listener: {error}");
                        }
                    }
                    BuiltinEvent::Rpc { socket, activity } => {
                        revision = revision.wrapping_add(1);
                        match activity {
                            Some(activity) => {
                                rpc_activities.insert(socket, (revision, activity));
                            }
                            None => {
                                rpc_activities.remove(&socket);
                            }
                        }
                        let current = rpc_activities
                            .values()
                            .max_by_key(|(seen, _)| *seen)
                            .map(|(_, activity)| activity.clone())
                            .or_else(|| process_activity.clone());
                        if current != last_published {
                            last_published = current.clone();
                            let detail = ipc_slot
                                .map(|slot| return_detail(slot, settings.game_detection))
                                .unwrap_or_else(|| "Built-in collector".to_owned());
                            publish(events, repaint, Source::BuiltIn, detail, current);
                        }
                    }
                }
            }
            result = async {
                match refresh.as_mut() {
                    Some(task) => Some(task.await),
                    None => std::future::pending().await,
                }
            }, if refresh.is_some() => {
                refresh = None;
                match result {
                    Some(Ok(Ok(index))) => {
                        if settings.debug {
                            log::debug!(
                                "rich presence: refreshed detectable database ({} executable keys)",
                                index.by_name.len()
                            );
                        }
                        detector = index;
                    }
                    Some(Ok(Err(error))) if settings.debug => {
                        log::debug!("rich presence: detectable database refresh failed: {error}");
                    }
                    Some(Err(error)) if settings.debug => {
                        log::debug!("rich presence: detectable database task failed: {error}");
                    }
                    _ => {}
                }
            }
            _ = external_probe.tick(), if settings.auto_reconnect => {
                if local_bridge_port_open().await {
                    if let Some(task) = ipc_task.take() {
                        task.abort();
                    }
                    return RunExit::PreferExternal;
                }
            }
        }
    }
}

#[cfg(not(target_os = "android"))]
fn return_detail(slot: u32, game_detection: bool) -> String {
    if game_detection {
        format!("Built-in · discord-ipc-{slot} · game detection")
    } else {
        format!("Built-in · discord-ipc-{slot}")
    }
}

#[cfg(not(target_os = "android"))]
async fn local_bridge_port_open() -> bool {
    tokio::time::timeout(
        std::time::Duration::from_millis(150),
        tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, 1337)),
    )
    .await
    .is_ok_and(|result| result.is_ok())
}

#[cfg(not(target_os = "android"))]
async fn run_ipc(sender: tokio::sync::mpsc::UnboundedSender<BuiltinEvent>) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        run_windows_ipc(sender).await
    }
    #[cfg(unix)]
    {
        run_unix_ipc(sender).await
    }
    #[cfg(not(any(target_os = "windows", unix)))]
    {
        let _ = sender;
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Discord IPC is unsupported on this platform",
        ))
    }
}

#[cfg(all(not(target_os = "android"), unix))]
struct UnixListenerGuard {
    listener: tokio::net::UnixListener,
    path: std::path::PathBuf,
}

#[cfg(all(not(target_os = "android"), unix))]
impl Drop for UnixListenerGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(all(not(target_os = "android"), unix))]
fn unix_socket_base() -> (std::path::PathBuf, bool) {
    for key in ["XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP"] {
        if let Ok(value) = std::env::var(key)
            && !value.is_empty()
        {
            return (std::path::PathBuf::from(value), true);
        }
    }
    (std::path::PathBuf::from("/tmp"), false)
}

#[cfg(all(not(target_os = "android"), unix))]
async fn bind_unix_ipc() -> std::io::Result<(UnixListenerGuard, u32)> {
    let (base, private_dir) = unix_socket_base();
    for slot in 0..10u32 {
        let path = base.join(format!("discord-ipc-{slot}"));
        if path.exists() {
            let live = tokio::time::timeout(
                std::time::Duration::from_millis(100),
                tokio::net::UnixStream::connect(&path),
            )
            .await
            .is_ok_and(|result| result.is_ok());
            if live {
                continue;
            }
            // Never unlink somebody else's well-known socket from /tmp.
            if private_dir {
                let _ = std::fs::remove_file(&path);
            } else {
                continue;
            }
        }
        match tokio::net::UnixListener::bind(&path) {
            Ok(listener) => {
                return Ok((UnixListenerGuard { listener, path }, slot));
            }
            Err(error) if matches!(
                error.kind(),
                std::io::ErrorKind::AddrInUse | std::io::ErrorKind::PermissionDenied
            ) => continue,
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AddrInUse,
        "all discord-ipc slots are occupied",
    ))
}

#[cfg(all(not(target_os = "android"), unix))]
async fn run_unix_ipc(
    sender: tokio::sync::mpsc::UnboundedSender<BuiltinEvent>,
) -> std::io::Result<()> {
    let (guard, slot) = bind_unix_ipc().await?;
    let _ = sender.send(BuiltinEvent::ListenerReady(slot));
    let mut clients = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            accepted = guard.listener.accept() => {
                let (stream, _) = accepted?;
                let tx = sender.clone();
                clients.spawn(async move {
                    let socket = next_socket_id();
                    let _ = handle_rpc_stream(stream, socket, tx).await;
                });
            }
            Some(_) = clients.join_next(), if !clients.is_empty() => {}
        }
    }
}

#[cfg(all(not(target_os = "android"), target_os = "windows"))]
async fn bind_windows_ipc() -> std::io::Result<(tokio::net::windows::named_pipe::NamedPipeServer, u32)> {
    use tokio::net::windows::named_pipe::ServerOptions;
    for slot in 0..10u32 {
        let path = format!(r"\\.\pipe\discord-ipc-{slot}");
        match ServerOptions::new().first_pipe_instance(true).create(&path) {
            Ok(server) => return Ok((server, slot)),
            Err(error) if matches!(
                error.kind(),
                std::io::ErrorKind::AddrInUse | std::io::ErrorKind::PermissionDenied
            ) => continue,
            Err(_) => continue,
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AddrInUse,
        "all discord-ipc slots are occupied",
    ))
}

#[cfg(all(not(target_os = "android"), target_os = "windows"))]
async fn run_windows_ipc(
    sender: tokio::sync::mpsc::UnboundedSender<BuiltinEvent>,
) -> std::io::Result<()> {
    use tokio::net::windows::named_pipe::ServerOptions;

    let (first, slot) = bind_windows_ipc().await?;
    let path = format!(r"\\.\pipe\discord-ipc-{slot}");
    let _ = sender.send(BuiltinEvent::ListenerReady(slot));
    let mut next = Some(first);
    let mut clients = tokio::task::JoinSet::new();

    loop {
        let server = match next.take() {
            Some(server) => server,
            None => ServerOptions::new().create(&path)?,
        };
        server.connect().await?;
        let tx = sender.clone();
        clients.spawn(async move {
            let socket = next_socket_id();
            let _ = handle_rpc_stream(server, socket, tx).await;
        });
        while clients.try_join_next().is_some() {}
    }
}

#[cfg(not(target_os = "android"))]
fn next_socket_id() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

#[cfg(not(target_os = "android"))]
async fn handle_rpc_stream<S>(
    mut stream: S,
    socket: u64,
    sender: tokio::sync::mpsc::UnboundedSender<BuiltinEvent>,
) -> std::io::Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut client_id: Option<String> = None;
    loop {
        let mut header = [0u8; 8];
        if stream.read_exact(&mut header).await.is_err() {
            break;
        }
        let opcode = u32::from_le_bytes(header[0..4].try_into().unwrap());
        let length = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
        if length > 256 * 1024 {
            break;
        }
        let mut payload = vec![0u8; length];
        stream.read_exact(&mut payload).await?;

        match opcode {
            0 => {
                let value: serde_json::Value = match serde_json::from_slice(&payload) {
                    Ok(value) => value,
                    Err(_) => break,
                };
                let Some(id) = value
                    .get("client_id")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
                else {
                    break;
                };
                client_id = Some(id);
                let ready = serde_json::json!({
                    "cmd": "DISPATCH",
                    "data": {
                        "v": 1,
                        "config": {
                            "cdn_host": "cdn.discordapp.com",
                            "api_endpoint": "//discord.com/api",
                            "environment": "production"
                        },
                        "user": {
                            "id": "1045800378228281345",
                            "username": "papo",
                            "discriminator": "0",
                            "global_name": "Papo",
                            "avatar": null,
                            "avatar_decoration_data": null,
                            "bot": false,
                            "flags": 0,
                            "premium_type": 0
                        }
                    },
                    "evt": "READY",
                    "nonce": null
                });
                write_rpc_packet(&mut stream, 1, &serde_json::to_vec(&ready).unwrap()).await?;
            }
            1 => {
                let value: serde_json::Value = match serde_json::from_slice(&payload) {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                if value.get("cmd").and_then(serde_json::Value::as_str) != Some("SET_ACTIVITY") {
                    continue;
                }
                let app_id = client_id.as_deref().unwrap_or_default();
                let activity_value = value
                    .get("args")
                    .and_then(|args| args.get("activity"));
                let activity = activity_value
                    .filter(|activity| !activity.is_null())
                    .and_then(|activity| {
                        let mut activity = activity.clone();
                        if let Some(object) = activity.as_object_mut() {
                            object.insert(
                                "application_id".to_owned(),
                                serde_json::Value::String(app_id.to_owned()),
                            );
                        }
                        activity_from_value(&activity)
                    });
                let _ = sender.send(BuiltinEvent::Rpc { socket, activity });

                let response = serde_json::json!({
                    "cmd": "SET_ACTIVITY",
                    "data": activity_value.cloned().unwrap_or(serde_json::Value::Null),
                    "evt": null,
                    "nonce": value.get("nonce").cloned().unwrap_or(serde_json::Value::Null)
                });
                write_rpc_packet(&mut stream, 1, &serde_json::to_vec(&response).unwrap()).await?;
            }
            2 => break,
            3 => {
                // Discord IPC ping/pong uses the same opaque payload.
                write_rpc_packet(&mut stream, 4, &payload).await?;
            }
            4 => {}
            _ => break,
        }
    }
    let _ = sender.send(BuiltinEvent::Rpc {
        socket,
        activity: None,
    });
    Ok(())
}

#[cfg(not(target_os = "android"))]
async fn write_rpc_packet<S>(
    stream: &mut S,
    opcode: u32,
    payload: &[u8],
) -> std::io::Result<()>
where
    S: tokio::io::AsyncWrite + Unpin,
{
    use tokio::io::AsyncWriteExt;
    stream.write_all(&opcode.to_le_bytes()).await?;
    stream
        .write_all(&(payload.len() as u32).to_le_bytes())
        .await?;
    stream.write_all(payload).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn override_builds_one_canonical_activity() {
        let custom = ActivityOverride {
            enabled: true,
            kind: OverrideKind::Working,
            name: "Zed".to_owned(),
            details: "Editing rich_presence.rs".to_owned(),
            state: "papo-frontend".to_owned(),
            elapsed: true,
            duration_minutes: "90".to_owned(),
        };
        let start = DateTime::<Utc>::from_timestamp(1_700_000_000, 0).unwrap();
        let activity = custom.activity(start).unwrap();
        assert_eq!(activity.kind, ActivityKind::Working);
        assert_eq!(activity.name, "Zed");
        assert_eq!(activity.started_at, Some(start));
        assert_eq!(activity.ends_at, Some(start + ChronoDuration::minutes(90)));
    }

    #[cfg(not(target_os = "android"))]
    #[test]
    fn parses_arrpc_bridge_activity_and_clear() {
        let set = r#"{
            "activity": {
                "application_id": "123",
                "name": "Hades II",
                "type": 0,
                "details": "Fields of Mourning",
                "state": "Night 42",
                "timestamps": {"start": 1700000000}
            },
            "pid": 42,
            "socketId": "game"
        }"#;
        let (socket, activity) = parse_bridge_message(set).unwrap();
        assert_eq!(socket, "game");
        let activity = activity.unwrap();
        assert_eq!(activity.name, "Hades II");
        assert_eq!(activity.kind, ActivityKind::Playing);
        assert_eq!(activity.details.as_deref(), Some("Fields of Mourning"));
        assert!(activity.started_at.is_some());

        let clear = r#"{"activity":null,"pid":42,"socketId":"game"}"#;
        let (socket, activity) = parse_bridge_message(clear).unwrap();
        assert_eq!(socket, "game");
        assert!(activity.is_none());
    }
}
