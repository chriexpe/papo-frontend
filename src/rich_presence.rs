//! Desktop Rich Presence acquisition.
//!
//! Papo is a consumer first. If an arRPC-compatible JSON bridge already
//! exists, Papo connects to it and does not start another provider.
//! Otherwise Papo starts a pinned native rsrpc build as the fallback provider
//! and consumes the exact same arRPC-compatible bridge on localhost:1337.
//!
//! No Node/Bun/npm runtime is involved. The fallback is one native binary,
//! downloaded only when needed, checksum-verified, and kept in Papo's cache.
//! Android never starts a provider; it only renders activities received from
//! Papo servers.

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};

use crate::state::{Activity, ActivityKind};

const DEFAULT_HOST: &str = "127.0.0.1";
const DEFAULT_PORT: &str = "1337";
const LOCAL_BRIDGE: &str = "ws://127.0.0.1:1337";
const RSRPC_BUILD: &str = "nightly-2026-03-29";

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
    #[serde(default = "yes")]
    pub elapsed: bool,
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
    /// Start rsrpc only when the configured arRPC-compatible bridge is absent.
    #[serde(default = "yes")]
    pub built_in: bool,
    /// Passed to rsrpc; disabling this keeps RPC activity but disables its
    /// process-scanning fallback.
    #[serde(default = "yes")]
    pub game_detection: bool,
    #[serde(default = "yes")]
    pub auto_reconnect: bool,
    #[serde(default = "default_reconnect_interval")]
    pub reconnect_interval: String,
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
            reconnect_interval: default_reconnect_interval(),
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

fn default_reconnect_interval() -> String {
    "5".to_owned()
}

impl Settings {
    pub fn external_port_number(&self) -> u16 {
        self.external_port.trim().parse().unwrap_or(1337)
    }

    fn endpoint(&self) -> String {
        let host = self.external_host.trim();
        let host = if host.parse::<std::net::Ipv6Addr>().is_ok() {
            format!("[{host}]")
        } else if host.is_empty() {
            DEFAULT_HOST.to_owned()
        } else {
            host.to_owned()
        };
        format!("ws://{host}:{}", self.external_port_number())
    }

