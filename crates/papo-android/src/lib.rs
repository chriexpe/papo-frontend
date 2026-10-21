//! Thin presentation adapter. All transport, reduction, reconciliation and
//! outgoing delivery remain in papo-core. No egui or media engine is linked.
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use papo_core::api::models::ReactionRequest;
use papo_core::api::net::{Command, NetworkAvailability, NetworkHint, Update, Wake};
use papo_core::api::ws::Connection;
use papo_core::cache::{ClientDb, OutgoingState, now_millis};
use papo_core::notification::NotificationCoordinator;
use papo_core::runtime::{DEFAULT_DRAIN_LIMIT, ServerRuntime};
use papo_core::state::{ChannelKind, Emoji, Screen, TimelineStatus};
use papo_core::storage::SecretStore;
use serde::Deserialize;
use serde_json::{Value, json};

pub mod bindings;
pub mod demo;
pub mod media;
uniffi::setup_scaffolding!();

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
    SelectChannel {
        channel_id: String,
    },
    OpenDm {
        user_id: String,
    },
    Send {
        local_id: String,
        channel_id: String,
        content: String,
        reply_to: Option<String>,
    },
    SendAttachments {
        local_id: String,
        channel_id: String,
        content: String,
        reply_to: Option<String>,
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
        add: bool,
    },
    Retry {
        local_id: String,
    },
    Dismiss {
        local_id: String,
    },
    Older,
    Refresh,
    Typing {
        channel_id: String,
    },
    Visible {
        channel_id: String,
        ids: Vec<String>,
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
}

