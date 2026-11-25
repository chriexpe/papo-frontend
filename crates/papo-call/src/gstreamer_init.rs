//! The process's one GStreamer initialisation, shared by calls and message playback: lazy (the
//! first call or the first video, never at startup) and idempotent. Which elements each user
//! needs is that user's to check.

use gstreamer as gst;
use std::{fmt, sync::OnceLock};

/// GStreamer could not start in this process (its library or registry is unusable).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GstreamerUnavailable;

impl fmt::Display for GstreamerUnavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("GStreamer could not be initialised")
    }
}

impl std::error::Error for GstreamerUnavailable {}

/// Initialises GStreamer and this platform's element ranks once per process; later calls return
/// the first outcome at once. Blocks while the registry loads (a first run may scan plugins), so
/// call it off the UI thread.
///
/// # Errors
/// [`GstreamerUnavailable`] when initialisation failed, now or on the first call.
pub fn init_gstreamer() -> Result<(), GstreamerUnavailable> {
    static OUTCOME: OnceLock<Result<(), GstreamerUnavailable>> = OnceLock::new();
    *OUTCOME.get_or_init(|| {
        gst::init().map_err(|_| GstreamerUnavailable)?;
        crate::platform::initialize_platform();
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_init_returns_the_first_outcome() {
        let first = init_gstreamer();
        assert_eq!(init_gstreamer(), first);
        assert!(first.is_ok());
    }
}
