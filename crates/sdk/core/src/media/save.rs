//! Saving an attachment's original, or an embed's relayed picture, into a folder or a file the
//! person chose, with progress and cancel. The bytes are the original's exactly: from this
//! account's cache when it holds them, else from the backend (an original streamed as it
//! arrives, never held whole; an embed picture in one answer), never added to the cache.
//! Fetching for display and the cache's grants live elsewhere; the destination's naming lives
//! in `save_names`.
//!
//! A folder save writes a hidden partial file beside its destination and renames it into the
//! first free name only once every byte is written and synced, so a name in the folder is either
//! absent or complete, and never replaces an existing file. A chosen file (the system's save
//! dialog asked first) is replaced the same way, from a partial file beside it, or written in
//! place where a sandbox grants only that one file. Nothing is opened for writing before the
//! source answered, so a refused save never touches the destination. The destination file is
//! `destination`'s.

use super::MediaError;
use crate::ClientError;
use crate::{Session, Uuid};
use std::{
    io::Write,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

mod destination;
mod source;
use destination::Sink;
use source::{Saving, Source, open_source};

/// The largest original a save writes: the attachment limit.
const MAX_SAVE_BYTES: u64 = crate::MAX_ATTACHMENT_BYTES;

/// Bytes gathered before each write to the destination, so a large save makes few blocking
/// writes.
const WRITE_BYTES: usize = 1024 * 1024;

/// The shortest interval between two progress publications; the first, the last and the end
/// state are always published.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

/// Where a save writes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveTarget {
    /// Into `folder` under `name` (made safe), or the first free "name (1)", "name (2)"…: never
    /// over an existing file.
    Folder {
        /// An existing folder.
        folder: PathBuf,
        /// The file's name as uploaded; only its last component is used.
        name: String,
    },
    /// Exactly this file, replacing what is there (the system's save dialog asked first). What
    /// is there stays until the new file is complete, except inside a sandbox that grants only
    /// this one file, where it is written in place once the original answered.
    File(PathBuf),
}

/// How far a save has come.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SaveProgress {
    /// Bytes written so far.
    pub saved_bytes: u64,
    /// The original's size, once known.
    pub total_bytes: Option<u64>,
}

/// Why a save ended without a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SaveFailure {
    /// The attachment is gone or no longer readable by this account.
    Gone,
    /// The original could not be read: the backend could not be reached or stopped sending, or
    /// the cached copy was damaged.
    Unreachable,
    /// The original passed the attachment limit.
    TooLarge,
    /// The destination could not be written.
    Destination(std::io::ErrorKind),
    /// Cancelled, every handle dropped, or the account or client closed.
    Cancelled,
}

/// Where a save stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveState {
    /// Writing.
    Saving(SaveProgress),
    /// Done: the file's path.
    Saved(PathBuf),
    /// Ended without a file; nothing partial is left behind.
    Failed(SaveFailure),
}

impl SaveState {
    /// Whether the save has ended.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        !matches!(self, SaveState::Saving(_))
    }
}

/// Stops the save once the last handle is gone; nothing else can still want its file.
struct SaveGuard(CancellationToken);

impl Drop for SaveGuard {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

/// A save in progress. Clone it freely; the save stops when [`MediaSave::cancel`] is called or
/// every clone is dropped.
#[derive(Clone)]
pub struct MediaSave {
    state: watch::Receiver<SaveState>,
    /// The state this handle's [`MediaSave::changed`] last returned.
    returned: Option<SaveState>,
    stop: CancellationToken,
    _guard: Arc<SaveGuard>,
}

impl std::fmt::Debug for MediaSave {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaSave").finish_non_exhaustive()
    }
}

impl MediaSave {
    /// Where it stands now, without waiting.
    #[must_use]
    pub fn state(&self) -> SaveState {
        self.state.borrow().clone()
    }

    /// The state this handle has not returned yet, waiting for one; `None` once the end has
    /// been returned. A save that ended before the call still returns its end once.
    pub async fn changed(&mut self) -> Option<SaveState> {
        loop {
            let now = self.state.borrow_and_update().clone();
            if self.returned.as_ref() != Some(&now) {
                self.returned = Some(now.clone());
                return Some(now);
            }
            if now.is_finished() {
                return None;
            }
            if self.state.changed().await.is_err() {
                // The task always publishes an end before it goes; read it on the next turn.
                let end = self.state.borrow().clone();
                if self.returned.as_ref() == Some(&end) {
                    return None;
                }
            }
        }
    }

