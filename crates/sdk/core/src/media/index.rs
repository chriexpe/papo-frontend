//! Rebuildable reference grants and physical-file LRU; network authorization belongs to `MediaService`.

use super::{MediaError, MediaSource, MediaStore};
use crate::{AccountId, Session, storage_worker::WorkerError};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Reference {
    pub digest: [u8; 32],
    pub content_type: Option<String>,
    pub etag: Option<String>,
    pub modified: Option<String>,
    #[serde(default)]
    pub range: Option<super::MediaRange>,
}

pub(super) fn initialize(connection: &mut Connection) -> Result<(), WorkerError> {
    let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version == 1 {
        for query in [
            "SELECT digest,last_used FROM files LIMIT 0",
            "SELECT account,source,digest,metadata FROM refs LIMIT 0",
            "SELECT account,channel,opened_order FROM history_access LIMIT 0",
        ] {
            connection
                .prepare(query)
                .map_err(|_| WorkerError::IntegrityCheckFailed)?;
        }
        return Ok(());
    }
    if version != 0 {
        return Err(WorkerError::IntegrityCheckFailed);
    }
    let tables: i64 = connection.query_row(
        "SELECT count(*) FROM sqlite_schema WHERE type='table'",
        [],
        |row| row.get(0),
    )?;
    if tables != 0 {
        return Err(WorkerError::IntegrityCheckFailed);
    }
    connection.execute_batch(
        "CREATE TABLE files(digest BLOB PRIMARY KEY CHECK(length(digest)=32),last_used INTEGER NOT NULL);
         CREATE TABLE refs(account BLOB NOT NULL,source BLOB NOT NULL,digest BLOB NOT NULL REFERENCES files(digest),metadata BLOB NOT NULL,PRIMARY KEY(account,source));
         CREATE INDEX refs_digest ON refs(digest);
         CREATE INDEX files_lru ON files(last_used,digest);
         CREATE TABLE history_access(account BLOB NOT NULL,channel BLOB NOT NULL,opened_order INTEGER NOT NULL,PRIMARY KEY(account,channel));
         CREATE INDEX history_access_order ON history_access(opened_order);
         PRAGMA user_version=1;"
    )?;
    Ok(())
}

pub(super) fn account_key(account: AccountId) -> Vec<u8> {
    [
        account.server().as_bytes().as_slice(),
        account.user().as_bytes().as_slice(),
    ]
    .concat()
}

pub(super) fn source_key(source: &MediaSource) -> Result<Vec<u8>, MediaError> {
    serde_json::to_vec(source).map_err(|_| MediaError::InvalidData)
}

