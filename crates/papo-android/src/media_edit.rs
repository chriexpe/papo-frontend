//! Local media editing shares the same image pipeline as egui. Kotlin owns
//! gestures and previews; core owns cropping, animation and upload budgets.
use crate::bindings::CoreError;
use papo_core::media_prepare;
use std::path::Path;

#[derive(Clone, Copy, Debug, uniffi::Enum)]
pub enum MediaEditPurpose {
    Avatar,
    ServerIcon,
    Banner,
    Sticker,
    Attachment,
}
impl MediaEditPurpose {
    fn limits(self) -> (u32, u32, usize) {
        match self {
            Self::Avatar | Self::ServerIcon => (512, 512, 2 * 1024 * 1024),
            Self::Banner => (1536, 512, 2 * 1024 * 1024),
            Self::Sticker => (512, 512, 256 * 1024),
            Self::Attachment => (u16::MAX.into(), u16::MAX.into(), 32 * 1024 * 1024),
        }
    }
}
#[derive(Clone, Copy, Debug, uniffi::Record)]
pub struct MediaCrop {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct MediaImageInfo {
    pub width: u32,
    pub height: u32,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct PreparedMediaImage {
    pub blob: String,
    pub format: String,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
    pub shrunk: bool,
}
#[uniffi::export]
pub fn inspect_media_image(path: String) -> Result<MediaImageInfo, CoreError> {
    let info = media_prepare::inspect_source(Path::new(&path))?;
    Ok(MediaImageInfo {
        width: info.width,
        height: info.height,
    })
}
#[uniffi::export]
pub fn prepare_media_image(
    path: String,
    crop: MediaCrop,
    purpose: MediaEditPurpose,
) -> Result<PreparedMediaImage, CoreError> {
    let (width, height, bytes) = purpose.limits();
    let output = media_prepare::fit_cropped(
        Path::new(&path),
        Some(media_prepare::Crop {
            x: crop.x,
            y: crop.y,
            width: crop.width,
            height: crop.height,
        }),
        width,
        height,
        bytes,
    )?;
    Ok(PreparedMediaImage {
        blob: output.blob,
        format: output.format.into(),
        width: output.width,
        height: output.height,
        bytes: output.bytes as u64,
        shrunk: output.shrunk,
    })
}

/// Semantic failures are translated by the Android presentation layer.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum AttachmentImageError {
    #[error("unavailable")]
    Unavailable,
    #[error("unsupported")]
    Unsupported,
    #[error("too_large")]
    TooLarge,
    #[error("invalid_image")]
    InvalidImage,
}

#[uniffi::export]
pub fn strip_attachment_metadata(path: String) -> Result<PreparedMediaImage, AttachmentImageError> {
    use base64::Engine as _;
    use std::io::Read as _;
    let file = std::fs::File::open(path).map_err(|_| AttachmentImageError::Unavailable)?;
    let mut raw = Vec::new();
    file.take(32 * 1024 * 1024 + 1)
        .read_to_end(&mut raw)
        .map_err(|_| AttachmentImageError::Unavailable)?;
    let (bytes, format) =
        media_prepare::strip_image_metadata(&raw).map_err(|error| match error {
            media_prepare::ImagePrivacyError::Unsupported => AttachmentImageError::Unsupported,
            media_prepare::ImagePrivacyError::TooLarge => AttachmentImageError::TooLarge,
            media_prepare::ImagePrivacyError::InvalidImage => AttachmentImageError::InvalidImage,
        })?;
    let info = media_prepare::inspect_image_bytes(&bytes)
        .map_err(|_| AttachmentImageError::InvalidImage)?;
    Ok(PreparedMediaImage {
        blob: base64::engine::general_purpose::STANDARD.encode(&bytes),
        format: format!("{format:?}").to_uppercase(),
        width: info.width,
        height: info.height,
        bytes: bytes.len() as u64,
        shrunk: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn purposes_follow_the_real_backend_budgets() {
        assert_eq!(MediaEditPurpose::Sticker.limits(), (512, 512, 262144));
        assert_eq!(
            MediaEditPurpose::Avatar.limits(),
            MediaEditPurpose::ServerIcon.limits()
        );
        assert_eq!(MediaEditPurpose::Banner.limits(), (1536, 512, 2097152));
    }
}
