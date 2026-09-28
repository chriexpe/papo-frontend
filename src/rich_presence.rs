//! Desktop Rich Presence acquisition.
//!
//! Papo is a consumer first. If an arRPC-compatible JSON bridge already
//! exists, Papo connects to it and does not start another provider.
//! Otherwise Papo starts the native rsrpc helper bundled with the desktop
//! package and consumes the exact same arRPC-compatible bridge on localhost:1337.
//!
//! No Node/Bun/npm runtime and no first-run download are involved. Release
//! packages ship the matching rsrpc executable beside Papo (or in libexec).
//! Android never starts a provider; it only renders activities received from
//! Papo servers.

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};

use crate::state::{Activity, ActivityKind};

const DEFAULT_HOST: &str = "127.0.0.1";
const DEFAULT_PORT: &str = "1337";
#[cfg(not(target_os = "android"))]
const LOCAL_BRIDGE: &str = "ws://127.0.0.1:1337";

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
    /// Square art for the activity, cropped in Settings and kept in Papo's
    /// data directory.
    #[serde(default)]
    pub image: Option<std::path::PathBuf>,
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
            image: None,
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
    /// Show games found by process scanning. Applied in Papo, so it also
    /// covers an external arRPC/rsRPC; activity reported by games over RPC
    /// is always shown.
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

    #[cfg(not(target_os = "android"))]
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

    /// The configured bridge is the local 127.0.0.1:1337 one, however it is
    /// spelled (`localhost`, `::1`). That is where an owned rsRPC listens, so
    /// connecting there must not be mistaken for an external bridge.
    #[cfg(not(target_os = "android"))]
    fn targets_local_bridge(&self) -> bool {
        let host = self.external_host.trim();
        let loopback = host.is_empty()
            || host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback());
        loopback && self.external_port_number() == 1337
    }

    #[cfg(not(target_os = "android"))]
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
            image: self
                .image
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned()),
        })
    }
}

fn nonempty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

pub struct Manager {
    settings: Settings,
    /// What the activity source reports, before the manual override.
    source: Snapshot,
    /// `source` with the manual override applied; what the UI reads.
    snapshot: Snapshot,
    /// When the manual activity started. Kept while it stays enabled, so
    /// editing its text does not reset the elapsed timer.
    override_started: Option<DateTime<Utc>>,
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
            let mut manager = Self {
                source: android_snapshot(&settings),
                snapshot: Snapshot::default(),
                override_started: None,
                settings,
            };
            manager.compose();
            manager
        }

        #[cfg(not(target_os = "android"))]
        {
            let (control_tx, control_rx) = tokio::sync::mpsc::unbounded_channel();
            let (event_tx, event_rx) = std::sync::mpsc::channel();
            let worker_settings = settings.clone();
            std::thread::Builder::new()
                .name("papo-rich-presence".to_owned())
                .spawn(move || {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build();
                    match runtime {
                        Ok(runtime) => runtime.block_on(
                            Worker {
                                settings: worker_settings,
                                controls: control_rx,
                                events: event_tx,
                                repaint,
                                rsrpc: None,
                            }
                            .run(),
                        ),
                        Err(error) => log::warn!(
                            "rich presence: não foi possível iniciar o runtime: {error}"
                        ),
                    }
                })
                .ok();
            let mut manager = Self {
                settings,
                source: Snapshot::default(),
                snapshot: Snapshot::default(),
                override_started: None,
                control: control_tx,
                events: event_rx,
            };
            manager.compose();
            manager
        }
    }

    pub fn configure(&mut self, settings: Settings) {
        if self.settings == settings {
            return;
        }
        self.settings = settings.clone();

        #[cfg(target_os = "android")]
        {
            self.source = android_snapshot(&settings);
        }
        #[cfg(not(target_os = "android"))]
        {
            // The worker decides what the change actually needs; most edits
            // (manual activity, intervals, detection filter) never touch the
            // connection or the rsRPC process.
            let _ = self.control.send(Control::Configure(Box::new(settings)));
        }
        self.compose();
    }

    pub fn restart(&mut self) {
        #[cfg(not(target_os = "android"))]
        {
            let _ = self.control.send(Control::Restart);
        }
    }

    pub fn pump(&mut self) -> bool {
        #[cfg(not(target_os = "android"))]
        {
            let mut latest = None;
            while let Ok(next) = self.events.try_recv() {
                latest = Some(next);
            }
            if let Some(next) = latest
                && self.source != next
            {
                self.source = next;
                return self.compose();
            }
        }
        false
    }

    pub fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }

    /// Applies the manual override on top of the source snapshot. Returns
    /// whether the visible snapshot changed.
    fn compose(&mut self) -> bool {
        let manual = &self.settings.override_activity;
        let next = if self.settings.enabled && manual.enabled {
            let started = *self.override_started.get_or_insert_with(Utc::now);
            Snapshot {
                source: Source::Override,
                detail: String::new(),
                activity: manual.activity(started),
            }
        } else {
            self.override_started = None;
            self.source.clone()
        };
        let changed = self.snapshot != next;
        self.snapshot = next;
        changed
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
    Configure(Box<Settings>),
    Restart,
}

