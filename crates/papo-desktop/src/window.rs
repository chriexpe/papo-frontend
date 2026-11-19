//! The main window's identity and geometry.

use rune::AppSpec;

use crate::shell::{self, Shell};
use crate::strings;

/// The application id the desktop sees (Wayland `app_id`, X11 `WM_CLASS`).
/// It names the `.desktop` file, the icon and the Flatpak.
pub(crate) const APP_ID: &str = "io.github.chriexpe.Papo";

/// The window's size when it first opens, in points.
const INITIAL_SIZE_PT: (f32, f32) = (1000.0, 700.0);
/// The smallest window, in points: 360 wide leaves room for the collapsed
/// server pill.
const MIN_SIZE_PT: (f32, f32) = (360.0, 400.0);

/// The main window.
pub(crate) fn spec() -> AppSpec<Shell> {
    AppSpec {
        min_size: Some(MIN_SIZE_PT),
        ..AppSpec::new(
            strings::window_title(),
            INITIAL_SIZE_PT,
            Shell::default,
            shell::view,
        )
    }
}
