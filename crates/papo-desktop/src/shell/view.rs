//! Draws the shell: nothing while starting, why the app cannot work, the
//! server and account screens filling the window, or the app with its popup
//! and the "Connected to" pill. Pure: reads the shell and the system's light
//! or dark preference.

use rune::{Cx, El, HelpLook, col, el, icon, row, text};

use super::outgoing::resend_confirm;
use super::{Shell, Stage};
use crate::design::{
    FOCUS_RING_PT, HEADER_PILL_HEIGHT_PT, HINT_DELAY_S, HINT_GAP_PT, HINT_MAX_WIDTH_PT,
    HINT_PAD_PT, HINT_RADIUS_PT, PANE_EDGE_PT, PILL_ICON_PT, PILL_OUTLINE_PT, Palette,
    RADIUS_CONTROL_PT, RADIUS_PILL_PT, SPACE_LG_PT, SPACE_SM_PT, SPACE_XL_PT, TYPE_BODY, TYPE_HINT,
    Theme,
};
use crate::sdk_bridge::OpenFailure;
use crate::sign_in::{self, Host};
use crate::strings;
use crate::{
    channel_pane, composer, direct_pane, links, menu, notice, rail, remove_server, timeline,
};

/// Draws the shell. Pure: it reads state and the system's light or dark
/// preference, and changes nothing.
pub(crate) fn view(shell: &Shell, cx: &mut Cx<Shell>) -> El {
    let theme = Theme::following_system(cx.system_dark());
    let palette = Palette::new(theme);
    cx.help_look(hint_look(&palette, cx.system_dark()));
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
                let mut app = app(shell, theme, &palette, cx);
                if let Some(server) = connected {
                    app = app.child(connected_pill(server, &palette));
                }
                if let Some(picker) = shell.composer.picker() {
                    app = app.child(composer::picker::view(
                        picker,
                        composer::COMPOSER_PILL_KEY,
                        &palette,
                        cx,
                    ));
                }
                if let Some(notice) = &shell.notice {
                    app = app.child(notice::view(notice, composer::COMPOSER_PILL_KEY, &palette));
                }
                if let Some(menu) = &shell.nav.menu {
                    app = app.children(menu::pointer_point(menu));
                    let body = if menu.card.is_some() {
                        super::profile_card::body(shell, &palette)
                    } else {
                        super::reactions_list::body(shell, &palette)
                    };
                    // A list or card whose conversation went away draws
                    // nothing; the next poll closes it.
                    if body.is_some() || (menu.reactions.is_none() && menu.card.is_none()) {
                        app = app.child(menu::view(menu, body, &palette, cx));
                    }
                }
                let removing = shell
                    .nav
                    .removing
                    .and_then(|id| shell.nav.servers().get(id));
                if let Some(server) = removing {
                    app = app.child(remove_server::confirm(&server.name, &palette));
                }
                if let Some(prompt) = &shell.content.link {
                    app = app.child(links::confirm(prompt, &palette));
                }
                if let Some(resend) = resend_confirm(shell, &palette) {
                    app = app.child(resend);
                }
                let style = timeline_style(shell, theme, &palette);
                if let Some(delete) = super::deleting::delete_confirm(shell, &style) {
                    app = app.child(delete);
                }
                if let Some(flow) = flow {
                    app = app.child(sign_in::popup(flow, &shell.fields, &palette));
                }
                app
            }
        },
    }
}

/// Every hint's pill (`docs/design.md` §9, Links): a link's full address,
/// and when a message was edited. Rune keeps one look per window, so both
/// take the address's mono.
fn hint_look(palette: &Palette, dark: bool) -> HelpLook {
    let (vertical, horizontal) = HINT_PAD_PT;
    HelpLook::system(dark)
        .fill(palette.surface_pill)
        .ink(palette.text_secondary)
        .ring(PILL_OUTLINE_PT, palette.line_pill)
        .radius(HINT_RADIUS_PT)
        .padding(horizontal, vertical)
        .font(TYPE_HINT)
        .max_width(Some(HINT_MAX_WIDTH_PT))
        .gap(HINT_GAP_PT)
        .delay(HINT_DELAY_S)
}

/// How the conversation's rows look, and the message a confirmation
/// shows.
fn timeline_style(shell: &Shell, theme: Theme, palette: &Palette) -> timeline::Style {
    let selected = shell
        .composer
        .shown()
        .and_then(|(_, draft)| draft.mode.target());
    timeline::Style {
        layout: shell.message_layout,
        theme,
        palette: *palette,
        time: shell.local_time.clone(),
        today: shell.today,
        hourglass: shell.prefs.unread_hourglass,
        selected,
        reacting: shell.reacting(),
    }
}

/// The rail, the channel pane and the conversation with the composer pill
/// at its foot.
fn app(shell: &Shell, theme: Theme, palette: &Palette, cx: &Cx<Shell>) -> El {
    let title = shell.title();
    let placeholder = title.placeholder();
    let shown = shell.composer.shown();
    let open = shown.and_then(|(key, _)| shell.tabs.shown().filter(|open| open.key == key));
    let original = shown.and_then(|(_, draft)| {
        let reply = draft.mode.reply()?;
        timeline::replies::reply_text(open, reply.id, reply.original.as_ref())
    });
    let context = composer::PillContext {
        placeholder: &placeholder,
        enter_sends: shell.writing.enter_sends,
        original: original.as_ref(),
        me: shown.map_or(papo_sdk::Uuid::nil(), |(key, _)| key.account.user()),
    };
    let state = shell.picked_dm_access().zip(shell.nav.picked_dm());
    let composer = match (state, &title) {
        (Some((access, picked)), super::title::Title::Direct { name, .. }) => {
            super::direct::state_pill(access, name, picked, palette)
        }
        _ => composer::pill(&shell.composer, &context, palette),
    };
    let style = timeline_style(shell, theme, palette);
    let header = super::pins_panel::header(shell, &title, palette);
    let panel = super::pins_panel::panel(shell, &style, cx);
    let conversation = col()
        .key("conversation")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .fill(palette.surface_content)
        .children(header)
        .child(timeline::view(shell, style))
        .child(
            col()
                .w_full()
                .padding(SPACE_LG_PT)
                .child(composer)
                .children(typing_line(shell, palette)),
        )
        .children(panel);
    let edge = || el().w(PANE_EDGE_PT).h_full().fill(palette.line_subtle);
    row()
        .w_full()
        .h_full()
        .fill(palette.surface_content)
        // A press anywhere no control takes: work outside a header panel.
        .on_background_press(|shell: &mut Shell| {
            shell.worked_outside();
        })
        .child(rail::view(&shell.nav, &shell.icons, palette))
        .child(edge())
        .child(if shell.nav.direct_shown() {
            direct_pane::view(shell, palette)
        } else {
            channel_pane::view(&shell.nav, &shell.icons, palette)
        })
        .child(edge())
        .child(conversation)
        .children(super::profile_pane::pane(shell, palette))
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
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, palette.focus_ring)
        .on_press(Shell::dismiss_connected)
        .child(icon(&crate::icons::X, PILL_ICON_PT).color(palette.text_secondary));
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

/// Who else is typing in the shown conversation, over its composer.
fn typing_line(shell: &Shell, palette: &Palette) -> Option<El> {
    let open = shell.tabs.shown()?;
    let people = shell.people(open.key.account)?;
    crate::typing_line::view(
        people.typing_in(open.key.channel),
        open.resources.as_deref(),
        palette,
    )
}
