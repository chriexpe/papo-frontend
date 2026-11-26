//! Copy image (`docs/design.md` §14.7, §14.8): a picture's original, decoded off the UI thread
//! with its orientation applied (a GIF's first frame), put on the clipboard as PNG.

use std::io::Cursor;

use papo_sdk::MediaSource;
use rune::ClipboardContent;
use rune::clipboard::ClipboardImage;

use super::Shell;
use crate::message_media::received::MediaAt;

impl Shell {
    /// Copies picture `at` (its whole picture from `source`) as PNG.
    pub(super) fn copy_image(&mut self, at: MediaAt, source: MediaSource) {
        let Some(sdk) = self.sdk.clone() else {
            return;
        };
        let updates = sdk.updates();
        sdk.run(move |client| async move {
            let Ok(session) = client.session(at.account) else {
                return;
            };
            let Ok(data) = session.media().fetch(source, false).await else {
                return;
            };
            let png = tokio::task::spawn_blocking(move || png_of(&data.value.bytes))
                .await
                .ok()
                .flatten();
            let Some(image) = png.and_then(ClipboardImage::from_png) else {
                log::debug!("a picture could not be copied");
                return;
            };
            updates.send_tx(move |_shell, tx| {
                tx.copy_rich(ClipboardContent::new("").with_image(image));
                false
            });
        });
    }
}

/// `encoded` (JPEG, PNG, WebP or GIF) as PNG: its first frame, its orientation applied.
fn png_of(encoded: &[u8]) -> Option<Vec<u8>> {
    use image::ImageDecoder;
    let reader = image::ImageReader::new(Cursor::new(encoded))
        .with_guessed_format()
        .ok()?;
    let mut decoder = reader.into_decoder().ok()?;
    let orientation = decoder.orientation().ok();
    let mut picture = image::DynamicImage::from_decoder(decoder).ok()?;
    if let Some(orientation) = orientation {
        picture.apply_orientation(orientation);
    }
    let mut png = Vec::new();
    picture
        .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
        .ok()?;
    Some(png)
}

#[cfg(test)]
mod tests {
    //! A picture turned into PNG.

    use super::png_of;

    #[test]
    fn a_picture_becomes_png_and_noise_does_not() {
        let mut gif = Vec::new();
        image::RgbaImage::from_pixel(3, 2, image::Rgba([10, 20, 30, 255]))
            .write_to(&mut std::io::Cursor::new(&mut gif), image::ImageFormat::Gif)
            .unwrap();
        let png = png_of(&gif).expect("a PNG");
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        let back = image::load_from_memory(&png).unwrap();
        assert_eq!((back.width(), back.height()), (3, 2));
        assert_eq!(png_of(b"not a picture"), None);
    }
}