/// What a control message requires from the worker, weakest first.
#[cfg(not(target_os = "android"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Effect {
    /// Nothing visible changes.
    Nothing,
    /// Same connection; recompute the published activity.
    Republish,
    /// Keep a live connection, but stop waiting and try now if idle.
    Retry,
    /// Drop the bridge connection and connect again. An owned rsRPC keeps
    /// running, so games connected to it over IPC are not disturbed.
    Reconnect,
    /// Stop the owned rsRPC as well.
    Restart,
}

/// How a connection phase ended.
#[cfg(not(target_os = "android"))]
#[derive(Debug, PartialEq, Eq)]
enum Step {
    /// Settings changed or a better source appeared: go again right away.
    Again,
    /// The source is unavailable: wait before trying again.
    Failed,
    /// Papo is closing.
    Closed,
}

#[cfg(not(target_os = "android"))]
type BridgeStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

#[cfg(not(target_os = "android"))]
struct Worker {
    settings: Settings,
    controls: tokio::sync::mpsc::UnboundedReceiver<Control>,
    events: std::sync::mpsc::Sender<Snapshot>,
    repaint: egui::Context,
    rsrpc: Option<RsrpcProcess>,
}

#[cfg(not(target_os = "android"))]
impl Worker {
    async fn run(mut self) {
        loop {
            let step = if self.settings.enabled {
                self.connect().await
            } else {
                self.stop_rsrpc();
                self.publish(Source::Disabled, "Rich Presence disabled", None);
                Step::Failed
            };
            match step {
                Step::Again => {}
                Step::Closed => return,
                Step::Failed => {
                    if !self.wait().await {
                        return;
                    }
                }
            }
        }
    }

    fn publish(&self, source: Source, detail: impl Into<String>, activity: Option<Activity>) {
        let _ = self.events.send(Snapshot {
            source,
            detail: detail.into(),
            activity,
        });
        self.repaint.request_repaint();
    }

    /// Takes a control message into account and says what it requires.
    fn apply(&mut self, control: Control) -> Effect {
        let next = match control {
            Control::Restart => return Effect::Restart,
            Control::Configure(next) => *next,
        };
        let before = std::mem::replace(&mut self.settings, next);
        effect_of(&before, &self.settings, self.rsrpc.is_some())
    }

    /// Waits for a control message that asks for a new attempt, or for the
    /// reconnect interval when auto reconnect is on.
    async fn wait(&mut self) -> bool {
        let deadline = (self.settings.enabled && self.settings.auto_reconnect)
            .then(|| tokio::time::Instant::now() + self.settings.reconnect_delay());
        loop {
            let control = match deadline {
                Some(deadline) => tokio::select! {
                    control = self.controls.recv() => control,
                    _ = tokio::time::sleep_until(deadline) => return true,
                },
                None => self.controls.recv().await,
            };
            let Some(control) = control else {
                return false;
            };
            let effect = self.apply(control);
            if effect == Effect::Restart {
                self.stop_rsrpc();
            }
            if effect >= Effect::Retry {
                return true;
            }
        }
    }

