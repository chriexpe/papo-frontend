//! Android call boundary: core owns protocol state; papo-call owns media; Android owns devices.
use crate::Action;
use crate::bindings::{Backend, CoreError, NativeClient};
use papo_core::api::net::Command;
use papo_core::api::ws::Connection;
use papo_core::runtime::{CallRuntimeEffect, RuntimeEffect, ServerRuntime};
use papo_core::state::call::Phase;
use papo_core::state::{ChannelKind, Store};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

/// Notification/UI handles must stay unique after forgetting/reopening a server or process restart.
fn next_attempt() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static LAST: AtomicU64 = AtomicU64::new(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|time| time.as_nanos() as u64)
        .unwrap_or(1);
    let mut last = LAST.load(Ordering::Relaxed);
    loop {
        let next = now.max(last.saturating_add(1));
        match LAST.compare_exchange_weak(last, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return next,
            Err(current) => last = current,
        }
    }
}

#[uniffi::export(with_foreign)]
pub trait CallHost: Send + Sync {
    fn ready(&self) -> bool;
    fn microphone_allowed(&self) -> bool;
    fn camera_allowed(&self) -> bool;
    fn camera_device(&self, front: bool) -> String;
    fn display_rotation(&self) -> i32;
    fn state_changed(&self);
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct VoiceMemberView {
    pub user_id: String,
    pub state_known: bool,
    pub muted: bool,
    pub camera: bool,
    pub screen: bool,
    pub speaking: bool,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct VoiceRoomView {
    pub channel_id: String,
    pub members: Vec<VoiceMemberView>,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct VoicePlaybackView {
    pub user_id: String,
    pub volume_percent: u16,
    pub muted: bool,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct VoiceCallView {
    pub channel_id: String,
    pub attempt: u64,
    pub phase: String,
    pub connected: bool,
    pub playback_available: bool,
    pub muted: bool,
    pub camera: bool,
    pub front_camera: bool,
    pub error: String,
    pub warning: String,
    pub playback: Vec<VoicePlaybackView>,
    pub rooms: Vec<VoiceRoomView>,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct CallVideoFrame {
    pub width: u32,
    pub height: u32,
    pub sequence: u64,
    pub rgba: Vec<u8>,
}

pub(crate) fn project(store: &Store) -> VoiceCallView {
    let mut rooms: Vec<_> = store
        .call
        .rooms
        .iter()
        .map(|(channel, members)| VoiceRoomView {
            channel_id: channel.clone(),
            members: members
                .iter()
                .map(|person| VoiceMemberView {
                    user_id: person.user_id.clone(),
                    state_known: person.state_known,
                    muted: person.muted,
                    camera: person.camera_on,
                    screen: person.screen_sharing,
                    speaking: channel == &store.call.channel_id
                        && store.call.speaking(&person.user_id),
                })
                .collect(),
        })
        .collect();
    rooms.sort_by(|a, b| a.channel_id.cmp(&b.channel_id));
    VoiceCallView {
        channel_id: store.call.channel_id.clone(),
        attempt: store.call.attempt,
        phase: match store.call.phase {
            Phase::Off => "off",
            Phase::Joining => "joining",
            Phase::In => "in",
        }
        .into(),
        playback_available: store.call.audio_routes_known,
        muted: store.call.muted,
        camera: store.call.camera,
        front_camera: true,
        rooms,
        ..Default::default()
    }
}

#[derive(Default)]
pub(crate) struct Calls {
    host: Option<Arc<dyn CallHost>>,
    owner: String,
    error: String,
    warning: String,
    playback: HashMap<String, VoicePlaybackView>,
    front: bool,
    leaving: Option<(String, String, std::time::Instant)>,
    queued: Option<(String, String)>,
    #[cfg(feature = "calls")]
    media: Option<Media>,
}
#[cfg(feature = "calls")]
struct Media {
    call: papo_call::Call,
    devices: Arc<Devices>,
    attempt: u64,
    ready: bool,
    camera_revision: u64,
}
#[cfg(feature = "calls")]
struct Devices {
    allowed: std::sync::atomic::AtomicBool,
    rotation: std::sync::atomic::AtomicI32,
    cameras: [String; 2],
}
#[cfg(feature = "calls")]
impl Devices {
    fn new(host: &dyn CallHost) -> Self {
        Self {
            allowed: std::sync::atomic::AtomicBool::new(host.camera_allowed()),
            rotation: std::sync::atomic::AtomicI32::new(host.display_rotation()),
            cameras: [host.camera_device(false), host.camera_device(true)],
        }
    }
    fn refresh(&self, host: &dyn CallHost) {
        use std::sync::atomic::Ordering;
        self.allowed.store(host.camera_allowed(), Ordering::Relaxed);
        self.rotation
            .store(host.display_rotation(), Ordering::Relaxed);
    }
}
#[cfg(feature = "calls")]
struct Host {
    devices: Arc<Devices>,
    ready: bool,
    wake: std::sync::mpsc::SyncSender<()>,
}
#[cfg(feature = "calls")]
impl Host {
    fn new(callback: Arc<dyn CallHost>, devices: Arc<Devices>) -> Self {
        let ready = callback.ready();
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        // JNA may detach native threads after a foreign callback. GStreamer's Android plugins
        // cache JNIEnv in thread-local storage; detaching that media thread invalidates it and
        // crashes a later camera/JNI operation. Only this separate notifier calls into Kotlin.
        std::thread::Builder::new()
            .name("papo-call-wake".into())
            .spawn(move || {
                while rx.recv().is_ok() {
                    callback.state_changed();
                }
            })
            .expect("call wake worker");
        Self {
            devices,
            ready,
            wake: tx,
        }
    }
}
#[cfg(feature = "calls")]
impl papo_call::Host for Host {
    fn initialize(&self) -> bool {
        if !self.ready || gstreamer::init().is_err() {
            return false;
        }
        papo_call::platform::initialize_platform();
        true
    }
    fn camera_allowed(&self) -> bool {
        self.devices
            .allowed
            .load(std::sync::atomic::Ordering::Relaxed)
    }
    fn camera_device(&self, front: bool) -> String {
        self.devices.cameras[usize::from(front)].clone()
    }
    fn display_rotation(&self) -> i32 {
        self.devices
            .rotation
            .load(std::sync::atomic::Ordering::Relaxed)
    }
    fn wake(&self) {
        let _ = self.wake.try_send(());
    }
    fn frame_ready(&self) {}
}

impl Calls {
    pub fn clear_error(&mut self) {
        self.error.clear();
        self.warning.clear();
    }
    pub fn projection(&self, store: &Store) -> VoiceCallView {
        let mut value = project(store);
        value.error = self.error.clone();
        value.warning = self.warning.clone();
        value.front_camera = self.front;
        value.playback = self.playback.values().cloned().collect();
        value.playback.sort_by(|a, b| a.user_id.cmp(&b.user_id));
        #[cfg(feature = "calls")]
        {
            value.connected = self
                .media
                .as_ref()
                .is_some_and(|media| media.call.shared().is_live());
        }
        value
    }
    pub(super) fn stop(&mut self, runtime: &mut ServerRuntime, tell_server: bool) {
        #[cfg(not(feature = "calls"))]
        let _ = tell_server;
        // The protocol has no attempt id on leave acknowledgements. Do not enter another
        // room until the old acknowledgement has passed through Store; otherwise a late
        // same-room leave can terminate the new session. ICE fetches alone never join.
        #[cfg(feature = "calls")]
        if tell_server && self.media.is_some() && runtime.store.call.active() {
            self.leaving = Some((
                runtime.store.call.channel_id.clone(),
                runtime.store.me.clone(),
                std::time::Instant::now(),
            ));
            if let Some(host) = self.host.clone() {
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(10));
                    host.state_changed();
                });
            }
            runtime.net.send(Command::VoiceSignal(serde_json::json!({"type":"voice_leave", "channel_id": runtime.store.call.channel_id}).to_string()));
        }
        runtime.net.set_event_callback(None);
        #[cfg(feature = "calls")]
        {
            self.media = None;
        }
        let channel = runtime.store.call.channel_id.clone();
        runtime.store.call.remove(&channel, &runtime.store.me);
        runtime.store.call.left();
        self.playback.clear();
        self.owner.clear();
    }
    pub fn effect(&mut self, runtime: &mut ServerRuntime, effect: RuntimeEffect) {
        let RuntimeEffect::Call(effect) = effect else {
            return;
        };
        match *effect {
            #[cfg(feature = "calls")]
            CallRuntimeEffect::VoiceReady {
                channel_id,
                attempt,
                servers,
            } if runtime.store.call.current(&channel_id, attempt) => {
                let Some(host) = self.host.clone() else {
                    self.error = "media-unavailable".into();
                    self.stop(runtime, true);
                    return;
                };
                let devices = Arc::new(Devices::new(host.as_ref()));
                match papo_call::Call::start(
                    channel_id.clone(),
                    papo_call::IceConfig::from_servers(&servers),
                    Arc::new(Host::new(host, Arc::clone(&devices))),
                    runtime.net.sender(),
                ) {
                    Some(call) => {
                        runtime.net.set_event_callback(Some(call.event_callback()));
                        runtime.net.send(Command::VoiceSignal(
                            serde_json::json!({"type": "voice_join", "channel_id": channel_id})
                                .to_string(),
                        ));
                        self.media = Some(Media {
                            call,
                            devices,
                            attempt,
                            ready: false,
                            camera_revision: 0,
                        });
                    }
                    None => {
                        self.error = "media-unavailable".into();
                        self.stop(runtime, true);
                    }
                }
            }
            CallRuntimeEffect::ConnectionOffline => {
                self.leaving = None;
                self.queued = None;
                if !self.owner.is_empty() {
                    self.error = "connection-lost".into();
                    self.stop(runtime, false);
                }
            }
            CallRuntimeEffect::VoiceFailed {
                was_current: true, ..
            } => {
                self.error = "join-failed".into();
                self.stop(runtime, false);
            }
            CallRuntimeEffect::Event { event, .. } => {
                if let papo_core::api::ws::Event::VoiceLeft {
                    channel_id,
                    user_id,
                } = event.as_ref()
                    && self.leaving.as_ref().is_some_and(|(channel, owner, _)| {
                        channel == channel_id && owner == user_id
                    })
                {
                    self.leaving = None;
                    if let Some((channel, owner)) = self.queued.take()
                        && owner == runtime.store.me
                        && runtime.store.connection == Connection::Online
                    {
                        self.begin_join(runtime, channel);
                    }
                }
                if let papo_core::api::ws::Event::Failure {
                    code: Some(code), ..
                } = *event
                    && !runtime.store.call.active()
                {
                    self.error = code;
                }
            }
            _ => {}
        }
    }
    #[cfg_attr(not(feature = "calls"), allow(dead_code))]
    fn begin_join(&mut self, runtime: &mut ServerRuntime, channel_id: String) {
        if !runtime
            .store
            .channel(&channel_id)
            .is_some_and(|channel| channel.kind == ChannelKind::Voice)
        {
            self.error = "call-room-unavailable".into();
            return;
        }
        self.owner = runtime.store.me.clone();
        self.error.clear();
        self.warning.clear();
        self.front = true;
        runtime.store.call.joining(channel_id.clone());
        let attempt = next_attempt();
        runtime.store.call.attempt = attempt;
        runtime.net.send(Command::FetchVoiceIce {
            channel_id,
            attempt,
        });
    }
    pub fn pump(&mut self, runtime: &mut ServerRuntime) {
        if self
            .leaving
            .as_ref()
            .is_some_and(|(_, _, since)| since.elapsed() >= std::time::Duration::from_secs(10))
        {
            // Keep the barrier until acknowledgement/disconnect. A timeout cannot prove
            // that the old server session disappeared or make its future events safe.
            if self.queued.take().is_some() {
                self.error = "leave-timeout".into();
            }
        }
        if !self.owner.is_empty()
            && (self.owner != runtime.store.me
                || !runtime.store.call.active()
                || !runtime
                    .store
                    .channel(&runtime.store.call.channel_id)
                    .is_some_and(|channel| channel.kind == ChannelKind::Voice))
        {
            self.stop(runtime, false);
            #[cfg(feature = "calls")]
            return;
        }
        #[cfg(feature = "calls")]
        if let Some(media) = &mut self.media {
            if !runtime
                .store
                .call
                .current(&media.call.channel_id, media.attempt)
            {
                self.stop(runtime, false);
                return;
            }
            if let Some(host) = &self.host {
                media.devices.refresh(host.as_ref());
            }
            let shared = media.call.shared();
            if shared.failed() {
                log::warn!("call media failure: {:?}", shared.error());
                self.error = "media-failed".into();
                self.stop(runtime, true);
                return;
            }
            if shared.take_warning().is_some() {
                self.warning = "device-unavailable".into();
            }
            if !media.ready && runtime.store.call.phase == Phase::In {
                media.call.send(papo_call::Command::Ready);
                media
                    .call
                    .send(papo_call::Command::Muted(runtime.store.call.muted));
                media.ready = true;
            }
            let (revision, camera) = shared.camera_state();
            if revision != media.camera_revision {
                media.camera_revision = revision;
                runtime.store.call.camera = camera;
            }
            if media.ready {
                let me = &runtime.store.me;
                let mut cameras: Vec<_> = runtime
                    .store
                    .call
                    .members()
                    .iter()
                    .filter(|m| m.camera_on && &m.user_id != me)
                    .map(|m| m.user_id.clone())
                    .collect();
                let mut screens: Vec<_> = runtime
                    .store
                    .call
                    .members()
                    .iter()
                    .filter(|m| m.screen_sharing && &m.user_id != me)
                    .map(|m| m.user_id.clone())
                    .collect();
                cameras.sort();
                screens.sort();
                // Repeated desired sets are idempotent; the engine owns its six SFU slots.
                media.call.send(papo_call::Command::Watching(cameras));
                media
                    .call
                    .send(papo_call::Command::WatchingScreens(screens));
            }
        }
    }
    pub fn action(&mut self, runtime: &mut ServerRuntime, action: Action) -> Result<(), String> {
        if let Action::JoinVoice { channel_id } = action {
            if runtime.store.screen != papo_core::state::Screen::Chat
                || runtime.store.connection != Connection::Online
            {
                return Err("call-offline".into());
            }
            if !runtime
                .store
                .channel(&channel_id)
                .is_some_and(|channel| channel.kind == ChannelKind::Voice)
            {
                return Err("call-room-unavailable".into());
            }
            let host = self.host.as_ref().ok_or("media-unavailable")?;
            if !host.ready() {
                return Err("media-unavailable".into());
            }
            if !host.microphone_allowed() {
                return Err("microphone-permission".into());
            }
            #[cfg(not(feature = "calls"))]
            return Err("media-unavailable".into());
            #[cfg(feature = "calls")]
            {
                self.stop(runtime, true);
                if self.leaving.as_ref().is_some_and(|(_, _, since)| {
                    since.elapsed() >= std::time::Duration::from_secs(10)
                }) {
                    return Err("leave-timeout".into());
                }
                if self.leaving.is_some() {
                    self.queued = Some((channel_id, runtime.store.me.clone()));
                } else {
                    self.begin_join(runtime, channel_id);
                }
                return Ok(());
            }
        }
        let attempt = match &action {
            Action::LeaveVoice { attempt }
            | Action::MuteVoice { attempt, .. }
            | Action::CameraVoice { attempt, .. }
            | Action::VoicePlayback { attempt, .. }
            | Action::CameraFacing { attempt, .. } => *attempt,
            _ => return Err("invalid-call-action".into()),
        };
        if matches!(action, Action::LeaveVoice { .. }) && runtime.store.call.attempt == attempt {
            // A queued replacement has no media/room yet, but must still be cancellable.
            self.queued = None;
        }
        if !runtime.store.call.active()
            || runtime.store.call.attempt != attempt
            || self.owner != runtime.store.me
        {
            return Ok(());
        }
        match action {
            Action::LeaveVoice { .. } => self.stop(runtime, true),
            Action::MuteVoice { muted, .. } => {
                if !muted
                    && !self
                        .host
                        .as_ref()
                        .is_some_and(|host| host.microphone_allowed())
                {
                    return Err("microphone-permission".into());
                }
                runtime.store.call.muted = muted;
                #[cfg(feature = "calls")]
                if let Some(media) = &self.media {
                    media.call.send(papo_call::Command::Muted(muted));
                }
            }
            Action::CameraVoice { on, .. } => {
                if on && !self.host.as_ref().is_some_and(|host| host.camera_allowed()) {
                    return Err("camera-permission".into());
                }
                #[cfg(feature = "calls")]
                if let Some(media) = &self.media {
                    if let Some(host) = &self.host {
                        media.devices.refresh(host.as_ref());
                    }
                    if media.ready {
                        media.call.send(papo_call::Command::Camera(on));
                    }
                }
            }
            Action::CameraFacing { front, .. } => {
                self.front = front;
                #[cfg(feature = "calls")]
                if let Some(media) = &self.media {
                    media.call.send(papo_call::Command::CameraFacing(front));
                }
            }
            Action::VoicePlayback {
                user_id,
                volume_percent,
                muted,
                ..
            } => {
                if user_id == runtime.store.me
                    || !runtime
                        .store
                        .call
                        .members()
                        .iter()
                        .any(|member| member.user_id == user_id)
                {
                    return Ok(());
                }
                if !runtime.store.call.audio_routes_known {
                    return Err("audio-routing-unavailable".into());
                }
                let volume_percent = volume_percent.min(200);
                self.playback.insert(
                    user_id.clone(),
                    VoicePlaybackView {
                        user_id: user_id.clone(),
                        volume_percent,
                        muted,
                    },
                );
                #[cfg(feature = "calls")]
                if let Some(media) = &self.media {
                    media.call.send(papo_call::Command::Playback {
                        user_id,
                        setting: papo_call::Playback {
                            volume_percent,
                            muted,
                        },
                    });
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn frame(
        &self,
        store: &Store,
        owner: &str,
        attempt: u64,
        user: &str,
        kind: &str,
        after: u64,
    ) -> Option<CallVideoFrame> {
        if owner != store.me
            || !store.call.active()
            || attempt != store.call.attempt
            || owner != self.owner
        {
            return None;
        }
        #[cfg(feature = "calls")]
        {
            let media = self.media.as_ref()?;
            let shared = media.call.shared();
            let (sequence, frame) = if user == owner && kind == "video" {
                (
                    shared.preview_sequence(),
                    shared.preview.lock().ok()?.clone()?,
                )
            } else {
                let kind = match kind {
                    "video" => papo_call::Kind::Camera,
                    "screen" => papo_call::Kind::Screen,
                    _ => return None,
                };
                let tiles = shared.tiles.lock().ok()?;
                let tile = tiles.iter().find(|tile| {
                    tile.publisher.as_deref() == Some(user) && tile.kind == Some(kind)
                })?;
                (tile.seq, tile.frame.clone()?)
            };
            if sequence == after {
                return None;
            }
            Some(CallVideoFrame {
                width: frame.width as u32,
                height: frame.height as u32,
                sequence,
                rgba: frame.pixels.iter().flatten().copied().collect(),
            })
        }
        #[cfg(not(feature = "calls"))]
        {
            let _ = (user, kind, after);
            None
        }
    }
}

#[uniffi::export]
impl NativeClient {
    pub fn set_call_host(&self, host: Arc<dyn CallHost>) -> Result<(), CoreError> {
        let mut backend = self
            .backend
            .lock()
            .map_err(|_| "Client lock failed".to_owned())?;
        if let Some(Backend::Live(client)) = backend.as_mut() {
            client.calls.host = Some(host);
        }
        Ok(())
    }
    pub fn call_video_frame(
        &self,
        owner: String,
        attempt: u64,
        user: String,
        kind: String,
        after: u64,
    ) -> Result<Option<CallVideoFrame>, CoreError> {
        let backend = self
            .backend
            .lock()
            .map_err(|_| "Client lock failed".to_owned())?;
        Ok(match backend.as_ref() {
            Some(Backend::Live(client)) => {
                client
                    .calls
                    .frame(&client.runtime.store, &owner, attempt, &user, &kind, after)
            }
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn attempt_ids_do_not_restart_with_a_new_client() {
        let first = super::next_attempt();
        let _replacement = super::Calls::default();
        let second = super::next_attempt();
        assert!(second > first);
    }

    #[test]
    fn concurrent_clients_receive_distinct_attempt_ids() {
        let start = std::sync::Arc::new(std::sync::Barrier::new(8));
        let workers: Vec<_> = (0..8)
            .map(|_| {
                let start = std::sync::Arc::clone(&start);
                std::thread::spawn(move || {
                    start.wait();
                    (0..128).map(|_| super::next_attempt()).collect::<Vec<_>>()
                })
            })
            .collect();
        let mut ids: Vec<_> = workers
            .into_iter()
            .flat_map(|worker| worker.join().unwrap())
            .collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 8 * 128);
    }
}
