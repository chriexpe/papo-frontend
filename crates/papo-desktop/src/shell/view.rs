//! Draws the shell: nothing while starting, why the app cannot work, the
//! server and account screens filling the window, or the app with its popup
//! and the "Connected to" pill. Pure: reads the shell and the system's light
//! or dark preference.

use rune::{Cx, El, HelpLook, col, el, icon, row, text};

use super::outgoing::resend_confirm;
use super::{Shell, Stage};
use crate::design::{
    COMPOSER_HEIGHT, FOCUS_RING_PT, HEADER_PILL_HEIGHT, HINT_DELAY_S, HINT_GAP_PT,
    HINT_MAX_WIDTH_PT, HINT_PAD_PT, HINT_RADIUS, Look, PANE_EDGE_PT, PILL_ICON_PT, PILL_OUTLINE_PT,
    RADIUS_CONTROL, RADIUS_PILL, SPACE_LG, SPACE_SM, SPACE_XL, Surfaces, TYPE_BODY, TYPE_HINT,
};
use crate::sdk_bridge::OpenFailure;
use crate::sign_in::{self, Host};
use crate::strings;
use crate::{
    channel_pane, composer, direct_pane, links, menu, notice, rail, remove_server, timeline,
};

/// The window's root element, whose size the grain is tiled to.
const ROOT_KEY: &str = "shell-root";

/// Draws the shell. Pure: it reads state and the system's light or dark
/// preference, and changes nothing.
pub(crate) fn view(shell: &Shell, cx: &mut Cx<Shell>) -> El {
    let look = shell.look(cx.system_dark());
    let grain = shell.grain.as_ref().and_then(|tiles| {
        crate::grain::overlay(
            tiles,
            shell.shown_appearance().grain,
            look.theme().is_dark(),
            cx.layout_size(ROOT_KEY),
            cx.env().scale,
        )
    });
    // The grain lies over everything (`docs/design.md` §4), taking no
    // input.
    col()
        .key(ROOT_KEY)
        .w_full()
        .h_full()
        .child(body(shell, &look, cx))
        .children(grain)
}

/// The shell itself: nothing while starting, why the app cannot work, the
/// first-run screens, or the app with its overlays.
fn body(shell: &Shell, look: &Look, cx: &mut Cx<Shell>) -> El {
    let look = *look;
    cx.help_look(hint_look(&look, look.theme().is_dark()));
    let surface = col().w_full().h_full().fill(look.palette.surface_content);
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
                    .font(look.font(TYPE_BODY))
                    .color(look.palette.text_primary)
                    .role("alert"),
            )
        }
        Stage::Running {
            flow, connected, ..
        } => match flow {
            Some(flow) if flow.host() == Host::Window => surface
                .center()
                .padding(look.space(SPACE_LG))
                .child(sign_in::screen(flow, &shell.fields, &look)),
            _ => {
                let mut app = app(shell, &look, cx);
                if let Some(server) = connected {
                    app = app.child(connected_pill(server, &look));
                }
                if let Some(picker) = shell.composer.picker() {
                    app = app.child(composer::picker::view(
                        picker,
                        composer::COMPOSER_PILL_KEY,
                        &look,
                        cx,
                    ));
                }
                if let Some(notice) = &shell.notice {
                    app = app.child(notice::view(notice, composer::COMPOSER_PILL_KEY, &look));
                }
                // Under the menu: a choice's menu opens over the panel.
                if let Some(panel) = &shell.settings {
                    let view = crate::settings::view::PanelView {
                        panel,
                        saved: shell.appearance.appearance(),
                        system_dark: cx.system_dark(),
                        system_font_known: shell.system_font.is_some(),
                        grain: shell
                            .grain
                            .as_ref()
                            .map(|tiles| tiles.pixels(look.theme().is_dark())),
                    };
                    app = app.child(crate::settings::view(&view, &look));
                }
                if let Some(menu) = &shell.nav.menu {
                    app = app.children(menu::pointer_point(menu));
                    let body = if menu.people {
                        direct_pane::new_message::body(shell, &look)
                    } else if menu.card.is_some() {
                        super::profile_card::body(shell, &look)
                    } else {
                        super::reactions_list::body(shell, &look)
                    };
                    // A list or card whose conversation went away draws
                    // nothing; the next poll closes it.
                    let bodiless = menu.reactions.is_none() && menu.card.is_none() && !menu.people;
                    if body.is_some() || bodiless {
                        app = app.child(menu::view(menu, body, &look, cx));
                    }
                }
                let removing = shell
                    .nav
                    .removing
                    .and_then(|id| shell.nav.servers().get(id));
                if let Some(server) = removing {
                    app = app.child(remove_server::confirm(&server.name, &look));
                }
                if let Some(jump) = super::jump_view::view(shell, &look) {
                    app = app.child(jump);
                }
                if shell.shortcuts_open {
                    app = app.child(crate::shortcuts_sheet::view(&look));
                }
                if let Some(prompt) = &shell.content.link {
                    app = app.child(links::confirm(prompt, &look));
                }
                if let Some(resend) = resend_confirm(shell, &look) {
                    app = app.child(resend);
                }
                let style = timeline_style(shell, &look, [0.0, 0.0]);
                if let Some(delete) = super::deleting::delete_confirm(shell, &style) {
                    app = app.child(delete);
                }
                if let Some(flow) = flow {
                    app = app.child(sign_in::popup(flow, &shell.fields, &look));
                }
                app
            }
        },
    }
}

