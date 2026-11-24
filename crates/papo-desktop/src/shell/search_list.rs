//! Draws the list under the search field (`docs/design.md` §10, Search):
//! the filters with what each takes, or a filter's values (people with
//! their avatar and @username, channels, `has:` kinds) under "Which Lia?"
//! for an ambiguous name; the highlighted row on `surface.selected`. What
//! it holds and what a pick writes are `search_field`'s. Pure.

use chrono::Datelike;
use papo_sdk::{SearchDate, SearchKeyword};
use papo_sdk_ui::{DateSuggestion, SearchSuggestion, SuggestedValue};
use rune::{El, ScrollSpec, col, icon, row, text};

use super::Shell;
use super::search_field::{FieldList, has_text, keyword_text};
use crate::channel_pane::avatar;
use crate::design::{
    Look, MENU_ROW_HEIGHT_PT, PILL_ICON_PT, RADIUS_PILL, SPACE_SM, TYPE_BODY, TYPE_CAPTION,
    TYPE_SMALL,
};
use crate::strings;

/// The rows shown before the list scrolls.
const VISIBLE_ROWS: f32 = 8.0;

/// The element key of row `index` of the list under the field.
pub(crate) fn option_key(index: usize) -> String {
    format!("search-option-{index}")
}

/// The filters, or a filter's values with "Which Lia?" over an
/// ambiguous name's.
pub(super) fn list_view(list: &FieldList, highlight: usize, this_year: i32, look: &Look) -> El {
    let (heading, rows): (Option<String>, Vec<El>) = match list {
        FieldList::Filters(keywords) => (
            Some(strings::search_filters().to_owned()),
            keywords
                .iter()
                .enumerate()
                .map(|(index, keyword)| filter_row(*keyword, index, index == highlight, look))
                .collect(),
        ),
        FieldList::Values { which, items, .. } => (
            which.as_deref().map(strings::search_which),
            items
                .iter()
                .enumerate()
                .map(|(index, item)| value_row(item, index, index == highlight, look))
                .collect(),
        ),
        FieldList::Dates { items, .. } => (
            None,
            items
                .iter()
                .enumerate()
                .map(|(index, item)| date_row(item, index, index == highlight, this_year, look))
                .collect(),
        ),
    };
    col()
        .key("search-list")
        .w_full()
        .px(look.space(SPACE_SM))
        .pb(look.space(SPACE_SM))
        .role("listbox")
        .label(strings::search_filters())
        .children(heading.map(|heading| {
            text(heading)
                .font(look.font(TYPE_CAPTION))
                .color(look.palette.text_muted)
                .px(look.space(SPACE_SM))
                .py(look.space(SPACE_SM))
        }))
        .child(
            col()
                .w_full()
                .max_h(MENU_ROW_HEIGHT_PT * VISIBLE_ROWS)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                // The list is read again at the press: the text may have
                // changed since it was drawn.
                .children(rows.into_iter().enumerate().map(|(index, row)| {
                    row.on_press_tx(move |shell: &mut Shell, tx| shell.search_picked(index, tx))
                })),
        )
}

/// One option row: what `body` holds, highlighted when `chosen`.
fn option_row(index: usize, label: String, chosen: bool, look: &Look) -> El {
    let line = row()
        .key(option_key(index))
        .w_full()
        .h(MENU_ROW_HEIGHT_PT)
        .px(look.space(SPACE_SM))
        .gap(look.space(SPACE_SM))
        .center_y()
        // Concentric with the panel, whose edge is one `space.sm` out.
        .radius(Look::nested_radius(
            look.radius(RADIUS_PILL),
            look.space(SPACE_SM),
        ))
        .role("option")
        .label(label)
        .focus_selected(chosen);
    // `surface.hover` is too close to the pill's own surface to show.
    if chosen {
        line.fill(look.on_pill(look.palette.surface_selected))
    } else {
        line
    }
}