    /// Runs `future` while still answering control messages. A change that
    /// needs a new attempt cancels it; anything weaker is applied in place.
    async fn guarded<F: std::future::Future>(&mut self, future: F) -> Result<F::Output, Step> {
        tokio::pin!(future);
        loop {
            tokio::select! {
                output = &mut future => return Ok(output),
                control = self.controls.recv() => {
                    let Some(control) = control else {
                        return Err(Step::Closed);
                    };
                    match self.apply(control) {
                        Effect::Restart => {
                            self.stop_rsrpc();
                            return Err(Step::Again);
                        }
                        Effect::Reconnect | Effect::Retry => return Err(Step::Again),
                        Effect::Republish | Effect::Nothing => {}
                    }
                }
            }
        }
    }

    async fn connect(&mut self) -> Step {
        self.reap_rsrpc();
        let endpoint = self.settings.endpoint();
        let local = self.settings.targets_local_bridge();
        self.publish(Source::Connecting, endpoint.clone(), None);

        // 1. The configured bridge. With the default endpoint this is also
        //    how Papo reattaches to an rsRPC it already started.
        let external_error = match self.guarded(open_bridge(&endpoint, local)).await {
            Err(step) => return step,
            Ok(Ok(stream)) => {
                let source = if local && self.rsrpc.is_some() {
                    Source::BuiltIn
                } else {
                    // An external bridge showed up: the fallback is no
                    // longer needed.
                    self.stop_rsrpc();
                    Source::External
                };
                return self.serve(stream, source, endpoint).await;
            }
            Ok(Err(error)) => error,
        };
        if self.settings.debug {
            log::debug!("rich presence: bridge unavailable: {external_error}");
        }

        if !self.settings.built_in {
            self.stop_rsrpc();
            self.publish(Source::Error, external_error, None);
            return Step::Failed;
        }

        // 2. Our rsRPC is still running (the configured endpoint is elsewhere
        //    or the bridge dropped): use it again instead of spawning another.
        if self.rsrpc.is_some() {
            return self.serve_local(Source::BuiltIn).await;
        }

        // 3. Someone else owns 127.0.0.1:1337. If it is a bridge, share it;
        //    if it is not, say so and wait instead of spinning on it.
        if local_bridge_port_open().await {
            if !local {
                match self.guarded(open_bridge(LOCAL_BRIDGE, true)).await {
                    Err(step) => return step,
                    Ok(Ok(stream)) => {
                        return self
                            .serve(stream, Source::External, LOCAL_BRIDGE.to_owned())
                            .await;
                    }
                    Ok(Err(_)) => {}
                }
            }
            self.publish(
                Source::Error,
                "127.0.0.1:1337 is in use by something that is not an arRPC bridge",
                None,
            );
            return Step::Failed;
        }

        // 4. Nothing is there: start the bundled rsRPC.
        self.publish(Source::Connecting, "Starting rsRPC fallback", None);
        match RsrpcProcess::spawn(&self.settings) {
            Ok(process) => self.rsrpc = Some(process),
            Err(error) => {
                self.publish(Source::Error, format!("rsRPC fallback: {error}"), None);
                return Step::Failed;
            }
        }
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(6);
        while !local_bridge_port_open().await {
            if let Some(status) = self.reap_rsrpc() {
                self.publish(
                    Source::Error,
                    format!("rsRPC exited before bridge startup ({status})"),
                    None,
                );
                return Step::Failed;
            }
            if self.rsrpc.is_none() {
                return Step::Again;
            }
            if tokio::time::Instant::now() >= deadline {
                self.stop_rsrpc();
                self.publish(
                    Source::Error,
                    "rsRPC did not open port 1337 within 6 seconds",
                    None,
                );
                return Step::Failed;
            }
            if let Err(step) = self
                .guarded(tokio::time::sleep(std::time::Duration::from_millis(100)))
                .await
            {
                return step;
            }
        }
        log::debug!("rich presence: rsRPC fallback is ready on 127.0.0.1:1337");
        self.serve_local(Source::BuiltIn).await
    }

    async fn serve_local(&mut self, source: Source) -> Step {
        match self.guarded(open_bridge(LOCAL_BRIDGE, true)).await {
            Err(step) => step,
            Ok(Ok(stream)) => self.serve(stream, source, LOCAL_BRIDGE.to_owned()).await,
            Ok(Err(error)) => {
                self.publish(Source::Error, format!("rsRPC bridge: {error}"), None);
                Step::Failed
            }
        }
    }

