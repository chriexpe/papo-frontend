//! Thin presentation adapter. All transport, reduction, reconciliation and
//! outgoing delivery remain in papo-core. No egui or media engine is linked.
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use papo_core::api::models::ReactionRequest;
use papo_core::api::net::{Command, NetworkAvailability, NetworkHint, Update, Wake};
use papo_core::api::ws::Connection;
use papo_core::cache::{ClientDb, OutgoingState, now_millis};
use papo_core::runtime::{DEFAULT_DRAIN_LIMIT, ServerRuntime};
use papo_core::state::{ChannelKind, Emoji, Screen, TimelineStatus};
use papo_core::storage::SecretStore;
use serde::Deserialize;
use serde_json::{Value, json};

mod account;
mod admin_audit;
mod administration;
pub mod bindings;
pub mod demo;
mod discovery;
mod inbox;
mod legacy;
mod markdown;
pub mod media;
mod media_edit;
mod message_results;
mod moderation;
mod notifications;
mod projection;
use projection::project;
#[cfg(any(test, all(debug_assertions, target_os = "android")))]
mod reading_debug;
mod recovery;
uniffi::setup_scaffolding!();

fn reply_notifications_default() -> bool {
    true
}

/// The wire contract is deliberately small and versioned. Kotlin only renders
/// projections; it never writes the core Store or implements backend requests.
#[derive(Debug, Deserialize, uniffi::Enum)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Login {
        username: String,
        password: String,
    },
    Register {
        username: String,
        password: String,
    },
    Unlock {
        password: String,
    },
    CreateServer {
        name: String,
    },
    Search {
        query: String,
        more: bool,
    },
    ClearSearch,
    ChannelNotifications {
        channel_id: String,
        setting: String,
    },
    MarkServerRead,
    SaveServer {
        name: String,
        public: bool,
        password: Option<String>,
    },
    SaveChannel {
        channel_id: String,
        name: String,
        topic: String,
    },
    SaveRole {
        role_id: String,
        name: String,
        color: String,
        permissions: discovery::Capabilities,
    },
    SelectChannel {
        channel_id: String,
    },
    OpenDm {
        user_id: String,
    },
    LoadProfile {
        user_id: String,
    },
    Send {
        local_id: String,
        channel_id: String,
        content: String,
        reply_to: Option<String>,
        #[serde(default = "reply_notifications_default")]
        notify_reply: bool,
    },
    SendAttachments {
        local_id: String,
        channel_id: String,
        content: String,
        reply_to: Option<String>,
        #[serde(default = "reply_notifications_default")]
        notify_reply: bool,
        files: Vec<UploadFile>,
    },
    Edit {
        message_id: String,
        content: String,
    },
    Delete {
        message_id: String,
    },
    React {
        channel_id: String,
        message_id: String,
        emoji: String,
        custom: bool,
        add: bool,
    },
    ReactionDetails {
        channel_id: String,
        message_id: String,
        more: bool,
    },
    Retry {
        local_id: String,
    },
    Dismiss {
        local_id: String,
    },
    JumpToMessage {
        channel_id: String,
        message_id: String,
    },
    Latest {
        channel_id: String,
    },
    Newer {
        channel_id: String,
    },
    Older,
    Refresh,
    Typing {
        channel_id: String,
    },
    Visible {
        channel_id: String,
        ids: Vec<String>,
        at_latest: bool,
        owner_user_id: String,
    },
    CatchUp {
        channel_id: String,
        owner_user_id: String,
    },
    CancelCatchUp {
        channel_id: String,
    },
    BreakReadSpan {
        channel_id: String,
    },
    ClearCatchUp {
        channel_id: String,
        owner_user_id: String,
    },
    Network {
        available: bool,
        epoch: u64,
    },
    ClearError,
    Logout,
}

#[derive(Clone, Debug, Deserialize, uniffi::Record)]
pub struct UploadFile {
    pub path: String,
    pub name: String,
    pub mime: String,
    pub size: u64,
    #[serde(default)]
    pub spoiler: bool,
}

fn prepare_uploads(files: Vec<UploadFile>) -> Result<Vec<papo_core::api::client::Upload>, String> {
    if files.is_empty() || files.len() > 10 {
        return Err("Choose between one and ten attachments".into());
    }
    files
        .into_iter()
        .map(|file| {
            let path = PathBuf::from(&file.path);
            let metadata = std::fs::metadata(&path)
                .map_err(|_| format!("{} is no longer available. Choose it again.", file.name))?;
            if !metadata.is_file() || metadata.len() != file.size {
                return Err(format!("{} changed. Choose it again.", file.name));
            }
            if file.name.trim().is_empty()
                || file.name.len()
                    + if file.spoiler && !file.name.starts_with("SPOILER_") {
                        8
                    } else {
                        0
                    }
                    > 255
                || file
                    .name
                    .chars()
                    .any(|c| c.is_control() || matches!(c, '/' | '\\'))
                || matches!(file.name.as_str(), "." | "..")
            {
                return Err("Invalid attachment filename".into());
            }
            Ok(papo_core::api::client::Upload {
                path,
                name: file.name,
                mime: file.mime,
                size: file.size,
                spoiler: file.spoiler,
                anonymize: false,
            })
        })
        .collect()
}

