//! Durable device registry mutations; probing, authentication and observable lists live above it.

use crate::data_root::ProfileLease;
use crate::{
    AccountId, SavedAccount, SavedServer, ServerAddress, ServerId, StorageError, UnixMicros,
};
use crate::{
    secret::CredentialGeneration,
    storage_worker::{DatabaseWriter, ReadPool},
};
use rusqlite::{OptionalExtension, params};
use std::{path::PathBuf, sync::Arc};

/// Cloneable registry access. Successful mutations have completed a FULL commit.
#[derive(Clone)]
pub struct Registry {
    pub(crate) writer: DatabaseWriter,
    pub(crate) readers: Arc<ReadPool>,
    pub(crate) path: PathBuf,
    pub(crate) lease: Arc<ProfileLease>,
}

impl Registry {
    /// Saves a validated address; network probing belongs to session 5.
    ///
    /// # Errors
    /// Returns `AlreadyExists` for an existing or pending-removal identity, or a storage failure.
    pub async fn add_server(
        &self,
        address: ServerAddress,
        added_at: UnixMicros,
    ) -> Result<(), StorageError> {
        self.writer.write(move |tx| {
            let id = address.id();
            if tx.query_row("SELECT 1 FROM servers WHERE id=?", [id.as_bytes()], |_| Ok(())).optional()?.is_some() {
                return Err(StorageError::AlreadyExists);
            }
            tx.execute("INSERT INTO servers(id,canonical_address,position,added_at_us) VALUES(?1,?2,(SELECT coalesce(max(position),-1)+1 FROM servers),?3)", params![id.as_bytes(),address.as_str(),added_at.0])?;
            Ok(())
        }).await
    }

    /// Returns visible servers in persisted order, excluding every pending removal.
    ///
    /// # Errors
    /// Returns a database/read-worker failure or malformed durable data.
    pub async fn servers(&self) -> Result<Vec<SavedServer>, StorageError> {
        let lease = Arc::clone(&self.lease);
        Ok(self.readers.query(self.path.clone(), move |tx| {
            let _lease=lease;
            tx.prepare("SELECT id,canonical_address,added_at_us FROM servers WHERE hidden=0 ORDER BY position,id")?
                .query_map([], |row| {
                    let address = ServerAddress::parse(&row.get::<_,String>(1)?).map_err(|_| rusqlite::Error::InvalidQuery)?;
                    if row.get::<_,Vec<u8>>(0)? != address.id().as_bytes() { return Err(rusqlite::Error::InvalidQuery); }
                    Ok(SavedServer { address, added_at: UnixMicros(row.get(2)?) })
                })?.collect()
        }).await?)
    }

    /// Reorders exactly the visible server set atomically.
    ///
    /// # Errors
    /// Returns `InvalidOrder` for duplicates/missing entries, otherwise a storage failure.
    pub async fn reorder_servers(&self, order: Vec<ServerId>) -> Result<(), StorageError> {
        self.writer
            .write(move |tx| {
                let count: i64 =
                    tx.query_row("SELECT count(*) FROM servers WHERE hidden=0", [], |r| {
                        r.get(0)
                    })?;
                let count = usize::try_from(count).map_err(|_| StorageError::InvalidState)?;
                if count != order.len()
                    || order.iter().collect::<std::collections::HashSet<_>>().len() != count
                {
                    return Err(StorageError::InvalidOrder);
                }
                for (position, id) in order.iter().enumerate() {
                    let position =
                        i64::try_from(position).map_err(|_| StorageError::InvalidOrder)?;
                    if tx.execute(
                        "UPDATE servers SET position=?1 WHERE id=?2 AND hidden=0",
                        params![position, id.as_bytes()],
                    )? != 1
                    {
                        return Err(StorageError::InvalidOrder);
                    }
                }
                Ok(())
            })
            .await
    }

    /// Registers account metadata and allocates a never-reused credential generation.
    /// Setting active clears the previous active account in the same FULL transaction.
    ///
    /// # Errors
    /// Returns `NotFound` for a hidden/missing server, `AlreadyExists`, or a storage failure.
    pub async fn add_account(
        &self,
        id: AccountId,
        display_name: Option<String>,
        avatar_ref: Option<String>,
        active: bool,
        created_at: UnixMicros,
    ) -> Result<CredentialGeneration, StorageError> {
        self.writer.write(move |tx| {
            visible_server(tx, id.server())?;
            if tx.query_row("SELECT 1 FROM accounts WHERE server_id=?1 AND user_id=?2",params![id.server().as_bytes(),id.user().as_bytes()], |_|Ok(())).optional()?.is_some() { return Err(StorageError::AlreadyExists); }
            if active { tx.execute("UPDATE accounts SET active=0 WHERE server_id=?", [id.server().as_bytes()])?; }
            tx.execute("INSERT INTO accounts(server_id,user_id,display_name,avatar_ref,position,active,created_at_us) VALUES(?1,?2,?3,?4,(SELECT coalesce(max(position),-1)+1 FROM accounts WHERE server_id=?1),?5,?6)", params![id.server().as_bytes(),id.user().as_bytes(),display_name,avatar_ref,active,created_at.0])?;
            let generation = u64::try_from(tx.last_insert_rowid()).map_err(|_| StorageError::InvalidState)?;
            Ok(CredentialGeneration::new(generation))
        }).await
    }

