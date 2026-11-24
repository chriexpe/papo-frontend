//! The settings panel's look (`docs/design.md` §8, The panel): grown from
//! the account pill, its bottom edge one gap above the pill and its other
//! edges one margin (the pill's own inset) from the window, its width
//! capped and centred on a wide window; the nav with the search field and
//! the pages; a page's head (← on a sub-page, the title, its scope, ✕); and
//! its body, or the search's results grouped by page.
//!
//! Owns the panel's layout. The rows are `rows`' and `controls`'.

use rune::{El, ScrollSpec, col, el, icon, row, text, text_input};

use super::controls::{self, Ctx};
use super::rows::{self, Context, page_title};
use super::search::results;
use super::{PageId, SEARCH_KEY, SETTINGS_KEY, SettingsPanel};
use crate::appearance::Appearance;
use crate::design::settings::{
    BODY_PADDING_PT, GROUP_GAP, HEAD_BUTTON_PT, HEAD_MIN_HEIGHT_PT, HEAD_PADDING_PT,
    NAV_ICON_CIRCLE_PT, NAV_PADDING, NAV_ROW_INSET_PT, NAV_ROW_PADDING, NAV_WIDTH_PT,
    PANEL_MAX_WIDTH_PT, PANEL_RADIUS, SEARCH_HEIGHT,
};
use crate::design::{
    FIELD_OUTLINE_PT, FOCUS_RING_PT, Look, PANE_EDGE_PT, PANE_PILL_HEIGHT, PILL_ICON_PT,
    RAIL_WIDTH_PT, ROW_ICON_PT, SPACE_MD, SPACE_SM, SPACE_XS, Surfaces, TYPE_BODY,
    TYPE_BODY_MEDIUM, TYPE_CAPTION, TYPE_CAPTION_MEDIUM, TYPE_SMALL, TYPE_TITLE,
};
use crate::icons::{ARROW_LEFT, PALETTE, SEARCH, X};
use crate::shell::Shell;
use crate::strings;

/// What the panel is drawn from.
pub(crate) struct PanelView<'a> {
    pub(crate) panel: &'a SettingsPanel,
    /// The saved settings (the draft is the panel's own).
    pub(crate) saved: &'a Appearance,
    pub(crate) system_dark: bool,
    pub(crate) system_font_known: bool,
    /// The grain's tile for the theme drawn, for Grain's slider.
    pub(crate) grain: Option<&'a rune::Pixels>,
}

/// The panel over the app. Escape closes a menu first (its own), then the
/// search, then the panel (`Shell::settings_escape`).
pub(crate) fn view(panel: &PanelView<'_>, look: &Look) -> El {
    let appearance = panel.panel.draft().unwrap_or(panel.saved);
    let context = Context {
        appearance,
        system_dark: panel.system_dark,
        system_font_known: panel.system_font_known,
    };
    // One margin from the window on three sides, the pill's own inset; the
    // bottom edge one gap above the account pill.
    let margin = RAIL_WIDTH_PT + look.space(SPACE_MD);
    let bottom = look.space(SPACE_MD) * 2.0 + look.space(PANE_PILL_HEIGHT);
    let body = main(panel.panel, &context, panel.grain, look);
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
        .child(nav(panel.panel, appearance, panel.system_dark, look))
        .child(body);
    // The holder spans the margins and centres the panel in them, so a
    // wide window caps it and a narrow one keeps it at the margin.
    row()
        .pin(Some(margin), Some(margin), Some(margin), Some(bottom))
        .justify_center()
        .child(frame)
}

/// The nav: the search field, then the pages under "App".
fn nav(panel: &SettingsPanel, appearance: &Appearance, system_dark: bool, look: &Look) -> El {
    let current = panel.query().is_empty().then(|| panel.page().nav());
    let appearance_row = nav_row(
        PageId::Appearance,
        &rows::appearance_summary(appearance, system_dark),
        current == Some(PageId::Appearance),
        look,
    );
    col()
        .w(NAV_WIDTH_PT)
        .h_full()
        .padding(look.space(NAV_PADDING))
        .gap(look.space(SPACE_XS))
        .border_sides([0.0, PANE_EDGE_PT, 0.0, 0.0], look.palette.line_subtle)
        .role("navigation")
        .label(strings::settings_title())
        .child(search_field(panel, look))
        .child(
            el().px(NAV_ROW_INSET_PT)
                .pt(look.space(SPACE_MD))
                .pb(look.space(SPACE_SM))
                .child(
                    text(strings::settings_group_app().to_uppercase())
                        .font(look.font(TYPE_CAPTION_MEDIUM))
                        .color(look.palette.text_muted),
                ),
        )
        .child(appearance_row)
}

