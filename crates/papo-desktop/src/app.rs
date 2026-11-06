//! The application's lifetime: how it starts, what it binds, when it ends.
//!
//! This module owns the GPUI application, the app-wide actions with their
//! key bindings, and the rule for when the process exits. It opens the main
//! window through `window` and leaves everything inside a window to views.
//!
//! Rules:
//! - `gpui_kit::init` runs before anything else touches the Kit.
//! - The process quits when its last window closes. Close-to-tray will
//!   change this rule (`docs/papo-desktop.md`, phase 6); until then nothing
//!   runs without a window.
//! - A failure to open the main window is logged and ends the process. It
//!   never panics.

use gpui::{App, KeyBinding, QuitMode, actions};

use crate::{session, window};

actions!(
    papo,
    [
        /// Quits the application from any window.
        Quit
    ]
);

/// The key binding for [`Quit`]. `secondary` is Cmd on macOS and Ctrl on
/// Linux and Windows. Only `KeyBinding::load` resolves it, against the host
/// OS: `KeyBinding::new` panics on it (gpui-fast 0.1.13, `keymap/binding.rs`).
const QUIT_KEYSTROKE: &str = "secondary-q";

/// Runs the GPUI application on the current thread until it quits.
pub(crate) fn run() {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(|cx| {
            gpui_kit::init(cx);
            install(cx);
            if let Err(error) = window::open_main(cx) {
                log::error!("app: the main window did not open: {error:#}");
                cx.quit();
            }
        });
}

/// Registers the app-wide actions, their key bindings and the exit rule.
pub(crate) fn install(cx: &mut App) {
    // Otherwise GPUI auto-quits after our last-window observer starts the
    // asynchronous shutdown (gpui-fast 0.1.13, App::update_window).
    cx.set_quit_mode(QuitMode::Explicit);
    session::install(cx);
    match KeyBinding::load(QUIT_KEYSTROKE, Box::new(Quit), None, false, None, cx) {
        Ok(binding) => cx.bind_keys([binding]),
        Err(error) => log::error!("app: the key binding {QUIT_KEYSTROKE} is invalid: {error}"),
    }
    cx.on_action(|_: &Quit, cx| session::quit_after_shutdown(cx));
    cx.on_window_closed(|cx, _| {
        if cx.windows().is_empty() {
            session::quit_after_shutdown(cx);
        }
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;

    use super::{Quit, install};

    #[gpui_kit::test]
    fn quit_has_one_key_binding(cx: &mut TestAppContext) {
        cx.update(install);

        let bindings = cx.update(|cx| {
            cx.key_bindings()
                .borrow()
                .bindings_for_action(&Quit)
                .count()
        });

        assert_eq!(bindings, 1);
    }
}