pub fn normalize_url(input: &str) -> Result<String, String> {
    let input = input.trim();
    let value = if input.contains("://") {
        input.to_owned()
    } else {
        format!("https://{input}")
    };
    let url = url::Url::parse(&value).map_err(|_| "Enter a valid server address".to_owned())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("Use an http(s) server address without credentials, query or fragment".into());
    }
    Ok(url.as_str().trim_end_matches('/').to_owned())
}

pub struct Client {
    #[cfg(all(debug_assertions, target_os = "android"))]
    reading_debug: reading_debug::ReadingDebug,
    runtime: ServerRuntime,
    notification_listener: Option<Arc<dyn notifications::NotificationListener>>,
    notification_visible: bool,
    push_registration: Option<(String, String)>,
    _notification_lease: notifications::InteractiveLease,
    // Serialize one user's mutation per message/emoji until HTTP settles.
    pending_reactions: HashMap<(String, Emoji), PendingReaction>,
    navigation_serial: u64,
    navigation_target: String,
    search_query: String,
    watched_pins: Option<(String, String)>,
    watched_pins_dirty: bool,
    catching_up: Option<(String, String)>,
    read_in_flight: HashSet<String>,
    read_attempts: HashMap<String, (String, std::time::Instant)>,
    read_seeks: HashMap<String, (String, (chrono::DateTime<chrono::Utc>, String))>,
}

struct PendingReaction {
    previous: bool,
    previous_count: u32,
    in_flight: bool,
    desired: bool,
    channel_id: String,
}

// All simultaneous server runtimes share one database worker, as in egui.
// Weak entries don't keep the worker alive after the last client is closed.
static DATABASES: OnceLock<Mutex<HashMap<PathBuf, Weak<ClientDb>>>> = OnceLock::new();
fn database(path: PathBuf) -> Result<Arc<ClientDb>, String> {
    let mut databases = DATABASES
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| "Database registry failed")?;
    if let Some(cache) = databases.get(&path).and_then(Weak::upgrade) {
        return Ok(cache);
    }
    let cache = Arc::new(ClientDb::open(Some(path.clone())));
    if !cache.is_enabled() {
        return Err("Cannot open the message database; messages have not been sent".into());
    }
    databases.insert(path, Arc::downgrade(&cache));
    Ok(cache)
}

impl Client {
    pub fn open(
        url: &str,
        db_path: PathBuf,
        secrets: Arc<dyn SecretStore>,
        wake: Wake,
    ) -> Result<Self, String> {
        Self::open_notifying(url, db_path, secrets, wake, None)
    }

    fn open_notifying(
        url: &str,
        db_path: PathBuf,
        secrets: Arc<dyn SecretStore>,
        wake: Wake,
        listener: Option<Arc<dyn notifications::NotificationListener>>,
    ) -> Result<Self, String> {
        #[cfg(target_os = "android")]
        android_logger::init_once(
            android_logger::Config::default()
                .with_tag("PapoCompose")
                .with_max_level(log::LevelFilter::Info),
        );
        let url = normalize_url(url)?;
        let lease = notifications::InteractiveLease::acquire(&url, &db_path);
        #[cfg(all(debug_assertions, target_os = "android"))]
        let reading_debug =
            reading_debug::ReadingDebug::new(&db_path, &papo_core::server_key(&url));
        let cache = database(db_path)?;
        let notifications = notifications::coordinator(Arc::clone(&cache), listener.clone());
        Ok(Self {
            #[cfg(all(debug_assertions, target_os = "android"))]
            reading_debug,
            runtime: ServerRuntime::open(url, wake, secrets, cache, notifications),
            notification_listener: listener,
            notification_visible: false,
            push_registration: None,
            _notification_lease: lease,
            pending_reactions: HashMap::new(),
            navigation_serial: 0,
            navigation_target: String::new(),
            search_query: String::new(),
            watched_pins: None,
            watched_pins_dirty: false,
            catching_up: None,
            read_in_flight: HashSet::new(),
            read_attempts: HashMap::new(),
            read_seeks: HashMap::new(),
        })
    }

