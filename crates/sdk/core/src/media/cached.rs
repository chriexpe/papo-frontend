//! Bounded cache-hit admission and post-read authorization checks; hashing runs outside media exclusion.

use super::{
    MediaData, MediaError, MediaSource, MediaStore, files::ReadToWriterResult, index::Reference,
};
use crate::{Generation, Session, generation::Epoch};
use std::{io::Write, sync::Arc};

pub(super) enum CachedWrite<W> {
    Hit(Reference, u64),
    Miss(W),
}

impl MediaStore {
    pub(super) async fn verify_reference_metadata(
        &self,
        session: &Session,
        epoch: &Epoch,
        source: &MediaSource,
        repair: bool,
    ) -> Result<Option<Reference>, MediaError> {
        let Some((reference, pin, permit)) = self.acquire_pinned(session, epoch, source).await?
        else {
            return Ok(None);
        };
        let result = self
            .verify_reference_file(reference.clone(), pin, permit)
            .await;
        let _gate = self.gate.lock().await;
        session.inner.epochs.validate(epoch.generation)?;
        if self.reference(session.account(), source).await?.as_ref() != Some(&reference) {
            return Err(MediaError::Unavailable);
        }
        match result {
            Ok(_permit) => {
                self.touch_last_used(session, epoch.generation, reference.digest)
                    .await?;
                Ok(Some(reference))
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

    pub(super) async fn cached_reference_to_writer<W: Write + Send + 'static>(
        &self,
        session: &Session,
        epoch: &Epoch,
        source: &MediaSource,
        repair: bool,
        writer: W,
    ) -> Result<CachedWrite<W>, MediaError> {
        let Some((reference, pin, permit)) = self.acquire_pinned(session, epoch, source).await?
        else {
            return Ok(CachedWrite::Miss(writer));
        };
        let result = self
            .read_reference_to_writer(reference.clone(), pin, permit, writer)
            .await?;
        let _gate = self.gate.lock().await;
        session.inner.epochs.validate(epoch.generation)?;
        if self.reference(session.account(), source).await?.as_ref() != Some(&reference) {
            return Err(MediaError::Unavailable);
        }
        match result {
            ReadToWriterResult::Written(size_bytes, _permit) => {
                self.touch_last_used(session, epoch.generation, reference.digest)
                    .await?;
                Ok(CachedWrite::Hit(reference, size_bytes))
            }
            ReadToWriterResult::RecoverableError(
                MediaError::Unavailable | MediaError::InvalidData | MediaError::TooLarge,
                writer,
            ) if repair => {
                self.revoke_current(session, epoch.generation, source)
                    .await?;
                Ok(CachedWrite::Miss(writer))
            }
            ReadToWriterResult::RecoverableError(error, _) => Err(error),
        }
    }

    pub(super) async fn cached_reference(
        &self,
        session: &Session,
        epoch: &Epoch,
        source: &MediaSource,
        repair: bool,
    ) -> Result<Option<(Reference, MediaData)>, MediaError> {
        let Some((reference, pin, permit)) = self.acquire_pinned(session, epoch, source).await?
        else {
            return Ok(None);
        };
        let result = self.read_reference(reference.clone(), pin, permit).await;
        let _gate = self.gate.lock().await;
        session.inner.epochs.validate(epoch.generation)?;
        if self.reference(session.account(), source).await?.as_ref() != Some(&reference) {
            return Err(MediaError::Unavailable);
        }
        match result {
            Ok((data, _permit)) => {
                self.touch_last_used(session, epoch.generation, reference.digest)
                    .await?;
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

    async fn acquire_pinned(
        &self,
        session: &Session,
        epoch: &Epoch,
        source: &MediaSource,
    ) -> Result<
        Option<(
            Reference,
            super::pins::ReadPin,
            tokio::sync::OwnedSemaphorePermit,
        )>,
        MediaError,
    > {
        // Bound allocations before acquiring either exclusion or a file pin.
        let permit = Arc::clone(&self.reads)
            .acquire_owned()
            .await
            .map_err(|_| MediaError::TaskStopped)?;
        let _gate = self.gate.lock().await;
        session.inner.epochs.validate(epoch.generation)?;
        let Some(reference) = self.reference(session.account(), source).await? else {
            return Ok(None);
        };
        let pin = self.pins.pin(reference.digest);
        Ok(Some((reference, pin, permit)))
    }

    async fn touch_last_used(
        &self,
        session: &Session,
        generation: Generation,
        digest: [u8; 32],
    ) -> Result<(), MediaError> {
        let owner = Arc::clone(&session.inner);
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
        session.inner.epochs.validate(generation)?;
        Ok(())
    }
}