    async fn serve(&mut self, stream: BridgeStream, source: Source, endpoint: String) -> Step {
        use futures_util::StreamExt;

        if self.settings.debug {
            log::debug!("rich presence: connected to {endpoint}");
        }
        let detail = match source {
            Source::BuiltIn => format!("rsRPC · {endpoint}"),
            _ => endpoint.clone(),
        };
        let (_, mut reader) = stream.split();
        let mut activities = Activities::default();
        self.publish(source.clone(), detail.clone(), None);

        // While serving our own fallback, keep looking for the configured
        // bridge so Papo hands over to it once it starts.
        let configured = self.settings.endpoint();
        let probing = source == Source::BuiltIn && !self.settings.targets_local_bridge();
        let mut probe = tokio::time::interval(self.settings.reconnect_delay());
        probe.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        probe.tick().await;

        loop {
            tokio::select! {
                control = self.controls.recv() => {
                    let Some(control) = control else {
                        return Step::Closed;
                    };
                    match self.apply(control) {
                        Effect::Restart => {
                            self.stop_rsrpc();
                            return Step::Again;
                        }
                        Effect::Reconnect => return Step::Again,
                        Effect::Republish => {
                            let current = activities.current(self.settings.game_detection);
                            self.publish(source.clone(), detail.clone(), current);
                        }
                        Effect::Retry | Effect::Nothing => {}
                    }
                }
                _ = probe.tick(), if probing => {
                    if open_bridge(&configured, false).await.is_ok() {
                        log::info!("rich presence: {configured} is available, leaving the rsRPC fallback");
                        self.stop_rsrpc();
                        return Step::Again;
                    }
                }
                message = reader.next() => {
                    let text = match message {
                        Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) => text.to_string(),
                        Some(Ok(tokio_tungstenite::tungstenite::Message::Binary(bytes))) => {
                            match String::from_utf8(bytes.to_vec()) {
                                Ok(text) => text,
                                Err(_) => continue,
                            }
                        }
                        Some(Ok(tokio_tungstenite::tungstenite::Message::Close(_))) | None => {
                            return self.lost(&endpoint, "connection closed".to_owned());
                        }
                        Some(Ok(_)) => continue,
                        Some(Err(error)) => return self.lost(&endpoint, error.to_string()),
                    };
                    match parse_bridge_message(&text) {
                        Some(frame) => {
                            activities.apply(frame);
                            let current = activities.current(self.settings.game_detection);
                            self.publish(source.clone(), detail.clone(), current);
                        }
                        None if self.settings.debug => {
                            log::debug!("rich presence: ignored bridge frame: {text}");
                        }
                        None => {}
                    }
                }
            }
        }
    }

    fn lost(&mut self, endpoint: &str, error: String) -> Step {
        if self.settings.debug {
            log::debug!("rich presence: {endpoint} disconnected: {error}");
        }
        let detail = match self.reap_rsrpc() {
            Some(status) => format!("rsRPC exited ({status})"),
            None => format!("{endpoint}: {error}"),
        };
        self.publish(Source::Error, detail, None);
        Step::Failed
    }

    /// Forgets an owned rsRPC that already exited, returning its status.
    fn reap_rsrpc(&mut self) -> Option<std::process::ExitStatus> {
        let status = self.rsrpc.as_mut()?.try_wait().ok().flatten()?;
        self.rsrpc = None;
        Some(status)
    }

    fn stop_rsrpc(&mut self) {
        if let Some(mut process) = self.rsrpc.take() {
            process.stop();
        }
    }
}

/// What a settings change requires. Only the switches that actually change
/// the provider touch the connection; editing text, intervals or the
/// detection filter never restarts rsRPC (which would drop every game's IPC
/// connection and reset detected play time).
#[cfg(not(target_os = "android"))]
fn effect_of(before: &Settings, after: &Settings, owns_rsrpc: bool) -> Effect {
    let mut effect = Effect::Nothing;
    if before.enabled != after.enabled {
        effect = effect.max(if after.enabled {
            Effect::Retry
        } else {
            Effect::Restart
        });
    }
    if before.endpoint() != after.endpoint() {
        effect = effect.max(Effect::Reconnect);
    }
    if before.built_in != after.built_in {
        effect = effect.max(if after.built_in {
            Effect::Retry
        } else if owns_rsrpc {
            Effect::Restart
        } else {
            Effect::Nothing
        });
    }
    // --debug is an rsRPC argument; an external bridge is unaffected.
    if before.debug != after.debug && owns_rsrpc {
        effect = effect.max(Effect::Restart);
    }
    if before.game_detection != after.game_detection {
        effect = effect.max(Effect::Republish);
    }
    if before.auto_reconnect != after.auto_reconnect
        || before.reconnect_delay() != after.reconnect_delay()
    {
        // Start the pending wait over with the new timing.
        effect = effect.max(Effect::Retry);
    }
    effect
}