/// Every hint's pill (`docs/design.md` §9, Links): a link's full address,
/// and when a message was edited. Rune keeps one look per window, so both
/// take the address's mono.
fn hint_look(look: &Look, dark: bool) -> HelpLook {
    let (vertical, horizontal) = HINT_PAD_PT;
    HelpLook::system(dark)
        .fill(look.palette.surface_pill)
        .ink(look.palette.text_secondary)
        .ring(PILL_OUTLINE_PT, look.palette.line_pill)
        .radius(look.radius(HINT_RADIUS))
        .padding(horizontal, vertical)
        .font(look.font(TYPE_HINT))
        .max_width(Some(HINT_MAX_WIDTH_PT))
        .gap(HINT_GAP_PT)
        .delay(HINT_DELAY_S)
}

/// How the conversation's rows look, and the message a confirmation
/// shows.
fn timeline_style(shell: &Shell, look: &Look, insets_pt: [f32; 2]) -> timeline::Style {
    let selected = shell
        .composer
        .shown()
        .and_then(|(_, draft)| draft.mode.target());
    timeline::Style {
        layout: shell.appearance.appearance().messages,
        look: *look,
        time: shell.local_time.clone(),
        today: shell.today,
        hourglass: shell.prefs.unread_hourglass,
        selected,
        reacting: shell.reacting(),
        insets_pt,
    }
}

/// The header row's and the composer area's keys: they float over the
/// rows, which keep their heights as room at their ends.
const HEADER_KEY: &str = "conversation-header";
const FOOT_KEY: &str = "conversation-foot";

/// How far the header pills and the composer area reach over the
/// conversation, from the last frame's layout; before the first, their
/// heights at rest.
fn floating_insets(cx: &Cx<Shell>, look: &Look) -> [f32; 2] {
    let header = cx.layout_size(HEADER_KEY).map_or_else(
        || look.space(SPACE_LG) + look.space(HEADER_PILL_HEIGHT),
        |[_, height]| height,
    );
    let foot = cx.layout_size(FOOT_KEY).map_or_else(
        || look.space(SPACE_LG) * 2.0 + look.space(COMPOSER_HEIGHT),
        |[_, height]| height,
    );
    [header, foot]
}