    // UniFFI calls run on a dedicated Kotlin dispatcher, never on Android's UI
    // thread. Cache responses are bounded and checked against the owner epoch.
    fn hydrate(&mut self, channel: &str, before: Option<(i64, u32, String)>, older: bool) {
        let epoch = self.runtime.store.cache_restore_epoch();
        let result = self
            .runtime
            .cache
            .load_channel_page_async(&self.runtime.server_key, channel, before)
            .and_then(|rx| rx.recv_timeout(Duration::from_secs(2)).ok());
        if epoch != self.runtime.store.cache_restore_epoch() {
            return;
        }
        match result {
            Some(Ok(page)) => self.runtime.store.restore_cached_page(page, older),
            _ => self.runtime.store.cached_page_failed(channel, older),
        }
    }

    pub fn poll(&mut self) -> Value {
        #[cfg(all(debug_assertions, target_os = "android"))]
        let reading_before = self.reading_debug_state();
        #[cfg(all(debug_assertions, target_os = "android"))]
        let mut reading_updates = Vec::new();
        self.sync_notification_context();
        let previous_owner = self.runtime.store.me.clone();
        // A hard bound prevents a busy socket from monopolizing the actor.
        let mut events = Vec::new();
        let mut count = 0;
        while count < DEFAULT_DRAIN_LIMIT {
            let Some(update) = self.runtime.net.try_recv() else {
                break;
            };
            #[cfg(all(debug_assertions, target_os = "android"))]
            if reading_updates.len() < 32 {
                reading_updates.push(Self::reading_debug_update(&update));
            }
            match &update {
                Update::RemoteReadAnchor {
                    owner_user_id,
                    channel_id,
                    message_id,
                    next,
                    failed,
                    ..
                } if owner_user_id == &self.runtime.store.me => {
                    let key = format!("anchor/{channel_id}");
                    self.read_in_flight.remove(&key);
                    if *failed {
                        self.read_attempts
                            .insert(key, (message_id.clone(), std::time::Instant::now()));
                    } else {
                        self.read_attempts.remove(&key);
                    }
                    if let Some(cursor) = next {
                        self.read_seeks
                            .insert(channel_id.clone(), (message_id.clone(), cursor.clone()));
                    } else if !failed {
                        self.read_seeks.remove(channel_id);
                    }
                }
                Update::ChannelReadSynced {
                    owner_user_id,
                    channel_id,
                    message_id,
                    success,
                } if owner_user_id == &self.runtime.store.me => {
                    self.read_in_flight.remove(channel_id);
                    if *success {
                        self.read_attempts.remove(channel_id);
                    } else {
                        self.read_attempts.insert(
                            channel_id.clone(),
                            (message_id.clone(), std::time::Instant::now()),
                        );
                    }
                }
                Update::NotificationReadsSynced {
                    owner_user_id,
                    success,
                    ..
                } if owner_user_id == &self.runtime.store.me => {
                    self.read_in_flight.remove("notifications");
                    if *success {
                        self.read_attempts.remove("notifications");
                    } else {
                        self.read_attempts.insert(
                            "notifications".into(),
                            (String::new(), std::time::Instant::now()),
                        );
                    }
                }
                Update::Connection(Connection::Offline) => {
                    self.read_in_flight.clear();
                }
                Update::ReactionComplete {
                    message_id,
                    emoji,
                    success,
                    ..
                } => {
                    if let Some(kind) =
                        Emoji::from_parts(emoji.unicode.clone(), emoji.emoji_id.clone())
                        && let Some(pending) = self
                            .pending_reactions
                            .remove(&(message_id.clone(), kind.clone()))
                    {
                        if !success {
                            self.runtime.store.restore_reaction_local(
                                message_id,
                                &kind,
                                pending.previous,
                                pending.previous_count,
                            );
                        } else {
                            self.runtime.store.set_reaction_local(
                                message_id,
                                &kind,
                                pending.desired,
                            );
                            if pending.desired != pending.in_flight {
                                // Coalesce rapid taps to the user's latest intent; never race
                                // POST against DELETE or retry a failed request automatically.
                                self.runtime.net.send(Command::React {
                                    channel_id: pending.channel_id.clone(),
                                    message_id: message_id.clone(),
                                    emoji: emoji.clone(),
                                    add: pending.desired,
                                });
                                self.pending_reactions.insert(
                                    (message_id.clone(), kind),
                                    PendingReaction {
                                        previous: pending.in_flight,
                                        previous_count: if pending.in_flight == pending.previous {
                                            pending.previous_count
                                        } else if pending.in_flight {
                                            pending.previous_count.saturating_add(1)
                                        } else {
                                            pending.previous_count.saturating_sub(1)
                                        },
                                        in_flight: pending.desired,
                                        ..pending
                                    },
                                );
                            }
                        }
                    }
                }
                Update::Event(event) => {
                    if let papo_core::api::ws::Event::Reaction {
                        message_id,
                        unicode,
                        emoji_id,
                        count,
                    } = event.as_ref()
                        && let Some(kind) = Emoji::from_parts(unicode.clone(), emoji_id.clone())
                        && let Some(pending) =
                            self.pending_reactions.get_mut(&(message_id.clone(), kind))
                    {
                        pending.previous_count = (*count).clamp(0, i64::from(u32::MAX)) as u32;
                    }
                }
                Update::Session(None) => self.pending_reactions.clear(),
                Update::Outgoing(item) => {
                    events.push(json!({"type":"accepted", "localId": item.local_id}))
                }
                Update::SendConfirmed { local_id, .. } => {
                    events.push(json!({"type":"accepted", "localId":local_id}));
                }
                Update::UploadFailed {
                    local_id,
                    message,
                    state,
                } => {
                    events.push(json!({"type":if *state == OutgoingState::UnknownOutcome { "unknown" } else { "rejected" }, "localId":local_id,"error":message}));
                }
                Update::OutgoingRejected {
                    local_id, message, ..
                } => events.push(json!({"type":"rejected", "localId": local_id, "error": message})),
                _ => {}
            }
            self.observe_pin_update(&update);
            self.runtime.process_update(update);
            self.sync_notification_context();
            count += 1;
        }
        if !self.navigation_target.is_empty()
            && !self
                .runtime
                .store
                .loading_message_context(&self.runtime.store.selected_channel)
            && !self
                .runtime
                .store
                .messages_in(&self.runtime.store.selected_channel)
                .any(|m| m.id == self.navigation_target)
        {
            self.navigation_target.clear();
        }
        if self.runtime.store.me != previous_owner {
            self.search_query.clear();
            self.watched_pins = None;
            self.watched_pins_dirty = false;
            self.catching_up = None;
            self.read_in_flight.clear();
            self.read_attempts.clear();
            self.read_seeks.clear();
            self.runtime.store.reset_search();
        }
        self.refresh_watched_pins();
        let mut more = count == DEFAULT_DRAIN_LIMIT;
        if let Some(id) = self.runtime.store.channel_needing_cache() {
            self.runtime.store.mark_cache_loading(&id);
            self.hydrate(&id, None, false);
        }
        if let Some(id) = self.runtime.store.channel_needing_messages() {
            let ticket = self.runtime.store.mark_loading(&id);
            self.runtime.net.send(Command::LoadMessages { ticket });
        }
        more |= self.advance_catch_up();
        self.flush_store();
        self.sync_read_receipts();
        let mut snapshot = self.snapshot();
        snapshot["more"] = json!(more);
        snapshot["events"] = json!(events);
        #[cfg(all(debug_assertions, target_os = "android"))]
        {
            let after = self.reading_debug_state();
            self.reading_debug.record(
                "poll",
                reading_before,
                after,
                json!({"updates":reading_updates}),
            );
        }
        snapshot
    }

