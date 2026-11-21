//! Draws the shell: nothing while starting, why the app cannot work, the
//! server and account screens filling the window, or the app with its popup
//! and the "Connected to" pill. Pure: reads the shell and the system's light
//! or dark preference.

use rune::{Cx, El, col, el, icon, row, text, text_input};

use super::{COMPOSER_KEY, Shell, Stage};
use crate::design::{
    COMPOSER_HEIGHT_PT, FOCUS_RING_PT, HEADER_PILL_HEIGHT_PT, PANE_EDGE_PT, PILL_CROSS_PT,
    PILL_OUTLINE_PT, Palette, RADIUS_COMPOSER_PT, RADIUS_CONTROL_PT, RADIUS_PILL_PT, SPACE_LG_PT,
    SPACE_SM_PT, SPACE_XL_PT, TYPE_BODY, Theme,
};
use crate::sdk_bridge::OpenFailure;
use crate::sign_in::{self, Host};
use crate::strings;
use crate::{channel_pane, menu, rail, remove_server, timeline};

/// Draws the shell. Pure: it reads state and the system's light or dark
/// preference, and changes nothing.
pub(crate) fn view(shell: &Shell, cx: &mut Cx<Shell>) -> El {
    let theme = Theme::following_system(cx.system_dark());
    let palette = Palette::new(theme);
    let surface = col().w_full().h_full().fill(palette.surface_content);
    match &shell.stage {
        Stage::Starting => surface,
        Stage::Unavailable(failure) => {
            let words = match failure {
                OpenFailure::ProfileInUse => strings::startup_profile_in_use(),
                OpenFailure::Failed => strings::startup_open_failed(),
            };
            surface.center().child(
                text(words)
                    .key("unavailable")
                    .font(TYPE_BODY)
                    .color(palette.text_primary)
                    .role("alert"),
            )
        }
        Stage::Running {
            flow, connected, ..
        } => match flow {
            Some(flow) if flow.host() == Host::Window => surface
                .center()
                .padding(SPACE_LG_PT)
                .child(sign_in::screen(flow, &shell.fields, &palette)),
            _ => {
                let mut app = app(shell, theme, &palette);
                if let Some(server) = connected {
                    app = app.child(connected_pill(server, &palette));
                }
                if let Some(menu) = &shell.nav.menu {
                    app = app.child(menu::view(menu, &palette, cx));
                }
                let removing = shell
                    .nav
                    .removing
                    .and_then(|id| shell.nav.servers().get(id));
                if let Some(server) = removing {
                    app = app.child(remove_server::confirm(&server.name, &palette));
                }
                if let Some(flow) = flow {
                    app = app.child(sign_in::popup(flow, &shell.fields, &palette));
                }
                app
            }
        },
    }
}

/// The rail, the channel pane and the conversation with the composer pill
/// at its foot.
fn app(shell: &Shell, theme: Theme, palette: &Palette) -> El {
    let field = text_input(&shell.composer, TYPE_BODY)
        .key(COMPOSER_KEY)
        .label(strings::composer_label())
        .single_line()
        .grow(1.0)
        .color(palette.text_primary)
        .placeholder(strings::composer_placeholder(), palette.text_muted)
        .caret(palette.accent, palette.text_selection)
        .on_edit(|shell: &mut Shell| &mut shell.composer);
    let pill = row()
        .key("composer-pill")
        .w_full()
        .h(COMPOSER_HEIGHT_PT)
        .px(SPACE_LG_PT)
        .center_y()
        .radius(RADIUS_COMPOSER_PT)
        .fill(palette.surface_pill)
        .border(PILL_OUTLINE_PT, palette.line_pill)
        .child(field);
    let style = timeline::Style {
        layout: shell.message_layout,
        theme,
        palette: *palette,
        time: shell.local_time.clone(),
        today: shell.local_time.today(),
    };
    let conversation = col()
        .key("conversation")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .fill(palette.surface_content)
        .child(timeline::view(shell, style))
        .child(col().w_full().padding(SPACE_LG_PT).child(pill));
    let edge = || el().w(PANE_EDGE_PT).h_full().fill(palette.line_subtle);
    row()
        .w_full()
        .h_full()
        .fill(palette.surface_content)
        .child(rail::view(&shell.nav, &shell.icons, palette))
        .child(edge())
        .child(channel_pane::view(&shell.nav, &shell.icons, palette))
        .child(edge())
        .child(conversation)
}

/// "Connected to {server}", floating at the top until dismissed.
fn connected_pill(server: &str, palette: &Palette) -> El {
    let dismiss = row()
        .key("connected-dismiss")
        .size(
            HEADER_PILL_HEIGHT_PT - 2.0 * SPACE_SM_PT,
            HEADER_PILL_HEIGHT_PT - 2.0 * SPACE_SM_PT,
        )
        .center()
        .radius(RADIUS_CONTROL_PT)
        .role("button")
        .label(strings::dismiss())
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, palette.accent)
        .on_press(Shell::dismiss_connected)
        .child(icon(&crate::icons::CROSS, PILL_CROSS_PT).color(palette.text_secondary));
    let pill = row()
        .key("connected")
        .h(HEADER_PILL_HEIGHT_PT)
        .pl(SPACE_XL_PT)
        .pr(SPACE_SM_PT)
        .gap(SPACE_SM_PT)
        .center_y()
        .radius(RADIUS_PILL_PT)
        .fill(palette.surface_pill)
        .border(PILL_OUTLINE_PT, palette.line_pill)
        .role("status")
        .live_region(rune::LiveRegion::Polite)
        .child(
            text(strings::connected_to(server))
                .font(TYPE_BODY)
                .color(palette.text_primary),
        )
        .child(dismiss);
    row()
        .pin(Some(0.0), Some(SPACE_XL_PT), Some(0.0), None)
        .justify_center()
        .child(pill)
}
