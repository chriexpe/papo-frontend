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
