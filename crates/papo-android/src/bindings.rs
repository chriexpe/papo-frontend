//! Generated, typed Kotlin boundary. UniFFI owns foreign object lifetimes,
//! callbacks and error conversion; no JNI signatures or integer handles.
use super::*;
use papo_core::storage::{Secret, StorageError, StorageResult};

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum CoreError {
    #[error("{details}")]
    Operation { details: String },
}
impl From<String> for CoreError {
    fn from(message: String) -> Self {
        Self::Operation { details: message }
    }
}
impl From<uniffi::UnexpectedUniFFICallbackError> for CoreError {
    fn from(error: uniffi::UnexpectedUniFFICallbackError) -> Self {
        Self::Operation {
            details: format!("Android service failed: {error}"),
        }
    }
}

#[uniffi::export(with_foreign)]
pub trait Credentials: Send + Sync {
    fn load(&self, server: String, secret: String) -> Result<Option<String>, CoreError>;
    fn store(&self, server: String, secret: String, value: String) -> Result<(), CoreError>;
    fn remove(&self, server: String, secret: String) -> Result<(), CoreError>;
}

#[uniffi::export(with_foreign)]
pub trait WakeListener: Send + Sync {
    fn on_wake(&self);
}

struct ForeignSecrets(Arc<dyn Credentials>);
impl SecretStore for ForeignSecrets {
    fn load(&self, server: &str, secret: Secret) -> StorageResult<Option<String>> {
        self.0
            .load(server.to_owned(), secret.key().to_owned())
            .map_err(|e| StorageError::new(e.to_string()))
    }
    fn store(&self, server: &str, secret: Secret, value: &str) -> StorageResult<()> {
        self.0
            .store(server.to_owned(), secret.key().to_owned(), value.to_owned())
            .map_err(|e| StorageError::new(e.to_string()))
    }
    fn remove(&self, server: &str, secret: Secret) -> StorageResult<()> {
        self.0
            .remove(server.to_owned(), secret.key().to_owned())
            .map_err(|e| StorageError::new(e.to_string()))
    }
}

#[derive(uniffi::Object)]
pub struct NativeClient {
    backend: Mutex<Option<Backend>>,
}
enum Backend {
    Live(Box<Client>),
    Demo(Box<demo::Demo>),
}

#[uniffi::export]
impl NativeClient {
    #[uniffi::constructor]
    pub fn open(
        url: String,
        database: String,
        credentials: Arc<dyn Credentials>,
        listener: Arc<dyn WakeListener>,
    ) -> Result<Arc<Self>, CoreError> {
        #[cfg(target_os = "android")]
        android_logger::init_once(
            android_logger::Config::default()
                .with_tag("PapoCompose")
                .with_max_level(log::LevelFilter::Info),
        );
        let client = Client::open(
            &url,
            PathBuf::from(database),
            Arc::new(ForeignSecrets(credentials)),
            Wake::new(move || listener.on_wake()),
        )
        .map_err(CoreError::from)?;
        Ok(Arc::new(Self {
            backend: Mutex::new(Some(Backend::Live(Box::new(client)))),
        }))
    }

    #[uniffi::constructor]
    pub fn demo() -> Arc<Self> {
        Arc::new(Self {
            backend: Mutex::new(Some(Backend::Demo(Box::default()))),
        })
    }

    pub fn poll(&self) -> Result<ClientSnapshot, CoreError> {
        let mut backend = self
            .backend
            .lock()
            .map_err(|_| "Native client lock failed".to_owned())?;
        let value = match backend
            .as_mut()
            .ok_or_else(|| "Client is closed".to_owned())?
        {
            Backend::Live(client) => client.poll(),
            Backend::Demo(client) => client.poll(),
        };
        serde_json::from_value(value).map_err(|e| CoreError::Operation {
            details: format!("Invalid presentation state: {e}"),
        })
    }

    pub fn dispatch(&self, action: Action) -> Result<(), CoreError> {
        let mut backend = self
            .backend
            .lock()
            .map_err(|_| "Native client lock failed".to_owned())?;
        match backend
            .as_mut()
            .ok_or_else(|| "Client is closed".to_owned())?
        {
            Backend::Live(client) => client.dispatch(action),
            Backend::Demo(client) => client.dispatch(action),
        }
        .map_err(CoreError::from)
    }

    /// Explicitly stops transport even if a foreign reference is retained by a
    /// coroutine. Resource fetching never holds this lock across network I/O.
    pub fn shutdown(&self) {
        if let Ok(mut backend) = self.backend.lock() {
            backend.take();
        }
    }

    pub fn media_request(
        &self,
        resource: media::MediaResource,
    ) -> Result<media::MediaRequest, CoreError> {
        let backend = self
            .backend
            .lock()
            .map_err(|_| "Native client lock failed".to_owned())?;
        match backend
            .as_ref()
            .ok_or_else(|| "Client is closed".to_owned())?
        {
            Backend::Live(client) => media::request(&client.runtime, resource),
            Backend::Demo(client) => client.media_request(resource),
        }
        .map_err(CoreError::from)
    }

