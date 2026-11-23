//! Draws the search panel (`docs/design.md` §10, Search): it takes the
//! actions pill's place, its field stretching left with 🔍 kept where it
//! was, and grows down: the list under the field (`search_field`), or
//! "{n} results" with the scope choice and the results, each a message on
//! its own with its channel's line when it differs from the one above,
//! more paging in near the end. Shrunk (`PanelPeek`), it keeps its field
//! and one line for the opened result. Pure: reads the shell.

use papo_sdk::{FilterValue, SearchProblem};
use papo_sdk_ui::{SearchCount, SearchResults};
use rune::{Cx, El, KeyEvent, col, el, icon, row, text, text_input};

use super::Shell;
use super::search::{
    Found, Place, SEARCH_BUTTON_KEY, SEARCH_FIELD_KEY, SEARCH_PANEL_KEY, Scope, Search, Showing,
};
use super::search_field::{FieldList, field_spans};
use super::title::Title;
use super::{search_hits, search_list};
use crate::design::{
    FOCUS_RING_PT, HEADER_PILL_HEIGHT_PT, PILL_BUTTON_PT, PILL_ICON_PT, PILL_OUTLINE_PT, Palette,
    RADIUS_CONTROL_PT, RADIUS_PILL_PT, SPACE_LG_PT, SPACE_MD_PT, SPACE_SM_PT, SPACE_XL_PT,
    TYPE_BODY, TYPE_SMALL,
};
use crate::strings;
use crate::timeline::Style;

/// The panel's widest (`size.panel`), in points: the pins panel's.
const PANEL_MAX_PT: f32 = 440.0;
/// How much of the conversation's height the panel may take.
const PANEL_HEIGHT_SHARE: f32 = 0.7;

/// 🔍, in the accent while the panel is open.
pub(super) fn search_button(open: bool, palette: &Palette) -> El {
    let tone = if open {
        palette.accent
    } else {
        palette.text_secondary
    };
    row()
        .key(SEARCH_BUTTON_KEY)
        .size(PILL_BUTTON_PT, PILL_BUTTON_PT)
        .center()
        .radius(RADIUS_CONTROL_PT)
        .role("button")
        .label(strings::search_title())
        .help(strings::search_title())
        .help_keys([strings::shortcut_search()])
        .expanded(open)
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, palette.focus_ring)
        .hover(palette.surface_hover)
        .on_press_tx(|shell: &mut Shell, tx| shell.search_pressed(false, tx))
        .child(icon(&crate::icons::SEARCH, PILL_ICON_PT).color(tone))
}

/// The open panel over the conversation's top-right.
pub(super) fn panel(shell: &Shell, title: &Title<'_>, style: &Style, cx: &Cx<Shell>) -> Option<El> {
    let search = shell.shown_search()?;
    let open = shell.tabs.shown()?;
    let palette = style.palette;
    let [width_pt, height_pt] = cx
        .layout_size("conversation")
        .unwrap_or([PANEL_MAX_PT, 600.0]);
    let width = PANEL_MAX_PT.min(width_pt - 2.0 * SPACE_LG_PT);
    let list = shell.search_list();
    let head = head(shell, search, title, list.as_ref(), &palette);
    let body = if search.showing == Showing::Peek {
        search_hits::peek_line(open, search, &palette)
    } else {
        open_body(
            shell,
            search,
            list.as_ref(),
            style,
            height_pt * PANEL_HEIGHT_SHARE,
        )
    };
    Some(
        col()
            .key(SEARCH_PANEL_KEY)
            .w(width)
            .radius(RADIUS_PILL_PT)
            .fill(palette.surface_pill)
            .border(PILL_OUTLINE_PT, palette.line_pill)
            .role("dialog")
            .label(strings::search_title())
            .pin(None, Some(SPACE_LG_PT), Some(SPACE_LG_PT), None)
            .on_key(|shell: &mut Shell, event: &KeyEvent, tx| {
                if event.key == rune::Key::Escape {
                    shell.close_search(tx);
                    return true;
                }
                false
            })
            // As the pins panel: leaving it after opening a result shrinks
            // it, coming back stretches it.
            .on_hover_change(|shell: &mut Shell, hovered, tx| {
                if hovered {
                    shell.search_hovered(tx);
                } else {
                    shell.search_left();
                }
            })
            .on_background_press_tx(|shell: &mut Shell, tx| {
                shell.search_hovered(tx);
            })
            .child(head)
            .children(body),
    )
}

