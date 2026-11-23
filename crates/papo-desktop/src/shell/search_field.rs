//! The search field and the list under it (`docs/design.md` §10, Search):
//! filters typed and styled in place (the keyword in `text.muted`, a bound
//! value as a chip in the link colour on `accent.soft`); focused and
//! empty, the list of filters; after a keyword, its values; an ambiguous
//! name, "Which Lia?" and its matches. ↑ and ↓ move through the list,
//! Enter or Tab picks, and the keyboard stays in the field; without a
//! list, ↓ goes into the results.
//!
//! Owns what the list holds and what a pick writes. Reading the text and
//! finding the values are the SDK's (`SearchDraft`, `search_suggestions`);
//! the panel's look is `search_panel`'s.

use papo_sdk::{FilterValue, HasKind, SearchDraft, SearchKeyword, SearchScope};
use papo_sdk_ui::{SearchSuggestion, SuggestedValue, search_candidates, search_suggestions};
use rune::{KeyEvent, Span, Tx};

use super::Shell;
use super::search::SEARCH_FIELD_KEY;
use crate::design::{CHIP_PAD_PT, Palette};
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
}

impl FieldList {
    /// How many rows it has.
    pub(crate) fn len(&self) -> usize {
        match self {
            FieldList::Filters(keywords) => keywords.len(),
            FieldList::Values { items, .. } => items.len(),
        }
    }
}

/// The filters a scope offers, in the list's order: a DM has no channels
/// and offers no mentions.
pub(crate) fn offered_filters(scope: SearchScope) -> Vec<SearchKeyword> {
    if matches!(scope, SearchScope::Direct { .. }) {
        vec![SearchKeyword::From, SearchKeyword::Has]
    } else {
        vec![
            SearchKeyword::From,
            SearchKeyword::In,
            SearchKeyword::Has,
            SearchKeyword::Mentions,
        ]
    }
}

/// A keyword as this locale writes it, colon included.
pub(crate) fn keyword_text(keyword: SearchKeyword) -> &'static str {
    match keyword {
        SearchKeyword::From => strings::search_keyword_from(),
        SearchKeyword::In => strings::search_keyword_in(),
        SearchKeyword::Has => strings::search_keyword_has(),
        SearchKeyword::Mentions => strings::search_keyword_mentions(),
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
fn written(item: &SearchSuggestion) -> &str {
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
) -> Option<FieldList> {
    if text.trim().is_empty() {
        return Some(FieldList::Filters(offered_filters(scope)));
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
pub(crate) fn field_spans(draft: &SearchDraft, palette: &Palette) -> Vec<Span> {
    let mut spans = Vec::new();
    for filter in &draft.filters {
        let keyword = &filter.keyword_range;
        spans.push(Span::new(keyword.start, keyword.end).color(palette.text_muted));
        let bound = matches!(
            filter.resolved,
            FilterValue::Person(_) | FilterValue::Channel(_) | FilterValue::Has(_)
        );
        let value = &filter.value_range;
        if bound && value.start < value.end {
            spans.push(
                Span::new(value.start, value.end)
                    .color(palette.link)
                    .bg(palette.accent_soft, CHIP_PAD_PT),
            );
        }
    }
    spans
}

impl Shell {
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
        )
    }

    /// A key in the field: the list's ↑, ↓, Enter and Tab, Enter to search
    /// at once without one, Escape to fold. Returns whether it was taken.
    pub(crate) fn search_key(&mut self, key: &KeyEvent, tx: &mut Tx) -> bool {
        let mods = key.mods;
        if mods.ctrl || mods.alt || mods.cmd || mods.logo {
            return false;
        }
        let rows = self.search_list().as_ref().map_or(0, FieldList::len);
        match key.key {
            rune::Key::Escape => {
                self.close_search(tx);
                true
            }
            rune::Key::Down | rune::Key::Up if rows > 0 => {
                let by = if key.key == rune::Key::Down { 1 } else { -1 };
                if let Some(search) = self.search.as_mut() {
                    search.highlight =
                        crate::composer::picker::wrapped_step(search.highlight, rows, by);
                }
                true
            }
            rune::Key::Enter | rune::Key::Tab if rows > 0 && !mods.shift => {
                let at = self.search.as_ref().map_or(0, |search| search.highlight);
                self.search_picked(at.min(rows - 1), tx);
                true
            }
            // Without a list, ↓ goes into the results (`docs/design.md`
            // §12: lists move with ↑ and ↓).
            rune::Key::Down if !mods.shift => self.focus_first_hit(tx),
            // A value typed out in full ends as a picked one does.
            rune::Key::Tab if !mods.shift => self.end_typed_value(tx),
            rune::Key::Enter => {
                self.end_typed_value(tx);
                self.search_now();
                true
            }
            _ => false,
        }
    }

    /// Row `at` of the list now was picked: a filter's keyword goes in,
    /// or a value replaces what was typed for it, followed by a space.
    pub(crate) fn search_picked(&mut self, at: usize, tx: &mut Tx) {
        let Some(list) = self.search_list() else {
            return;
        };
        let Some(search) = self.search.as_mut() else {
            return;
        };
        let edit = &mut search.edit;
        match &list {
            FieldList::Filters(keywords) => {
                let Some(keyword) = keywords.get(at) else {
                    return;
                };
                edit.replace_text(keyword_text(*keyword));
                edit.set_caret(edit.text.len());
            }
            FieldList::Values {
                value_range, items, ..
            } => {
                let Some(item) = items.get(at) else {
                    return;
                };
                let follows_space = edit
                    .text
                    .get(value_range.end..)
                    .and_then(|rest| rest.chars().next())
                    .is_some_and(char::is_whitespace);
                let mut value = written(item).to_owned();
                if !follows_space {
                    value.push(' ');
                }
                edit.set_selection(value_range.start, value_range.end);
                edit.insert(&value);
            }
        }
        search.highlight = 0;
        self.search_edited(tx);
        tx.focus(SEARCH_FIELD_KEY, true);
    }

    /// Clear filters, when nothing was found: the words stay.
    pub(crate) fn search_clear_filters(&mut self, tx: &mut Tx) {
        let Some(draft) = self.search_draft() else {
            return;
        };
        let Some(search) = self.search.as_mut() else {
            return;
        };
        let words: Vec<&str> = draft
            .words
            .iter()
            .filter_map(|range| search.edit.text.get(range.clone()))
            .collect();
        let text = words.join(" ");
        search.edit.replace_text(text);
        search.edit.set_caret(search.edit.text.len());
        self.search_now();
        tx.focus(SEARCH_FIELD_KEY, true);
    }

    /// Enter or Tab with the caret at the end of a filter's bound value
    /// (typed out in full, not picked): a space follows it, as after a
    /// pick. Returns whether it did.
    fn end_typed_value(&mut self, tx: &mut Tx) -> bool {
        let Some(draft) = self.search_draft() else {
            return false;
        };
        let Some(search) = self.search.as_mut() else {
            return false;
        };
        let caret = search.edit.range().1;
        let ends_bound = draft.filter_at(caret).is_some_and(|filter| {
            filter.value_range.end == caret
                && matches!(
                    filter.resolved,
                    FilterValue::Person(_) | FilterValue::Channel(_) | FilterValue::Has(_)
                )
        });
        let spaced = search
            .edit
            .text
            .get(caret..)
            .and_then(|rest| rest.chars().next())
            .is_some_and(char::is_whitespace);
        if !ends_bound || spaced {
            return false;
        }
        search.edit.set_caret(caret);
        search.edit.insert(" ");
        self.search_edited(tx);
        true
    }
}
