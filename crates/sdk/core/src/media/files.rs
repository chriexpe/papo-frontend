//! Immutable encoded-file installation and pinned reads; orphan scanning and reference authorization live elsewhere.

use super::{MediaData, MediaError, MediaStore, index::Reference};
use sha2::{Digest, Sha256};
use std::{fs, io::Write, path::Path, sync::Arc};

#[cfg(test)]
pub(super) type ReadProbe = Arc<dyn Fn([u8; 32]) + Send + Sync>;

#[cfg(test)]
pub(super) fn install(root: &crate::DataRoot, bytes: &[u8]) -> Result<[u8; 32], MediaError> {
    install_checked(root, bytes, &super::pins::ReadPins::default())
}

fn install_checked(
    root: &crate::DataRoot,
    bytes: &[u8],
    pins: &super::pins::ReadPins,
) -> Result<[u8; 32], MediaError> {
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    let path = root.media_content(&digest);
    let parent = path.parent().ok_or(MediaError::InvalidData)?;
    crate::private_fs::staging_directory(parent)?;
    if path.exists() {
        if read(&path, digest).is_ok_and(|existing| existing.as_ref() == bytes) {
            return Ok(digest);
        }
        if pins.contains(&digest) {
            return Err(MediaError::Unavailable);
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
        pin: super::pins::ReadPin,
        permit: tokio::sync::OwnedSemaphorePermit,
    ) -> Result<(MediaData, tokio::sync::OwnedSemaphorePermit), MediaError> {
        let path = self.root.media_content(&reference.digest);
        let lease = Arc::clone(&self.lease);
        #[cfg(test)]
        let probe = self
            .read_probe
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let (bytes, permit) = tokio::task::spawn_blocking(move || {
            let (_lease, _pin) = (lease, pin);
            #[cfg(test)]
            if let Some(probe) = probe {
                probe(reference.digest);
            }
            Ok::<_, MediaError>((read(&path, reference.digest)?, permit))
        })
        .await
        .map_err(|_| MediaError::TaskStopped)??;
        Ok((
            MediaData {
                bytes,
                content_type: reference.content_type,
                range: reference.range,
            },
            permit,
        ))
    }

    /// The caller holds the media gate until the installed file has a committed grant.
    pub(super) async fn install(&self, bytes: Arc<[u8]>) -> Result<[u8; 32], MediaError> {
        let root = self.root.clone();
        let lease = Arc::clone(&self.lease);
        let pins = self.pins.clone();
        tokio::task::spawn_blocking(move || {
            let _lease = lease;
            install_checked(&root, &bytes, &pins)
        })
        .await
        .map_err(|_| MediaError::TaskStopped)?
    }
}