/// The search field: the search icon, the field, and ✕ while it holds text.
fn search_field(panel: &SettingsPanel, look: &Look) -> El {
    // Concentric with the panel, whose edge is the nav's padding out.
    let field_radius = Look::nested_radius(look.radius(PANEL_RADIUS), look.space(NAV_PADDING));
    let clear_side = HEAD_BUTTON_PT - SPACE_XS.0 * 2.0;
    let mut field = row()
        .w_full()
        .h(look.space(SEARCH_HEIGHT))
        .px(look.space(SPACE_MD) + SPACE_XS.0)
        .gap(look.space(SPACE_SM))
        .center_y()
        .radius(field_radius)
        .fill(look.palette.surface_tile)
        .border(FIELD_OUTLINE_PT, look.palette.line_pill)
        .role("search")
        .child(icon(&SEARCH, ROW_ICON_PT).color(look.palette.accent))
        .child(
            text_input(&panel.search, look.font(TYPE_BODY))
                .key(SEARCH_KEY)
                .grow(1.0)
                .min_w(0.0)
                .color(look.palette.text_primary)
                .placeholder(strings::settings_search(), look.palette.text_muted)
                .caret(look.palette.accent, look.palette.text_selection)
                .label(strings::settings_search())
                .autofocus()
                .on_edit(Shell::settings_search_mut)
                .on_input_key(|shell: &mut Shell, key: &rune::KeyEvent, tx| {
                    shell.settings_key(key, tx)
                }),
        );
    if !panel.query().is_empty() {
        field = field.child(
            el().key("settings-search-clear")
                .size(clear_side, clear_side)
                .center()
                // Concentric with the field, centred in its height.
                .radius(Look::nested_radius(
                    field_radius,
                    (look.space(SEARCH_HEIGHT) - clear_side) / 2.0,
                ))
                .role("button")
                .label(strings::settings_search_clear())
                .focusable(true)
                .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
                .on_press_tx(|shell: &mut Shell, tx| shell.settings_clear_search(tx))
                .child(icon(&X, ROW_ICON_PT).color(look.palette.text_muted)),
        );
    }
    el().w_full().pb(look.space(SPACE_MD)).child(field)
}

/// A page in the nav: its icon in a circle, its name and one-line summary.
fn nav_row(page: PageId, summary: &str, current: bool, look: &Look) -> El {
    let fill = if current {
        look.on_panel(look.palette.surface_selected)
    } else {
        rune::Color::TRANSPARENT
    };
    let weight = if current {
        crate::design::TYPE_BODY_STRONG
    } else {
        TYPE_BODY_MEDIUM
    };
    row()
        .key(format!("settings-nav-{page:?}"))
        .w_full()
        .py(look.space(NAV_ROW_PADDING))
        .px(NAV_ROW_INSET_PT)
        .gap(look.space(SPACE_MD) + SPACE_SM.0)
        .center_y()
        // As the search field above it: concentric with the panel, whose
        // edge is the nav's padding out.
        .radius(Look::nested_radius(
            look.radius(PANEL_RADIUS),
            look.space(NAV_PADDING),
        ))
        .fill(fill)
        .role("link")
        .label(format!("{}, {summary}", page_title(page)))
        .focusable(true)
        .focus_highlight(look.on_panel(look.palette.surface_hover))
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .on_press_tx(move |shell: &mut Shell, tx| shell.settings_page(page, tx))
        .child(
            el().size(NAV_ICON_CIRCLE_PT, NAV_ICON_CIRCLE_PT)
                .radius(NAV_ICON_CIRCLE_PT / 2.0)
                .center()
                .fill(look.palette.surface_tile)
                .a11y_hidden()
                .child(icon(&PALETTE, PILL_ICON_PT).color(look.palette.text_secondary)),
        )
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .child(
                    text(page_title(page))
                        .font(look.font(weight))
                        .color(look.palette.text_primary),
                )
                .child(
                    text(summary.to_owned())
                        .font(look.font(TYPE_SMALL))
                        .color(look.palette.text_muted)
                        .max_lines(1),
                ),
        )
}

/// The page or the search results, under their head.
fn main(
    panel: &SettingsPanel,
    context: &Context<'_>,
    grain: Option<&rune::Pixels>,
    look: &Look,
) -> El {
    let cx = Ctx {
        look,
        appearance: context.appearance,
        failed: panel.failed_row(),
        colour: panel.colour,
        grain,
    };
    let query = panel.query();
    let (title, scope, back, blocks) = if query.is_empty() {
        let page = panel.page();
        let blocks: Vec<El> = rows::page(page, context)
            .into_iter()
            .map(|group| controls::group(group, &cx))
            .collect();
        (
            page_title(page).to_owned(),
            strings::settings_scope_device().to_owned(),
            page.parent(),
            blocks,
        )
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
