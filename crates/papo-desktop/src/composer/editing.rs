//! What the field's keys do beyond typing, on the shown buffer: a typed
//! character marked as typing (only it may close a pair), Tab past a
//! closer, a delimiter or shortcut wrapping the selection, and the field
//! picker's query (`@` or `:`), rows and pick. Each one that changes the text leaves
//! the shell to run it as an edit (`Composer::edited`), so it is saved as
//! typing is.

use super::markdown::{FieldStyle, field_style};

use super::emoji;
use super::mentions;
use super::pairs::{self, Keystroke};
use super::picker::{Choice, FieldQuery, Picker, Rows};
use super::{Composer, DraftState};
use crate::design::Look;
use crate::timeline::TimelineKey;
use papo_sdk_ui::message_content::SelectionFormat;

impl Composer {
    /// A key reached the field at `clock_s`; `typed` when it types. The
    /// change it makes in this event may close a pair; every key clears the
    /// last one's mark.
    pub(crate) fn key_reached(&mut self, clock_s: f64, typed: Option<Keystroke>) {
        self.typed = typed.map(|key| (clock_s, key));
    }

    /// Tab: the caret steps past the innermost closer waiting in the shown
    /// field. Whether there was one.
    pub(crate) fn skip_pair(&mut self) -> bool {
        self.shown_editable()
            .is_some_and(|draft| draft.pairs.skip(&mut draft.edit))
    }

    /// A delimiter typed, or a shortcut pressed (`by_shortcut`), over the
    /// shown field's selection: it wraps or unwraps it. Whether the field
    /// changed.
    pub(crate) fn wrap(&mut self, format: &SelectionFormat, by_shortcut: bool) -> bool {
        self.shown_editable()
            .is_some_and(|draft| pairs::wrap(&mut draft.edit, format, by_shortcut))
    }

    /// The shown field's style over its text.
    pub(crate) fn field_style(&self, look: &Look) -> FieldStyle {
        self.shown().map_or_else(FieldStyle::default, |(_, draft)| {
            field_style(&draft.edit.text, &draft.preview, look)
        })
    }

    /// The query at the shown field's caret (an `@` first, then a `:`), unless Escape closed
    /// it.
    pub(crate) fn field_query(&self) -> Option<(TimelineKey, FieldQuery)> {
        let (key, draft) = self.shown()?;
        if draft.state == DraftState::Loading {
            return None;
        }
        let query = current_query(draft)?;
        if self.dismissed_at == Some((key, query.place())) {
            return None;
        }
        Some((key, query))
    }

    /// Shows the picker for `query` with what it found; nothing found closes it. The highlight
    /// stays on its row while the query only narrows.
    pub(crate) fn show_picker(&mut self, key: TimelineKey, query: FieldQuery, rows: Rows) {
        if rows.is_empty() {
            self.picker = None;
            return;
        }
        let picker = Picker {
            key,
            query,
            rows,
            selected: 0,
        };
        let kept = self
            .picker
            .as_ref()
            .filter(|open| {
                open.key == key && open.query.byte_range().start == picker.query.byte_range().start
            })
            .and_then(Picker::chosen)
            .and_then(|chosen| picker.position(&chosen))
            .unwrap_or(0);
        self.picker = Some(Picker {
            selected: kept,
            ..picker
        });
    }

    /// No query at the caret any more, or nothing to show.
    pub(crate) fn close_picker(&mut self) {
        self.picker = None;
        let at = self
            .shown()
            .and_then(|(key, draft)| Some((key, current_query(draft)?.place())));
        if at != self.dismissed_at {
            // Another `@` or `:` (or none) gets its picker back.
            self.dismissed_at = None;
        }
    }

    /// Escape: the picker closes and stays closed for this `@` or `:`.
    pub(crate) fn dismiss_picker(&mut self) {
        if let Some(open) = self.picker.take() {
            self.dismissed_at = Some((open.key, open.query.place()));
        }
    }

    /// The open picker, while the shown field has the keyboard and its caret still ends the
    /// picker's query. Rune tells no handler when a field's caret only moves, so this is
    /// checked rather than heard: the caret leaving the query hides the picker, and its keys go
    /// back to the field.
    pub(crate) fn picker(&self) -> Option<&Picker> {
        let open = self.picker.as_ref()?;
        let (key, draft) = self.shown()?;
        let current = draft.edit.focused
            && key == open.key
            && current_query(draft).as_ref() == Some(&open.query);
        current.then_some(open)
    }

    /// ↑ or ↓: the highlight moves `by` rows.
    pub(crate) fn step_picker(&mut self, by: isize) {
        if self.picker().is_some()
            && let Some(open) = self.picker.as_mut()
        {
            open.step(by);
        }
    }

    /// Enter, Tab or a click on the row of `choice`: the query becomes its mention or its emoji.
    /// Whether the field changed.
    pub(crate) fn pick(&mut self, choice: &Choice) -> bool {
        if self.picker().is_none() {
            return false;
        }
        let Some(open) = self.picker.take() else {
            return false;
        };
        let Some(draft) = self.shown_editable() else {
            return false;
        };
        match (&open.query, &open.rows, choice) {
            (FieldQuery::Mention(query), Rows::People(people), Choice::Person(target)) => {
                let Some(candidate) = people.iter().find(|c| c.target == *target) else {
                    return false;
                };
                mentions::pick(&mut draft.edit, query, candidate);
            }
            (FieldQuery::Emoji(query), Rows::Emoji(_), Choice::Emoji(suggestion)) => {
                emoji::pick(&mut draft.edit, query, suggestion);
            }
            _ => return false,
        }
        true
    }

    /// The shown buffer, unless its draft is still being read.
    fn shown_editable(&mut self) -> Option<&mut super::Draft> {
        let key = self.shown?;
        self.drafts
            .get_mut(&key)
            .filter(|draft| draft.state != DraftState::Loading)
    }

    /// The shown field's text as it would be sent.
    #[cfg(test)]
    pub(crate) fn wire_of_shown(&self) -> String {
        self.shown()
            .map(|(_, draft)| mentions::wire(&draft.edit))
            .unwrap_or_default()
    }
}

/// The query `draft`'s caret ends: an `@` first, then a `:` outside code.
fn current_query(draft: &super::Draft) -> Option<FieldQuery> {
    if let Some(query) = mentions::query(&draft.edit) {
        return Some(FieldQuery::Mention(query));
    }
    emoji::query(&draft.edit, &draft.preview).map(FieldQuery::Emoji)
}
