//! The writer a staging copy streams into: hashing, the 100 MiB ceiling and cancellation on
//! every write. Publishing the finished file is `files`'.

use super::{MAX_FILE_BYTES, OutgoingError};
use crate::{ClientError, StorageError};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{ErrorKind, Write},
};

/// Why a staging sink stopped accepting bytes; kept apart from `io::Error` so a transform's own
/// failure cannot hide a cancellation or the size ceiling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SinkStop {
    Cancelled,
    TooLarge,
    Storage(ErrorKind),
}

/// The staged file being written: hashes every byte, enforces the 100 MiB ceiling and honours
/// cancellation on each write, so a transform streams into staging without a second copy.
pub(super) struct StagingSink<C: FnMut() -> bool, N: FnMut()> {
    pub(super) file: std::io::BufWriter<File>,
    pub(super) hasher: Sha256,
    pub(super) total_bytes: u64,
    pub(super) cancelled: C,
    pub(super) copied_chunk: N,
    pub(super) stop: Option<SinkStop>,
}

impl<C: FnMut() -> bool, N: FnMut()> Write for StagingSink<C, N> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if (self.cancelled)() {
            return Err(self.fail(SinkStop::Cancelled));
        }
        let added = u64::try_from(bytes.len()).map_err(|_| self.fail(SinkStop::TooLarge))?;
        let total = self
            .total_bytes
            .checked_add(added)
            .filter(|total| *total <= MAX_FILE_BYTES)
            .ok_or_else(|| self.fail(SinkStop::TooLarge))?;
        self.file
            .write_all(bytes)
            .map_err(|error| self.fail(SinkStop::Storage(error.kind())))?;
        self.hasher.update(bytes);
        self.total_bytes = total;
        (self.copied_chunk)();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<C: FnMut() -> bool, N: FnMut()> StagingSink<C, N> {
    fn fail(&mut self, stop: SinkStop) -> std::io::Error {
        self.stop = Some(stop);
        std::io::Error::other("staging sink stopped")
    }

    /// The sink's own failure, if it stopped the writer, as the typed outgoing error.
    pub(super) fn stop_error(&self) -> Option<OutgoingError> {
        self.stop.map(|stop| match stop {
            SinkStop::Cancelled => ClientError::Closed.into(),
            SinkStop::TooLarge => OutgoingError::InvalidInput,
            SinkStop::Storage(kind) => {
                OutgoingError::Storage(StorageError::from(std::io::Error::from(kind)))
            }
        })
    }
}