fn prepare_uploads(files: Vec<UploadFile>) -> Result<Vec<papo_core::api::client::Upload>, String> {
    if files.is_empty() || files.len() > 10 {
        return Err("Choose between one and ten attachments".into());
    }
    files.into_iter().map(|file| {
        let path = PathBuf::from(&file.path);
        let metadata = std::fs::metadata(&path).map_err(|_| format!("{} is no longer available. Choose it again.", file.name))?;
        if !metadata.is_file() || metadata.len() != file.size {
            return Err(format!("{} changed. Choose it again.", file.name));
        }
        Ok(papo_core::api::client::Upload {
            path, name: file.name, mime: file.mime, size: file.size,
            spoiler: false, anonymize: false,
        })
    }).collect()
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
    runtime: ServerRuntime,
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
        let url = normalize_url(url)?;
        let cache = database(db_path)?;
        let notifications = Arc::new(NotificationCoordinator::new(Arc::clone(&cache), None));
        Ok(Self {
            runtime: ServerRuntime::open(url, wake, secrets, cache, notifications),
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
        // A hard bound prevents a busy socket from monopolizing the actor.
        let mut events = Vec::new();
        let mut count = 0;
        while count < DEFAULT_DRAIN_LIMIT {
            let Some(update) = self.runtime.net.try_recv() else {
                break;
            };
            match &update {
                Update::Outgoing(item) => {
                    events.push(json!({"type":"accepted", "localId": item.local_id}))
                }
                Update::SendConfirmed { local_id, .. } => {
                    events.push(json!({"type":"accepted", "localId":local_id}));
                }
                Update::UploadFailed { local_id, message, state } => {
                    events.push(json!({"type":if *state == OutgoingState::UnknownOutcome { "unknown" } else { "rejected" }, "localId":local_id,"error":message}));
                }
                Update::OutgoingRejected {
                    local_id, message, ..
                } => events.push(json!({"type":"rejected", "localId": local_id, "error": message})),
                _ => {}
            }
            self.runtime.process_update(update);
            count += 1;
        }
        let more = count == DEFAULT_DRAIN_LIMIT;
        if let Some(id) = self.runtime.store.channel_needing_cache() {
            self.runtime.store.mark_cache_loading(&id);
            self.hydrate(&id, None, false);
        }
        if let Some(id) = self.runtime.store.channel_needing_messages() {
            let ticket = self.runtime.store.mark_loading(&id);
            self.runtime.net.send(Command::LoadMessages { ticket });
        }
        self.flush_store();
        let mut snapshot = self.snapshot();
        snapshot["more"] = json!(more);
        snapshot["events"] = json!(events);
        snapshot
    }

    fn flush_store(&mut self) {
        let ops = self.runtime.store.take_cache_ops();
        if !ops.is_empty() {
            self.runtime.cache.submit(&self.runtime.server_key, ops);
        }
    }

    pub fn dispatch(&mut self, action: Action) -> Result<(), String> {
        let store = &mut self.runtime.store;
        let command = match action {
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
                let channel = store
                    .channels
                    .iter()
                    .find(|c| c.id == channel_id)
                    .ok_or("Channel no longer exists")?;
                if !matches!(channel.kind, ChannelKind::Text | ChannelKind::Direct) {
                    return Err("Select a text conversation".into());
                }
                store.reset_read_span(&channel_id);
                store.selected_channel = channel_id;
                return Ok(());
            }
            Action::OpenDm { user_id } => Command::OpenDirectMessage { user_id },
            Action::Send {
                local_id,
                channel_id,
                content,
                reply_to,
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
                    notify_reply: true,
                    created_at: now_millis(),
                }
            }
            Action::SendAttachments { local_id, channel_id, content, reply_to, files } => {
                if store.screen != Screen::Chat || store.me.is_empty() {
                    return Err("Sign in before sending attachments".into());
                }
                if store.connection != Connection::Online {
                    return Err("Connect to the server before sending attachments. Your files are kept here.".into());
                }
                if local_id.is_empty() || !store.channels.iter().any(|c| c.id == channel_id && matches!(c.kind, ChannelKind::Text | ChannelKind::Direct)) {
                    return Err("Select a text conversation".into());
                }
                Command::SendMessage {
                    local_id, owner_user_id: store.me.clone(), channel_id, content,
                    reply_to, notify_reply: true, embeds: vec![], attachments: prepare_uploads(files)?,
                }
            }
            Action::Edit {
                message_id,
                content,
            } => Command::EditMessage {
                embeds: None,
                message_id,
                content,
            },
            Action::Delete { message_id } => Command::DeleteMessage { message_id },
            Action::React {
                channel_id,
                message_id,
                emoji,
                add,
            } => Command::React {
                channel_id,
                message_id,
                emoji: ReactionRequest::unicode(emoji),
                add,
            },
            Action::Retry { local_id } => Command::RetryOutgoing {
                local_id,
                owner_user_id: store.me.clone(),
            },
            Action::Dismiss { local_id } => Command::DismissOutgoing {
                local_id,
                owner_user_id: store.me.clone(),
            },
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
            Action::Visible { channel_id, ids } => {
                if channel_id != store.selected_channel {
                    return Ok(());
                }
                if store.connection == Connection::Online
                    && store.timeline_status(&channel_id) != TimelineStatus::Fresh
                {
                    return Ok(());
                }
                store.observe_visible_messages_with_policy(&channel_id, &ids, false);
                let ids = store.take_seen_notifications(&channel_id, &ids);
                self.flush_store();
                if ids.is_empty() {
                    return Ok(());
                }
                Command::MarkNotificationsRead {
                    user_id: self.runtime.store.me.clone(),
                    ids,
                }
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
        project(
            &self.runtime.store,
            &self.runtime.url,
            &self.runtime.server_key,
        )
    }
}

fn project(s: &papo_core::state::Store, url: &str, key: &str) -> Value {
    let channels: Vec<_> = s.channels.iter().map(|c| json!({
            "id": c.id, "name": c.name, "kind": match c.kind { ChannelKind::Text => "text", ChannelKind::Voice => "voice", ChannelKind::Category => "category", ChannelKind::Direct => "direct" },
            "topic": c.topic.as_deref().unwrap_or_default(), "unread": c.unread, "mentions": c.mentions,
        })).collect();
    let messages: Vec<_> = s.messages_in(&s.selected_channel).map(|m| {
            let name = s.members.iter().find(|u| u.id == m.author_id).map(|u| u.name.as_str())
                .or_else(|| m.external_author.as_ref().map(|a| a.name.as_str())).unwrap_or(&m.author_id);
            let reactions: Vec<_> = m.reactions.iter().map(|r| json!({ "emoji": match &r.emoji { Emoji::Unicode(e) => e.clone(), Emoji::Custom(_) => "✦".to_owned() }, "count": r.count, "mine": r.mine, "custom": matches!(r.emoji, Emoji::Custom(_)) })).collect();
            json!({ "id": m.id, "authorId": m.author_id, "author": name, "content": m.content,
                "at": m.at.timestamp_millis(), "edited": m.edited, "mine": m.mine(&s.me), "replyTo": m.reply_to.as_deref().unwrap_or_default(),
                "replyContent": m.reply_to.as_ref().and_then(|id| s.message(id)).map(|m| m.content.as_str()).unwrap_or_default(),
                "pending": m.pending, "delivery": s.outgoing_state(&m.id).map(|state| match state { OutgoingState::Queued => "queued", OutgoingState::Sending => "sending", OutgoingState::UnknownOutcome => "unknown", OutgoingState::FailedPermanent => "failed" }).unwrap_or_default(),
                "reactions": reactions, "attachments": m.attachments.iter().map(media::AttachmentView::from).collect::<Vec<_>>(), "embeds": m.previews.iter().map(media::EmbedView::from).collect::<Vec<_>>(),
            })
        }).collect();
    let members: Vec<_> = s.members.iter().map(|u| json!({ "id": u.id, "name": u.name, "presence": format!("{:?}", u.presence).to_lowercase() })).collect();
    let dms: Vec<_> = s
        .direct_messages
        .iter()
        .map(|dm| json!({ "id": dm.id, "name": dm.user.display_name(), "kind": "direct", "topic": "", "unread": dm.unread_count > 0, "mentions": dm.unread_count }))
        .collect();
    json!({ "version": 1, "url": url, "key": key,
        "screen": match s.screen { Screen::Starting => "starting", Screen::Auth => "auth", Screen::NeedsServer => "needs_server", Screen::Chat => "chat" },
        "connection": match s.connection { Connection::Online => "online", Connection::Connecting => "connecting", Connection::Offline => "offline" },
        "serverName": s.server.as_ref().map(|s| s.name.as_str()).unwrap_or(url),
        "me": s.me, "myName": s.my_name, "selectedChannel": s.selected_channel, "channels": channels,
        "messages": messages, "members": members, "dms": dms, "typing": s.typing_names(), "locked": s.locked,
        "busy": s.busy, "error": s.error.as_deref().unwrap_or_default(), "canLoadOlder": s.can_load_older(&s.selected_channel),
        "loadingOlder": s.loading_older(&s.selected_channel), "timeline": format!("{:?}", s.timeline_status(&s.selected_channel)).to_lowercase(),
    })
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