    pub fn encode_message(
        &self,
        text: String,
        mentions: Vec<MentionSelection>,
    ) -> Result<String, CoreError> {
        let backend = self
            .backend
            .lock()
            .map_err(|_| "Client lock failed".to_owned())?;
        let store = match backend.as_ref().ok_or("Client is closed".to_owned())? {
            Backend::Live(client) => &client.runtime.store,
            Backend::Demo(client) => &client.store,
        };
        let bindings = mentions
            .into_iter()
            .map(|m| papo_core::state::MentionBinding {
                start: m.start as usize,
                label: m.label,
                user_id: m.user_id,
            })
            .collect::<Vec<_>>();
        Ok(store.encode_mentions(&text, &bindings))
    }

    pub fn emoji_image(&self, id: String) -> Result<Vec<u8>, CoreError> {
        use base64::Engine;
        let blob = {
            let backend = self
                .backend
                .lock()
                .map_err(|_| "Client lock failed".to_owned())?;
            let emojis = match backend.as_ref().ok_or("Client is closed".to_owned())? {
                Backend::Live(client) => &client.runtime.store.emojis,
                Backend::Demo(client) => &client.store.emojis,
            };
            emojis
                .iter()
                .find(|e| e.id == id)
                .and_then(|e| e.blob.clone())
                .ok_or("Emoji image is unavailable".to_owned())?
        };
        let encoded = blob.split_once("base64,").map(|(_, b)| b).unwrap_or(&blob);
        base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|_| CoreError::from("Invalid emoji image".to_owned()))
    }

    /// Decoded on demand, outside presentation snapshots. The profile revision
    /// lets Android invalidate its account-scoped image cache after an update.
    pub fn avatar_image(&self, id: String) -> Result<Vec<u8>, CoreError> {
        use base64::Engine;
        let blob = {
            let backend = self
                .backend
                .lock()
                .map_err(|_| "Client lock failed".to_owned())?;
            let store = match backend.as_ref().ok_or("Client is closed".to_owned())? {
                Backend::Live(client) => &client.runtime.store,
                Backend::Demo(client) => &client.store,
            };
            store
                .avatars
                .get(&id)
                .cloned()
                .ok_or("Avatar is unavailable".to_owned())?
        };
        let encoded = blob
            .split_once("base64,")
            .map(|(_, data)| data)
            .unwrap_or(&blob);
        base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|_| CoreError::from("Invalid avatar image".to_owned()))
    }

    // Provider work runs on Kotlin's IO dispatcher and never takes the server lock.
    pub fn gif_page(&self, query: String, offset: u32) -> Result<GifPageView, CoreError> {
        let demo = {
            let backend = self
                .backend
                .lock()
                .map_err(|_| "Client lock failed".to_owned())?;
            matches!(backend.as_ref(), Some(Backend::Demo(_)))
        };
        if demo {
            return Ok(GifPageView {
                items: vec![GifView {
                    id: "demo-motion".into(),
                    title: "A little motion".into(),
                    gif_url: "file:///android_asset/demo/animation.gif".into(),
                    display_url: "file:///android_asset/demo/animation.gif".into(),
                    preview_url: "file:///android_asset/demo/animation.gif".into(),
                    width: 480,
                    height: 320,
                }],
                more: false,
            });
        }
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| CoreError::from(e.to_string()))?;
        let page = rt
            .block_on(papo_core::giphy::page(&query, offset))
            .map_err(CoreError::from)?;
        Ok(GifPageView {
            items: page.items.into_iter().map(GifView::from).collect(),
            more: page.more,
        })
    }
    pub fn gif_item(&self, id: String) -> Result<GifView, CoreError> {
        let demo = {
            let backend = self
                .backend
                .lock()
                .map_err(|_| "Client lock failed".to_owned())?;
            matches!(backend.as_ref(), Some(Backend::Demo(_)))
        };
        if demo && id == "demo-motion" {
            return Ok(self.gif_page(String::new(), 0)?.items.remove(0));
        }
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| CoreError::from(e.to_string()))?;
        rt.block_on(papo_core::giphy::item(&id))
            .map(GifView::from)
            .map_err(CoreError::from)
    }

    pub fn embed_image(&self, id: String, author: bool) -> Result<Vec<u8>, CoreError> {
        let api = {
            let backend = self
                .backend
                .lock()
                .map_err(|_| "Native client lock failed".to_owned())?;
            match backend
                .as_ref()
                .ok_or_else(|| "Client is closed".to_owned())?
            {
                Backend::Demo(_) => return Ok(media::demo_image(&id)),
                Backend::Live(client) => {
                    media::check_embed(&client.runtime, &id)?;
                    papo_core::api::client::Api::new(
                        &client.runtime.url,
                        Arc::clone(&client.runtime.net.session),
                    )
                    .map_err(|e| e.to_string())?
                }
            }
        };
        media::embed_image(api, id, author).map_err(CoreError::from)
    }
}

