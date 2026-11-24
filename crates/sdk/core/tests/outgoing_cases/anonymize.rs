//! Anonymizing and edits over loopback: what the server receives is the cleaned picture under a
//! random name, a refused file queues nothing, and the draft keeps the original until Send.

use super::*;
use image::{ExtendedColorType, ImageEncoder};
use papo_sdk::{Anonymize, AttachmentEdit, CropRect, ImageError, UnsupportedFormat};
use sha2::{Digest, Sha256};

const SECRET: &[u8] = b"GPS-SECRET-48.8584N-2.2945E";

/// A real JPEG with an EXIF block (orientation upright) and the secret in EXIF, a comment and
/// trailing bytes.
fn photo() -> Vec<u8> {
    let (width, height) = (24_u32, 16_u32);
    let pixels: Vec<u8> = (0..width * height * 3)
        .map(|n| u8::try_from(n % 251).unwrap())
        .collect();
    let mut plain = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut plain, 90)
        .write_image(&pixels, width, height, ExtendedColorType::Rgb8)
        .unwrap();
    let mut exif =
        b"Exif\0\0II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x01\0\0\0\0\0\0\0".to_vec();
    exif.extend_from_slice(SECRET);
    let mut file = plain[..2].to_vec();
    for (marker, payload) in [(0xE1_u8, exif), (0xFE, SECRET.to_vec())] {
        file.extend_from_slice(&[0xFF, marker]);
        file.extend_from_slice(&u16::try_from(payload.len() + 2).unwrap().to_be_bytes());
        file.extend_from_slice(&payload);
    }
    file.extend_from_slice(&plain[2..]);
    file.extend_from_slice(SECRET);
    file
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn anonymous() -> AttachmentEdit {
    AttachmentEdit {
        anonymize: Anonymize::Required,
        ..AttachmentEdit::default()
    }
}

fn message_with(attachment: OutgoingAttachment) -> OperationKind {
    OperationKind::Message(OutgoingMessage {
        attachments: vec![attachment],
        ..Default::default()
    })
}

