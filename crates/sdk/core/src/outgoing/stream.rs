//! Incremental file-backed multipart attachment stream with bounded buffers, hash checks and account fences.

use super::{
    Operation, OperationKind, OutgoingError, UploadProgress, files::Reference,
    progress::ProgressReporter,
};
use crate::{Session, data_root::ProfileLease};
use bytes::Bytes;
use futures_core::Stream;
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    future::Future,
    io::{self, ErrorKind, Read},
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};
use tokio::task::JoinHandle;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

const UPLOAD_CHUNK_BYTES: usize = 64 * 1024;

pub(super) struct OpenedAttachment {
    pub(super) filename: String,
    pub(super) reference: Reference,
    pub(super) file: File,
}

/// Validates and opens an operation's staged attachment files before `Sending` is claimed.
pub(super) async fn open_attachments(
    session: &Session,
    operation: &Operation,
) -> Result<(Vec<OpenedAttachment>, Option<UploadProgress>), OutgoingError> {
    let OperationKind::Message(message) = operation.kind.as_ref() else {
        return Ok((Vec::new(), None));
    };
    if message.attachments.is_empty() {
        return Ok((Vec::new(), None));
    }
    let account = session.account();
    let mut items = Vec::with_capacity(message.attachments.len());
    let mut total_bytes: u64 = 0;
    for attachment in &message.attachments {
        let staged = attachment.staged().ok_or(OutgoingError::InvalidData)?;
        if staged.account() != account {
            return Err(OutgoingError::NotFound);
        }
        total_bytes = total_bytes
            .checked_add(staged.size_bytes())
            .ok_or(OutgoingError::InvalidData)?;
        items.push((
            attachment.filename.clone(),
            Reference {
                digest: *staged.digest(),
                size_bytes: staged.size_bytes(),
            },
        ));
    }
    let opened = session
        .client
        .inner
        .registry
        .open_upload_files(session, items)
        .await?;
    let progress = UploadProgress::initial(account, operation.id, total_bytes);
    Ok((opened, Some(progress)))
}

struct ReaderState {
    file: File,
    expected_digest: [u8; 32],
    expected_size: u64,
    bytes_read: u64,
    hasher: Sha256,
}

enum ReadStep {
    Chunk(Bytes, ReaderState),
    LastChunk(Bytes),
    Eof,
}

struct SlotInner {
    reader: Option<ReaderState>,
    closed: bool,
}

pub(super) struct ActiveUploadSlot {
    inner: Mutex<SlotInner>,
}

impl ActiveUploadSlot {
    fn new(reader: ReaderState) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(SlotInner {
                reader: Some(reader),
                closed: false,
            }),
        })
    }

    fn take(&self) -> Option<ReaderState> {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if inner.closed {
            None
        } else {
            inner.reader.take()
        }
    }

    fn restore(&self, reader: ReaderState) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !inner.closed {
            inner.reader = Some(reader);
        }
    }

    pub(super) fn close(&self) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        inner.closed = true;
        drop(inner.reader.take());
    }
}

enum StreamState {
    Idle,
    Reading(JoinHandle<io::Result<ReadStep>>),
    Done,
}

/// Streams one already-opened immutable staged file in 64 KiB chunks on `spawn_blocking`.
///
/// Every in-flight blocking read holds a session [`TaskTracker`] token and [`ProfileLease`], while
/// idle file handles sit inside [`ActiveUploadSlot`] so [`crate::session::SessionInner::stop`] can
/// close them immediately even if a stalled HTTP connection still retains the stream body.
pub(super) struct StagedFileStream {
    slot: Arc<ActiveUploadSlot>,
    state: StreamState,
    root_cancel: CancellationToken,
    epoch_cancel: CancellationToken,
    tasks: TaskTracker,
    lease: Arc<ProfileLease>,
    progress: Arc<ProgressReporter>,
}

impl StagedFileStream {
    pub(super) fn new(
        session: &Session,
        file: File,
        reference: Reference,
        epoch_cancel: CancellationToken,
        lease: Arc<ProfileLease>,
        progress: Arc<ProgressReporter>,
    ) -> Self {
        let slot = ActiveUploadSlot::new(ReaderState {
            file,
            expected_digest: reference.digest,
            expected_size: reference.size_bytes,
            bytes_read: 0,
            hasher: Sha256::new(),
        });
        session.inner.outgoing.register_upload_slot(&slot);
        Self {
            slot,
            state: StreamState::Idle,
            root_cancel: session.inner.root.clone(),
            epoch_cancel,
            tasks: session.inner.tasks.clone(),
            lease,
            progress,
        }
    }

    fn is_cancelled(&self) -> bool {
        self.root_cancel.is_cancelled() || self.epoch_cancel.is_cancelled()
    }
}

impl Drop for StagedFileStream {
    fn drop(&mut self) {
        self.slot.close();
    }
}

