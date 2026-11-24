//! Profile photo and banner from a staged picture: re-encoded from pixels at the fixed size.

use image::{ExtendedColorType, ImageEncoder};
use papo_sdk::{ImageError, OperationError, RefreshStatus};

use super::{Fixture, Result, setup};

fn photo() -> Vec<u8> {
    let pixels: Vec<u8> = (0..64_u32 * 48 * 3)
        .map(|n| u8::try_from(n % 251).unwrap())
        .collect();
    let mut file = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut file, 90)
        .write_image(&pixels, 64, 48, ExtendedColorType::Rgb8)
        .unwrap();
    file
}

#[test]
fn a_staged_picture_becomes_the_profile_photo_and_banner() -> Result {
    let f = Fixture::new(papo_test_server::Config::default())?;
    f.run(async {
        let (first, _, _) = setup(&f).await?;
        let staged = first
            .outgoing()
            .stage_attachment(std::io::Cursor::new(photo()))
            .await?;
        let receipt = first
            .operations()
            .update_avatar_from_image(&staged, 1, None)
            .await?;
        assert_eq!(receipt.refresh, RefreshStatus::Updated);
        let user = first.account().user();
        let resources = first.subscribe_resources().borrow().clone();
        let profile = resources.profiles.get(user).ok_or("missing profile")?;
        assert_eq!(profile.avatar_format.as_deref(), Some("jpeg"));
        first
            .operations()
            .update_banner_from_image(&staged, 0, None)
            .await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn a_picture_that_cannot_be_prepared_sends_nothing() -> Result {
    let f = Fixture::new(papo_test_server::Config::default())?;
    f.run(async {
        let (first, _, _) = setup(&f).await?;
        let staged = first
            .outgoing()
            .stage_attachment(std::io::Cursor::new(b"%PDF-1.7".to_vec()))
            .await?;
        let refused = first
            .operations()
            .update_avatar_from_image(&staged, 0, None)
            .await;
        assert!(matches!(
            refused,
            Err(OperationError::Image(ImageError::NotAnImage))
        ));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}
