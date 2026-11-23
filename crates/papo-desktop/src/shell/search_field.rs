//! The search field and the list under it (`docs/design.md` §10, Search):
//! filters typed and styled in place (the keyword in `text.muted`, a bound
//! value as a chip in the link colour on `accent.soft`); focused and
//! empty, the list of filters; after a keyword, its values; an ambiguous
//! name, "Which Lia?" and its matches; a date's value, what it means. The
//! keys and picks are `search_keys`'s.
//!
//! Owns what the list holds and how a value is written. Reading the text and
//! finding the values are the SDK's (`SearchDraft`, `search_suggestions`);
//! the panel's look is `search_panel`'s.

use chrono::NaiveDate;
use papo_sdk::{
    DateOrder, DateWords, FilterValue, HasKind, SearchCalendar, SearchDraft, SearchKeyword,
    SearchNames, SearchScope,
};
use papo_sdk_ui::{
    DateSuggestion, SearchSuggestion, SuggestedValue, search_candidates, search_date_suggestions,
    search_suggestions,
};
use rune::Span;

use super::Shell;
use crate::design::{CHIP_PAD, Look};
use crate::strings;

/// What the list under the field shows.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum FieldList {
    /// The empty field: the filters.
    Filters(Vec<SearchKeyword>),
    /// The values of the filter at the caret, or of an ambiguous one
    /// (`which`: its typed name, for "Which Lia?").
    Values {
        keyword: SearchKeyword,
        /// The value's byte range in the field, which a pick replaces.
        value_range: std::ops::Range<usize>,
        which: Option<String>,
        items: Vec<SearchSuggestion>,
    },
    /// What the date filter at the caret can be, each with what it means.
    Dates {
        /// The value's byte range in the field, which a pick replaces.
        value_range: std::ops::Range<usize>,
        items: Vec<DateSuggestion>,
        /// The value is already read: its one row says what it means,
        /// over the results, and takes no keys (Enter searches).
        read: bool,
    },
}

impl FieldList {
    /// How many rows it has.
    pub(crate) fn len(&self) -> usize {
        match self {
            FieldList::Filters(keywords) => keywords.len(),
            FieldList::Values { items, .. } => items.len(),
            FieldList::Dates { items, .. } => items.len(),
        }
    }

    /// Rows the keys move through: none for a date already read.
    pub(crate) fn key_rows(&self) -> usize {
        match self {
            FieldList::Dates { read: true, .. } => 0,
            list => list.len(),
        }
    }
}

/// The filters a scope offers, in the list's order, the dates last: a DM
/// has no channels and offers no mentions.
pub(crate) fn offered_filters(scope: SearchScope) -> Vec<SearchKeyword> {
    let dates = [
        SearchKeyword::Before,
        SearchKeyword::After,
        SearchKeyword::During,
    ];
    let mut keywords = if matches!(scope, SearchScope::Direct { .. }) {
        vec![SearchKeyword::From, SearchKeyword::Has]
    } else {
        vec![
            SearchKeyword::From,
            SearchKeyword::In,
            SearchKeyword::Has,
            SearchKeyword::Mentions,
        ]
    };
    keywords.extend(dates);
    keywords
}

/// A keyword as this locale writes it, colon included.
pub(crate) fn keyword_text(keyword: SearchKeyword) -> &'static str {
    match keyword {
        SearchKeyword::From => strings::search_keyword_from(),
        SearchKeyword::In => strings::search_keyword_in(),
        SearchKeyword::Has => strings::search_keyword_has(),
        SearchKeyword::Mentions => strings::search_keyword_mentions(),
        SearchKeyword::Before => strings::search_keyword_before(),
        SearchKeyword::After => strings::search_keyword_after(),
        SearchKeyword::During => strings::search_keyword_during(),
    }
}

/// A `has:` kind as this locale writes and lists it.
pub(crate) fn has_text(kind: HasKind) -> &'static str {
    match kind {
        HasKind::Link => strings::search_has_link(),
        HasKind::File => strings::search_has_file(),
    }
}

/// What a picked value writes: a kind in this locale's spelling, else
/// the SDK's binding.
pub(super) fn written(item: &SearchSuggestion) -> &str {
    match item.value {
        SuggestedValue::Has(kind) => has_text(kind),
        _ => &item.binding,
    }
}

