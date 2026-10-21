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
    pub dms: Vec<ChannelView>,
    pub messages: Vec<MessageView>,
    pub locked: bool,
    pub busy: bool,
    pub error: String,
    pub typing: Vec<String>,
    pub can_load_older: bool,
    pub loading_older: bool,
    pub timeline: String,
    #[serde(default)]
    pub more: bool,
    #[serde(default)]
    pub events: Vec<DeliveryEvent>,
}

#[derive(Clone, Debug, Deserialize, uniffi::Record)]
pub struct ChannelView {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub topic: String,
    pub unread: bool,
    pub mentions: i32,
}
#[derive(Clone, Debug, Deserialize, uniffi::Record)]
pub struct MemberView {
    pub id: String,
    pub name: String,
    pub presence: String,
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
