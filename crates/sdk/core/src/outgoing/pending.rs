//! An attachment still being copied into staging: a handle returned before the copy ends, so a
//! message can be sent while its file is still preparing. The copy is the ordinary staging copy;
//! this module only runs it in the background and publishes its result.

use super::{ClientError, OutgoingError, OutgoingService, StagedAttachment, Uuid};
use std::{
    io::{ErrorKind, Read},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

/// Why a background staging copy ended without a staged file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum StagingFailure {
    /// The source passed the 100 MiB attachment limit.
    TooLarge,
    /// The source could not be read (the file went away, permission, I/O).
    Unreadable(ErrorKind),
    /// Every handle was dropped, or the account or client closed, before the copy finished.
    Cancelled,
    /// Staging storage failed.
    Storage,
}

static NEXT_PENDING: AtomicU64 = AtomicU64::new(1);

type Outcome = Option<Result<StagedAttachment, StagingFailure>>;

/// Stops the copy once the last handle is gone; nothing else can still want its result.
struct CopyGuard(CancellationToken);

impl Drop for CopyGuard {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

/// A file being copied into this account's staging. Clone it freely; the copy stops when every
/// clone (and every queued message holding one) is dropped.
#[derive(Clone)]
pub struct PendingAttachment {
    id: Uuid,
    outcome: watch::Receiver<Outcome>,
    _guard: Arc<CopyGuard>,
}

impl std::fmt::Debug for PendingAttachment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingAttachment")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl PendingAttachment {
    /// A process-local identity, stable for the copy's life; clients key tray tiles by it.
    #[must_use]
    pub const fn id(&self) -> Uuid {
        self.id
    }

    /// The result once the copy has ended, without waiting.
    #[must_use]
    pub fn outcome(&self) -> Option<Result<StagedAttachment, StagingFailure>> {
        self.outcome.borrow().clone()
    }

    /// Waits for the copy to end.
    /// # Errors
    /// Returns the typed reason the copy produced no staged file.
    pub async fn staged(&self) -> Result<StagedAttachment, StagingFailure> {
        let mut outcome = self.outcome.clone();
        loop {
            if let Some(result) = outcome.borrow_and_update().clone() {
                return result;
            }
            if outcome.changed().await.is_err() {
                return Err(StagingFailure::Cancelled);
            }
        }
    }
}

/// Fails the copy's next read once its token is cancelled, so an abandoned copy stops promptly.
struct Cancellable<R> {
    reader: R,
    token: CancellationToken,
}

impl<R: Read> Read for Cancellable<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if self.token.is_cancelled() {
            // Not `Interrupted`: the copy loop retries that one.
            return Err(std::io::Error::other("staging abandoned"));
        }
        self.reader.read(buffer)
    }
}

fn failure(error: &OutgoingError, abandoned: bool) -> StagingFailure {
    if abandoned {
        return StagingFailure::Cancelled;
    }
    match error {
        OutgoingError::InvalidInput => StagingFailure::TooLarge,
        OutgoingError::Source(kind) => StagingFailure::Unreadable(*kind),
        OutgoingError::Session(_) => StagingFailure::Cancelled,
        _ => StagingFailure::Storage,
    }
}

impl OutgoingService {
    /// Starts copying an attachment source into staging and returns at once with a handle.
    ///
    /// The copy is [`Self::stage_attachment`]'s, run in the background; the handle can go into a
    /// message ([`super::OutgoingAttachment::from_pending`]) before it ends. The reader is driven
    /// on a blocking worker, with the same caveats as `stage_attachment`.
    /// # Errors
    /// Returns a revoked account or closed client.
    pub fn begin_staging<R>(&self, reader: R) -> Result<PendingAttachment, OutgoingError>
    where
        R: Read + Send + 'static,
    {
        self.check()?;
        let session = self.session.clone();
        let token = session.inner.tasks.token();
        if session.is_closed() {
            return Err(ClientError::Closed.into());
        }
        let id = Uuid::from_u64_pair(0, NEXT_PENDING.fetch_add(1, Ordering::Relaxed));
        let (sender, outcome) = watch::channel(None);
        let stop = CancellationToken::new();
        let reader = Cancellable {
            reader,
            token: stop.clone(),
        };
        let service = self.clone();
        let copy_stop = stop.clone();
        session.client.inner.runtime.spawn(async move {
            let _token = token;
            let result = async {
                service.ensure().await?;
                service
                    .session
                    .client
                    .inner
                    .registry
                    .stage_reader(&service.session, reader, id)
                    .await
            }
            .await;
            let result = result.map_err(|error| failure(&error, copy_stop.is_cancelled()));
            let _ = sender.send(Some(result));
        });
        Ok(PendingAttachment {
            id,
            outcome,
            _guard: Arc::new(CopyGuard(stop)),
        })
    }
}