    /// Waits for the end.
    /// # Errors
    /// Returns why no file was written.
    pub async fn finished(&self) -> Result<PathBuf, SaveFailure> {
        let mut state = self.state.clone();
        loop {
            match state.borrow_and_update().clone() {
                SaveState::Saved(path) => return Ok(path),
                SaveState::Failed(failure) => return Err(failure),
                SaveState::Saving(_) => {}
            }
            if state.changed().await.is_err() {
                return match state.borrow().clone() {
                    SaveState::Saved(path) => Ok(path),
                    SaveState::Failed(failure) => Err(failure),
                    SaveState::Saving(_) => Err(SaveFailure::Cancelled),
                };
            }
        }
    }

    /// Stops the save; what it wrote is removed.
    pub fn cancel(&self) {
        self.stop.cancel();
    }
}

impl super::MediaService {
    /// Starts saving attachment `attachment`'s original to `target` and returns at once with a
    /// handle that reports its progress.
    ///
    /// The bytes come from this account's cache when it holds the original, else from the
    /// backend, which authorizes the request; a denied request retires the account's cached
    /// copies, as a fetch does. Nothing is added to the cache. On a closed account or client the
    /// handle has ended as cancelled.
    #[must_use]
    pub fn save_attachment(&self, attachment: Uuid, target: SaveTarget) -> MediaSave {
        self.start_save(Saving::Attachment(attachment), target)
    }

    /// Starts saving embed `embed`'s relayed picture (its [`super::MediaSource::EmbedImage`] at
    /// `revision`) to `target`, as [`Self::save_attachment`] saves an original: from
    /// this account's cache when it holds the picture, else through the relay; never from the
    /// site.
    #[must_use]
    pub fn save_embed_image(&self, embed: Uuid, revision: String, target: SaveTarget) -> MediaSave {
        self.start_save(Saving::EmbedImage { embed, revision }, target)
    }

    fn start_save(&self, saving: Saving, target: SaveTarget) -> MediaSave {
        let (sender, state) = watch::channel(SaveState::Saving(SaveProgress {
            saved_bytes: 0,
            total_bytes: None,
        }));
        let stop = CancellationToken::new();
        let handle = MediaSave {
            state,
            returned: None,
            stop: stop.clone(),
            _guard: Arc::new(SaveGuard(stop.clone())),
        };
        let mut publisher = Publisher::new(sender);
        let session = self.session.clone();
        if session.is_closed() {
            publisher.finish(SaveState::Failed(SaveFailure::Cancelled));
            return handle;
        }
        let token = session.inner.tasks.token();
        let runtime = session.client.inner.runtime.clone();
        runtime.spawn(async move {
            let _token = token;
            let result = save(&session, &saving, &target, &stop, &mut publisher).await;
            publisher.finish(match result {
                Ok(path) => SaveState::Saved(path),
                Err(failure) => SaveState::Failed(failure),
            });
        });
        handle
    }
}

/// Publishes progress at most every [`PROGRESS_INTERVAL`], and an end even when the task is
/// dropped before it reaches one.
struct Publisher {
    sender: watch::Sender<SaveState>,
    last: Option<Instant>,
    ended: bool,
}

impl Publisher {
    fn new(sender: watch::Sender<SaveState>) -> Publisher {
        Publisher {
            sender,
            last: None,
            ended: false,
        }
    }

    /// Publishes `progress` unless one went out less than [`PROGRESS_INTERVAL`] ago.
    fn progress(&mut self, progress: SaveProgress) {
        let now = Instant::now();
        if self
            .last
            .is_some_and(|last| now.duration_since(last) < PROGRESS_INTERVAL)
        {
            return;
        }
        self.publish(progress);
    }

    /// Publishes `progress` at once.
    fn publish(&mut self, progress: SaveProgress) {
        self.last = Some(Instant::now());
        self.sender.send_replace(SaveState::Saving(progress));
    }

    fn finish(&mut self, end: SaveState) {
        self.ended = true;
        self.sender.send_replace(end);
    }
}

impl Drop for Publisher {
    fn drop(&mut self) {
        // A task dropped mid-save (the runtime shutting down) still ends its handles.
        if !self.ended {
            self.sender
                .send_replace(SaveState::Failed(SaveFailure::Cancelled));
        }
    }
}