    /// Reuse normal cache/server pagination until the persisted frontier is
    /// covered. Do not navigate to an approximate anchor or expose a fake head.
    fn advance_catch_up(&mut self) -> bool {
        let Some((owner, channel)) = self.catching_up.clone() else {
            return false;
        };
        let store = &mut self.runtime.store;
        if owner != store.me || channel != store.selected_channel || store.error.is_some() {
            self.catching_up = None;
            return false;
        }
        if store.loading_older(&channel)
            || store.loading_message_context(&channel)
            || store.connection == Connection::Online
                && store.timeline_status(&channel) != TimelineStatus::Fresh
        {
            return false;
        }
        if store.catch_up_needs_history(&channel) && store.can_load_older(&channel) {
            let previous = store.messages.len();
            if let Err(error) = self.dispatch(Action::Older) {
                self.runtime.store.error = Some(error);
                self.catching_up = None;
            }
            return self.runtime.store.messages.len() != previous;
        }
        if store.catch_up_needs_history(&channel) && store.connection != Connection::Online {
            store.error = Some("Connect to load earlier unread messages.".into());
            self.catching_up = None;
            return false;
        }
        self.navigation_target = store
            .hourglass_navigation_target(&channel)
            .or_else(|| store.newest_loaded(&channel))
            .unwrap_or_default();
        self.navigation_serial = self.navigation_serial.saturating_add(1);
        store.reset_read_span(&channel);
        self.catching_up = None;
        false
    }

