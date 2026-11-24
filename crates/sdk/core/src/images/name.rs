//! The random file name given to anonymized media, so a photo does not go out as `IMG_2041`.

use super::error::ImageError;
use std::path::Path;

const ALPHABET: &[u8; 36] = b"abcdefghijklmnopqrstuvwxyz0123456789";
/// Characters in the random stem.
pub const NAME_LENGTH: usize = 15;
/// Longest extension kept; anything longer is dropped rather than carried into a name.
const MAX_EXTENSION: usize = 8;

/// A random 15-character name (lowercase letters and digits) keeping `original`'s extension.
///
/// Draws from the operating system's entropy. A name is not a secret, only unremarkable.
/// # Errors
/// Returns [`ImageError::Source`] when the system has no entropy source.
pub fn random_attachment_name(original: &str) -> Result<String, ImageError> {
    let mut entropy = [0_u8; NAME_LENGTH];
    getrandom::fill(&mut entropy).map_err(|_| ImageError::Source(std::io::ErrorKind::Other))?;
    Ok(name_from_entropy(&entropy, original))
}

/// Builds the name from caller-supplied bytes, so tests need no randomness. Each byte picks one
/// character; the bias of `byte % 36` is irrelevant for a non-secret name.
pub(crate) fn name_from_entropy(entropy: &[u8; NAME_LENGTH], original: &str) -> String {
    let mut name: String = entropy
        .iter()
        .map(|byte| char::from(ALPHABET[usize::from(*byte) % ALPHABET.len()]))
        .collect();
    if let Some(extension) = kept_extension(original) {
        name.push('.');
        name.push_str(&extension);
    }
    name
}

/// The original's extension, lowercased, when it is short and plain ASCII alphanumerics.
fn kept_extension(original: &str) -> Option<String> {
    let extension = Path::new(original).extension()?.to_str()?;
    let plain = !extension.is_empty()
        && extension.len() <= MAX_EXTENSION
        && extension.bytes().all(|byte| byte.is_ascii_alphanumeric());
    plain.then(|| extension.to_ascii_lowercase())
}

/// Replaces `original`'s extension with `extension` unless it already names the same format.
pub(crate) fn with_extension(
    original: &str,
    accepts: impl Fn(&str) -> bool,
    extension: &str,
) -> String {
    match kept_extension(original) {
        Some(current) if accepts(&current) => original.to_owned(),
        _ => {
            let stem = Path::new(original)
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or(original);
            format!("{stem}.{extension}")
        }
    }
}

#[cfg(test)]
mod tests {
    //! Names are built from injected entropy; the shape is the contract.

    use super::*;

    #[test]
    fn a_name_is_fifteen_letters_and_digits_with_the_extension_kept() {
        let name = name_from_entropy(
            &[0, 1, 25, 26, 35, 36, 71, 200, 7, 8, 9, 10, 11, 12, 13],
            "IMG_2041.JPG",
        );
        let (stem, extension) = name.split_once('.').unwrap();
        assert_eq!(stem.len(), NAME_LENGTH);
        assert!(
            stem.bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        );
        assert_eq!(extension, "jpg");
    }

    #[test]
    fn nothing_of_the_original_stem_survives() {
        let name = name_from_entropy(&[3; NAME_LENGTH], "holiday-with-ana.png");
        assert!(!name.contains("holiday") && !name.contains("ana"));
    }

    #[test]
    fn a_missing_odd_or_long_extension_is_dropped() {
        assert!(!name_from_entropy(&[1; NAME_LENGTH], "noext").contains('.'));
        assert!(!name_from_entropy(&[1; NAME_LENGTH], "a.tar gz").contains('.'));
        assert!(!name_from_entropy(&[1; NAME_LENGTH], "a.verylongextension").contains('.'));
    }

    #[test]
    fn the_system_source_gives_the_same_shape() {
        let name = random_attachment_name("a.webp").unwrap();
        assert_eq!(name.len(), NAME_LENGTH + ".webp".len());
    }

    #[test]
    fn an_extension_is_replaced_only_when_the_format_changed() {
        let png = |extension: &str| extension == "png";
        assert_eq!(with_extension("shot.png", png, "png"), "shot.png");
        assert_eq!(with_extension("shot.gif", png, "png"), "shot.png");
        assert_eq!(with_extension("shot", png, "png"), "shot.png");
    }
}
