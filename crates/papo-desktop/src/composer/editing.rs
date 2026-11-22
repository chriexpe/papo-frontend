//! What the field's keys do beyond typing, on the shown buffer: a typed
//! character marked as typing (only it may close a pair), Tab past a
//! closer, a delimiter or shortcut wrapping the selection, and the @
//! picker's query, rows and pick. Each one that changes the text leaves
//! the shell to run it as an edit (`Composer::edited`), so it is saved as
//! typing is.

use super::markdown::{FieldStyle, field_style};
use super::mentions::{self, Candidate, Query};
use super::picker::Picker;
use super::{Composer, DraftState, pairs};
use crate::design::Palette;
use crate::timeline::TimelineKey;
use papo_sdk_ui::message_content::SelectionFormat;

impl Composer {
    /// A typed character or Backspace reached the field at `clock_s`; the
    /// change it makes in this event may close a pair.
    pub(crate) fn key_typed(&mut self, clock_s: f64) {
        self.typed_at_s = Some(clock_s);
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
    pub(crate) fn field_style(&self, palette: &Palette) -> FieldStyle {
        self.shown().map_or_else(FieldStyle::default, |(_, draft)| {
            field_style(&draft.edit.text, &draft.preview, palette)
        })
    }

    /// The @ query at the shown field's caret, unless Escape closed it.
    pub(crate) fn mention_query(&self) -> Option<(TimelineKey, Query)> {
        let (key, draft) = self.shown()?;
        if draft.state == DraftState::Loading {
            return None;
        }
        let query = mentions::query(&draft.edit)?;
        if self.dismissed_at == Some((key, query.range.start)) {
            return None;
        }
        Some((key, query))
    }

    /// Shows the picker for `query` with the people it found; none closes
    /// it. The highlight stays on its row while the query only narrows.
    pub(crate) fn show_picker(
        &mut self,
        key: TimelineKey,
        query: Query,
        candidates: Vec<Candidate>,
    ) {
        if candidates.is_empty() {
            self.picker = None;
            return;
        }
        let selected = self
            .picker
            .as_ref()
            .filter(|open| open.key == key && open.query.range.start == query.range.start)
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

    /// No @ query at the caret any more.
    pub(crate) fn close_picker(&mut self) {
        self.picker = None;
        if self.mention_query().is_none() {
            // Another `@` gets its picker back.
            self.dismissed_at = None;
        }
    }

    /// Escape: the picker closes and stays closed for this `@`.
    pub(crate) fn dismiss_picker(&mut self) {
        if let Some(open) = self.picker.take() {
            self.dismissed_at = Some((open.key, open.query.range.start));
        }
    }

    /// The open picker, over the shown field.
    pub(crate) fn picker(&self) -> Option<&Picker> {
        self.picker
            .as_ref()
            .filter(|open| Some(open.key) == self.shown)
    }

    /// ↑ or ↓: the highlight moves `by` rows.
    pub(crate) fn step_picker(&mut self, by: isize) {
        if let Some(open) = self.picker.as_mut() {
            open.step(by);
        }
    }

    /// Enter, Tab or a click on row `index`: the query becomes its mention.
    /// Whether the field changed.
    pub(crate) fn pick(&mut self, index: usize) -> bool {
        let Some(open) = self.picker.take() else {
            return false;
        };
        let Some(candidate) = open.candidates.get(index) else {
            return false;
        };
        if self.shown != Some(open.key) {
            return false;
        }
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
