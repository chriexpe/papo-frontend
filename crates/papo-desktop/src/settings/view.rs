//! The settings panel's look (`docs/design.md` §8, The panel): grown from
//! the account pill, its bottom edge one gap above the pill and its other
//! edges one margin (the pill's own inset) from the window, its width
//! capped and centred on a wide window; the nav with the search field and
//! the pages; a page's head (← on a sub-page, the title, its scope, ✕); and
//! its body, or the search's results grouped by page.
//!
//! Owns the panel's layout. The rows are `rows`' and `controls`'.

use std::collections::BTreeSet;

use rune::{El, ScrollSpec, col, el, icon, row, text};

use super::controls::{self, Ctx};
use super::rows::account::AccountFacts;
use super::rows::{self, Context, DiagnosticsFacts, page_title};
use super::search::results;
use super::{PageId, SETTINGS_KEY, SettingsPanel};
use crate::appearance::Appearance;
use crate::conversation_settings::ConversationSettings;
use crate::design::settings::{
    BODY_PADDING_PT, GROUP_GAP, HEAD_BUTTON_PT, HEAD_MIN_HEIGHT_PT, HEAD_PADDING_PT,
    PANEL_MAX_WIDTH_PT, PANEL_RADIUS,
};
use crate::design::{
    FOCUS_RING_PT, Look, PANE_PILL_HEIGHT, RAIL_WIDTH_PT, ROW_ICON_PT, SPACE_MD, SPACE_SM,
    Surfaces, TYPE_CAPTION, TYPE_SMALL, TYPE_TITLE,
};
use crate::general_settings::GeneralSettings;
use crate::icons::{ARROW_LEFT, X};
use crate::shell::Shell;
use crate::strings;

/// What the panel is drawn from.
pub(crate) struct PanelView<'a> {
    pub(crate) panel: &'a SettingsPanel,
    /// The saved settings (the draft is the panel's own).
    pub(crate) saved: &'a Appearance,
    pub(crate) conversations: &'a ConversationSettings,
    pub(crate) general: &'a GeneralSettings,
    pub(crate) trusted: &'a BTreeSet<String>,
    /// Embedded players' switch and allowed sites.
    pub(crate) players: &'a crate::embedded_players::PlayerSettings,
    /// The player sites with data stored.
    pub(crate) sites_with_data: &'a BTreeSet<papo_sdk_ui::PlayerProvider>,
    pub(crate) diagnostics: &'a DiagnosticsFacts,
    /// The panel's account; none when no account is signed in.
    pub(crate) account: Option<&'a AccountFacts>,
    pub(crate) system_dark: bool,
    pub(crate) system_font_known: bool,
    pub(crate) reduced_motion: bool,
    /// The grain's tile for the theme drawn, for Grain's slider.
    pub(crate) grain: Option<&'a rune::Pixels>,
    /// The panel's account's own photo and banner, once fetched.
    pub(crate) pictures: Option<&'a crate::shell::Pictures>,
}

/// The panel over the app, with `preview` over the Profile page's rows
/// (§8: what others see). Escape closes a menu or an edit first (its
/// own), then the search, then the panel (`Shell::settings_key`).
pub(crate) fn view(panel: &PanelView<'_>, preview: Option<El>, look: &Look) -> El {
    let appearance = panel.panel.draft().unwrap_or(panel.saved);
    let context = Context {
        appearance,
        conversations: panel.conversations,
        general: panel.general,
        trusted: panel.trusted,
        players: panel.players,
        sites_with_data: panel.sites_with_data,
        diagnostics: panel.diagnostics,
        account: panel.account,
        system_dark: panel.system_dark,
        system_font_known: panel.system_font_known,
        reduced_motion: panel.reduced_motion,
        pictures: panel.pictures,
    };
    // One margin from the window on three sides, the pill's own inset; the
    // bottom edge one gap above the account pill.
    let margin = RAIL_WIDTH_PT + look.space(SPACE_MD);
    let bottom = look.space(SPACE_MD) * 2.0 + look.space(PANE_PILL_HEIGHT);
    let body = main(panel.panel, &context, panel.grain, preview, look);
    let frame = row()
        .key(SETTINGS_KEY)
        .w_full()
        .max_w(PANEL_MAX_WIDTH_PT)
        .h_full()
        .radius(look.radius(PANEL_RADIUS))
        .panel_surface(look)
        .role("dialog")
        .label(strings::settings_title())
        .modal_tx(true, Shell::dismiss_settings)
        .on_key(|shell: &mut Shell, key: &rune::KeyEvent, tx| shell.settings_key(key, tx))
        .child(super::nav::view(panel.panel, &context, look))
        .child(body);
    // The holder spans the margins and centres the panel in them, so a
    // wide window caps it and a narrow one keeps it at the margin.
    row()
        .pin(Some(margin), Some(margin), Some(margin), Some(bottom))
        .justify_center()
        .child(frame)
}