    fn reconnect_delay(&self) -> std::time::Duration {
        let seconds = self
            .reconnect_interval
            .trim()
            .parse::<u64>()
            .unwrap_or(5)
            .clamp(1, 300);
        std::time::Duration::from_secs(seconds)
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
    let mut rsrpc: Option<RsrpcProcess> = None;

    loop {
        if !settings.enabled {
            rsrpc.take();
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
            // Manual activity does not need a local activity provider.
            rsrpc.take();
            publish(
                &events,
                &repaint,
                Source::Override,
                "Manual override",
                settings.override_activity.activity(Utc::now()),
            );
            match controls.recv().await {
                Some(Control::Configure(next)) => settings = next,
                Some(Control::Restart) => {}
                None => return,
            }
            continue;
        }

        if let Some(process) = rsrpc.as_mut() {
            match process.try_wait() {
                Ok(Some(status)) => {
                    let detail = format!("rsRPC exited ({status})");
                    rsrpc = None;
                    publish(&events, &repaint, Source::Error, detail, None);
                }
                Ok(None) => {
                    match run_bridge(
                        &settings,
                        LOCAL_BRIDGE,
                        Source::BuiltIn,
                        std::time::Duration::from_secs(2),
                        &mut controls,
                        &events,
                        &repaint,
                    )
                    .await
                    {
                        RunExit::Control(Control::Configure(next)) => {
                            rsrpc.take();
                            settings = next;
                        }
                        RunExit::Control(Control::Restart) => {
                            rsrpc.take();
                        }
                        RunExit::Closed => return,
                        RunExit::Disconnected(error) => {
                            if settings.debug {
                                log::debug!("rich presence: rsRPC bridge disconnected: {error}");
                            }
                            if process.try_wait().ok().flatten().is_some() {
                                rsrpc = None;
                            }
                            if !wait_after_failure(&settings, &mut controls).await {
                                return;
                            }
                        }
                    }
                    continue;
                }
                Err(error) => {
                    rsrpc = None;
                    publish(
                        &events,
                        &repaint,
                        Source::Error,
                        format!("rsRPC status: {error}"),
                        None,
                    );
                }
            }
        }

        let endpoint = settings.endpoint();
        publish(
            &events,
            &repaint,
            Source::Connecting,
            endpoint.clone(),
            None,
        );

        match run_bridge(
            &settings,
            &endpoint,
            Source::External,
            external_timeout(&endpoint),
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
            RunExit::Disconnected(external_error) => {
                if settings.debug {
                    log::debug!(
                        "rich presence: arRPC-compatible bridge unavailable: {external_error}"
                    );
                }

                if settings.built_in {
                    // Avoid a race where another provider claimed :1337 after
                    // the initial probe but before the fallback starts.
                    if local_bridge_port_open().await {
                        continue;
                    }

                    publish(
                        &events,
                        &repaint,
                        Source::Connecting,
                        "Starting rsRPC fallback",
                        None,
                    );
                    match RsrpcProcess::start(&settings).await {
                        Ok(process) => {
                            rsrpc = Some(process);
                            continue;
                        }
                        Err(error) => {
                            publish(
                                &events,
                                &repaint,
                                Source::Error,
                                format!("rsRPC fallback: {error}"),
                                None,
                            );
                        }
                    }
                } else {
                    publish(
                        &events,
                        &repaint,
                        Source::Error,
                        external_error,
                        None,
                    );
                }

                if !wait_after_failure(&settings, &mut controls).await {
                    return;
                }

                while let Ok(control) = controls.try_recv() {
                    match control {
                        Control::Configure(next) => settings = next,
                        Control::Restart => {}
                    }
                }
            }
        }
    }
}

#[cfg(not(target_os = "android"))]
fn external_timeout(endpoint: &str) -> std::time::Duration {
    if endpoint == LOCAL_BRIDGE {
        std::time::Duration::from_millis(450)
    } else {
        std::time::Duration::from_secs(4)
    }
}

#[cfg(not(target_os = "android"))]
async fn wait_after_failure(
    settings: &Settings,
    controls: &mut tokio::sync::mpsc::UnboundedReceiver<Control>,
) -> bool {
    if !settings.auto_reconnect {
        return controls.recv().await.is_some();
    }

    tokio::select! {
        control = controls.recv() => control.is_some(),
        _ = tokio::time::sleep(settings.reconnect_delay()) => true,
    }
}

#[cfg(not(target_os = "android"))]
async fn run_bridge(
    settings: &Settings,
    endpoint: &str,
    source: Source,
    timeout: std::time::Duration,
    controls: &mut tokio::sync::mpsc::UnboundedReceiver<Control>,
    events: &std::sync::mpsc::Sender<Snapshot>,
    repaint: &egui::Context,
) -> RunExit {
    use futures_util::StreamExt;

    let connection = tokio::time::timeout(timeout, tokio_tungstenite::connect_async(endpoint)).await;
    let (stream, _) = match connection {
        Ok(Ok(connection)) => connection,
        Ok(Err(error)) => {
            return RunExit::Disconnected(format!("{endpoint}: {error}"));
        }
        Err(_) => {
            return RunExit::Disconnected(format!("{endpoint}: connection timed out"));
        }
    };

    if settings.debug {
        log::debug!("rich presence: connected to {endpoint}");
    }

    let detail = match source {
        Source::BuiltIn => format!("rsRPC · {endpoint}"),
        _ => endpoint.to_owned(),
    };
    publish(events, repaint, source.clone(), detail.clone(), None);

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
                    return RunExit::Disconnected(format!("{endpoint}: connection closed"));
                };
                let message = match message {
                    Ok(message) => message,
                    Err(error) => return RunExit::Disconnected(format!("{endpoint}: {error}")),
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
                        return RunExit::Disconnected(format!("{endpoint}: connection closed"));
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
                    publish(events, repaint, source.clone(), detail.clone(), current);
                } else if settings.debug {
                    log::debug!("rich presence: ignored unrecognized bridge frame");
                }
            }
        }
    }
}