#[derive(Clone, Debug, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ClientSnapshot {
    pub version: i32,
    pub url: String,
    pub key: String,
    pub screen: String,
    pub connection: String,
    pub server_name: String,
    pub me: String,
    pub my_name: String,
    pub selected_channel: String,
    pub channels: Vec<ChannelView>,
    pub members: Vec<MemberView>,
    #[serde(default)]
    pub profiles: Vec<ProfileView>,
    pub emojis: Vec<EmojiView>,
    pub reaction_details: Vec<ReactionDetailsView>,
    pub reaction_loading: Vec<String>,
    pub dms: Vec<ChannelView>,
    pub messages: Vec<MessageView>,
    pub locked: bool,
    pub busy: bool,
    pub error: String,
    pub typing: Vec<String>,
    pub can_load_older: bool,
    pub loading_older: bool,
    pub browsing_history: bool,
    pub loading_context: bool,
    pub can_load_newer: bool,
    pub navigation_serial: u64,
    pub navigation_target: String,
    pub timeline: String,
    #[serde(default)]
    pub more: bool,
    #[serde(default)]
    pub events: Vec<DeliveryEvent>,
}

#[derive(Clone, Debug, Deserialize, uniffi::Record)]
pub struct ChannelView {
    pub id: String,
    #[serde(default, rename = "userId")]
    pub user_id: String,
    pub name: String,
    pub kind: String,
    pub topic: String,
    pub unread: bool,
    pub mentions: i32,
}
#[derive(Clone, Debug, Deserialize, uniffi::Record)]
pub struct MemberView {
    pub id: String,
    pub username: String,
    pub name: String,
    pub presence: String,
}
#[derive(Clone, Debug, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ProfileView {
    pub id: String,
    pub avatar_revision: String,
    pub loaded: bool,
    pub banner: String,
    pub description: String,
    pub created_at: String,
    pub status_message: String,
    pub roles: Vec<ProfileRoleView>,
}
#[derive(Clone, Debug, Deserialize, uniffi::Record)]
pub struct ProfileRoleView {
    pub id: String,
    pub name: String,
    pub color: String,
}
#[derive(Clone, Debug, Deserialize, uniffi::Record)]
pub struct ReactionView {
    pub emoji: String,
    pub count: i32,
    pub mine: bool,
    pub custom: bool,
}
#[derive(Clone, Debug, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct DeliveryEvent {
    #[serde(rename = "type")]
    pub kind: String,
    pub local_id: String,
    #[serde(default)]
    pub error: String,
}
#[derive(Clone, Debug, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct MessageView {
    pub id: String,
    pub author_id: String,
    pub author: String,
    #[serde(default)]
    pub external_author: Option<ExternalAuthorView>,
    pub content: String,
    pub at: i64,
    pub edited: bool,
    pub mine: bool,
    pub reply_to: String,
    pub reply_content: String,
    pub pending: bool,
    pub delivery: String,
    pub reactions: Vec<ReactionView>,
    pub attachments: Vec<media::AttachmentView>,
    pub embeds: Vec<media::EmbedView>,
}

#[derive(Clone, Debug, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ExternalAuthorView {
    pub source: String,
    pub id: String,
    pub name: String,
    pub avatar_url: String,
    pub profile_url: String,
}

#[derive(Clone, Debug, Deserialize, uniffi::Record)]
pub struct EmojiView {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub revision: String,
}
#[derive(Clone, Debug, Deserialize, uniffi::Record)]
pub struct ReactorView {
    pub id: String,
    pub name: String,
}
#[derive(Clone, Debug, Deserialize, uniffi::Record)]
pub struct ReactionGroupView {
    pub emoji: String,
    pub custom: bool,
    pub users: Vec<ReactorView>,
}
#[derive(Clone, Debug, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ReactionDetailsView {
    pub message_id: String,
    pub groups: Vec<ReactionGroupView>,
    pub more: bool,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct GifView {
    pub id: String,
    pub title: String,
    pub gif_url: String,
    pub display_url: String,
    pub preview_url: String,
    pub width: u32,
    pub height: u32,
}
impl From<papo_core::giphy::GifItem> for GifView {
    fn from(g: papo_core::giphy::GifItem) -> Self {
        Self {
            id: g.id,
            title: g.title,
            gif_url: g.gif_url,
            display_url: g.display_url,
            preview_url: g.preview_url,
            width: g.width,
            height: g.height,
        }
    }
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct GifPageView {
    pub items: Vec<GifView>,
    pub more: bool,
}

/// Start is a Unicode scalar index, matching papo-core's mention bindings.
#[derive(Clone, Debug, uniffi::Record)]
pub struct MentionSelection {
    pub start: u32,
    pub label: String,
    pub user_id: String,
}