/// The open panel under its field: the list there, a date's meaning over
/// the results, or the results.
fn open_body(
    shell: &Shell,
    search: &Search,
    list: Option<&FieldList>,
    style: &Style,
    max_height_pt: f32,
) -> Option<El> {
    let palette = style.palette;
    let this_year = chrono::Datelike::year(&shell.search_calendar().today);
    match list {
        // A date already read: what it means, over the results.
        Some(list @ FieldList::Dates { read: true, .. }) => {
            let meaning = search_list::list_view(list, usize::MAX, this_year, &palette);
            Some(col().w_full().child(meaning).children(results_area(
                shell,
                search,
                style,
                max_height_pt,
            )))
        }
        Some(list) => {
            // The same row the field names to assistive technology.
            let at = search.highlight.min(list.len().saturating_sub(1));
            Some(search_list::list_view(list, at, this_year, &palette))
        }
        None => results_area(shell, search, style, max_height_pt),
    }
}

/// The field, its ✕ while it has text, and 🔍.
fn head(
    shell: &Shell,
    search: &Search,
    title: &Title<'_>,
    list: Option<&FieldList>,
    palette: &Palette,
) -> El {
    let placeholder = match title {
        Title::Direct { name, .. } => strings::search_placeholder_dm(name),
        Title::Channel(_) => {
            let server = shell
                .nav
                .selected()
                .map_or("", |server| server.name.as_str());
            strings::search_placeholder(server)
        }
    };
    let spans = shell
        .search_draft()
        .map(|draft| field_spans(&draft, palette))
        .unwrap_or_default();
    let mut field = text_input(&search.edit, TYPE_BODY)
        .key(SEARCH_FIELD_KEY)
        .label(strings::search_title())
        .grow(1.0)
        .min_w(0.0)
        .color(palette.text_primary)
        .placeholder(&placeholder, palette.text_muted)
        .caret(palette.accent, palette.text_selection)
        .spans(spans)
        .on_edit(Shell::search_field_mut)
        .on_input_change(Shell::search_edited)
        .on_input_key(|shell: &mut Shell, key: &KeyEvent, tx| shell.search_key(key, tx));
    if let Some(list) = list.filter(|list| list.key_rows() > 0) {
        let at = search.highlight.min(list.len() - 1);
        field = field
            .active_descendant(search_list::option_key(at))
            .expanded(true);
    }
    let clear = (!search.edit.text.is_empty()).then(|| {
        row()
            .key("search-clear")
            .size(PILL_BUTTON_PT, PILL_BUTTON_PT)
            .center()
            .radius(RADIUS_CONTROL_PT)
            .role("button")
            .label(strings::search_clear())
            .focusable(true)
            .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, palette.focus_ring)
            .hover(palette.surface_hover)
            .on_press_tx(|shell: &mut Shell, tx| shell.clear_search(tx))
            .child(icon(&crate::icons::X, PILL_ICON_PT).color(palette.text_secondary))
    });
    row()
        .w_full()
        .h(HEADER_PILL_HEIGHT_PT)
        .pl(SPACE_LG_PT)
        .pr(SPACE_SM_PT)
        .gap(SPACE_SM_PT)
        .center_y()
        .child(field)
        .children(clear)
        .child(search_button(true, palette))
}

/// The results, their count and scope; nothing found; a failure; a wait.
fn results_area(shell: &Shell, search: &Search, style: &Style, max_height_pt: f32) -> Option<El> {
    let palette = style.palette;
    match &search.found {
        Found::Nothing => None,
        Found::Held => {
            // Held text finds nothing, and says so; an ambiguous name has
            // its list instead, and an open quote is still being typed.
            let says = shell.search_draft().is_some_and(|draft| {
                let ambiguous = draft
                    .filters
                    .iter()
                    .any(|filter| matches!(filter.resolved, FilterValue::Ambiguous(_)));
                !ambiguous && draft.query() != Err(&SearchProblem::UnclosedQuote)
            });
            says.then(|| nothing_found(shell, search, &palette))
        }
        Found::Loading => Some(
            el().w_full()
                .h(HEADER_PILL_HEIGHT_PT * 2.0)
                .busy(true)
                .label(strings::search_loading()),
        ),
        Found::Failed => Some(failed(&palette)),
        Found::Ready(results) if results.items().is_empty() => Some(
            col()
                .w_full()
                .child(count_row(search, results, &palette))
                .child(nothing_found(shell, search, &palette)),
        ),
        Found::Ready(results) => Some(
            col()
                .w_full()
                .child(count_row(search, results, &palette))
                .child(search_hits::hits(search, style, max_height_pt)),
        ),
    }
}