#[cfg(not(target_os = "android"))]
fn parse_bridge_message(text: &str) -> Option<(String, Option<Activity>)> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    value.get("activity")?;
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

#[cfg(not(target_os = "android"))]
struct RsrpcProcess {
    child: std::process::Child,
}

#[cfg(not(target_os = "android"))]
impl RsrpcProcess {
    async fn start(settings: &Settings) -> Result<Self, String> {
        let binary = ensure_rsrpc_binary().await?;

        let mut command = std::process::Command::new(binary);
        if !settings.game_detection {
            command.arg("--no-process-scanning");
        }
        if settings.debug {
            command.arg("--debug");
        }
        command
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = command
            .spawn()
            .map_err(|error| format!("could not start rsRPC: {error}"))?;

        if let Some(stdout) = child.stdout.take() {
            pipe_rsrpc_log(stdout, "stdout");
        }
        if let Some(stderr) = child.stderr.take() {
            pipe_rsrpc_log(stderr, "stderr");
        }

        let mut process = Self { child };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(6);
        loop {
            if local_bridge_port_open().await {
                log::debug!("rich presence: rsRPC fallback is ready on 127.0.0.1:1337");
                return Ok(process);
            }
            if let Some(status) = process
                .try_wait()
                .map_err(|error| format!("rsRPC status: {error}"))?
            {
                return Err(format!("rsRPC exited before bridge startup ({status})"));
            }
            if std::time::Instant::now() >= deadline {
                process.stop();
                return Err("rsRPC did not open port 1337 within 6 seconds".to_owned());
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        self.child.try_wait()
    }

    fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(not(target_os = "android"))]
impl Drop for RsrpcProcess {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(not(target_os = "android"))]
fn pipe_rsrpc_log<R>(reader: R, stream: &'static str)
where
    R: std::io::Read + Send + 'static,
{
    std::thread::Builder::new()
        .name(format!("papo-rsrpc-{stream}"))
        .spawn(move || {
            use std::io::BufRead;
            let reader = std::io::BufReader::new(reader);
            for line in reader.lines().map_while(Result::ok) {
                log::debug!("rsRPC {stream}: {line}");
            }
        })
        .ok();
}

#[cfg(not(target_os = "android"))]
#[derive(Clone, Copy)]
struct RsrpcAsset {
    name: &'static str,
    url: &'static str,
    sha256: &'static str,
}

#[cfg(not(target_os = "android"))]
fn rsrpc_asset() -> Result<RsrpcAsset, String> {
    let asset = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => RsrpcAsset {
            name: "rsrpc-x86_64-unknown-linux-gnu",
            url: "https://github.com/pog5/rsrpc/releases/download/nightly/rsrpc-x86_64-unknown-linux-gnu",
            sha256: "9c575b67960fe9763613702a08494bf4ab4ef0d4535fb1619abfd8279feea2db",
        },
        ("linux", "aarch64") => RsrpcAsset {
            name: "rsrpc-aarch64-unknown-linux-gnu",
            url: "https://github.com/pog5/rsrpc/releases/download/nightly/rsrpc-aarch64-unknown-linux-gnu",
            sha256: "116c4d8f6b5dcd65c2bf9a2036d2e2613e780e206ed0ead72224e71c509ca551",
        },
        ("windows", "x86_64") => RsrpcAsset {
            name: "rsrpc-x86_64-pc-windows-msvc.exe",
            url: "https://github.com/pog5/rsrpc/releases/download/nightly/rsrpc-x86_64-pc-windows-msvc.exe",
            sha256: "062e893ee5eacba02f64e24877c11fd4ae310c20a1373b436aea3a84b519a97b",
        },
        ("windows", "aarch64") => RsrpcAsset {
            name: "rsrpc-aarch64-pc-windows-msvc.exe",
            url: "https://github.com/pog5/rsrpc/releases/download/nightly/rsrpc-aarch64-pc-windows-msvc.exe",
            sha256: "5764abea0485ae03d3588183aaa151b4d35d3cd3b132cdfd1f28ea4383729184",
        },
        ("macos", "x86_64") => RsrpcAsset {
            name: "rsrpc-x86_64-apple-darwin",
            url: "https://github.com/pog5/rsrpc/releases/download/nightly/rsrpc-x86_64-apple-darwin",
            sha256: "b2e5b6ac8dc1842bcc80740abaafa9ddb75aacd1322f57d851208c13f9dc13ca",
        },
        ("macos", "aarch64") => RsrpcAsset {
            name: "rsrpc-aarch64-apple-darwin",
            url: "https://github.com/pog5/rsrpc/releases/download/nightly/rsrpc-aarch64-apple-darwin",
            sha256: "4f49a8ff5c5b568b7b4b1eeac7a701e79b07aa1b5b544342bba7c4769445bdc1",
        },
        (os, arch) => {
            return Err(format!("no rsRPC build for {os}/{arch}"));
        }
    };
    Ok(asset)
}

#[cfg(not(target_os = "android"))]
async fn ensure_rsrpc_binary() -> Result<std::path::PathBuf, String> {
    let asset = rsrpc_asset()?;
    let dir = crate::platform::dirs::cache_dir()
        .join("rich-presence")
        .join("rsrpc")
        .join(RSRPC_BUILD);
    let path = dir.join(if cfg!(target_os = "windows") {
        "rsrpc.exe"
    } else {
        "rsrpc"
    });

    if let Ok(bytes) = tokio::fs::read(&path).await
        && sha256_hex(&bytes) == asset.sha256
    {
        ensure_executable(&path).await?;
        return Ok(path);
    }

    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|error| format!("rsRPC cache directory: {error}"))?;

    let client = reqwest::Client::builder()
        .user_agent(concat!("Papo/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|error| format!("rsRPC downloader: {error}"))?;
    let response = client
        .get(asset.url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| format!("rsRPC download: {error}"))?;
    let bytes = response
        .bytes()
        .await
        .map_err(|error| format!("rsRPC download body: {error}"))?;
    if bytes.len() > 32 * 1024 * 1024 {
        return Err(format!("rsRPC download is unexpectedly large: {} bytes", bytes.len()));
    }

    let actual = sha256_hex(&bytes);
    if actual != asset.sha256 {
        return Err(format!(
            "rsRPC checksum mismatch for {} (expected {}, got {})",
            asset.name, asset.sha256, actual
        ));
    }

    let temp = dir.join(format!("{}.download", asset.name));
    tokio::fs::write(&temp, &bytes)
        .await
        .map_err(|error| format!("save rsRPC: {error}"))?;
    tokio::fs::rename(&temp, &path)
        .await
        .map_err(|error| format!("install rsRPC: {error}"))?;
    ensure_executable(&path).await?;

    Ok(path)
}

#[cfg(not(target_os = "android"))]
fn sha256_hex(bytes: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, bytes)
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(all(not(target_os = "android"), unix))]
async fn ensure_executable(path: &std::path::Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = tokio::fs::metadata(path)
        .await
        .map_err(|error| format!("rsRPC metadata: {error}"))?
        .permissions();
    if permissions.mode() & 0o111 == 0 {
        permissions.set_mode(0o700);
        tokio::fs::set_permissions(path, permissions)
            .await
            .map_err(|error| format!("rsRPC permissions: {error}"))?;
    }
    Ok(())
}

#[cfg(all(not(target_os = "android"), not(unix)))]
async fn ensure_executable(_path: &std::path::Path) -> Result<(), String> {
    Ok(())
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

    #[test]
    fn reconnect_interval_is_bounded() {
        let mut settings = Settings::default();
        settings.reconnect_interval = "0".to_owned();
        assert_eq!(settings.reconnect_delay(), std::time::Duration::from_secs(1));
        settings.reconnect_interval = "9999".to_owned();
        assert_eq!(settings.reconnect_delay(), std::time::Duration::from_secs(300));
    }
}