/// The list for `text` with the caret at byte `caret`, read as `draft`;
/// none when the results show instead.
pub(crate) fn field_list(
    text: &str,
    caret: usize,
    draft: &SearchDraft,
    snapshot: &papo_sdk::ResourceSnapshot,
    scope: SearchScope,
    calendar: SearchCalendar,
) -> Option<FieldList> {
    if text.trim().is_empty() {
        return Some(FieldList::Filters(offered_filters(scope)));
    }
    // A date's list stays while its value is typed, so what it means shows.
    if let Some(filter) = draft
        .filter_at(caret)
        .filter(|filter| filter.keyword.takes_date())
    {
        let items = search_date_suggestions(&filter.value, calendar);
        if !items.is_empty() {
            let read = matches!(items.as_slice(), [only] if only.binding == filter.value.trim());
            return Some(FieldList::Dates {
                value_range: filter.value_range.clone(),
                items,
                read,
            });
        }
    }
    // The filter being typed: its values, until one is picked (the value
    // is that one's binding).
    if let Some(filter) = draft.filter_at(caret)
        && filter.resolved != FilterValue::Unavailable
    {
        let items = search_suggestions(snapshot, filter.keyword, &filter.value, scope);
        let picked = matches!(items.as_slice(), [only] if written(only) == filter.value);
        if !items.is_empty() && !picked {
            return Some(FieldList::Values {
                keyword: filter.keyword,
                value_range: filter.value_range.clone(),
                which: None,
                items,
            });
        }
    }
    // Elsewhere, an ambiguous name stops the search until one is picked.
    draft.filters.iter().find_map(|filter| {
        let FilterValue::Ambiguous(candidates) = &filter.resolved else {
            return None;
        };
        Some(FieldList::Values {
            keyword: filter.keyword,
            value_range: filter.value_range.clone(),
            which: Some(filter.value.clone()),
            items: search_candidates(snapshot, filter.keyword, candidates, scope),
        })
    })
}

/// The field's styling: each keyword in `text.muted`, each bound value a
/// chip in the link colour on `accent.soft`.
pub(crate) fn field_spans(draft: &SearchDraft, look: &Look) -> Vec<Span> {
    let mut spans = Vec::new();
    for filter in &draft.filters {
        let keyword = &filter.keyword_range;
        spans.push(Span::new(keyword.start, keyword.end).color(look.palette.text_muted));
        let bound = matches!(
            filter.resolved,
            FilterValue::Person(_)
                | FilterValue::Channel(_)
                | FilterValue::Has(_)
                | FilterValue::Date(_)
        );
        let value = &filter.value_range;
        if bound && value.start < value.end {
            spans.push(
                Span::new(value.start, value.end)
                    .color(look.palette.link)
                    .bg(look.palette.accent_soft, look.space(CHIP_PAD)),
            );
        }
    }
    spans
}

impl Shell {
    /// The field's text read against the shown conversation's names.
    pub(crate) fn search_draft(&self) -> Option<SearchDraft> {
        let search = self.search.as_ref()?;
        let open = self
            .tabs
            .shown()
            .filter(|open| open.key.account == search.account)?;
        let resources = open.resources.as_deref()?;
        let names = SearchNames { resources };
        Some(SearchDraft::read(
            &search.edit.text,
            &names,
            search.sdk_scope(),
            self.search_calendar(),
        ))
    }

    /// The reader's calendar for date filters: today in the zone the
    /// conversation shows times in, and this locale's way of writing dates.
    pub(crate) fn search_calendar(&self) -> SearchCalendar {
        // `today` counts days from 1970-01-01, which is day 719 163 of the
        // common era.
        let today = i32::try_from(self.today + 719_163)
            .ok()
            .and_then(NaiveDate::from_num_days_from_ce_opt)
            .unwrap_or_default();
        SearchCalendar {
            today,
            order: if strings::search_day_first() {
                DateOrder::DayFirst
            } else {
                DateOrder::MonthFirst
            },
            words: if strings::search_portuguese_dates() {
                DateWords::Portuguese
            } else {
                DateWords::English
            },
        }
    }

    /// The list under the field now, if any.
    pub(crate) fn search_list(&self) -> Option<FieldList> {
        let search = self.shown_search()?;
        let open = self.tabs.shown()?;
        let snapshot = open.resources.as_deref()?;
        let draft = self.search_draft()?;
        let caret = search.edit.range().1;
        field_list(
            &search.edit.text,
            caret,
            &draft,
            snapshot,
            search.sdk_scope(),
            self.search_calendar(),
        )
    }
}
