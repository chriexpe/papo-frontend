//! What the field's keys do beyond typing, on the shown buffer: a typed
//! character marked as typing (only it may close a pair), Tab past a
//! closer, a delimiter or shortcut wrapping the selection, and the @
//! picker's query, rows and pick. Each one that changes the text leaves
//! the shell to run it as an edit (`Composer::edited`), so it is saved as
//! typing is.

use super::markdown::{FieldStyle, field_style};
use papo_sdk::MentionTarget;
use papo_sdk_ui::{MentionQuery, MentionSuggestion};

use super::mentions;
use super::pairs::{self, Keystroke};
use super::picker::Picker;
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

    /// The @ query at the shown field's caret, unless Escape closed it.
    pub(crate) fn mention_query(&self) -> Option<(TimelineKey, MentionQuery)> {
        let (key, draft) = self.shown()?;
        if draft.state == DraftState::Loading {
            return None;
        }
        let query = mentions::query(&draft.edit)?;
        if self.dismissed_at == Some((key, query.byte_range.start)) {
            return None;
        }
        Some((key, query))
    }

    /// Shows the picker for `query` with the people it found; none closes
    /// it. The highlight stays on its row while the query only narrows.
    pub(crate) fn show_picker(
        &mut self,
        key: TimelineKey,
        query: MentionQuery,
        candidates: Vec<MentionSuggestion>,
    ) {
        if candidates.is_empty() {
            self.picker = None;
            return;
        }
        let selected = self
            .picker
            .as_ref()
            .filter(|open| open.key == key && open.query.byte_range.start == query.byte_range.start)
            .and_then(|open| open.chosen())
            .and_then(|chosen| candidates.iter().position(|c| c == chosen))
            .unwrap_or(0);
        self.picker = Some(Picker {
            key,
            query,
            candidates,
            selected,
        });
    }

    /// No @ query at the caret any more, or none to show.
    pub(crate) fn close_picker(&mut self) {
        self.picker = None;
        let at = self
            .shown()
            .and_then(|(key, draft)| Some((key, mentions::query(&draft.edit)?.byte_range.start)));
        if at != self.dismissed_at {
            // Another `@` (or none) gets its picker back.
            self.dismissed_at = None;
        }
    }

    /// Escape: the picker closes and stays closed for this `@`.
    pub(crate) fn dismiss_picker(&mut self) {
        if let Some(open) = self.picker.take() {
            self.dismissed_at = Some((open.key, open.query.byte_range.start));
        }
    }

    /// The open picker, while the shown field has the keyboard and its
    /// caret still ends the picker's query. Rune tells no handler when a
    /// field's caret only moves, so this is checked rather than heard: the
    /// caret leaving the `@` hides the picker, and its keys go back to the
    /// field.
    pub(crate) fn picker(&self) -> Option<&Picker> {
        let open = self.picker.as_ref()?;
        let (key, draft) = self.shown()?;
        let current = draft.edit.focused
            && key == open.key
            && mentions::query(&draft.edit).as_ref() == Some(&open.query);
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

    /// Enter, Tab or a click on the row of `target`: the query becomes its
    /// mention. Whether the field changed.
    pub(crate) fn pick(&mut self, target: MentionTarget) -> bool {
        if self.picker().is_none() {
            return false;
        }
        let Some(open) = self.picker.take() else {
            return false;
        };
        let Some(candidate) = open.candidates.iter().find(|c| c.target == target) else {
            return false;
        };
        let Some(draft) = self.shown_editable() else {
            return false;
        };
        mentions::pick(&mut draft.edit, &open.query, candidate);
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