    fn sync_read_receipts(&mut self) {
        let store = &self.runtime.store;
        if store.connection != Connection::Online || store.me.is_empty() {
            return;
        }
        for (channel_id, message_id) in store.pending_remote_read_anchors() {
            if channel_id != store.selected_channel {
                continue;
            }
            let key = format!("anchor/{channel_id}");
            if self.read_in_flight.contains(&key) {
                continue;
            }
            if self
                .read_attempts
                .get(&key)
                .is_some_and(|(id, at)| id == &message_id && at.elapsed() < Duration::from_secs(10))
            {
                continue;
            }
            self.read_in_flight.insert(key.clone());
            self.read_attempts
                .insert(key, (message_id.clone(), std::time::Instant::now()));
            let before = self
                .read_seeks
                .get(&channel_id)
                .filter(|(target, _)| target == &message_id)
                .map(|(_, cursor)| cursor.clone());
            self.runtime.net.send(Command::LoadReadAnchor {
                owner_user_id: store.me.clone(),
                channel_id,
                message_id,
                before,
            });
        }
        let notifications = store.pending_notification_reads();
        if !notifications.is_empty()
            && !self.read_in_flight.contains("notifications")
            && self
                .read_attempts
                .get("notifications")
                .is_none_or(|(_, at)| at.elapsed() >= Duration::from_secs(10))
        {
            self.read_in_flight.insert("notifications".into());
            self.read_attempts.insert(
                "notifications".into(),
                (String::new(), std::time::Instant::now()),
            );
            self.runtime.net.send(Command::MarkNotificationsRead {
                user_id: store.me.clone(),
                ids: notifications,
            });
        }
        for (channel_id, message_id) in store.pending_read_receipts() {
            if self.read_in_flight.contains(&channel_id) {
                continue;
            }
            if self
                .read_attempts
                .get(&channel_id)
                .is_some_and(|(_, at)| at.elapsed() < Duration::from_secs(10))
            {
                continue;
            }
            self.read_in_flight.insert(channel_id.clone());
            self.read_attempts.insert(
                channel_id.clone(),
                (message_id.clone(), std::time::Instant::now()),
            );
            self.runtime.net.send(Command::MarkChannelRead {
                owner_user_id: store.me.clone(),
                channel_id,
                message_id,
            });
        }
    }

    fn flush_store(&mut self) {
        let ops = self.runtime.store.take_cache_ops();
        if !ops.is_empty() {
            self.runtime.cache.submit(&self.runtime.server_key, ops);
        }
    }