fn filter_row(keyword: SearchKeyword, index: usize, chosen: bool, look: &Look) -> El {
    let (shape, hint) = match keyword {
        SearchKeyword::From => (&crate::icons::USER, strings::search_hint_person()),
        SearchKeyword::In => (&crate::icons::HASH, strings::search_hint_channel()),
        SearchKeyword::Has => (&crate::icons::PAPERCLIP, strings::search_hint_has()),
        SearchKeyword::Mentions => (&crate::icons::AT, strings::search_hint_person()),
        SearchKeyword::Before | SearchKeyword::After | SearchKeyword::During => {
            (&crate::icons::CALENDAR, strings::search_hint_date())
        }
    };
    let spelled = keyword_text(keyword);
    option_row(index, format!("{spelled} {hint}"), chosen, look)
        .child(icon(shape, PILL_ICON_PT).color(look.palette.text_secondary))
        .child(
            text(spelled)
                .font(look.font(TYPE_BODY))
                .color(look.palette.text_primary),
        )
        .child(
            text(hint)
                .font(look.font(TYPE_SMALL))
                .color(look.palette.text_muted)
                .truncate(),
        )
}

fn value_row(item: &SearchSuggestion, index: usize, chosen: bool, look: &Look) -> El {
    match item.value {
        SuggestedValue::Person(_) => {
            let name = item.name.clone().unwrap_or_default();
            let handle = item
                .username
                .as_ref()
                .filter(|username| **username != name)
                .map(|username| format!("@{username}"));
            let label = handle
                .as_ref()
                .map_or_else(|| name.clone(), |handle| format!("{name} {handle}"));
            option_row(index, label, chosen, look)
                .child(avatar(&name, None, look))
                .child(
                    text(name)
                        .font(look.font(TYPE_BODY))
                        .color(look.palette.text_primary)
                        .truncate(),
                )
                .children(handle.map(|handle| {
                    text(handle)
                        .font(look.font(TYPE_SMALL))
                        .color(look.palette.text_muted)
                        .truncate()
                }))
        }
        SuggestedValue::Channel(_) => {
            let name = item.name.clone().unwrap_or_default();
            option_row(index, format!("#{name}"), chosen, look)
                .child(icon(&crate::icons::HASH, PILL_ICON_PT).color(look.palette.text_secondary))
                .child(
                    text(name)
                        .font(look.font(TYPE_BODY))
                        .color(look.palette.text_primary)
                        .truncate(),
                )
        }
        SuggestedValue::Has(kind) => {
            let spelled = has_text(kind);
            option_row(index, spelled.to_owned(), chosen, look)
                .child(
                    icon(&crate::icons::PAPERCLIP, PILL_ICON_PT).color(look.palette.text_secondary),
                )
                .child(
                    text(spelled)
                        .font(look.font(TYPE_BODY))
                        .color(look.palette.text_primary),
                )
        }
    }
}

/// A date value and what it means: "2d · Friday, 21 November".
fn date_row(item: &DateSuggestion, index: usize, chosen: bool, this_year: i32, look: &Look) -> El {
    let meaning = date_meaning(item.date, this_year);
    option_row(index, format!("{} {meaning}", item.binding), chosen, look)
        .child(icon(&crate::icons::CALENDAR, PILL_ICON_PT).color(look.palette.text_secondary))
        .child(
            text(item.binding.clone())
                .font(look.font(TYPE_BODY))
                .color(look.palette.text_primary),
        )
        .child(
            text(meaning)
                .font(look.font(TYPE_SMALL))
                .color(look.palette.text_muted)
                .truncate(),
        )
}

/// What a date value means, in words: a day with its weekday (and its
/// year when not this one), a month and its year, a whole year.
fn date_meaning(date: SearchDate, this_year: i32) -> String {
    match date {
        SearchDate::Day(day) => {
            let weekday = day.weekday().num_days_from_sunday();
            if day.year() == this_year {
                strings::date(weekday, day.day(), day.month())
            } else {
                strings::date_with_year(weekday, day.day(), day.month(), i64::from(day.year()))
            }
        }
        SearchDate::Month { year, month } => strings::search_month_year(month, year),
        SearchDate::Year(year) => strings::search_year(year),
    }
}
