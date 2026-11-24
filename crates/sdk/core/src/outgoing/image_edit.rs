//! Applies an attachment's anonymize, turn and crop when its message is enqueued, inspects staged
//! images and prepares fixed-size pictures. The bytes are read from the staged original and the
//! result is staged beside it, through the same bounded slots, digest and fsync as any copy.

use super::{
    ClientError, OutgoingAttachment, OutgoingAttachmentContent, OutgoingError, OutgoingMessage,
    StagedAttachment, Uuid,
    files::{self, Reference},
    persistence::data_root,
};
use crate::{
    AttachmentEdit, CropRect, FixedImage, ImageFormat, ImageInfo, Registry, Session,
    images::{self, FixedUpload},
    random_attachment_name,
};
use papo_api::wire::EmbedInput;
use std::io::{BufReader, Cursor};

/// Attempts at a random name that is not already used by another file of the same message.
const NAME_ATTEMPTS: usize = 8;

impl Registry {
    /// Replaces every edited attachment of `message` with its cleaned or re-encoded copy, renames
    /// it as the edit asks and clears the edit. A failure leaves nothing half applied: the caller
    /// has not committed anything yet.
    pub(super) async fn outgoing_apply_edits(
        &self,
        session: &Session,
        message: &mut OutgoingMessage,
        identity: Uuid,
    ) -> Result<(), OutgoingError> {
        for index in 0..message.attachments.len() {
            if message.attachments[index].edit().is_identity() {
                continue;
            }
            let edit = message.attachments[index].take_edit();
            // A random name alone leaves the bytes exactly as they are.
            let mut written = None;
            if edit.rewrites_bytes() {
                let original = self
                    .staged_original(session, &mut message.attachments[index], identity)
                    .await?;
                let (edited, format) = self.transform_staged(session, &original, edit).await?;
                message.attachments[index].set_staged(edited);
                written = Some(format);
            }
            let previous = message.attachments[index].filename.clone();
            let renamed = renamed(&previous, edit, written, &message.attachments, index)?;
            rename_embed_references(&mut message.embeds, &previous, &renamed);
            message.attachments[index].filename = renamed;
        }
        Ok(())
    }

    /// The attachment's bytes as a staged blob, staging inline bytes first.
    async fn staged_original(
        &self,
        session: &Session,
        attachment: &mut OutgoingAttachment,
        identity: Uuid,
    ) -> Result<StagedAttachment, OutgoingError> {
        match attachment.take_content() {
            OutgoingAttachmentContent::Staged(staged) => {
                if staged.account() != session.account() {
                    return Err(OutgoingError::NotFound);
                }
                Ok(staged)
            }
            OutgoingAttachmentContent::Bytes(bytes) => {
                self.stage_reader(session, Cursor::new(bytes), identity)
                    .await
            }
            OutgoingAttachmentContent::Released => Err(OutgoingError::InvalidInput),
        }
    }

    /// Streams the staged original through the image path into a new staged blob.
    async fn transform_staged(
        &self,
        session: &Session,
        original: &StagedAttachment,
        edit: AttachmentEdit,
    ) -> Result<(StagedAttachment, ImageFormat), OutgoingError> {
        let guard = self
            .file_guard(session, session.client.inner.outgoing_files.clone())
            .await?;
        let token = session.inner.tasks.token();
        if session.is_closed() {
            return Err(ClientError::Closed.into());
        }
        let root = data_root(self)?;
        let session = session.clone();
        let lease = self.lease.clone();
        let account = session.account();
        let source = Reference {
            digest: *original.digest(),
            size_bytes: original.size_bytes(),
        };
        tokio::task::spawn_blocking(move || {
            let (_guard, _token, _lease) = (guard, token, lease);
            let mut reader = BufReader::new(files::open_for_upload(&root, account, source)?);
            let mut written = None;
            let reference = files::stage_stream(
                &root,
                account,
                || session.is_closed(),
                || {},
                |sink| {
                    written = Some(images::apply(&mut reader, sink, &edit)?);
                    Ok(())
                },
            )?;
            let format = written.ok_or(OutgoingError::InvalidState)?;
            let pin = session
                .inner
                .outgoing
                .pin_staged(files::digest_hex(&reference.digest));
            Ok((
                StagedAttachment::with_pin(account, reference.digest, reference.size_bytes, pin),
                format,
            ))
        })
        .await
        .map_err(|_| OutgoingError::InvalidData)?
    }