#[test]
fn an_anonymized_photo_reaches_the_server_without_metadata_under_a_random_name() -> Result {
    let f = Fixture::new(Config::default())?;
    f.run(async {
        let (session, channel) = signed_in(&f).await?;
        let original = photo();
        session
            .outgoing()
            .enqueue(
                channel,
                id(1),
                message_with(
                    OutgoingAttachment::from_bytes("IMG_2041.JPG", original.clone())
                        .with_edit(anonymous()),
                ),
                UnixMicros(0),
            )
            .await?;
        state(&session, id(1), OperationState::Sent).await?;
        let upload = f.server().take_received_upload().await.ok_or("no upload")?;
        let name = &upload.files[0].filename;
        let (stem, extension) = name.split_once('.').ok_or("no extension")?;
        assert_eq!(stem.len(), papo_sdk::NAME_LENGTH);
        assert!(
            stem.bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        );
        assert_eq!(extension, "jpg");
        let body = f.server().take_message_body().await.ok_or("no body")?;
        assert!(
            !contains(&body, SECRET) && !contains(&body, b"IMG_2041") && !contains(&body, b"Exif")
        );
        // The size and digest the server saw are the cleaned file's, not the original's.
        assert!(upload.files[0].size_bytes < original.len() as u64);
        assert_ne!(
            upload.files[0].sha256,
            <[u8; 32]>::from(Sha256::digest(&original))
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn a_random_name_alone_renames_any_file_and_leaves_its_bytes_alone() -> Result {
    let f = Fixture::new(Config::default())?;
    f.run(async {
        let (session, channel) = signed_in(&f).await?;
        let original = b"%PDF-1.7 not an image".to_vec();
        let edit = AttachmentEdit {
            random_name: true,
            ..AttachmentEdit::default()
        };
        session
            .outgoing()
            .enqueue(
                channel,
                id(1),
                message_with(
                    OutgoingAttachment::from_bytes("tax-return-2026.pdf", original.clone())
                        .with_edit(edit),
                ),
                UnixMicros(0),
            )
            .await?;
        state(&session, id(1), OperationState::Sent).await?;
        let upload = f.server().take_received_upload().await.ok_or("no upload")?;
        let (stem, extension) = upload.files[0]
            .filename
            .split_once('.')
            .ok_or("no extension")?;
        assert_eq!(stem.len(), papo_sdk::NAME_LENGTH);
        assert_eq!(extension, "pdf");
        assert_eq!(
            upload.files[0].sha256,
            <[u8; 32]>::from(Sha256::digest(&original))
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn with_no_edit_the_bytes_and_name_go_unchanged() -> Result {
    let f = Fixture::new(Config::default())?;
    f.run(async {
        let (session, channel) = signed_in(&f).await?;
        let original = photo();
        session
            .outgoing()
            .enqueue(
                channel,
                id(1),
                message_with(OutgoingAttachment::from_bytes(
                    "IMG_2041.JPG",
                    original.clone(),
                )),
                UnixMicros(0),
            )
            .await?;
        state(&session, id(1), OperationState::Sent).await?;
        let upload = f.server().take_received_upload().await.ok_or("no upload")?;
        assert_eq!(upload.files[0].filename, "IMG_2041.JPG");
        assert_eq!(upload.files[0].size_bytes, original.len() as u64);
        assert_eq!(
            upload.files[0].sha256,
            <[u8; 32]>::from(Sha256::digest(&original))
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn a_file_that_cannot_be_cleaned_is_refused_and_nothing_is_queued() -> Result {
    let f = Fixture::new(Config::default())?;
    f.run(async {
        let (session, channel) = signed_in(&f).await?;
        let queue = session.outgoing();
        let heic = [b"\0\0\0\x18ftypheic".as_slice(), &[0; 64]].concat();
        let refused = queue
            .enqueue(
                channel,
                id(1),
                message_with(
                    OutgoingAttachment::from_bytes("IMG_1.HEIC", heic).with_edit(anonymous()),
                ),
                UnixMicros(0),
            )
            .await;
        assert!(matches!(
            refused,
            Err(OutgoingError::Image(ImageError::Unsupported(
                UnsupportedFormat::Heic
            )))
        ));
        let video = [b"\0\0\0\x18ftypisom".as_slice(), &[0; 64]].concat();
        let refused = queue
            .enqueue(
                channel,
                id(2),
                message_with(
                    OutgoingAttachment::from_bytes("clip.mp4", video).with_edit(anonymous()),
                ),
                UnixMicros(0),
            )
            .await;
        assert!(matches!(
            refused,
            Err(OutgoingError::Image(ImageError::NotAnImage))
        ));
        assert!(queue.subscribe().await?.borrow().operations.is_empty());
        assert!(f.server().take_received_upload().await.is_none());
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn a_draft_keeps_the_original_and_its_edit_until_the_message_is_sent() -> Result {
    let f = Fixture::new(Config::default())?;
    f.run(async {
        let (session, channel) = signed_in(&f).await?;
        let queue = session.outgoing();
        let original = photo();
        let edit = AttachmentEdit {
            anonymize: Anonymize::Required,
            random_name: false,
            quarter_turns: 1,
            crop: Some(CropRect {
                x: 0,
                y: 0,
                width: 8,
                height: 8,
            }),
        };
        let revision = queue
            .save_draft(
                channel,
                DraftBody {
                    message: OutgoingMessage {
                        attachments: vec![
                            OutgoingAttachment::from_bytes("IMG_2041.jpg", original.clone())
                                .with_edit(edit),
                        ],
                        ..Default::default()
                    },
                    ..Default::default()
                },
                UnixMicros(0),
                None,
            )
            .await?;
        let draft = queue.draft(channel).await?.ok_or("no draft")?;
        let kept = &draft.body.message.attachments[0];
        assert_eq!(*kept.edit(), edit);
        assert_eq!(kept.filename, "IMG_2041.jpg");
        assert_eq!(kept.size_bytes(), original.len() as u64);
        queue
            .enqueue_draft(channel, id(1), revision, UnixMicros(0))
            .await?;
        state(&session, id(1), OperationState::Sent).await?;
        let upload = f.server().take_received_upload().await.ok_or("no upload")?;
        assert!(upload.files[0].size_bytes < original.len() as u64);
        assert_ne!(upload.files[0].filename, "IMG_2041.jpg");
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn a_staged_image_can_be_inspected_before_sending() -> Result {
    let f = Fixture::new(Config::default())?;
    f.run(async {
        let (session, _) = signed_in(&f).await?;
        let queue = session.outgoing();
        let staged = queue
            .stage_attachment(std::io::Cursor::new(photo()))
            .await?;
        let info = queue.inspect_image(&staged).await?;
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut queue.open_staged(&staged).await?, &mut bytes)?;
        assert_eq!(bytes, photo());
        assert_eq!((info.width, info.height, info.animated), (24, 16, false));
        let not_image = queue
            .stage_attachment(std::io::Cursor::new(b"%PDF-1.7".to_vec()))
            .await?;
        assert!(matches!(
            queue.inspect_image(&not_image).await,
            Err(OutgoingError::Image(ImageError::NotAnImage))
        ));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

fn where_supported() -> AttachmentEdit {
    AttachmentEdit {
        anonymize: Anonymize::WhereSupported,
        ..AttachmentEdit::default()
    }
}

#[test]
fn anonymizing_where_supported_cleans_a_photo_and_renames_it() -> Result {
    let f = Fixture::new(Config::default())?;
    f.run(async {
        let (session, channel) = signed_in(&f).await?;
        session
            .outgoing()
            .enqueue(
                channel,
                id(1),
                message_with(
                    OutgoingAttachment::from_bytes("IMG_2041.JPG", photo())
                        .with_edit(where_supported()),
                ),
                UnixMicros(0),
            )
            .await?;
        state(&session, id(1), OperationState::Sent).await?;
        let upload = f.server().take_received_upload().await.ok_or("no upload")?;
        assert_ne!(upload.files[0].filename, "IMG_2041.JPG");
        let body = f.server().take_message_body().await.ok_or("no body")?;
        assert!(!contains(&body, SECRET) && !contains(&body, b"Exif"));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn anonymizing_where_supported_sends_a_document_unchanged_under_its_name() -> Result {
    let f = Fixture::new(Config::default())?;
    f.run(async {
        let (session, channel) = signed_in(&f).await?;
        let original = b"%PDF-1.7 the minutes".to_vec();
        session
            .outgoing()
            .enqueue(
                channel,
                id(1),
                message_with(
                    OutgoingAttachment::from_bytes("minutes.pdf", original.clone())
                        .with_edit(where_supported()),
                ),
                UnixMicros(0),
            )
            .await?;
        state(&session, id(1), OperationState::Sent).await?;
        let upload = f.server().take_received_upload().await.ok_or("no upload")?;
        assert_eq!(upload.files[0].filename, "minutes.pdf");
        assert_eq!(
            upload.files[0].sha256,
            <[u8; 32]>::from(Sha256::digest(&original))
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn anonymizing_where_supported_still_refuses_an_image_it_cannot_clean() -> Result {
    let f = Fixture::new(Config::default())?;
    f.run(async {
        let (session, channel) = signed_in(&f).await?;
        let queue = session.outgoing();
        let heic = [b"\0\0\0\x18ftypheic".as_slice(), &[0; 64]].concat();
        let refused = queue
            .enqueue(
                channel,
                id(1),
                message_with(
                    OutgoingAttachment::from_bytes("IMG_1.HEIC", heic).with_edit(where_supported()),
                ),
                UnixMicros(0),
            )
            .await;
        assert!(matches!(
            refused,
            Err(OutgoingError::Image(ImageError::Unsupported(
                UnsupportedFormat::Heic
            )))
        ));
        assert!(queue.subscribe().await?.borrow().operations.is_empty());
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn anonymizing_where_supported_still_gives_a_document_its_random_name() -> Result {
    let f = Fixture::new(Config::default())?;
    f.run(async {
        let (session, channel) = signed_in(&f).await?;
        let original = b"%PDF-1.7 the minutes".to_vec();
        let edit = AttachmentEdit {
            random_name: true,
            ..where_supported()
        };
        session
            .outgoing()
            .enqueue(
                channel,
                id(1),
                message_with(
                    OutgoingAttachment::from_bytes("minutes.pdf", original.clone()).with_edit(edit),
                ),
                UnixMicros(0),
            )
            .await?;
        state(&session, id(1), OperationState::Sent).await?;
        let upload = f.server().take_received_upload().await.ok_or("no upload")?;
        assert_ne!(upload.files[0].filename, "minutes.pdf");
        assert_eq!(
            upload.files[0]
                .filename
                .rsplit_once('.')
                .map(|(_, extension)| extension),
            Some("pdf")
        );
        assert_eq!(
            upload.files[0].sha256,
            <[u8; 32]>::from(Sha256::digest(&original))
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}