/// "{n} results" and, on a server, the scope choice.
fn count_row(search: &Search, results: &SearchResults, palette: &Palette) -> El {
    // The next search's answer is on its way: these results stay meanwhile.
    let loading = search.loading();
    let count = match results.count() {
        SearchCount::Exact(count) => strings::search_results(count as u64),
        SearchCount::AtLeast(count) => strings::search_results_more(count as u64),
    };
    let scope =
        matches!(search.place, Place::Server { .. }).then(|| scope_button(search.scope, palette));
    row()
        .w_full()
        .px(SPACE_LG_PT)
        .pb(SPACE_SM_PT)
        .gap(SPACE_SM_PT)
        .center_y()
        .busy(loading)
        .child(
            text(count)
                .font(TYPE_SMALL)
                // Dimmed while the next answer loads.
                .color(if loading {
                    palette.text_muted
                } else {
                    palette.text_secondary
                })
                .role("status")
                .live_region(rune::LiveRegion::Polite),
        )
        .child(el().grow(1.0))
        .children(scope)
}

/// "This server ⌄": opens the scope menu.
fn scope_button(scope: Scope, palette: &Palette) -> El {
    let label = match scope {
        Scope::Server => strings::search_scope_server(),
        Scope::Channel => strings::search_scope_channel(),
    };
    row()
        .key(SCOPE_KEY)
        .h(PILL_BUTTON_PT)
        .px(SPACE_SM_PT)
        .gap(SPACE_SM_PT / 2.0)
        .center_y()
        .radius(RADIUS_CONTROL_PT)
        .role("button")
        .label(format!("{}: {label}", strings::search_scope()))
        .expanded(false)
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, palette.focus_ring)
        .hover(palette.surface_hover)
        .on_press_tx(|shell: &mut Shell, tx| shell.open_scope_menu(tx))
        .child(text(label).font(TYPE_SMALL).color(palette.text_primary))
        .child(icon(&crate::icons::CHEVRON_DOWN, PILL_ICON_PT).color(palette.text_secondary))
}

/// The scope button's element key, which its menu grows from.
pub(crate) const SCOPE_KEY: &str = "search-scope";

/// "No results for “…”", with Clear filters when there are any.
fn nothing_found(shell: &Shell, search: &Search, palette: &Palette) -> El {
    let filtered = shell
        .search_draft()
        .is_some_and(|draft| !draft.filters.is_empty());
    let clear = filtered.then(|| {
        row()
            .key("search-clear-filters")
            .px(SPACE_MD_PT)
            .h(PILL_BUTTON_PT)
            .center()
            .radius(RADIUS_CONTROL_PT)
            .fill(palette.surface_hover)
            .role("button")
            .label(strings::search_clear_filters())
            .focusable(true)
            .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, palette.focus_ring)
            .on_press_tx(|shell: &mut Shell, tx| shell.search_clear_filters(tx))
            .child(
                text(strings::search_clear_filters())
                    .font(TYPE_SMALL)
                    .color(palette.text_primary),
            )
    });
    col()
        .w_full()
        .py(SPACE_XL_PT)
        .gap(SPACE_MD_PT)
        .center()
        .child(
            text(strings::search_none(search.edit.text.trim()))
                .font(TYPE_SMALL)
                .color(palette.text_secondary)
                .role("status")
                .wrap(),
        )
        .children(clear)
}

/// "Couldn't search. Try again".
fn failed(palette: &Palette) -> El {
    row()
        .w_full()
        .px(SPACE_LG_PT)
        .pb(SPACE_MD_PT)
        .gap(SPACE_SM_PT)
        .center_y()
        .child(
            text(strings::search_failed())
                .font(TYPE_SMALL)
                .color(palette.danger)
                .role("alert"),
        )
        .child(
            row()
                .key("search-retry")
                .role("button")
                .label(strings::search_retry())
                .focusable(true)
                .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, palette.focus_ring)
                .on_press(|shell: &mut Shell| shell.search_retry())
                .child(
                    text(strings::search_retry())
                        .font(TYPE_SMALL)
                        .color(palette.link),
                ),
        )
}
