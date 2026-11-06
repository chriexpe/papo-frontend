//! The shell: the root view of the main window.
//!
//! The shell will own the window's layout: which panes exist, where they
//! sit and which one has focus. Today it only fills the window with the
//! theme's background, so the bootstrap can be run, tested and measured
//! before any screen exists.
//!
//! Rules:
//! - The shell holds layout state only. Account, server and channel state
//!   comes from `papo-core` through the projection layer; the shell never
//!   copies it.
//! - Colours come from the theme (`cx.theme()`), never from literals here.
//!   Papo's own tokens will replace the Kit's defaults.

use gpui::{Context, InteractiveElement as _, IntoElement, Render, Styled as _, Window, div};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::ActiveTheme as _;

/// The root view of the main window.
pub(crate) struct Shell;

impl Render for Shell {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .id("shell")
            .test_support()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
    }
}

#[cfg(test)]
mod tests {
    use gpui::{
        AppContext as _, Bounds, Point, TestAppContext, WindowBounds, WindowOptions, px, size,
    };
    use gpui_kit::test::TestWindowExt as _;

    use super::Shell;

    #[gpui_kit::test]
    fn shell_fills_the_window(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let window_size = size(px(800.), px(600.));
        let (window, _) = cx
            .update(|cx| {
                gpui_kit::open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds {
                            origin: Point::default(),
                            size: window_size,
                        })),
                        ..WindowOptions::default()
                    },
                    cx,
                    |_, cx| cx.new(|_| Shell),
                )
            })
            .unwrap();

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(window.find("shell").bounds().size, window_size);
        })
        .unwrap();
    }
}