impl MediaStore {
    pub(crate) async fn invalidate_avatar(
        &self,
        account: AccountId,
        user: crate::Uuid,
    ) -> Result<(), MediaError> {
        let _gate = self.gate.lock().await;
        let account = account_key(account);
        loop {
            let account = account.clone();
            let changed: usize = self
                .writer
                .cache_write(move |tx| -> Result<_, MediaError> {
                    Ok(tx.execute(
                        "DELETE
                     FROM refs
                     WHERE rowid IN (SELECT rowid
                     FROM refs
                     WHERE account=?1
                     AND json_extract(source,'$.UserAvatar.user')=?2
                     LIMIT 100)",
                        params![account, user.to_string()],
                    )?)
                })
                .await?;
            if changed == 0 {
                return Ok(());
            }
        }
    }
    pub(crate) async fn opened(
        &self,
        account: AccountId,
        channel: crate::Uuid,
    ) -> Result<(), MediaError> {
        let account = account_key(account);
        self.writer
            .cache_write(move |tx| -> Result<(), MediaError> {
                tx.execute(
                    "INSERT INTO history_access(account,channel,opened_order)
                     VALUES(?1,?2,(SELECT coalesce(max(opened_order),0)+1
                     FROM history_access))
                     ON CONFLICT(account,channel)
                     DO UPDATE SET opened_order=excluded.opened_order",
                    params![account, channel.as_bytes()],
                )?;
                Ok(())
            })
            .await
    }
    pub(crate) async fn last_opened(
        &self,
        account: AccountId,
        channel: crate::Uuid,
    ) -> Result<i64, MediaError> {
        let account = account_key(account);
        self.writer
            .cache_write(move |tx| -> Result<i64, MediaError> {
                Ok(tx
                    .query_row(
                        "SELECT opened_order FROM history_access WHERE account=?1 AND channel=?2",
                        params![account, channel.as_bytes()],
                        |row| row.get(0),
                    )
                    .optional()?
                    .unwrap_or(0))
            })
            .await
    }
    pub(super) async fn revoke_current(
        &self,
        session: &Session,
        generation: crate::Generation,
        source: &MediaSource,
    ) -> Result<(), MediaError> {
        let account = account_key(session.account());
        let source = source_key(source)?;
        let session = session.clone();
        self.writer
            .cache_write(move |tx| {
                session
                    .inner
                    .epochs
                    .apply(generation, || -> Result<(), MediaError> {
                        tx.execute(
                            "DELETE FROM refs WHERE account=?1 AND source=?2",
                            params![account, source],
                        )?;
                        Ok(())
                    })?
            })
            .await
    }
    pub(super) async fn revoke_family(
        &self,
        session: &Session,
        generation: crate::Generation,
        source: &MediaSource,
    ) -> Result<(), MediaError> {
        let source = match source {
            MediaSource::Range { source, .. } => source.as_ref(),
            source => source,
        };
        let mut sources = vec![source.clone()];
        match source {
            MediaSource::Attachment(id) => sources.push(MediaSource::Thumbnail(*id)),
            MediaSource::Thumbnail(id) => sources.push(MediaSource::Attachment(*id)),
            _ => {}
        }
        let account = account_key(session.account());
        for source in sources {
            let source = source_key(&source)?;
            loop {
                let account = account.clone();
                let source = source.clone();
                let session = session.clone();
                let changed = self
                    .writer
                    .cache_write(move |tx| -> Result<_, MediaError> {
                        session
                            .inner
                            .epochs
                            .apply(generation, || -> Result<_, MediaError> {
                                Ok(tx.execute(
                                    "DELETE FROM refs WHERE rowid IN (
                                SELECT rowid FROM refs WHERE account=?1
                                AND (source=?2 OR
                                     json_extract(source,'$.Range.source')=CAST(?2 AS TEXT))
                                LIMIT 100)",
                                    params![account, source],
                                )?)
                            })?
                    })
                    .await?;
                if changed == 0 {
                    break;
                }
            }
        }
        Ok(())
    }

    pub(super) async fn reference(
        &self,
        account: AccountId,
        source: &MediaSource,
    ) -> Result<Option<Reference>, MediaError> {
        let account = account_key(account);
        let source = source_key(source)?;
        self.writer
            .cache_write(move |tx| {
                let bytes: Option<(Vec<u8>, Vec<u8>)> = tx
                    .query_row(
                        "SELECT refs.digest,metadata
                     FROM refs
                     JOIN files ON files.digest=refs.digest
                     WHERE account=?1
                     AND source=?2",
                        params![account, source],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .optional()?;
                bytes
                    .map(|(digest, bytes)| {
                        let reference: Reference =
                            serde_json::from_slice(&bytes).map_err(|_| MediaError::InvalidData)?;
                        if digest != reference.digest {
                            return Err(MediaError::InvalidData);
                        }
                        Ok(reference)
                    })
                    .transpose()
            })
            .await
    }

    pub(super) async fn grant(
        &self,
        session: Session,
        generation: crate::Generation,
        source: MediaSource,
        reference: Reference,
    ) -> Result<(), MediaError> {
        let account = account_key(session.account());
        let source = source_key(&source)?;
        let metadata = serde_json::to_vec(&reference).map_err(|_| MediaError::InvalidData)?;
        self.writer
            .cache_write(move |tx| {
                session
                    .inner
                    .epochs
                    .apply(generation, || -> Result<(), MediaError> {
                        let order: i64 = tx.query_row(
                            "SELECT coalesce(max(last_used),0)+1 FROM files",
                            [],
                            |row| row.get(0),
                        )?;
                        tx.execute(
                            "INSERT INTO files(digest,last_used)
                     VALUES(?1,?2)
                     ON CONFLICT(digest)
                     DO UPDATE SET last_used=excluded.last_used",
                            params![reference.digest.as_slice(), order],
                        )?;
                        tx.execute(
                            "INSERT INTO refs(account,source,digest,metadata)
                     VALUES(?1,?2,?3,?4)
                     ON CONFLICT(account,source)
                     DO UPDATE SET digest=excluded.digest,metadata=excluded.metadata",
                            params![account, source, reference.digest.as_slice(), metadata],
                        )?;
                        Ok(())
                    })?
            })
            .await
    }

    #[cfg(test)]
    pub(super) async fn touch(&self, digest: [u8; 32]) -> Result<(), MediaError> {
        self.writer
            .cache_write(move |tx| {
                tx.execute(
                    "UPDATE files SET last_used=(SELECT coalesce(max(last_used),0)+1
                     FROM files)
                     WHERE digest=?1",
                    [digest.as_slice()],
                )?;
                Ok(())
            })
            .await
    }
}
