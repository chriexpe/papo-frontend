//! Removal journal persistence; external cleanup is executed by the step runner.

use super::{RemovalFailureKind, RemovalJob, RemovalJobs, RemovalStep, RemovalTarget};
use crate::secret::{CredentialGeneration, SecretKey};
use crate::{AccountId, ServerAddress, StorageError};
use rusqlite::{OptionalExtension, params};

impl RemovalJobs {
    /// Returns unfinished jobs and their last persistent failure, in recovery order.
    ///
    /// # Errors
    /// Returns a read failure or malformed journal data.
    pub async fn pending(&self) -> Result<Vec<RemovalJob>, StorageError> {
        let lease = std::sync::Arc::clone(&self.registry.lease);
        Ok(self.registry.readers.query(self.registry.path.clone(), move |tx| {
            let _lease=lease;
            tx.prepare("SELECT j.id,s.canonical_address,j.user_id,j.step,j.failure,a.credential_generation,s.id FROM removal_jobs j JOIN servers s ON s.id=j.server_id LEFT JOIN accounts a ON a.server_id=j.server_id AND a.user_id=j.user_id ORDER BY j.user_id IS NULL,j.id")?
                .query_map([], |row| {
                    let server = ServerAddress::parse(&row.get::<_,String>(1)?).map_err(|_|rusqlite::Error::InvalidQuery)?.id();
                    if row.get::<_,Vec<u8>>(6)? != server.as_bytes() { return Err(rusqlite::Error::InvalidQuery); }
                    let user:Option<Vec<u8>> = row.get(2)?;
                    let (target,key) = if let Some(user) = user {
                        let account = AccountId::new(server,papo_api::Uuid::from_slice(&user).map_err(|_|rusqlite::Error::InvalidQuery)?);
                        let generation = u64::try_from(row.get::<_,i64>(5)?).map_err(|_|rusqlite::Error::InvalidQuery)?;
                        (RemovalTarget::Account(account),Some(SecretKey::new(account,CredentialGeneration::new(generation))))
                    } else { (RemovalTarget::Server(server),None) };
                    Ok(RemovalJob {id:row.get(0)?,target,key,step:step(row.get(3)?)?,failure:row.get::<_,Option<i64>>(4)?.map(failure).transpose()?})
                })?.collect()
        }).await?)
    }

    pub(super) async fn mark(
        &self,
        id: i64,
        step: RemovalStep,
        failure: Option<RemovalFailureKind>,
    ) -> Result<(), StorageError> {
        self.registry
            .writer
            .write(move |tx| {
                tx.execute(
                    "UPDATE removal_jobs SET step=?1,failure=?2 WHERE id=?3",
                    params![step as i64, failure.map(|f| f as i64), id],
                )?;
                Ok(())
            })
            .await
    }

    pub(super) async fn finalize(&self, job: RemovalJob) -> Result<(), StorageError> {
        self.registry.writer.write(move |tx| {
            match job.target {
                RemovalTarget::Account(account) => {
                    tx.execute("DELETE FROM accounts WHERE server_id=?1 AND user_id=?2",params![account.server().as_bytes(),account.user().as_bytes()])?;
                    tx.execute("DELETE FROM removal_jobs WHERE id=?",[job.id])?;
                }
                RemovalTarget::Server(server) => {
                    let pending = tx.query_row("SELECT 1 FROM removal_jobs WHERE server_id=? AND user_id IS NOT NULL LIMIT 1",[server.as_bytes()],|_|Ok(())).optional()?.is_some();
                    if !pending {
                        tx.execute("DELETE FROM removal_jobs WHERE id=?",[job.id])?;
                        tx.execute("DELETE FROM servers WHERE id=?",[server.as_bytes()])?;
                    }
                }
            }
            Ok(())
        }).await
    }
}

fn step(value: i64) -> rusqlite::Result<RemovalStep> {
    match value {
        0 => Ok(RemovalStep::StopSession),
        1 => Ok(RemovalStep::Secrets),
        2 => Ok(RemovalStep::Cache),
        3 => Ok(RemovalStep::MediaRefs),
        4 => Ok(RemovalStep::StateRows),
        5 => Ok(RemovalStep::Finalize),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

fn failure(value: i64) -> rusqlite::Result<RemovalFailureKind> {
    match value {
        1 => Ok(RemovalFailureKind::SessionStop),
        2 => Ok(RemovalFailureKind::SecretTimeout),
        3 => Ok(RemovalFailureKind::Canceled),
        4 => Ok(RemovalFailureKind::SecretStore),
        5 => Ok(RemovalFailureKind::SecretUncertain),
        6 => Ok(RemovalFailureKind::CredentialState),
        7 => Ok(RemovalFailureKind::CacheIo),
        8 => Ok(RemovalFailureKind::MediaRefs),
        9 => Ok(RemovalFailureKind::FileTask),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}