    pub fn dispatch(&mut self, action: Action) -> Result<(), String> {
        if let Action::Search { query, more } = action {
            return self.search(query, more);
        }
        let store = &mut self.runtime.store;
        let command = match action {
            Action::Search { .. } => unreachable!(),
            Action::ClearSearch => {
                store.reset_search();
                self.search_query.clear();
                return Ok(());
            }
            Action::ChannelNotifications {
                channel_id,
                setting,
            } => {
                discovery::check_notifications(store, &channel_id, &setting)?;
                Command::SetChannelNotifications {
                    channel_id,
                    setting,
                }
            }
            Action::MarkServerRead => {
                store.mark_all_read();
                store.queue_all_notification_reads();
                self.flush_store();
                return Ok(());
            }
            Action::SaveServer {
                name,
                public,
                password,
            } => {
                if !store.can_manage_server() {
                    return Err("You cannot change this server".into());
                }
                discovery::check_server(store, &name, public, password.as_deref())?;
                store.busy = true;
                store.error = None;
                Command::PatchServer(Box::new(papo_core::api::models::PatchServerRequest {
                    name: Some(name.trim().into()),
                    public: Some(public),
                    password,
                    ..Default::default()
                }))
            }
            Action::SaveChannel {
                channel_id,
                name,
                topic,
            } => {
                if !store.can_manage_channels() {
                    return Err("You cannot change channels".into());
                }
                if name.trim().is_empty()
                    || name.chars().count() > 64
                    || store.channel(&channel_id).is_none()
                {
                    return Err("Choose a channel name between 1 and 64 characters".into());
                }
                Command::UpdateChannel {
                    channel_id,
                    name: name.trim().into(),
                    topic: Some(topic),
                }
            }
            Action::SaveRole {
                role_id,
                name,
                color,
                permissions,
            } => {
                if !store.can_manage_roles() {
                    return Err("You cannot change roles".into());
                }
                if name.trim().is_empty()
                    || name.chars().count() > 64
                    || !store.roles.iter().any(|r| r.id == role_id)
                {
                    return Err("Choose a role name between 1 and 64 characters".into());
                }
                if !color.is_empty()
                    && !(color.len() == 7
                        && color.starts_with('#')
                        && color[1..].bytes().all(|b| b.is_ascii_hexdigit()))
                {
                    return Err("Use a color such as #0088FF".into());
                }
                Command::UpdateRole {
                    role_id,
                    name: name.trim().into(),
                    color: (!color.is_empty()).then_some(color),
                    permissions: permissions.into(),
                }
            }
            Action::Login { username, password } => {
                store.busy = true;
                store.error = None;
                Command::Login { username, password }
            }
            Action::Register { username, password } => {
                store.busy = true;
                store.error = None;
                Command::Register { username, password }
            }
            Action::Unlock { password } => {
                store.busy = true;
                store.error = None;
                Command::LoginServer { password }
            }
            Action::CreateServer { name } => Command::CreateServer { name },
            Action::SelectChannel { channel_id } => {
                self.catching_up = None;
                let channel = store
                    .channels
                    .iter()
                    .find(|c| c.id == channel_id)
                    .ok_or("Channel no longer exists")?;
                if !matches!(channel.kind, ChannelKind::Text | ChannelKind::Direct) {
                    return Err("Select a text conversation".into());
                }
                let previous = store.selected_channel.clone();
                if previous != channel_id {
                    store.end_message_context(&previous);
                    self.runtime.net.send(Command::CancelMessageNavigation {
                        channel_id: previous,
                    });
                    self.navigation_target.clear();
                }
                store.reset_read_span(&channel_id);
                store.selected_channel = channel_id;
                return Ok(());
            }
            Action::OpenDm { user_id } => Command::OpenDirectMessage { user_id },
            Action::LoadProfile { user_id } => Command::LoadProfile { user_id },
            Action::Send {
                local_id,
                channel_id,
                content,
                reply_to,
                notify_reply,
            } => {
                if store.screen != Screen::Chat || store.me.is_empty() {
                    return Err("Sign in before sending messages".into());
                }
                if content.trim().is_empty() {
                    return Err("Write a message first".into());
                }
                if local_id.is_empty()
                    || !store.channels.iter().any(|c| {
                        c.id == channel_id
                            && matches!(c.kind, ChannelKind::Text | ChannelKind::Direct)
                    })
                {
                    return Err("Select a text conversation".into());
                }
                Command::QueueMessage {
                    embeds: vec![],
                    local_id,
                    owner_user_id: store.me.clone(),
                    channel_id,
                    content,
                    reply_to,
                    notify_reply,
                    created_at: now_millis(),
                }
            }
            Action::SendAttachments {
                local_id,
                channel_id,
                content,
                reply_to,
                notify_reply,
                files,
            } => {
                if store.screen != Screen::Chat || store.me.is_empty() {
                    return Err("Sign in before sending attachments".into());
                }
                if store.connection != Connection::Online {
                    return Err("Connect to the server before sending attachments. Your files are kept here.".into());
                }
                if local_id.is_empty()
                    || !store.channels.iter().any(|c| {
                        c.id == channel_id
                            && matches!(c.kind, ChannelKind::Text | ChannelKind::Direct)
                    })
                {
                    return Err("Select a text conversation".into());
                }
                Command::SendMessage {
                    local_id,
                    owner_user_id: store.me.clone(),
                    channel_id,
                    content,
                    reply_to,
                    notify_reply,
                    embeds: vec![],
                    attachments: prepare_uploads(files)?,
                }
            }
            Action::Edit {
                message_id,
                content,
            } => {
                if !store
                    .preview_message(&message_id)
                    .is_some_and(|m| store.can_edit_message(m))
                {
                    return Err("Only the message author can edit it".into());
                }
                Command::EditMessage {
                    embeds: None,
                    message_id,
                    content,
                }
            }
            Action::Delete { message_id } => {
                if !store
                    .preview_message(&message_id)
                    .is_some_and(|m| store.can_delete_message(m))
                {
                    return Err("You cannot delete this message".into());
                }
                Command::DeleteMessage { message_id }
            }
            Action::React {
                channel_id,
                message_id,
                emoji,
                custom,
                add,
            } => {
                let kind = if custom {
                    Emoji::Custom(emoji.clone())
                } else {
                    Emoji::Unicode(emoji.clone())
                };
                let message = store
                    .message(&message_id)
                    .filter(|m| m.channel_id == channel_id)
                    .ok_or("Message is no longer available")?;
                let previous_count = message
                    .reactions
                    .iter()
                    .find(|r| r.emoji == kind)
                    .map(|r| r.count)
                    .unwrap_or(0);
                let previous = message
                    .reactions
                    .iter()
                    .find(|r| r.emoji == kind)
                    .is_some_and(|r| r.mine);
                let key = (message_id.clone(), kind.clone());
                if let Some(pending) = self.pending_reactions.get_mut(&key) {
                    pending.desired = add;
                    store.set_reaction_local(&message_id, &kind, add);
                    return Ok(());
                }
                if previous == add {
                    return Ok(());
                }
                self.pending_reactions.insert(
                    key,
                    PendingReaction {
                        previous,
                        previous_count,
                        in_flight: add,
                        desired: add,
                        channel_id: channel_id.clone(),
                    },
                );
                store.set_reaction_local(&message_id, &kind, add);
                Command::React {
                    channel_id,
                    message_id,
                    emoji: if custom {
                        ReactionRequest::custom(emoji)
                    } else {
                        ReactionRequest::unicode(emoji)
                    },
                    add,
                }
            }
            Action::ReactionDetails {
                channel_id,
                message_id,
                more,
            } => {
                if store
                    .message(&message_id)
                    .is_none_or(|m| m.channel_id != channel_id)
                {
                    return Err("Message is no longer available".into());
                }
                let cursor = if more {
                    store.reaction_details_cursor(&message_id)
                } else {
                    None
                };
                if !store.begin_reaction_details(&message_id) {
                    return Ok(());
                }
                Command::LoadReactionDetails {
                    channel_id,
                    message_id,
                    cursor,
                    append: more,
                }
            }
            Action::Retry { local_id } => Command::RetryOutgoing {
                local_id,
                owner_user_id: store.me.clone(),
            },
            Action::Dismiss { local_id } => Command::DismissOutgoing {
                local_id,
                owner_user_id: store.me.clone(),
            },
            Action::JumpToMessage {
                channel_id,
                message_id,
            } => {
                self.catching_up = None;
                if channel_id != store.selected_channel {
                    return Err("Open this conversation before opening the reply".into());
                }
                store.reset_read_span(&channel_id);
                let visible = store.messages_in(&channel_id).any(|m| m.id == message_id);
                let in_live = store
                    .messages
                    .iter()
                    .any(|m| m.id == message_id && m.channel_id == channel_id);
                if !visible && !in_live && store.connection != Connection::Online {
                    return Err(
                        "Connect to open an original message outside the loaded history.".into(),
                    );
                }
                self.navigation_serial = self.navigation_serial.saturating_add(1);
                self.navigation_target = message_id.clone();
                if visible {
                    return Ok(());
                }
                self.runtime.net.send(Command::CancelMessageNavigation {
                    channel_id: channel_id.clone(),
                });
                if in_live {
                    store.end_message_context(&channel_id);
                    return Ok(());
                }
                let at = store.message_timestamp(&channel_id, &message_id);
                let ticket = store.begin_message_context(&channel_id);
                Command::LoadMessageContext {
                    ticket,
                    message_id,
                    at,
                }
            }
            Action::Latest { channel_id } => {
                self.catching_up = None;
                if channel_id != store.selected_channel {
                    return Ok(());
                }
                store.reset_read_span(&channel_id);
                store.end_message_context(&channel_id);
                self.navigation_target.clear();
                self.navigation_serial = self.navigation_serial.saturating_add(1);
                Command::CancelMessageNavigation { channel_id }
            }
            Action::Newer { channel_id } => {
                if channel_id != store.selected_channel {
                    return Ok(());
                }
                let Some((since, last_id)) = store.begin_load_newer(&channel_id) else {
                    return Ok(());
                };
                Command::LoadNewerMessages {
                    ticket: store
                        .history_ticket(&channel_id)
                        .ok_or("History request expired")?,
                    since,
                    last_id,
                }
            }
            Action::Older => {
                let id = store.selected_channel.clone();
                if let Some(before) = store.begin_load_cached_older(&id) {
                    self.hydrate(&id, Some(before), true);
                    return Ok(());
                }
                let Some((since, last_id)) = store.begin_load_older(&id) else {
                    return Ok(());
                };
                Command::LoadOlderMessages {
                    ticket: store.history_ticket(&id).ok_or("History request expired")?,
                    since,
                    last_id,
                }
            }
            Action::Refresh => Command::ProbeConnection,
            Action::Typing { channel_id } => Command::Typing { channel_id },
            Action::CancelCatchUp { channel_id } => {
                if self
                    .catching_up
                    .as_ref()
                    .is_some_and(|(_, channel)| channel == &channel_id)
                {
                    self.catching_up = None;
                }
                return Ok(());
            }
            Action::CatchUp {
                channel_id,
                owner_user_id,
            } => {
                if owner_user_id != store.me || channel_id != store.selected_channel {
                    return Err("Account or conversation changed.".into());
                }
                store.error = None;
                store.reset_read_span(&channel_id);
                store.end_message_context(&channel_id);
                self.navigation_target.clear();
                self.catching_up = Some((owner_user_id, channel_id.clone()));
                Command::CancelMessageNavigation { channel_id }
            }
            Action::BreakReadSpan { channel_id } => {
                store.reset_read_span(&channel_id);
                return Ok(());
            }
            Action::ClearCatchUp {
                channel_id,
                owner_user_id,
            } => {
                if owner_user_id == store.me
                    && channel_id == store.selected_channel
                    && !store.browsing_history(&channel_id)
                {
                    store.mark_read(&channel_id);
                    self.flush_store();
                }
                return Ok(());
            }
            Action::Visible {
                channel_id,
                ids,
                at_latest,
                owner_user_id,
            } => {
                if owner_user_id != store.me || channel_id != store.selected_channel {
                    return Ok(());
                }
                if store.connection == Connection::Online
                    && (store.timeline_status(&channel_id) != TimelineStatus::Fresh
                        || store
                            .pending_remote_read_anchors()
                            .iter()
                            .any(|(channel, _)| channel == &channel_id))
                {
                    return Ok(());
                }
                store.observe_chat_viewport(&channel_id, &ids, at_latest);
                let ids = if at_latest
                    && !store.browsing_history(&channel_id)
                    && store
                        .newest_loaded(&channel_id)
                        .is_some_and(|id| ids.contains(&id))
                {
                    store.take_open_notifications(&channel_id)
                } else {
                    store.take_seen_notifications(&channel_id, &ids)
                };
                store.queue_notification_reads(&channel_id, ids);
                self.flush_store();
                return Ok(());
            }
            Action::Network { available, epoch } => Command::NetworkHint(NetworkHint {
                availability: if available {
                    NetworkAvailability::Available
                } else {
                    NetworkAvailability::Unavailable
                },
                epoch,
            }),
            Action::ClearError => {
                store.error = None;
                return Ok(());
            }
            Action::Logout => Command::Logout,
        };
        self.runtime.net.send(command);
        Ok(())
    }

