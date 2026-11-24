//! The service's attachment calls: inspecting and opening staged files, and cancelling a
//! message's upload. Staging itself is `file_io` and `pending`; dispatch stops uploads.

use super::{OperationState, OutgoingError, OutgoingService, StagedAttachment, Uuid};

impl OutgoingService {
    /// Reads a staged file's format, displayed size and whether it is animated, without decoding
    /// its pixels. A file that fails here (HEIC, TIFF, RAW, a video, a damaged image) is one that
    /// anonymizing would refuse, so a client can say so when the switch is flipped; the same check
    /// still runs at enqueue.
    /// # Errors
    /// Returns [`OutgoingError::Image`] with the typed reason, or account/storage failures.
    pub async fn inspect_image(
        &self,
        staged: &StagedAttachment,
    ) -> Result<crate::ImageInfo, OutgoingError> {
        let staged = staged.clone();
        self.run(move |this| async move {
            this.ensure().await?;
            this.session
                .client
                .inner
                .registry
                .outgoing_inspect_image(&this.session, &staged)
                .await
        })
        .await
    }
    /// Opens a staged attachment read-only, so a client can draw it (a tray tile after a relaunch,
    /// the editor, an unsent message's media). Decoding for display is the client's.
    ///
    /// The handle reads the immutable staged bytes; drop it once read, since a held handle can
    /// keep account removal from deleting the file on some platforms.
    /// # Errors
    /// Returns [`OutgoingError::NotFound`] for a blob of another account or one already
    /// collected, or storage failures.
    pub async fn open_staged(
        &self,
        staged: &StagedAttachment,
    ) -> Result<std::fs::File, OutgoingError> {
        let staged = staged.clone();
        self.run(move |this| async move {
            this.ensure().await?;
            this.session
                .client
                .inner
                .registry
                .open_staged_file(&this.session, &staged)
                .await
        })
        .await
    }
    /// Cancels a message's attachment upload (the whole message, never one file).
    ///
    /// A queued or preparing message becomes Failed at once (a preparing one keeps copying its
    /// files, so Retry can still send it). One being sent is stopped: if its last attachment
    /// byte had not yet left, the server never had the whole message and it becomes Failed
    /// ([`super::OperationFailure::Cancelled`]); otherwise it may already exist and becomes
    /// Unknown, as for a lost answer. Either way Retry and Discard decide what follows. The
    /// dispatcher records the outcome; this call returns once the stop is requested.
    /// # Errors
    /// Returns [`OutgoingError::InvalidState`] for a message without attachments or one that is
    /// not queued or sending, [`OutgoingError::NotFound`] for an unknown identity.
    pub async fn cancel_upload(&self, local: Uuid) -> Result<(), OutgoingError> {
        self.run(move |this| async move {
            this.ensure().await?;
            let _guard = this.session.inner.outgoing.serial.lock().await;
            let operation = this.operation(local)?;
            let has_files = matches!(
                operation.kind.as_ref(),
                super::OperationKind::Message(message) if !message.attachments.is_empty()
            );
            if !has_files {
                return Err(OutgoingError::InvalidState);
            }
            match operation.state {
                OperationState::Preparing => {
                    if !this.session.inner.outgoing.cancel_preparing(local) {
                        return Err(OutgoingError::InvalidState);
                    }
                    this.publish().await?;
                    Ok(())
                }
                OperationState::Queued => {
                    this.session
                        .client
                        .inner
                        .registry
                        .outgoing_transition(
                            this.session.clone(),
                            local,
                            OperationState::Queued,
                            OperationState::Failed,
                            None,
                            Some(super::OperationFailure::Cancelled),
                        )
                        .await?;
                    this.publish().await?;
                    this.session.inner.outgoing.wake.notify_one();
                    Ok(())
                }
                OperationState::Sending if this.session.inner.outgoing.cancel_in_flight(local) => {
                    Ok(())
                }
                _ => Err(OutgoingError::InvalidState),
            }
        })
        .await
    }
}
