//! What the reader did to a timeline's content that stays while it is
//! open: the spoilers revealed in each message, and the code block whose
//! Copy reads "Copied" for a moment. Kept on `OpenTimeline`, so switching
//! away and back keeps them; drawing them is `inline`'s and `code`'s.

use papo_sdk::Uuid;

use crate::timeline::OpenTimeline;

/// A code block whose Copy reads "Copied".
#[derive(Clone, Copy, Debug)]
pub(crate) struct Copied {
    message: Uuid,
    /// The block's text's index in its message.
    index: usize,
    until_s: f64,
}

impl OpenTimeline {
    /// The spoilers revealed in message `id`.
    pub(crate) fn revealed(&self, id: Uuid) -> &[u32] {
        self.revealed.get(&id).map_or(&[], Vec::as_slice)
    }

    /// Reveals spoiler `index` of message `id`. Returns whether it was
    /// hidden.
    pub(crate) fn reveal(&mut self, id: Uuid, index: u32) -> bool {
        let revealed = self.revealed.entry(id).or_default();
        if revealed.contains(&index) {
            return false;
        }
        revealed.push(index);
        true
    }

    /// The code block of message `id`, by its text's index, whose Copy
    /// reads "Copied".
    pub(crate) fn copied(&self, id: Uuid) -> Option<usize> {
        self.copied
            .filter(|copied| copied.message == id)
            .map(|copied| copied.index)
    }

    /// Code block `index` of message `id` was copied: its Copy reads
    /// "Copied" until `until_s`.
    pub(crate) fn code_copied(&mut self, id: Uuid, index: usize, until_s: f64) {
        self.copied = Some(Copied {
            message: id,
            index,
            until_s,
        });
    }

    /// Ends a "Copied" whose time is up at `now_s`. Returns whether one
    /// ended.
    pub(crate) fn end_copied(&mut self, now_s: f64) -> bool {
        if self.copied.is_some_and(|copied| copied.until_s <= now_s) {
            self.copied = None;
            return true;
        }
        false
    }
}