impl Stream for StagedFileStream {
    type Item = io::Result<Bytes>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.as_mut().get_mut();
        loop {
            if matches!(this.state, StreamState::Done) {
                return Poll::Ready(None);
            }
            if this.is_cancelled() {
                this.slot.close();
                this.state = StreamState::Done;
                return Poll::Ready(Some(Err(io::Error::new(
                    ErrorKind::Interrupted,
                    "upload cancelled",
                ))));
            }
            match &mut this.state {
                StreamState::Done => return Poll::Ready(None),
                StreamState::Idle => {
                    let Some(reader) = this.slot.take() else {
                        this.state = StreamState::Done;
                        return Poll::Ready(Some(Err(io::Error::new(
                            ErrorKind::Interrupted,
                            "upload closed",
                        ))));
                    };
                    let token = this.tasks.token();
                    let lease = Arc::clone(&this.lease);
                    let root_cancel = this.root_cancel.clone();
                    let handle = tokio::task::spawn_blocking(move || {
                        let (_token, _lease) = (token, lease);
                        if root_cancel.is_cancelled() {
                            return Err(io::Error::new(ErrorKind::Interrupted, "upload cancelled"));
                        }
                        let step = read_next_chunk(reader)?;
                        if root_cancel.is_cancelled() {
                            return Err(io::Error::new(ErrorKind::Interrupted, "upload cancelled"));
                        }
                        Ok(step)
                    });
                    this.state = StreamState::Reading(handle);
                }
                StreamState::Reading(handle) => match Pin::new(handle).poll(cx) {
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(Ok(Ok(ReadStep::Chunk(chunk, reader)))) => {
                        if this.is_cancelled() {
                            drop(reader);
                            this.state = StreamState::Done;
                            return Poll::Ready(Some(Err(io::Error::new(
                                ErrorKind::Interrupted,
                                "upload cancelled",
                            ))));
                        }
                        this.slot.restore(reader);
                        this.state = StreamState::Idle;
                        this.progress.advance(chunk.len() as u64);
                        return Poll::Ready(Some(Ok(chunk)));
                    }
                    Poll::Ready(Ok(Ok(ReadStep::LastChunk(chunk)))) => {
                        this.state = StreamState::Done;
                        this.progress.advance(chunk.len() as u64);
                        return Poll::Ready(Some(Ok(chunk)));
                    }
                    Poll::Ready(Ok(Ok(ReadStep::Eof))) => {
                        this.state = StreamState::Done;
                        this.progress.advance(0);
                        return Poll::Ready(None);
                    }
                    Poll::Ready(Ok(Err(error))) => {
                        this.state = StreamState::Done;
                        return Poll::Ready(Some(Err(error)));
                    }
                    Poll::Ready(Err(_)) => {
                        this.state = StreamState::Done;
                        return Poll::Ready(Some(Err(io::Error::other("upload read failed"))));
                    }
                },
            }
        }
    }
}

fn read_next_chunk(mut reader: ReaderState) -> io::Result<ReadStep> {
    let remaining = reader
        .expected_size
        .checked_sub(reader.bytes_read)
        .ok_or_else(|| io::Error::new(ErrorKind::InvalidData, "staged size overflow"))?;
    if remaining == 0 {
        verify_eof_and_digest(&mut reader)?;
        return Ok(ReadStep::Eof);
    }
    let want =
        usize::try_from(remaining.min(UPLOAD_CHUNK_BYTES as u64)).unwrap_or(UPLOAD_CHUNK_BYTES);
    let mut chunk = vec![0_u8; want];
    let count = loop {
        match reader.file.read(&mut chunk) {
            Ok(n) => break n,
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    };
    if count == 0 {
        return Err(io::Error::new(
            ErrorKind::UnexpectedEof,
            "staged file truncated during upload",
        ));
    }
    chunk.truncate(count);
    reader.hasher.update(&chunk);
    reader.bytes_read = reader
        .bytes_read
        .checked_add(count as u64)
        .ok_or_else(|| io::Error::new(ErrorKind::InvalidData, "staged length overflow"))?;
    if reader.bytes_read == reader.expected_size {
        verify_eof_and_digest(&mut reader)?;
        return Ok(ReadStep::LastChunk(Bytes::from(chunk)));
    }
    Ok(ReadStep::Chunk(Bytes::from(chunk), reader))
}

fn verify_eof_and_digest(reader: &mut ReaderState) -> io::Result<()> {
    let mut probe = [0_u8; 1];
    let extra = loop {
        match reader.file.read(&mut probe) {
            Ok(n) => break n,
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    };
    if extra != 0 {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "staged file grew during upload",
        ));
    }
    let actual: [u8; 32] = reader.hasher.clone().finalize().into();
    if actual != reader.expected_digest {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "staged file digest mismatch during upload",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
