//! Backend attachment spoiler filename prefix convention; content display policy belongs to clients.

/// Standard wire filename prefix marking an uploaded attachment as a spoiler.
pub const SPOILER_FILENAME_PREFIX: &str = "SPOILER_";

/// Returns `true` when `filename` begins with [`SPOILER_FILENAME_PREFIX`].
#[must_use]
pub fn is_spoiler_filename(filename: &str) -> bool {
    filename.starts_with(SPOILER_FILENAME_PREFIX)
}

/// Ensures `filename` carries [`SPOILER_FILENAME_PREFIX`] without double-prefixing.
#[must_use]
pub fn with_spoiler_prefix(filename: &str) -> String {
    if is_spoiler_filename(filename) {
        filename.to_owned()
    } else {
        format!("{SPOILER_FILENAME_PREFIX}{filename}")
    }
}

/// Strips a single leading [`SPOILER_FILENAME_PREFIX`], if present.
#[must_use]
pub fn without_spoiler_prefix(filename: &str) -> &str {
    filename
        .strip_prefix(SPOILER_FILENAME_PREFIX)
        .unwrap_or(filename)
}

#[cfg(test)]
mod tests {
    //! Verifies idempotent prefixing and stripping without filesystem or UI state.
    use super::*;

    #[test]
    fn spoiler_filename_helpers_round_trip_without_double_prefixing() {
        assert!(!is_spoiler_filename("photo.png"));
        assert!(is_spoiler_filename("SPOILER_photo.png"));
        assert_eq!(with_spoiler_prefix("photo.png"), "SPOILER_photo.png");
        assert_eq!(
            with_spoiler_prefix("SPOILER_photo.png"),
            "SPOILER_photo.png"
        );
        assert_eq!(without_spoiler_prefix("SPOILER_photo.png"), "photo.png");
        assert_eq!(without_spoiler_prefix("photo.png"), "photo.png");
    }
}
