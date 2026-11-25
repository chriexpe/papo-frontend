//! Where an audio's bytes come from (`docs/design.md` §14.3, §14.5): a received audio through
//! the SDK's media cache, an unsent one from its staged file in its account's outgoing queue,
//! which the server doesn't have yet. Reading and playing them is `audio_reads`' and `audio`'s.

use std::sync::Arc;

use papo_sdk::{AccountId, ClientHandle, MediaSource, OperationKind, StagedAttachment};

use super::Shell;
use super::staged_files::read_staged;
use crate::audio_store::AudioKey;
use crate::media_store::MediaKey;

/// Where one audio's bytes are read from.
pub(super) enum AudioSource {
    Received(MediaKey),
    Staged {
        account: AccountId,
        staged: StagedAttachment,
    },
}

impl Shell {
    /// Where audio `key` is read from: none for an unsent audio whose file isn't staged yet
    /// (it still copies) or whose message left the queue.
    pub(super) fn audio_source(&self, key: AudioKey) -> Option<AudioSource> {
        match key {
            AudioKey::Received(key) => Some(AudioSource::Received(key)),
            AudioKey::Unsent {
                account,
                local,
                index,
            } => {
                let outgoing = self.activity.get(&account)?.outgoing.as_deref()?;
                let operation = outgoing
                    .operations
                    .iter()
                    .find(|operation| operation.id == local)?;
                let OperationKind::Message(message) = operation.kind.as_ref() else {
                    return None;
                };
                let staged = message.attachments.get(index)?.staged()?.clone();
                Some(AudioSource::Staged { account, staged })
            }
        }
    }
}

impl AudioSource {
    /// Its bytes; none when they can't be had. A received audio is taken from the SDK's cache
    /// when `cached_first` and it is there, else fetched; an unsent one is read from staging.
    pub(super) async fn bytes(
        self,
        client: &ClientHandle,
        cached_first: bool,
    ) -> Option<Arc<[u8]>> {
        match self {
            AudioSource::Received(key) => {
                let session = client.session(key.account).ok()?;
                let media = session.media();
                let source = MediaSource::Attachment(key.attachment);
                if cached_first && let Ok(data) = media.cached(source.clone()).await {
                    return Some(data.value.bytes);
                }
                media
                    .fetch(source, false)
                    .await
                    .ok()
                    .map(|data| data.value.bytes)
            }
            AudioSource::Staged { account, staged } => {
                let session = client.session(account).ok()?;
                read_staged(&session, &staged, papo_sdk::MAX_ATTACHMENT_BYTES)
                    .await
                    .map(Arc::from)
            }
        }
    }
}