    pub fn snapshot(&self) -> Value {
        let mut value = project(
            &self.runtime.store,
            &self.runtime.url,
            &self.runtime.server_key,
        );
        value["pushRegistered"] = json!(
            self.push_registration
                .as_ref()
                .is_some_and(|(owner, _)| owner == &self.runtime.store.me)
        );
        value["searchQuery"] = json!(self.search_query);
        value["navigationSerial"] = json!(self.navigation_serial);
        value["navigationTarget"] = json!(self.navigation_target);
        value["reading"]["seekingCatchUp"] = json!(self.catching_up.is_some());
        if self.catching_up.is_some() {
            value["reading"]["syncPending"] = json!(true);
        }
        value
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.flush_store();
        self.runtime.cache.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn typing_projection_preserves_custom_phrase_and_group_fallback() {
        use papo_core::api::ws::Event;
        let mut demo = demo::Demo::new();
        demo.store
            .members
            .iter_mut()
            .find(|m| m.id == "ana")
            .unwrap()
            .typing_label = Some("is sketching…".into());
        for id in ["ana", "bruno"] {
            demo.store.apply(Update::Event(Box::new(Event::Typing {
                channel_id: "general".into(),
                user_id: id.into(),
                is_typing: true,
            })));
            let snapshot = project(&demo.store, "demo://papo", "demo");
            assert_eq!(
                snapshot["typingPhrase"],
                if id == "ana" { "is sketching…" } else { "" }
            );
            assert_eq!(
                snapshot["typing"].as_array().unwrap().len(),
                if id == "ana" { 1 } else { 2 }
            );
        }
    }

    #[test]
    fn addresses_are_canonical_and_credentials_are_rejected() {
        assert_eq!(
            normalize_url(" Papo.example/ ").unwrap(),
            "https://papo.example"
        );
        assert_eq!(
            normalize_url("http://10.0.2.2:8080").unwrap(),
            "http://10.0.2.2:8080"
        );
        for value in [
            "",
            "file:///tmp/papo",
            "https://user:password@papo.example",
            "https://papo.example?token=abc",
        ] {
            assert!(normalize_url(value).is_err(), "{value}");
        }
    }
    #[test]
    fn commands_preserve_message_identity_and_reject_unknown_fields() {
        let action: Action = serde_json::from_value(json!({"type":"send","local_id":"stable-id","channel_id":"c","content":"Olá 👋\nworld","reply_to":"m"})).unwrap();
        assert!(
            matches!(action, Action::Send { local_id, content, reply_to: Some(_), .. } if local_id == "stable-id" && content == "Olá 👋\nworld")
        );
        assert!(
            serde_json::from_value::<Action>(
                json!({"type":"login","username":"a","password":"b","surprise":true})
            )
            .is_err()
        );
    }
}

#[cfg(test)]
mod reaction_tests;