/// The page or the search results, under their head.
fn main(
    panel: &SettingsPanel,
    context: &Context<'_>,
    grain: Option<&rune::Pixels>,
    preview: Option<El>,
    look: &Look,
) -> El {
    let cx = Ctx {
        look,
        appearance: context.appearance,
        failed: panel.failed_row(),
        colour: panel.colour,
        grain,
        domain: &panel.domain,
        domain_refused: panel.domain_refused,
        editing: panel.editing.as_ref().map(|(row, edit)| (*row, edit)),
        waiting: panel.waiting,
        password: &panel.password,
        pictures: context.pictures,
    };
    let query = panel.query();
    let (title, scope, back, blocks) = if query.is_empty() {
        let page = panel.page();
        let previewed = (page == PageId::Profile).then_some(preview).flatten();
        let blocks: Vec<El> = previewed
            .into_iter()
            .chain(
                rows::page(page, context)
                    .into_iter()
                    .map(|group| controls::group(group, &cx)),
            )
            .collect();
        let scope = match context.account {
            Some(account) if page.of_account() => strings::settings_scope_account(&account.server),
            _ => strings::settings_scope_device().to_owned(),
        };
        (page_title(page).to_owned(), scope, page.parent(), blocks)
    } else {
        (
            strings::settings_search().to_owned(),
            format!("“{query}”"),
            None,
            search_blocks(query, context, &cx),
        )
    };
    let (top, sides, bottom) = BODY_PADDING_PT;
    col()
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .child(head(&title, &scope, back, look))
        .child(
            col()
                .key("settings-body")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(
                    col()
                        .w_full()
                        .pt(top)
                        .px(sides)
                        .pb(bottom)
                        .gap(look.space(GROUP_GAP))
                        .children(blocks),
                ),
        )
}

/// The search's results: each matching row as it works on its page, under
/// "Page › Row"; "No settings found" when nothing matches.
fn search_blocks(query: &str, context: &Context<'_>, cx: &Ctx<'_>) -> Vec<El> {
    let look = cx.look;
    let found = results(query, context);
    if found.is_empty() {
        return vec![
            text(strings::settings_no_results())
                .font(look.font(TYPE_SMALL))
                .color(look.palette.text_muted)
                .role("status"),
        ];
    }
    found
        .into_iter()
        .flat_map(|(page, rows)| {
            rows.into_iter().map(move |row| {
                let path = strings::settings_result_path(page_title(page), &row.title);
                col()
                    .w_full()
                    .gap(look.space(SPACE_SM))
                    .child(
                        el().px(look.space(SPACE_SM)).child(
                            text(path)
                                .font(look.font(TYPE_CAPTION))
                                .color(look.palette.text_secondary),
                        ),
                    )
                    .child(controls::tile(row, true, true, cx))
            })
        })
        .collect()
}

/// A page's head: ← on a sub-page, its title and scope, ✕.
fn head(title: &str, scope: &str, back: Option<PageId>, look: &Look) -> El {
    let [top, right, bottom, left] = HEAD_PADDING_PT;
    let button = |key: &'static str, shape: &'static rune::Icon, label: &'static str| {
        el().key(key)
            .size(HEAD_BUTTON_PT, HEAD_BUTTON_PT)
            .radius(HEAD_BUTTON_PT / 2.0)
            .center()
            .fill(look.palette.surface_tile)
            .role("button")
            .label(label)
            .focusable(true)
            .focus_highlight(look.palette.surface_hover)
            .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
            .child(icon(shape, ROW_ICON_PT).color(look.palette.text_secondary))
    };
    let mut line = row()
        .w_full()
        .min_h(HEAD_MIN_HEIGHT_PT)
        .pt(top)
        .pr(right)
        .pb(bottom)
        .pl(if back.is_some() {
            left - SPACE_MD.0
        } else {
            left
        })
        .gap(look.space(SPACE_MD))
        .center_y();
    if let Some(parent) = back {
        line = line.child(
            button("settings-back", &ARROW_LEFT, strings::settings_back())
                .on_press_tx(move |shell: &mut Shell, tx| shell.settings_page(parent, tx)),
        );
    }
    line.child(
        text(title.to_owned())
            .font(look.font(TYPE_TITLE))
            .color(look.palette.text_primary)
            .role("heading")
            .level(2),
    )
    .child(
        text(scope.to_owned())
            .font(look.font(TYPE_SMALL))
            .color(look.palette.text_muted)
            .grow(1.0)
            .min_w(0.0)
            .max_lines(1),
    )
    .child(
        button("settings-close", &X, strings::settings_close())
            .on_press_tx(|shell: &mut Shell, _tx| shell.close_settings()),
    )
}