/// Activities currently reported by the bridge, one per socket.
#[cfg(not(target_os = "android"))]
#[derive(Default)]
struct Activities {
    revision: u64,
    by_socket: std::collections::HashMap<String, (u64, BridgeActivity)>,
}

#[cfg(not(target_os = "android"))]
impl Activities {
    fn apply(&mut self, frame: BridgeFrame) {
        self.revision = self.revision.wrapping_add(1);
        match frame.activity {
            Some(activity) => {
                self.by_socket.insert(frame.socket, (self.revision, activity));
            }
            None => {
                self.by_socket.remove(&frame.socket);
            }
        }
    }

    /// The most recent activity, skipping process-detected games when game
    /// detection is off. The filter lives here, not in the provider, so it
    /// works the same with an external arRPC/rsRPC Papo does not control.
    fn current(&self, game_detection: bool) -> Option<Activity> {
        self.by_socket
            .values()
            .filter(|(_, activity)| game_detection || !activity.detected)
            .max_by_key(|(revision, _)| *revision)
            .map(|(_, activity)| activity.activity.clone())
    }
}

#[cfg(not(target_os = "android"))]
async fn open_bridge(endpoint: &str, local: bool) -> Result<BridgeStream, String> {
    // Local bridges answer in milliseconds, but a busy arRPC can take longer;
    // too short a limit made it look like a foreign process on the port.
    let timeout = if local {
        std::time::Duration::from_millis(1500)
    } else {
        std::time::Duration::from_secs(4)
    };
    match tokio::time::timeout(timeout, tokio_tungstenite::connect_async(endpoint)).await {
        Ok(Ok((stream, _))) => Ok(stream),
        Ok(Err(error)) => Err(format!("{endpoint}: {error}")),
        Err(_) => Err(format!("{endpoint}: connection timed out")),
    }
}

#[cfg(not(target_os = "android"))]
struct BridgeActivity {
    activity: Activity,
    /// Found by process scanning rather than reported by the game itself.
    detected: bool,
}

#[cfg(not(target_os = "android"))]
struct BridgeFrame {
    socket: String,
    activity: Option<BridgeActivity>,
}

#[cfg(not(target_os = "android"))]
fn parse_bridge_message(text: &str) -> Option<BridgeFrame> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let raw = value.get("activity")?;
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

    let activity = activity_from_value(raw).map(|activity| BridgeActivity {
        // arRPC (process/index.js) and rsRPC (server.rs) both publish a
        // scanned game under a socket named after its application id; real
        // IPC clients get their own connection id instead.
        detected: raw
            .get("application_id")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|id| id == socket),
        activity,
    });
    Some(BridgeFrame { socket, activity })
}

