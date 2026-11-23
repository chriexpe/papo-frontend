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
    FOCUS_RING_PT, HEADER_PILL_HEIGHT, Look, PILL_BUTTON_PT, PILL_ICON_PT, RADIUS_CONTROL,
    RADIUS_PILL, SPACE_LG, SPACE_MD, SPACE_SM, SPACE_XL, Surfaces, TYPE_BODY, TYPE_SMALL,
};
use crate::strings;
use crate::timeline::Style;

/// The panel's widest (`size.panel`), in points: the pins panel's.
const PANEL_MAX_PT: f32 = 440.0;
/// How much of the conversation's height the panel may take.
const PANEL_HEIGHT_SHARE: f32 = 0.7;

/// 🔍, in the accent while the panel is open.
pub(super) fn search_button(open: bool, look: &Look) -> El {
    let tone = if open {
        look.palette.accent
    } else {
        look.palette.text_secondary
    };
    row()
        .key(SEARCH_BUTTON_KEY)
        .size(PILL_BUTTON_PT, PILL_BUTTON_PT)
        .center()
        .radius(look.radius(RADIUS_CONTROL))
        .role("button")
        .label(strings::search_title())
        .help(strings::search_title())
        .help_keys([strings::shortcut_search()])
        .expanded(open)
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .hover(look.palette.surface_hover)
        .on_press_tx(|shell: &mut Shell, tx| shell.search_pressed(false, tx))
        .child(icon(&crate::icons::SEARCH, PILL_ICON_PT).color(tone))
}

/// The open panel over the conversation's top-right.
pub(super) fn panel(shell: &Shell, title: &Title<'_>, style: &Style, cx: &Cx<Shell>) -> Option<El> {
    let search = shell.shown_search()?;
    let open = shell.tabs.shown()?;
    let look = style.look;
    let [width_pt, height_pt] = cx
        .layout_size("conversation")
        .unwrap_or([PANEL_MAX_PT, 600.0]);
    let width = PANEL_MAX_PT.min(width_pt - 2.0 * look.space(SPACE_LG));
    let list = shell.search_list();
    let head = head(shell, search, title, list.as_ref(), &look);
    let body = if search.showing == Showing::Peek {
        search_hits::peek_line(open, search, &look)
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
            .radius(look.radius(RADIUS_PILL))
            .pill_surface(&look)
            .role("dialog")
            .label(strings::search_title())
            .pin(
                None,
                Some(look.space(SPACE_LG)),
                Some(look.space(SPACE_LG)),
                None,
            )
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
    let look = style.look;
    let this_year = chrono::Datelike::year(&shell.search_calendar().today);
    match list {
        // A date already read: what it means, over the results.
        Some(list @ FieldList::Dates { read: true, .. }) => {
            let meaning = search_list::list_view(list, usize::MAX, this_year, &look);
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
            Some(search_list::list_view(list, at, this_year, &look))
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
    look: &Look,
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
        .map(|draft| field_spans(&draft, look))
        .unwrap_or_default();
    let mut field = text_input(&search.edit, look.font(TYPE_BODY))
        .key(SEARCH_FIELD_KEY)
        .label(strings::search_title())
        .grow(1.0)
        .min_w(0.0)
        .color(look.palette.text_primary)
        .placeholder(&placeholder, look.palette.text_muted)
        .caret(look.palette.accent, look.palette.text_selection)
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
            .radius(look.radius(RADIUS_CONTROL))
            .role("button")
            .label(strings::search_clear())
            .focusable(true)
            .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
            .hover(look.palette.surface_hover)
            .on_press_tx(|shell: &mut Shell, tx| shell.clear_search(tx))
            .child(icon(&crate::icons::X, PILL_ICON_PT).color(look.palette.text_secondary))
    });
    row()
        .w_full()
        .h(look.space(HEADER_PILL_HEIGHT))
        .pl(look.space(SPACE_LG))
        .pr(look.space(SPACE_SM))
        .gap(look.space(SPACE_SM))
        .center_y()
        .child(field)
        .children(clear)
        .child(search_button(true, look))
}

/// The results, their count and scope; nothing found; a failure; a wait.
fn results_area(shell: &Shell, search: &Search, style: &Style, max_height_pt: f32) -> Option<El> {
    let look = style.look;
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
            says.then(|| nothing_found(shell, search, &look))
        }
        Found::Loading => Some(
            el().w_full()
                .h(look.space(HEADER_PILL_HEIGHT) * 2.0)
                .busy(true)
                .label(strings::search_loading()),
        ),
        Found::Failed => Some(failed(&look)),
        Found::Ready(results) if results.items().is_empty() => Some(
            col()
                .w_full()
                .child(count_row(search, results, &look))
                .child(nothing_found(shell, search, &look)),
        ),
        Found::Ready(results) => Some(
            col()
                .w_full()
                .child(count_row(search, results, &look))
                .child(search_hits::hits(search, style, max_height_pt)),
        ),
    }
}

