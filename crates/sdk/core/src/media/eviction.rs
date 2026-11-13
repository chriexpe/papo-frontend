//! Bounded reference retirement and physical unlink; source grants and fetch authorization live elsewhere.

use super::{MediaError, MediaStore, index::account_key};
use crate::AccountId;
use rusqlite::OptionalExtension;

impl MediaStore {
    pub(crate) async fn clear_refs(&self, account: AccountId) -> Result<(), MediaError> {
        let gate = std::sync::Arc::new(std::sync::Arc::clone(&self.gate).lock_owned().await);
        let account = account_key(account);
        loop {
            let account = account.clone();
            let digests:Vec<Vec<u8>> = self.writer.cache_write(move |tx| -> Result<_, MediaError> {
                let mut query=tx.prepare("SELECT DISTINCT digest
                     FROM refs
                     WHERE rowid IN (SELECT rowid
                     FROM refs
                     WHERE account=?1
                     LIMIT 100)")?;
                let digests=query.query_map([&account],|row|row.get(0))?.collect::<Result<Vec<_>,_>>()?;
                tx.execute("DELETE FROM refs WHERE rowid IN (SELECT rowid FROM refs WHERE account=?1 LIMIT 100)",[account])?;
                Ok(digests)
            }).await?;
            if digests.is_empty() {
                break;
            }
            for digest in digests {
                let digest: [u8; 32] = digest.try_into().map_err(|_| MediaError::InvalidData)?;
                let retired:bool=self.writer.cache_write(move |tx|->Result<_,MediaError> {
                    Ok(tx.execute("DELETE FROM files WHERE digest=?1 AND NOT EXISTS(SELECT 1 FROM refs WHERE digest=?1)",[digest.as_slice()])?==1)
                }).await?;
                if retired {
                    let path = self.root.media_content(&digest);
                    let lease = std::sync::Arc::clone(&self.lease);
                    let gate = std::sync::Arc::clone(&gate);
                    tokio::task::spawn_blocking(move || {
                        let (_lease, _gate) = (lease, gate);
                        super::files::unlink(&path)
                    })
                    .await
                    .map_err(|_| MediaError::TaskStopped)??;
                }
            }
        }
        Ok(())
    }

    pub(crate) async fn remove_account(&self, account: AccountId) -> Result<(), MediaError> {
        self.clear_refs(account).await?;
        let account = account_key(account);
        loop {
            let account = account.clone();
            let changed: usize = self
                .writer
                .cache_write(move |tx| -> Result<_, MediaError> {
                    Ok(tx.execute(
                        "DELETE
                     FROM history_access
                     WHERE rowid IN (SELECT rowid
                     FROM history_access
                     WHERE account=?1
                     LIMIT 100)",
                        [account],
                    )?)
                })
                .await?;
            if changed == 0 {
                return Ok(());
            }
        }
    }

    /// Remove all references before unlinking a victim. Cancellation can only leave an orphan.
    pub(crate) async fn evict_one(&self) -> Result<Option<u64>, MediaError> {
        let gate = std::sync::Arc::clone(&self.gate).lock_owned().await;
        let digest: Option<Vec<u8>> = self
            .writer
            .cache_write(|tx| -> Result<_, MediaError> {
                Ok(tx
                    .query_row(
                        "SELECT digest FROM files ORDER BY last_used,digest LIMIT 1",
                        [],
                        |row| row.get(0),
                    )
                    .optional()?)
            })
            .await?;
        let Some(digest) = digest else {
            return Ok(None);
        };
        let digest: [u8; 32] = digest.try_into().map_err(|_| MediaError::InvalidData)?;
        loop {
            let changed: usize = self.writer.cache_write(move |tx| -> Result<_, MediaError> {
                Ok(tx.execute("DELETE FROM refs WHERE rowid IN (SELECT rowid FROM refs WHERE digest=?1 LIMIT 100)", [digest.as_slice()])?)
            }).await?;
            if changed == 0 {
                break;
            }
        }
        self.writer
            .cache_write(move |tx| -> Result<_, MediaError> {
                tx.execute("DELETE FROM files WHERE digest=?1", [digest.as_slice()])?;
                Ok(())
            })
            .await?;
        let path = self.root.media_content(&digest);
        let lease = std::sync::Arc::clone(&self.lease);
        let removed_bytes = tokio::task::spawn_blocking(move || {
            let (_lease, _gate) = (lease, gate);
            let bytes = match std::fs::File::open(&path) {
                Ok(file) => {
                    fs2::FileExt::allocated_size(&file).map_err(|_| MediaError::Filesystem)?
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
                Err(_) => return Err(MediaError::Filesystem),
            };
            super::files::unlink(&path)?;
            Ok::<_, MediaError>(bytes)
        })
        .await
        .map_err(|_| MediaError::TaskStopped)??;
        Ok(Some(removed_bytes))
    }
}
