//! The main window: how it is opened and what the platform is told about it.
//!
//! This module owns the main window's `WindowOptions` (identity, title,
//! sizes) and the startup log line that records the display backend and the
//! GPU of every run. It does not own what the window shows; that is `shell`.
//!
//! Rules:
//! - The window identity is [`APP_ID`]. It is not the egui client's
//!   `io.github.chriexpe.Papo` until cutover: two clients with one app id are
//!   grouped as one application by the compositor, the taskbar and the
//!   `.desktop` file.
//! - Window chrome (client or server decorations, a custom title bar) is a
//!   design decision that is not made yet, so the platform default applies.

use gpui::{
    App, AppContext as _, Bounds, Pixels, Size, TitlebarOptions, Window, WindowBounds,
    WindowOptions, px, size,
};

use crate::shell::Shell;

/// The Wayland `app_id`, X11 `WM_CLASS` and, later, the `.desktop` file name
/// of this client while it runs beside the egui client.
pub(crate) const APP_ID: &str = "io.github.chriexpe.Papo.Desktop";

/// The window title. "Papo" is a name, so it is not translated.
const TITLE: &str = "Papo";

/// The first window's size in logical pixels, from the egui client
/// (`src/cli.rs`, `with_inner_size`) so both open alike on one screen.
const DEFAULT_SIZE: Size<Pixels> = size(px(1160.), px(740.));

/// The smallest window in logical pixels, from the egui client
/// (`src/cli.rs`, `with_min_inner_size`). Provisional: the shell's layout
/// sets the real value once it exists.
const MIN_SIZE: Size<Pixels> = size(px(360.), px(480.));

/// Opens the main window with the shell as its content.
///
/// # Errors
///
/// Returns GPUI's error when the platform cannot create a window, for
/// example when no display server is reachable or no GPU adapter passes
/// GPUI's configuration test.
pub(crate) fn open_main(cx: &mut App) -> gpui::Result<()> {
    let options = main_options(cx);
    gpui_kit::open_window(options, cx, |window, cx| {
        log_backend(window, cx);
        cx.new(|_| Shell)
    })?;
    Ok(())
}

/// The options of the main window, centred on the primary display.
fn main_options(cx: &App) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            DEFAULT_SIZE,
            cx,
        ))),
        titlebar: Some(TitlebarOptions {
            title: Some(TITLE.into()),
            ..TitlebarOptions::default()
        }),
        app_id: Some(APP_ID.to_owned()),
        window_min_size: Some(MIN_SIZE),
        ..WindowOptions::default()
    }
}

/// Records which display backend and GPU this run uses. Every bug report
/// about drawing, input or composition needs both.
fn log_backend(window: &Window, cx: &App) {
    match window.gpu_specs() {
        Some(gpu) => log::info!(
            "window: display backend {}, GPU {} ({}, {}){}",
            cx.compositor_name(),
            gpu.device_name,
            gpu.driver_name,
            gpu.driver_info,
            if gpu.is_software_emulated {
                ", software emulated"
            } else {
                ""
            },
        ),
        None => log::info!(
            "window: display backend {}, GPU not reported",
            cx.compositor_name()
        ),
    }
}

#[cfg(test)]
mod tests {
    use gpui::{TestAppContext, WindowBounds};

    use super::{APP_ID, DEFAULT_SIZE, MIN_SIZE, main_options};

    #[gpui_kit::test]
    fn main_window_does_not_take_the_egui_client_identity(cx: &mut TestAppContext) {
        let options = cx.update(|cx| main_options(cx));

        assert_eq!(options.app_id.as_deref(), Some(APP_ID));
        assert_ne!(APP_ID, "io.github.chriexpe.Papo");
    }

    #[gpui_kit::test]
    fn main_window_opens_at_the_default_size_and_keeps_the_minimum(cx: &mut TestAppContext) {
        let options = cx.update(|cx| main_options(cx));

        let Some(WindowBounds::Windowed(bounds)) = options.window_bounds else {
            panic!(
                "the main window opens windowed, got {:?}",
                options.window_bounds
            );
        };
        assert_eq!(bounds.size, DEFAULT_SIZE);
        assert_eq!(options.window_min_size, Some(MIN_SIZE));
    }
}