/// Runs one save to its end. The destination is opened only once the source answered, and
/// whatever it wrote is removed unless the save completes (`Sink` sees to it, even when this
/// future is dropped).
async fn save(
    session: &Session,
    saving: &Saving,
    target: &SaveTarget,
    stop: &CancellationToken,
    progress: &mut Publisher,
) -> Result<PathBuf, SaveFailure> {
    let epoch = session.inner.epochs.current();
    session
        .inner
        .epochs
        .validate(epoch.generation)
        .map_err(|_| SaveFailure::Cancelled)?;
    let cancelled = async {
        tokio::select! {
            () = stop.cancelled() => {}
            () = epoch.cancel.cancelled() => {}
        }
    };
    tokio::pin!(cancelled);
    let source = tokio::select! {
        biased;
        () = &mut cancelled => return Err(SaveFailure::Cancelled),
        source = open_source(session, &epoch, saving) => source?,
    };
    let total_bytes = match &source {
        Source::Cached(bytes) => Some(bytes.len() as u64),
        Source::Streamed(body) => body.length_bytes(),
    };
    progress.publish(SaveProgress {
        saved_bytes: 0,
        total_bytes,
    });
    let target = target.clone();
    let id = saving.id();
    let mut sink = blocking(move || Sink::open(&target, id)).await?;
    let poured = tokio::select! {
        biased;
        () = &mut cancelled => Err(SaveFailure::Cancelled),
        poured = pour(session, source, &mut sink, total_bytes, progress) => poured,
    };
    poured?;
    progress.publish(SaveProgress {
        saved_bytes: sink.written,
        total_bytes,
    });
    // Once every byte is written the save completes: finishing is not cancelled midway.
    blocking(move || sink.finish()).await
}

/// Writes every byte of `source` into `sink`, publishing progress.
async fn pour(
    session: &Session,
    source: Source,
    sink: &mut Sink,
    total_bytes: Option<u64>,
    progress: &mut Publisher,
) -> Result<(), SaveFailure> {
    let step = |sink: &Sink| SaveProgress {
        saved_bytes: sink.written,
        total_bytes,
    };
    match source {
        Source::Cached(bytes) => {
            for piece in bytes.chunks(WRITE_BYTES) {
                write(sink, piece.to_vec()).await?;
                progress.progress(step(sink));
            }
        }
        Source::Streamed(mut body) => {
            let mut pending = Vec::with_capacity(WRITE_BYTES);
            loop {
                // An idle timeout, not a whole-transfer one: a large original may take minutes.
                let chunk = tokio::time::timeout(
                    session.client.inner.config.request_timeout,
                    body.next_chunk(),
                )
                .await
                .map_err(|_| SaveFailure::Unreachable)?
                .map_err(|error| failure_of(&MediaError::Transport(error)))?;
                let end = chunk.is_none();
                if let Some(chunk) = chunk {
                    pending.extend_from_slice(&chunk);
                }
                if pending.len() >= WRITE_BYTES || (end && !pending.is_empty()) {
                    let piece = std::mem::replace(&mut pending, Vec::with_capacity(WRITE_BYTES));
                    write(sink, piece).await?;
                    progress.progress(step(sink));
                }
                if end {
                    break;
                }
            }
        }
    }
    Ok(())
}

/// Writes `piece` into `sink`'s file on a blocking thread. Dropped midway, the file goes with
/// the blocking task and `sink` removes what was written.
async fn write(sink: &mut Sink, piece: Vec<u8>) -> Result<(), SaveFailure> {
    let mut file = sink.take_file()?;
    let length = piece.len() as u64;
    let file = blocking(move || {
        file.write_all(&piece)
            .map(|()| file)
            .map_err(|error| SaveFailure::Destination(error.kind()))
    })
    .await?;
    sink.put_file(file, length);
    Ok(())
}

/// Runs blocking file work off the runtime's workers.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, SaveFailure> + Send + 'static,
) -> Result<T, SaveFailure> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|_| SaveFailure::Cancelled)?
}

/// The failure a media error means for a save.
fn failure_of(error: &MediaError) -> SaveFailure {
    if super::download::denied(error) {
        return SaveFailure::Gone;
    }
    match error {
        MediaError::TooLarge | MediaError::Transport(papo_api::media::MediaError::TooLarge) => {
            SaveFailure::TooLarge
        }
        // The account or client went, or changed under the save.
        MediaError::TaskStopped
        | MediaError::Session(
            ClientError::Closed
            | ClientError::Superseded
            | ClientError::SignedOut
            | ClientError::Identity
            | ClientError::Executor
            | ClientError::RuntimeContext,
        ) => SaveFailure::Cancelled,
        // Offline, a server error, a damaged answer or cached copy: try again later.
        _ => SaveFailure::Unreachable,
    }
}