#[cfg(not(target_os = "android"))]
fn activity_from_value(value: &serde_json::Value) -> Option<Activity> {
    if value.is_null() {
        return None;
    }
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
        image: None,
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
    fn spawn(settings: &Settings) -> Result<Self, String> {
        let binary = find_rsrpc_binary()?;
        let mut command = rsrpc_command(&binary)?;

        // Process scanning stays on: "Detect games and apps" filters detected
        // games inside Papo, which works for any bridge and needs no restart.
        if settings.debug {
            command.arg("--debug");
        }
        command
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }

        #[cfg(target_os = "linux")]
        {
            use std::os::unix::process::CommandExt;
            // If Papo dies without running Drop (crash, SIGKILL), take the
            // helper down too instead of leaving it on port 1337. SIGTERM so
            // flatpak-spawn can forward it to the host process.
            //
            // SAFETY: prctl is async-signal-safe and touches no parent memory;
            // this runs between fork and exec.
            unsafe {
                command.pre_exec(|| {
                    if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
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
        Ok(Self { child })
    }

    fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        self.child.try_wait()
    }

    fn stop(&mut self) {
        if matches!(self.child.try_wait(), Ok(Some(_))) {
            return;
        }
        // Ask first. Inside Flatpak our child is flatpak-spawn: it forwards
        // SIGTERM to rsRPC on the host, but SIGKILL would only kill
        // flatpak-spawn and leave rsRPC running outside the sandbox.
        #[cfg(unix)]
        {
            if let Ok(pid) = libc::pid_t::try_from(self.child.id()) {
                // SAFETY: plain kill(2) on our own, not yet reaped, child.
                unsafe {
                    libc::kill(pid, libc::SIGTERM);
                }
            }
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            while std::time::Instant::now() < deadline {
                if matches!(self.child.try_wait(), Ok(Some(_))) {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            log::warn!("rich presence: rsRPC ignored SIGTERM; killing it");
        }
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
fn rsrpc_command(binary: &std::path::Path) -> Result<std::process::Command, String> {
    if std::env::var_os("FLATPAK_ID").is_none() {
        return Ok(std::process::Command::new(binary));
    }

    // Flatpak has a private /proc, so a process scanner inside the sandbox
    // cannot see the user's games. Keep the release self-contained by copying
    // the bundled helper to Papo's host-visible app-data directory, then ask
    // Flatpak to launch that exact binary on the host.
    let data = crate::platform::dirs::data_dir()
        .ok_or_else(|| "could not locate Papo data directory for rsRPC".to_owned())?;
    let host_dir = data.join("rich-presence").join("rsrpc");
    std::fs::create_dir_all(&host_dir)
        .map_err(|error| format!("could not create rsRPC host directory: {error}"))?;
    let host_binary = host_dir.join("rsrpc");

    let bundled = std::fs::read(binary).map_err(|error| {
        format!(
            "bundled rsRPC helper is unreadable ({}): {error}",
            binary.display()
        )
    })?;
    // Compare contents, not sizes: an update can keep the same length.
    if std::fs::read(&host_binary).ok().as_deref() != Some(bundled.as_slice()) {
        // Write beside it and rename, so a copy still running on the host
        // (ETXTBSY) never blocks the update.
        let staged = host_dir.join("rsrpc.new");
        std::fs::write(&staged, &bundled)
            .map_err(|error| format!("could not stage rsRPC for Flatpak host: {error}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o700))
                .map_err(|error| format!("rsRPC host helper permissions: {error}"))?;
        }
        std::fs::rename(&staged, &host_binary)
            .map_err(|error| format!("could not stage rsRPC for Flatpak host: {error}"))?;
    }

    let mut command = std::process::Command::new("flatpak-spawn");
    command
        .arg("--host")
        .arg("--watch-bus")
        .arg(host_binary);
    Ok(command)
}

#[cfg(not(target_os = "android"))]
fn find_rsrpc_binary() -> Result<std::path::PathBuf, String> {
    let binary_name = if cfg!(target_os = "windows") {
        "rsrpc.exe"
    } else {
        "rsrpc"
    };

    // Developer/testing escape hatch. Packaged releases do not need this.
    if let Some(path) = std::env::var_os("PAPO_RSRPC_PATH") {
        let path = std::path::PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
        return Err(format!(
            "PAPO_RSRPC_PATH does not point to a file: {}",
            path.display()
        ));
    }

    let exe = std::env::current_exe()
        .map_err(|error| format!("could not locate Papo executable: {error}"))?;
    let exe_dir = exe
        .parent()
        .ok_or_else(|| format!("Papo executable has no parent: {}", exe.display()))?;

    // Portable Windows/Linux bundles keep the helper next to Papo. Native
    // Unix packages use ../libexec/papo/rsrpc from /usr/bin or /app/bin.
    let candidates = [
        exe_dir.join(binary_name),
        exe_dir.join("libexec").join("papo").join(binary_name),
        exe_dir
            .parent()
            .map(|prefix| prefix.join("libexec").join("papo").join(binary_name))
            .unwrap_or_default(),
        // Arch keeps package helpers in /usr/lib/<pkg> instead of libexec.
        exe_dir
            .parent()
            .map(|prefix| prefix.join("lib").join("papo").join(binary_name))
            .unwrap_or_default(),
        // macOS app bundles can keep the helper with the main executable.
        exe_dir
            .parent()
            .map(|contents| contents.join("Resources").join(binary_name))
            .unwrap_or_default(),
    ];

    candidates
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| {
            format!(
                "bundled rsRPC helper not found near {} (set PAPO_RSRPC_PATH for development)",
                exe.display()
            )
        })
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
            image: None,
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
        let frame = parse_bridge_message(set).unwrap();
        assert_eq!(frame.socket, "game");
        let bridged = frame.activity.unwrap();
        assert!(!bridged.detected);
        let activity = bridged.activity;
        assert_eq!(activity.name, "Hades II");
        assert_eq!(activity.kind, ActivityKind::Playing);
        assert_eq!(activity.details.as_deref(), Some("Fields of Mourning"));
        assert!(activity.started_at.is_some());

        let clear = r#"{"activity":null,"pid":42,"socketId":"game"}"#;
        let frame = parse_bridge_message(clear).unwrap();
        assert_eq!(frame.socket, "game");
        assert!(frame.activity.is_none());
    }

    #[cfg(not(target_os = "android"))]
    #[test]
    fn detection_switch_filters_only_scanned_games() {
        // Shape of a process-scan frame from arRPC and rsRPC: the socket is
        // the application id and there is no details/state.
        let scanned = r#"{"activity":{"application_id":"356875570916753438","name":"Minecraft","timestamps":{"start":1700000000000}},"pid":7,"socketId":"356875570916753438"}"#;
        let rpc = r#"{"activity":{"application_id":"42","name":"Hades II","details":"Night 3"},"pid":9,"socketId":"0"}"#;

        let mut activities = Activities::default();
        activities.apply(parse_bridge_message(rpc).unwrap());
        activities.apply(parse_bridge_message(scanned).unwrap());
        assert!(parse_bridge_message(scanned).unwrap().activity.unwrap().detected);

        assert_eq!(activities.current(true).unwrap().name, "Minecraft");
        assert_eq!(activities.current(false).unwrap().name, "Hades II");
    }

    #[cfg(not(target_os = "android"))]
    #[test]
    fn only_provider_switches_restart_rsrpc() {
        let base = Settings::default();
        let with = |edit: fn(&mut Settings)| {
            let mut next = base.clone();
            edit(&mut next);
            next
        };

        type Edit = fn(&mut Settings);
        let cases: [(Edit, Effect); 7] = [
            (|s| s.override_activity.name = "Zed".to_owned(), Effect::Nothing),
            (|s| s.game_detection = false, Effect::Republish),
            (|s| s.reconnect_interval = "30".to_owned(), Effect::Retry),
            (|s| s.external_port = "6463".to_owned(), Effect::Reconnect),
            (|s| s.debug = true, Effect::Restart),
            (|s| s.built_in = false, Effect::Restart),
            (|s| s.enabled = false, Effect::Restart),
        ];
        for (edit, expected) in cases {
            assert_eq!(effect_of(&base, &with(edit), true), expected);
        }
        // Without an owned rsRPC, debug and the fallback switch are free.
        assert_eq!(effect_of(&base, &with(|s| s.debug = true), false), Effect::Nothing);
        assert_eq!(effect_of(&base, &with(|s| s.built_in = false), false), Effect::Nothing);
    }

    #[cfg(not(target_os = "android"))]
    #[test]
    fn loopback_spellings_are_the_local_bridge() {
        for host in ["127.0.0.1", "localhost", "::1", " 127.0.0.1 ", ""] {
            let settings = Settings {
                external_host: host.to_owned(),
                ..Settings::default()
            };
            assert!(settings.targets_local_bridge(), "{host:?}");
        }
        let elsewhere = Settings {
            external_port: "6463".to_owned(),
            ..Settings::default()
        };
        assert!(!elsewhere.targets_local_bridge());
        let remote = Settings {
            external_host: "192.168.1.10".to_owned(),
            ..Settings::default()
        };
        assert!(!remote.targets_local_bridge());
    }

    #[cfg(not(target_os = "android"))]
    #[test]
    fn reconnect_interval_is_bounded() {
        let mut settings = Settings {
            reconnect_interval: "0".to_owned(),
            ..Settings::default()
        };
        assert_eq!(settings.reconnect_delay(), std::time::Duration::from_secs(1));
        settings.reconnect_interval = "9999".to_owned();
        assert_eq!(settings.reconnect_delay(), std::time::Duration::from_secs(300));
    }
}