    /// Reads a staged file's format, displayed size and animation without decoding its pixels.
    pub(super) async fn outgoing_inspect_image(
        &self,
        session: &Session,
        staged: &StagedAttachment,
    ) -> Result<ImageInfo, OutgoingError> {
        if staged.account() != session.account() {
            return Err(OutgoingError::NotFound);
        }
        let source = Reference {
            digest: *staged.digest(),
            size_bytes: staged.size_bytes(),
        };
        self.read_staged(session, source, images::inspect).await
    }

    /// Re-encodes a staged picture for a fixed-size upload (profile photo or banner).
    pub(crate) async fn outgoing_fixed_image(
        &self,
        session: &Session,
        staged: &StagedAttachment,
        target: FixedImage,
        quarter_turns: u8,
        crop: Option<CropRect>,
    ) -> Result<FixedUpload, OutgoingError> {
        if staged.account() != session.account() {
            return Err(OutgoingError::NotFound);
        }
        let source = Reference {
            digest: *staged.digest(),
            size_bytes: staged.size_bytes(),
        };
        self.read_staged(session, source, move |reader| {
            images::fixed(reader, target, quarter_turns, crop)
        })
        .await
    }

    /// Runs `work` over a staged file on a tracked blocking task, never on a Tokio worker.
    async fn read_staged<T: Send + 'static>(
        &self,
        session: &Session,
        source: Reference,
        work: impl FnOnce(&mut BufReader<std::fs::File>) -> Result<T, crate::ImageError>
        + Send
        + 'static,
    ) -> Result<T, OutgoingError> {
        let guard = self
            .file_guard(session, session.client.inner.outgoing_files.clone())
            .await?;
        let token = session.inner.tasks.token();
        if session.is_closed() {
            return Err(ClientError::Closed.into());
        }
        let root = data_root(self)?;
        let session = session.clone();
        let lease = self.lease.clone();
        let account = session.account();
        tokio::task::spawn_blocking(move || {
            let (_guard, _token, _lease) = (guard, token, lease);
            if session.is_closed() {
                return Err(OutgoingError::Session(ClientError::Closed));
            }
            let mut reader = BufReader::new(files::open_for_upload(&root, account, source)?);
            work(&mut reader).map_err(OutgoingError::Image)
        })
        .await
        .map_err(|_| OutgoingError::InvalidData)?
    }
}

/// The name the edited file goes out under: random when asked (or anonymizing), with the
/// extension of the format actually written when the bytes were re-encoded.
fn renamed(
    previous: &str,
    edit: AttachmentEdit,
    written: Option<ImageFormat>,
    attachments: &[OutgoingAttachment],
    index: usize,
) -> Result<String, OutgoingError> {
    let fit = |name: String| match written {
        Some(format) => images::with_extension(
            &name,
            |extension| format.accepts_extension(extension),
            format.extension(),
        ),
        None => name,
    };
    if !(edit.anonymize || edit.random_name) {
        return Ok(fit(previous.to_owned()));
    }
    let taken = |name: &str| {
        attachments
            .iter()
            .enumerate()
            .any(|(other, attachment)| other != index && attachment.filename == name)
    };
    for _ in 0..NAME_ATTEMPTS {
        let candidate = fit(random_attachment_name(previous)?);
        if !taken(&candidate) {
            return Ok(candidate);
        }
    }
    Err(OutgoingError::InvalidInput)
}

/// An embed names its attachment by file name; a rename must keep that link.
fn rename_embed_references(embeds: &mut [EmbedInput], previous: &str, renamed: &str) {
    for embed in embeds {
        for media in [&mut embed.thumbnail, &mut embed.image, &mut embed.video]
            .into_iter()
            .flatten()
        {
            if media.attachment_name.as_deref() == Some(previous) {
                media.attachment_name = Some(renamed.to_owned());
            }
        }
    }
}
