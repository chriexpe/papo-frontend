//! Draws the search results (`docs/design.md` §10, Search): each a
//! message on its own from what the hit carries, under its channel's line
//! when that differs from the result above, the opened one on
//! `highlight.selected`; only the rows on screen are built and the next
//! page's skeleton stands at the end while it loads. Shrunk (`PanelPeek`),
//! the opened result in one line. Pure.

use std::borrow::Cow;
use std::sync::Arc;

use papo_sdk_ui::SearchResultItem;
use rune::{El, GrowthSpec, KeyEvent, ScrollSpec, col, el, memo, row, text};

use super::Shell;
use super::search::{Found, Paging, Search, hit_row_key};
use crate::design::{
    FOCUS_RING_PT, Palette, RADIUS_CONTROL_PT, SPACE_LG_PT, SPACE_MD_PT, SPACE_SM_PT,
    TYPE_BODY_STRONG, TYPE_CAPTION, TYPE_SMALL,
};
use crate::strings;
use crate::timeline::replies::{OriginalText, original_text};
use crate::timeline::standalone::standalone_hit;
use crate::timeline::{OpenTimeline, Style};

/// A result's height before it is measured, in points.
const ESTIMATED_HIT_PT: f32 = 88.0;

/// The results, newest first; only the ones on screen are built, and a
/// skeleton row stands in for the next page while it loads.
pub(super) fn hits(search: &Search, style: &Style, max_height_pt: f32) -> El {
    let Found::Ready(results) = &search.found else {
        return el();
    };
    let loading = usize::from(search.paging == Paging::Loading);
    let count = results.items().len() + loading;
    let row_style = style.clone();
    col()
        .key("search-results")
        .w_full()
        .max_h(max_height_pt)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .px(SPACE_SM_PT)
        .pb(SPACE_SM_PT)
        .child(
            col()
                .key("search-rows")
                .w_full()
                .role("list")
                .label(strings::search_results_list())
                .focus_group()
                .growth(GrowthSpec::default())
                .virtual_rows(
                    count,
                    ESTIMATED_HIT_PT,
                    |shell: &Shell, index| {
                        Cow::Owned(
                            shell
                                .shown_results()
                                .and_then(|results| results.items().get(index))
                                .map_or_else(
                                    || format!("search-more-{index}"),
                                    |item| hit_row_key(item.id()),
                                ),
                        )
                    },
                    move |shell: &Shell, index| hit_row(shell, index, &row_style),
                )
                .on_visible_rows(|shell: &mut Shell, range| shell.search_rows_visible(range)),
        )
}

/// Result `index`, under its channel's line when that differs from the
/// result above; past the last, the next page's skeleton.
fn hit_row(shell: &Shell, index: usize, style: &Style) -> El {
    let palette = style.palette;
    let (Some(open), Some(search), Some(results)) = (
        shell.tabs.shown(),
        shell.shown_search(),
        shell.shown_results(),
    ) else {
        return el();
    };
    let Some(item) = results.items().get(index) else {
        return el()
            .w_full()
            .h(ESTIMATED_HIT_PT - SPACE_SM_PT)
            .my(SPACE_SM_PT / 2.0)
            .radius(RADIUS_CONTROL_PT)
            .fill(palette.surface_hover)
            .busy(true)
            .label(strings::search_loading());
    };
    let above = index
        .checked_sub(1)
        .and_then(|above| results.items().get(above));
    let channel_line = (above.is_none_or(|above| above.channel != item.channel))
        .then(|| item.channel_name.clone())
        .flatten();
    let marked = search.opened == Some(item.id());
    let id = item.id();
    let version = (
        style.theme,
        style.layout,
        style.today,
        Arc::as_ptr(&item.content) as usize,
        open.resources
            .as_ref()
            .map(|resources| Arc::as_ptr(resources) as usize),
        marked,
        channel_line.is_some(),
        open.revealed(id),
    );
    memo(format!("hit-{id}"), version, || {
        let shown = standalone_hit::<Shell>(open, item, style);
        let mut line = col()
            .key(hit_row_key(id))
            .w_full()
            .padding(SPACE_LG_PT)
            .radius(RADIUS_CONTROL_PT)
            .role("listitem")
            .label(strings::search_jump())
            .focusable(true)
            .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, palette.focus_ring)
            .on_press(move |shell: &mut Shell| shell.open_hit(id, false))
            .on_key(move |shell: &mut Shell, event: &KeyEvent, _tx| {
                if event.key != rune::Key::Enter {
                    return false;
                }
                shell.open_hit(id, true);
                true
            })
            .child(shown);
        line = if marked {
            line.fill(palette.surface_selected)
        } else {
            line.hover(palette.surface_hover)
        };
        col()
            .w_full()
            .children(channel_line.map(|name| {
                text(format!("#{name}"))
                    .font(TYPE_CAPTION)
                    .color(palette.text_muted)
                    .px(SPACE_LG_PT)
                    .pt(SPACE_SM_PT)
            }))
            .child(line)
    })
}

/// The opened result in one line, on `highlight.selected`.
pub(super) fn peek_line(open: &OpenTimeline, search: &Search, palette: &Palette) -> Option<El> {
    let id = search.opened?;
    let Found::Ready(results) = &search.found else {
        return None;
    };
    let item: &SearchResultItem = results.items().iter().find(|item| item.id() == id)?;
    let original = papo_sdk_ui::ReplyOriginal::Held {
        id,
        message: Arc::new(item.message.clone()),
        content: Arc::clone(&item.content),
    };
    let OriginalText::Message { name, snippet, .. } = original_text(&original, open) else {
        return None;
    };
    let line = row()
        .w_full()
        .px(SPACE_LG_PT)
        .py(SPACE_MD_PT)
        .gap(SPACE_SM_PT)
        .radius(RADIUS_CONTROL_PT)
        .fill(palette.surface_selected)
        .child(
            text(name)
                .font(TYPE_BODY_STRONG)
                .color(palette.text_primary),
        )
        .child(
            text(snippet)
                .font(TYPE_SMALL)
                .color(palette.text_secondary)
                .max_lines(1),
        );
    Some(col().w_full().px(SPACE_SM_PT).pb(SPACE_SM_PT).child(line))
}
