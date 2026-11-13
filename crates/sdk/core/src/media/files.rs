//! Immutable content installation and orphan collection; an existing file never creates an account grant.

use super::{MediaData, MediaError, MediaStore, index::Reference};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    io::Write,
    path::Path,
    sync::Arc,
    time::{Duration, SystemTime},
};

pub(super) fn install(root: &crate::DataRoot, bytes: &[u8]) -> Result<[u8; 32], MediaError> {
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    let path = root.media_content(&digest);
    let parent = path.parent().ok_or(MediaError::InvalidData)?;
    crate::private_fs::staging_directory(parent)?;
    if path.exists() {
        if read(&path, digest).is_ok_and(|existing| existing.as_ref() == bytes) {
            return Ok(digest);
        }
        unlink(&path)?;
    }
    let temporary = path.with_extension("partial");
    let mut file = crate::private_fs::staged_file(&temporary)?;
    file.set_len(0).map_err(|_| MediaError::Filesystem)?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|_| MediaError::Filesystem)?;
    drop(file);
    fs::rename(&temporary, &path).map_err(|_| MediaError::Filesystem)?;
    Ok(digest)
}

fn read(path: &Path, digest: [u8; 32]) -> Result<Arc<[u8]>, MediaError> {
    if fs::symlink_metadata(path)
        .map_err(|_| MediaError::Unavailable)?
        .file_type()
        .is_symlink()
    {
        return Err(MediaError::InvalidData);
    }
    if fs::metadata(path)
        .map_err(|_| MediaError::Unavailable)?
        .len()
        > 100 * 1024 * 1024
    {
        return Err(MediaError::TooLarge);
    }
    let bytes = fs::read(path).map_err(|_| MediaError::Unavailable)?;
    if <[u8; 32]>::from(Sha256::digest(&bytes)) != digest {
        return Err(MediaError::InvalidData);
    }
    Ok(bytes.into())
}

pub(super) fn unlink(path: &Path) -> Result<(), MediaError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(MediaError::Filesystem),
    }
}

impl MediaStore {
    pub(super) async fn read_reference(
        &self,
        reference: Reference,
    ) -> Result<MediaData, MediaError> {
        let path = self.root.media_content(&reference.digest);
        let lease = Arc::clone(&self.lease);
        let bytes = tokio::task::spawn_blocking(move || {
            let _lease = lease;
            read(&path, reference.digest)
        })
        .await
        .map_err(|_| MediaError::TaskStopped)??;
        self.touch(reference.digest).await?;
        Ok(MediaData {
            bytes,
            content_type: reference.content_type,
            range: reference.range,
        })
    }

    /// The caller holds the media gate until the installed file has a committed grant.
    pub(super) async fn install(&self, bytes: Arc<[u8]>) -> Result<[u8; 32], MediaError> {
        let root = self.root.clone();
        let lease = Arc::clone(&self.lease);
        tokio::task::spawn_blocking(move || {
            let _lease = lease;
            install(&root, &bytes)
        })
        .await
        .map_err(|_| MediaError::TaskStopped)?
    }

    pub(crate) async fn collect(&self, grace: Duration) -> Result<(), MediaError> {
        let store = self.clone();
        tokio::spawn(async move {
            let _gate = store.gate.lock().await;
            store.collect_locked(grace).await
        })
        .await
        .map_err(|_| MediaError::TaskStopped)?
    }

    pub(super) async fn collect_locked(&self, grace: Duration) -> Result<(), MediaError> {
        let mut protected = BTreeSet::new();
        let mut after = Vec::<u8>::new();
        loop {
            let cursor = after.clone();
            let rows: Vec<Vec<u8>> = self
                .writer
                .cache_write(move |tx| -> Result<_, MediaError> {
                    let mut query = tx.prepare(
                        "SELECT digest
                     FROM files
                     WHERE digest>?1
                     AND EXISTS(SELECT 1
                     FROM refs
                     WHERE refs.digest=files.digest)
                     ORDER BY digest
                     LIMIT 100",
                    )?;
                    Ok(query
                        .query_map([cursor], |row| row.get(0))?
                        .collect::<Result<_, _>>()?)
                })
                .await?;
            let Some(last) = rows.last() else {
                break;
            };
            after.clone_from(last);
            for row in rows {
                protected.insert(row.try_into().map_err(|_| MediaError::InvalidData)?);
            }
        }
        let root = self.root.clone();
        let lease = Arc::clone(&self.lease);
        tokio::task::spawn_blocking(move || {
            let _lease = lease;
            collect_files(&root, &protected, SystemTime::now(), grace)
        })
        .await
        .map_err(|_| MediaError::TaskStopped)?
    }
}

pub(super) fn collect_files(
    root: &crate::DataRoot,
    protected: &BTreeSet<[u8; 32]>,
    now: SystemTime,
    grace: Duration,
) -> Result<(), MediaError> {
    let directory = root.path().join("media");
    for prefix in fs::read_dir(directory).map_err(|_| MediaError::Filesystem)? {
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
        for entry in fs::read_dir(prefix.path()).map_err(|_| MediaError::Filesystem)? {
            let entry = entry.map_err(|_| MediaError::Filesystem)?;
            let metadata =
                fs::symlink_metadata(entry.path()).map_err(|_| MediaError::Filesystem)?;
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
            if protected.contains(&digest) && name == raw {
                continue;
            }
            let age = now
                .duration_since(metadata.modified().map_err(|_| MediaError::Filesystem)?)
                .unwrap_or_default();
            if age >= grace {
                unlink(&entry.path())?;
            }
        }
    }
    Ok(())
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