    /// Returns only this server's visible accounts in their durable order.
    ///
    /// # Errors
    /// Returns a storage/read-worker failure or malformed durable metadata.
    pub async fn accounts(&self, server: ServerId) -> Result<Vec<SavedAccount>, StorageError> {
        let lease = Arc::clone(&self.lease);
        Ok(self.readers.query(self.path.clone(), move |tx| {
            let _lease=lease;
            tx.prepare("SELECT a.user_id,a.display_name,a.avatar_ref,a.active,a.created_at_us,a.credential_generation FROM accounts a JOIN servers s ON s.id=a.server_id WHERE a.server_id=? AND a.hidden=0 AND s.hidden=0 ORDER BY a.position,a.user_id")?
                .query_map([server.as_bytes()], |row| {
                    let bytes: Vec<u8> = row.get(0)?;
                    let user = papo_api::Uuid::from_slice(&bytes).map_err(|_|rusqlite::Error::InvalidQuery)?;
                    let generation = u64::try_from(row.get::<_,i64>(5)?).map_err(|_|rusqlite::Error::InvalidQuery)?;
                    Ok(SavedAccount { id: AccountId::new(server,user),display_name:row.get(1)?,avatar_ref:row.get(2)?,active:row.get(3)?,created_at:UnixMicros(row.get(4)?),credential_generation:CredentialGeneration::new(generation) })
                })?.collect()
        }).await?)
    }

    /// Selects one registered account or clears this server's active account.
    ///
    /// # Errors
    /// Returns `NotFound` for hidden/missing identities, or a storage failure.
    pub async fn set_active_account(
        &self,
        server: ServerId,
        account: Option<AccountId>,
    ) -> Result<(), StorageError> {
        self.writer
            .write(move |tx| {
                visible_server(tx, server)?;
                if let Some(id) = account {
                    if id.server() != server {
                        return Err(StorageError::NotFound);
                    }
                    visible_account(tx, id)?;
                }
                tx.execute(
                    "UPDATE accounts SET active=0 WHERE server_id=?",
                    [server.as_bytes()],
                )?;
                if let Some(id) = account {
                    tx.execute(
                        "UPDATE accounts SET active=1 WHERE server_id=?1 AND user_id=?2",
                        params![server.as_bytes(), id.user().as_bytes()],
                    )?;
                }
                Ok(())
            })
            .await
    }

    /// Reorders exactly the visible accounts for one visible server.
    ///
    /// # Errors
    /// Returns `InvalidOrder`, `NotFound`, or a storage failure; no partial order commits.
    pub async fn reorder_accounts(
        &self,
        server: ServerId,
        order: Vec<AccountId>,
    ) -> Result<(), StorageError> {
        self.writer.write(move |tx| {
            visible_server(tx,server)?;
            let count:i64 = tx.query_row("SELECT count(*) FROM accounts WHERE server_id=? AND hidden=0",[server.as_bytes()],|r|r.get(0))?;
            let count = usize::try_from(count).map_err(|_|StorageError::InvalidState)?;
            if count!=order.len() || order.iter().collect::<std::collections::HashSet<_>>().len()!=count { return Err(StorageError::InvalidOrder); }
            for (position,id) in order.iter().enumerate() {
                if id.server()!=server { return Err(StorageError::InvalidOrder); }
                let position = i64::try_from(position).map_err(|_|StorageError::InvalidOrder)?;
                if tx.execute("UPDATE accounts SET position=?1 WHERE server_id=?2 AND user_id=?3 AND hidden=0",params![position,server.as_bytes(),id.user().as_bytes()])? != 1 { return Err(StorageError::InvalidOrder); }
            }
            Ok(())
        }).await
    }
}

pub(crate) fn visible_server(
    tx: &rusqlite::Transaction<'_>,
    id: ServerId,
) -> Result<(), StorageError> {
    tx.query_row(
        "SELECT 1 FROM servers WHERE id=? AND hidden=0",
        [id.as_bytes()],
        |_| Ok(()),
    )
    .optional()?
    .ok_or(StorageError::NotFound)
}

pub(crate) fn visible_account(
    tx: &rusqlite::Transaction<'_>,
    id: AccountId,
) -> Result<(), StorageError> {
    tx.query_row("SELECT 1 FROM accounts a JOIN servers s ON s.id=a.server_id WHERE a.server_id=?1 AND a.user_id=?2 AND a.hidden=0 AND s.hidden=0",params![id.server().as_bytes(),id.user().as_bytes()],|_|Ok(())).optional()?.ok_or(StorageError::NotFound)
}
