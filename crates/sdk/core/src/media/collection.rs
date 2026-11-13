//! Age-gated orphan scans in bounded filesystem turns; reference admission and pins share media exclusion.

use super::{MediaError, MediaStore};
use rusqlite::OptionalExtension;
use std::{
    fs,
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime},
};

const ENTRIES_PER_TURN: usize = 100;

/// Persistent directory iterators avoid rescanning a large prefix on each maintenance tick.
#[derive(Default)]
pub(super) struct Scan {
    prefixes: Option<fs::ReadDir>,
    entries: Option<fs::ReadDir>,
}
struct Candidate {
    path: PathBuf,
    digest: [u8; 32],
    partial: bool,
}
struct Batch {
    candidates: Vec<Candidate>,
    finished: bool,
}
impl Scan {
    fn next(
        &mut self,
        root: &crate::DataRoot,
        now: SystemTime,
        grace: Duration,
    ) -> Result<Batch, MediaError> {
        if self.prefixes.is_none() {
            self.prefixes =
                Some(fs::read_dir(root.path().join("media")).map_err(|_| MediaError::Filesystem)?);
        }
        let mut candidates = Vec::new();
        for _ in 0..ENTRIES_PER_TURN {
            if let Some(entries) = &mut self.entries {
                if let Some(entry) = entries.next() {
                    let entry = entry.map_err(|_| MediaError::Filesystem)?;
                    let metadata = match fs::symlink_metadata(entry.path()) {
                        Ok(metadata) => metadata,
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                        Err(_) => return Err(MediaError::Filesystem),
                    };
                    if !metadata.is_file() || metadata.file_type().is_symlink() {
                        continue;
                    }
                    let name = entry.file_name();
                    let Some(name) = name.to_str() else {
                        continue;
                    };
                    let raw = name.strip_suffix(".partial").unwrap_or(name);
                    let Some(digest) = parse_digest(raw) else {
                        continue;
                    };
                    let age = now
                        .duration_since(metadata.modified().map_err(|_| MediaError::Filesystem)?)
                        .unwrap_or_default();
                    if age >= grace {
                        candidates.push(Candidate {
                            path: entry.path(),
                            digest,
                            partial: raw != name,
                        });
                    }
                    continue;
                }
                self.entries = None;
            }
            let Some(prefix) = self.prefixes.as_mut().and_then(Iterator::next) else {
                self.prefixes = None;
                return Ok(Batch {
                    candidates,
                    finished: true,
                });
            };
            let prefix = prefix.map_err(|_| MediaError::Filesystem)?;
            let name = prefix.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if name.len() != 2
                || !name.bytes().all(|b| b.is_ascii_hexdigit())
                || !prefix
                    .file_type()
                    .map_err(|_| MediaError::Filesystem)?
                    .is_dir()
            {
                continue;
            }
            self.entries = match fs::read_dir(prefix.path()) {
                Ok(entries) => Some(entries),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(_) => return Err(MediaError::Filesystem),
            };
        }
        Ok(Batch {
            candidates,
            finished: false,
        })
    }
}
impl MediaStore {
    /// One turn inspects at most 100 directory entries, including non-content names.
    pub(crate) async fn collect_step(
        &self,
        grace: Duration,
        now: SystemTime,
    ) -> Result<bool, MediaError> {
        self.collect_scan_step(Arc::clone(&self.collection), grace, now)
            .await
    }

    async fn collect_scan_step(
        &self,
        scan: Arc<std::sync::Mutex<Scan>>,
        grace: Duration,
        now: SystemTime,
    ) -> Result<bool, MediaError> {
        let gate = Arc::new(Arc::clone(&self.gate).lock_owned().await);
        let root = self.root.clone();
        let lease = Arc::clone(&self.lease);
        let held = Arc::clone(&gate);
        let batch = tokio::task::spawn_blocking(move || {
            let (_lease, _gate) = (lease, held);
            let mut scan = scan
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let result = scan.next(&root, now, grace);
            if result.is_err() {
                *scan = Scan::default();
            }
            result
        })
        .await
        .map_err(|_| MediaError::TaskStopped)??;
        let finished = batch.finished;
        let pins = self.pins.snapshot();
        let eligible = self
            .writer
            .cache_write(move |tx| -> Result<_, MediaError> {
                let mut query = tx.prepare("SELECT 1 FROM refs WHERE digest=?1 LIMIT 1")?;
                let mut eligible = Vec::new();
                for candidate in batch.candidates {
                    if pins.contains(&candidate.digest) {
                        continue;
                    }
                    let referenced = query
                        .query_row([candidate.digest.as_slice()], |_| Ok(()))
                        .optional()?
                        .is_some();
                    if candidate.partial || !referenced {
                        if !candidate.partial {
                            tx.execute(
                                "DELETE FROM files WHERE digest=?1",
                                [candidate.digest.as_slice()],
                            )?;
                        }
                        eligible.push(candidate.path);
                    }
                }
                Ok(eligible)
            })
            .await?;
        let lease = Arc::clone(&self.lease);
        tokio::task::spawn_blocking(move || {
            let (_lease, _gate) = (lease, gate);
            for path in eligible {
                super::files::unlink(&path)?;
            }
            Ok::<_, MediaError>(())
        })
        .await
        .map_err(|_| MediaError::TaskStopped)??;
        Ok(finished)
    }

    pub(crate) async fn collect(&self, grace: Duration) -> Result<(), MediaError> {
        self.collect_at(grace, SystemTime::now()).await
    }

    pub(super) async fn collect_at(
        &self,
        grace: Duration,
        now: SystemTime,
    ) -> Result<(), MediaError> {
        let store = self.clone();
        tokio::spawn(async move {
            // An explicit collection starts a full pass; periodic work retains its own cursor.
            let scan = Arc::default();
            while !store
                .collect_scan_step(Arc::clone(&scan), grace, now)
                .await?
            {
                tokio::task::yield_now().await;
            }
            Ok(())
        })
        .await
        .map_err(|_| MediaError::TaskStopped)?
    }
}

fn parse_digest(raw: &str) -> Option<[u8; 32]> {
    if raw.len() != 64 || !raw.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut digest = [0; 32];
    for (index, byte) in digest.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&raw[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(digest)
}

#[cfg(test)]
pub(super) fn collect_files(
    root: &crate::DataRoot,
    protected: &std::collections::BTreeSet<[u8; 32]>,
    now: SystemTime,
    grace: Duration,
) -> Result<(), MediaError> {
    let mut scan = Scan::default();
    loop {
        let batch = scan.next(root, now, grace)?;
        for candidate in batch.candidates {
            if candidate.partial || !protected.contains(&candidate.digest) {
                super::files::unlink(&candidate.path)?;
            }
        }
        if batch.finished {
            return Ok(());
        }
    }
}
