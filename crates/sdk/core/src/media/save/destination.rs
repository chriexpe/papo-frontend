//! Where a save's bytes go: a hidden partial file beside the destination, synced, then renamed
//! into the first free name of a folder or over the chosen file; or, where a sandbox grants only
//! the chosen file, that file written in place. A `Sink` removes what it wrote unless it
//! finished, also when it is dropped mid-save. The names are `save_names`'; the save's flow is
//! `save`'s.

use super::super::save_names;
use super::{SaveFailure, SaveTarget};
use crate::Uuid;
use std::{
    fs::{self, File, OpenOptions},
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_PARTIAL: AtomicU64 = AtomicU64::new(1);

/// The most partial-file names tried before the folder counts as unwritable.
const MAX_PARTIAL_TRIES: u32 = 1_000;

/// What finishing does with the written file.
enum Ending {
    /// Rename it into the first free name for `name` in `folder`.
    IntoFolder { folder: PathBuf, name: String },
    /// Rename it over the chosen file.
    Over(PathBuf),
    /// It is the chosen file, written in place.
    InPlace,
}

/// The file a save writes into, and where it ends up.
pub(super) struct Sink {
    /// The open file; away while a blocking write holds it.
    file: Option<File>,
    /// The file written: a hidden partial file, or the chosen file in place.
    written_path: PathBuf,
    ending: Ending,
    /// Bytes written so far.
    pub(super) written: u64,
    /// Set once finishing succeeded: nothing is removed then.
    kept: bool,
}

impl Sink {
    /// Opens where `target`'s bytes go. Blocking.
    pub(super) fn open(target: &SaveTarget, attachment: Uuid) -> Result<Sink, SaveFailure> {
        let destination = |error: std::io::Error| SaveFailure::Destination(error.kind());
        match target {
            SaveTarget::Folder { folder, name } => {
                let (file, partial) = partial_beside(folder, attachment).map_err(destination)?;
                Ok(Sink::new(
                    file,
                    partial,
                    Ending::IntoFolder {
                        folder: folder.clone(),
                        name: save_names::safe_name(name),
                    },
                ))
            }
            SaveTarget::File(chosen) => {
                let folder = chosen.parent().unwrap_or_else(|| Path::new("."));
                if let Ok((file, partial)) = partial_beside(folder, attachment) {
                    return Ok(Sink::new(file, partial, Ending::Over(chosen.clone())));
                }
                // A sandbox grants only the chosen file: write it in place.
                let file = File::create(chosen).map_err(destination)?;
                Ok(Sink::new(file, chosen.clone(), Ending::InPlace))
            }
        }
    }

    fn new(file: File, written_path: PathBuf, ending: Ending) -> Sink {
        Sink {
            file: Some(file),
            written_path,
            ending,
            written: 0,
            kept: false,
        }
    }

    /// Hands the file to a blocking write.
    pub(super) fn take_file(&mut self) -> Result<File, SaveFailure> {
        self.file
            .take()
            .ok_or(SaveFailure::Destination(ErrorKind::Other))
    }

    /// Takes the file back after a write of `length` bytes.
    pub(super) fn put_file(&mut self, file: File, length: u64) {
        self.file = Some(file);
        self.written += length;
    }

    /// Syncs the file and puts it where it ends up. Blocking.
    pub(super) fn finish(mut self) -> Result<PathBuf, SaveFailure> {
        let destination = |error: std::io::Error| SaveFailure::Destination(error.kind());
        let file = self.take_file()?;
        file.sync_all().map_err(destination)?;
        drop(file);
        let end = match &self.ending {
            Ending::InPlace => self.written_path.clone(),
            Ending::Over(chosen) => {
                fs::rename(&self.written_path, chosen).map_err(destination)?;
                chosen.clone()
            }
            Ending::IntoFolder { folder, name } => {
                let free = save_names::claim_free(folder, name).map_err(destination)?;
                // The claim is an empty file of ours; the rename replaces it atomically.
                if let Err(error) = fs::rename(&self.written_path, &free) {
                    let _ = fs::remove_file(&free);
                    return Err(destination(error));
                }
                free
            }
        };
        self.kept = true;
        Ok(end)
    }
}

impl Drop for Sink {
    fn drop(&mut self) {
        // One unlink; on a save's normal failure path this runs on a blocking thread.
        if !self.kept {
            self.file = None;
            let _ = fs::remove_file(&self.written_path);
        }
    }
}

/// Creates a new hidden partial file in `folder`.
fn partial_beside(folder: &Path, attachment: Uuid) -> std::io::Result<(File, PathBuf)> {
    for _ in 0..MAX_PARTIAL_TRIES {
        let partial = folder.join(format!(
            ".{attachment}-{}.papo-part",
            NEXT_PARTIAL.fetch_add(1, Ordering::Relaxed)
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&partial)
        {
            Ok(file) => return Ok((file, partial)),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(ErrorKind::AlreadyExists.into())
}
