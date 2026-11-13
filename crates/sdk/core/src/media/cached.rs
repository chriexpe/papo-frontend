//! Bounded cache-hit admission and post-read authorization checks; hashing runs outside media exclusion.

use super::{MediaData, MediaError, MediaSource, MediaStore, index::Reference};
use crate::{Session, generation::Epoch};
use std::sync::Arc;

impl MediaStore {
    pub(super) async fn cached_reference(
        &self,
        session: &Session,
        epoch: &Epoch,
        source: &MediaSource,
        repair: bool,
    ) -> Result<Option<(Reference, MediaData)>, MediaError> {
        // Bound allocations before acquiring either exclusion or a file pin.
        let permit = Arc::clone(&self.reads)
            .acquire_owned()
            .await
            .map_err(|_| MediaError::TaskStopped)?;
        let (reference, pin) = {
            let _gate = self.gate.lock().await;
            session.inner.epochs.validate(epoch.generation)?;
            let Some(reference) = self.reference(session.account(), source).await? else {
                return Ok(None);
            };
            let pin = self.pins.pin(reference.digest);
            (reference, pin)
        };
        let result = self.read_reference(reference.clone(), pin, permit).await;
        let _gate = self.gate.lock().await;
        session.inner.epochs.validate(epoch.generation)?;
        if self.reference(session.account(), source).await?.as_ref() != Some(&reference) {
            return Err(MediaError::Unavailable);
        }
        match result {
            Ok((data, _permit)) => {
                let owner = Arc::clone(&session.inner);
                let generation = epoch.generation;
                let digest = reference.digest;
                let sql = "UPDATE files SET last_used=(SELECT coalesce(max(last_used),0)+1
                           FROM files) WHERE digest=?1";
                self.writer
                    .cache_write(move |tx| {
                        owner
                            .epochs
                            .apply(generation, || -> Result<(), MediaError> {
                                tx.execute(sql, [digest.as_slice()])?;
                                Ok(())
                            })?
                    })
                    .await?;
                session.inner.epochs.validate(epoch.generation)?;
                Ok(Some((reference, data)))
            }
            Err(MediaError::Unavailable | MediaError::InvalidData | MediaError::TooLarge)
                if repair =>
            {
                self.revoke_current(session, epoch.generation, source)
                    .await?;
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }
}
