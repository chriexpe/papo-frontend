//! The profile photo a profile carries inline: the backend sends it base64 in
//! `UserProfile::avatar_blob`, not as media to fetch. Owns decoding it within the backend's
//! avatar cap; fetching other people's pictures through the media cache is `media`'s.

use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use papo_api::wire::UserProfile;

use crate::FixedImage;

/// Why a profile's inline photo can't be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum InlineAvatarError {
    /// The text is not base64.
    #[error("inline avatar is not base64")]
    NotBase64,
    /// It decodes past the backend's avatar cap.
    #[error("inline avatar is too large")]
    TooLarge,
}

/// `profile`'s photo, decoded from its inline `avatar_blob`: the encoded image bytes (the
/// backend's `avatar_format` says which format). `None` when the profile has no photo.
///
/// The text is measured before it is decoded, so a blob past the backend's avatar cap
/// ([`FixedImage::Avatar`]'s `max_bytes`) costs no allocation.
/// # Errors
/// [`InlineAvatarError::NotBase64`] for text that isn't base64, [`InlineAvatarError::TooLarge`]
/// past the cap.
pub fn inline_avatar(profile: &UserProfile) -> Result<Option<Vec<u8>>, InlineAvatarError> {
    let Some(blob) = profile
        .avatar_blob
        .as_deref()
        .filter(|blob| !blob.is_empty())
    else {
        return Ok(None);
    };
    decode_within(blob, FixedImage::Avatar.max_bytes()).map(Some)
}

/// `blob` decoded, when its bytes fit in `max_bytes`.
fn decode_within(blob: &str, max_bytes: usize) -> Result<Vec<u8>, InlineAvatarError> {
    // Four base64 characters carry three bytes.
    if blob.len().div_ceil(4).saturating_mul(3) > max_bytes.saturating_add(2) {
        return Err(InlineAvatarError::TooLarge);
    }
    let bytes = BASE64
        .decode(blob.trim())
        .map_err(|_| InlineAvatarError::NotBase64)?;
    if bytes.len() > max_bytes {
        return Err(InlineAvatarError::TooLarge);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    //! What a blob decodes to, and what is refused before decoding.
    use super::*;

    fn with_blob(blob: Option<&str>) -> UserProfile {
        UserProfile {
            avatar_blob: blob.map(str::to_owned),
            ..UserProfile::default()
        }
    }

    #[test]
    fn a_blob_decodes_to_its_bytes_and_none_means_no_photo() {
        let png = b"\x89PNG\r\n\x1a\nrest";
        let blob = BASE64.encode(png);
        assert_eq!(
            inline_avatar(&with_blob(Some(&blob))),
            Ok(Some(png.to_vec()))
        );
        assert_eq!(inline_avatar(&with_blob(None)), Ok(None));
        assert_eq!(inline_avatar(&with_blob(Some(""))), Ok(None));
    }

    #[test]
    fn text_that_is_not_base64_is_refused() {
        assert_eq!(
            inline_avatar(&with_blob(Some("not base64 at all!"))),
            Err(InlineAvatarError::NotBase64)
        );
    }

    #[test]
    fn a_blob_past_the_cap_is_refused_before_decoding() {
        let blob = BASE64.encode(vec![0_u8; 11]);
        assert_eq!(decode_within(&blob, 10), Err(InlineAvatarError::TooLarge));
        assert_eq!(
            decode_within(&BASE64.encode(vec![0_u8; 10]), 10).map(|b| b.len()),
            Ok(10)
        );
    }
}