/// "{n} results" and, on a server, the scope choice.
fn count_row(search: &Search, results: &SearchResults, look: &Look) -> El {
    // The next search's answer is on its way: these results stay meanwhile.
    let loading = search.loading();
    let count = match results.count() {
        SearchCount::Exact(count) => strings::search_results(count as u64),
        SearchCount::AtLeast(count) => strings::search_results_more(count as u64),
    };
    let scope =
        matches!(search.place, Place::Server { .. }).then(|| scope_button(search.scope, look));
    row()
        .w_full()
        .px(look.space(SPACE_LG))
        .pb(look.space(SPACE_SM))
        .gap(look.space(SPACE_SM))
        .center_y()
        .busy(loading)
        .child(
            text(count)
                .font(look.font(TYPE_SMALL))
                // Dimmed while the next answer loads.
                .color(if loading {
                    look.palette.text_muted
                } else {
                    look.palette.text_secondary
                })
                .role("status")
                .live_region(rune::LiveRegion::Polite),
        )
        .child(el().grow(1.0))
        .children(scope)
}

/// "This server ⌄": opens the scope menu.
fn scope_button(scope: Scope, look: &Look) -> El {
    let label = match scope {
        Scope::Server => strings::search_scope_server(),
        Scope::Channel => strings::search_scope_channel(),
    };
    row()
        .key(SCOPE_KEY)
        .h(PILL_BUTTON_PT)
        .px(look.space(SPACE_SM))
        .gap(look.space(SPACE_SM) / 2.0)
        .center_y()
        .radius(look.radius(RADIUS_CONTROL))
        .role("button")
        .label(format!("{}: {label}", strings::search_scope()))
        .expanded(false)
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .hover(look.palette.surface_hover)
        .on_press_tx(|shell: &mut Shell, tx| shell.open_scope_menu(tx))
        .child(
            text(label)
                .font(look.font(TYPE_SMALL))
                .color(look.palette.text_primary),
        )
        .child(icon(&crate::icons::CHEVRON_DOWN, PILL_ICON_PT).color(look.palette.text_secondary))
}

/// The scope button's element key, which its menu grows from.
pub(crate) const SCOPE_KEY: &str = "search-scope";

/// "No results for “…”", with Clear filters when there are any.
fn nothing_found(shell: &Shell, search: &Search, look: &Look) -> El {
    let filtered = shell
        .search_draft()
        .is_some_and(|draft| !draft.filters.is_empty());
    let clear = filtered.then(|| {
        row()
            .key("search-clear-filters")
            .px(look.space(SPACE_MD))
            .h(PILL_BUTTON_PT)
            .center()
            .radius(look.radius(RADIUS_CONTROL))
            .fill(look.palette.surface_hover)
            .role("button")
            .label(strings::search_clear_filters())
            .focusable(true)
            .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
            .on_press_tx(|shell: &mut Shell, tx| shell.search_clear_filters(tx))
            .child(
                text(strings::search_clear_filters())
                    .font(look.font(TYPE_SMALL))
                    .color(look.palette.text_primary),
            )
    });
    col()
        .w_full()
        .py(look.space(SPACE_XL))
        .gap(look.space(SPACE_MD))
        .center()
        .child(
            text(strings::search_none(search.edit.text.trim()))
                .font(look.font(TYPE_SMALL))
                .color(look.palette.text_secondary)
                .role("status")
                .wrap(),
        )
        .children(clear)
}

/// "Couldn't search. Try again".
fn failed(look: &Look) -> El {
    row()
        .w_full()
        .px(look.space(SPACE_LG))
        .pb(look.space(SPACE_MD))
        .gap(look.space(SPACE_SM))
        .center_y()
        .child(
            text(strings::search_failed())
                .font(look.font(TYPE_SMALL))
                .color(look.palette.danger)
                .role("alert"),
        )
        .child(
            row()
                .key("search-retry")
                .role("button")
                .label(strings::search_retry())
                .focusable(true)
                .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
                .on_press(|shell: &mut Shell| shell.search_retry())
                .child(
                    text(strings::search_retry())
                        .font(look.font(TYPE_SMALL))
                        .color(look.palette.link),
                ),
        )
}