/// The rail, the channel pane and the conversation with the composer pill
/// at its foot.
fn app(shell: &Shell, look: &Look, cx: &Cx<Shell>) -> El {
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
            super::direct::state_pill(access, name, picked, look)
        }
        _ => composer::pill(&shell.composer, &context, look),
    };
    let style = timeline_style(shell, look, floating_insets(cx, look));
    let header = super::pins_panel::header(shell, &title, look);
    let panel = super::pins_panel::panel(shell, &style, cx)
        .or_else(|| super::search_panel::panel(shell, &title, &style, cx));
    // The rows fill the conversation and scroll under the header pills and
    // the composer, which float over them (`docs/design.md` §5, Glass).
    let foot = col()
        .key(FOOT_KEY)
        .pin(Some(0.0), None, Some(0.0), Some(0.0))
        // A taller composer, a reply bar or the typing line covers more rows.
        .on_layout_tx(|shell: &mut Shell, rect, tx| {
            shell.floating_moved(super::reading::Floating::Foot, rect[3]);
            tx.live_only();
        })
        .padding(look.space(SPACE_LG))
        .child(composer)
        .children(typing_line(shell, look));
    let header = header.map(|header| {
        header
            .pin(Some(0.0), Some(0.0), Some(0.0), None)
            .on_layout_tx(|shell: &mut Shell, rect, tx| {
                shell.floating_moved(super::reading::Floating::Header, rect[3]);
                tx.live_only();
            })
    });
    let conversation = col()
        .key("conversation")
        // A landmark, so F6 reaches it (`docs/design.md` §12).
        .role("main")
        .label(title.name())
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .fill(look.palette.surface_content)
        .child(timeline::view(shell, style))
        .children(header)
        .child(foot)
        .children(panel);
    let edge = || el().w(PANE_EDGE_PT).h_full().fill(look.palette.line_subtle);
    row()
        .w_full()
        .h_full()
        .fill(look.palette.surface_content)
        // A press anywhere no control takes: work outside a header panel.
        .on_background_press(|shell: &mut Shell| {
            shell.worked_outside();
        })
        .child(rail::view(&shell.nav, &shell.icons, look))
        .child(edge())
        .child(if shell.nav.direct_shown() {
            direct_pane::view(shell, look)
        } else {
            channel_pane::view(&shell.nav, &shell.icons, look)
        })
        .child(edge())
        .child(conversation)
        // A DM's profile pane, past the same hairline as the channel pane.
        .children(
            super::profile_pane::pane(shell, look)
                .into_iter()
                .flat_map(|pane| [edge(), pane]),
        )
}

/// "Connected to {server}", floating at the top until dismissed.
fn connected_pill(server: &str, look: &Look) -> El {
    let dismiss = row()
        .key("connected-dismiss")
        .size(
            look.space(HEADER_PILL_HEIGHT) - 2.0 * look.space(SPACE_SM),
            look.space(HEADER_PILL_HEIGHT) - 2.0 * look.space(SPACE_SM),
        )
        .center()
        .radius(look.radius(RADIUS_CONTROL))
        .role("button")
        .label(strings::dismiss())
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .on_press(Shell::dismiss_connected)
        .child(icon(&crate::icons::X, PILL_ICON_PT).color(look.palette.text_secondary));
    let pill = row()
        .key("connected")
        .h(look.space(HEADER_PILL_HEIGHT))
        .pl(look.space(SPACE_XL))
        .pr(look.space(SPACE_SM))
        .gap(look.space(SPACE_SM))
        .center_y()
        .radius(look.radius(RADIUS_PILL))
        .pill_surface(look)
        .role("status")
        .live_region(rune::LiveRegion::Polite)
        .child(
            text(strings::connected_to(server))
                .font(look.font(TYPE_BODY))
                .color(look.palette.text_primary),
        )
        .child(dismiss);
    row()
        .pin(Some(0.0), Some(look.space(SPACE_XL)), Some(0.0), None)
        .justify_center()
        .child(pill)
}

/// Who else is typing in the shown conversation, over its composer.
fn typing_line(shell: &Shell, look: &Look) -> Option<El> {
    let open = shell.tabs.shown()?;
    let people = shell.people(open.key.account)?;
    crate::typing_line::view(
        people.typing_in(open.key.channel),
        open.resources.as_deref(),
        look,
    )
}
