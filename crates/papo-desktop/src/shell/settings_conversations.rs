//! The shell's side of the Conversations settings (`docs/design.md` §8): a
//! row's change applied and saved off the UI thread, the message being read
//! kept in its place (Show times and Show avatars reflow every row), and a
//! failed save taking the settings back and showing under its row.
//!
//! Owns these transitions; the values and their storage are
//! `conversation_settings`'.

use rune::Tx;

use super::Shell;
use crate::conversation_settings::ConversationSettings;
use crate::settings::{Change, RowId, Target};

impl Shell {
    /// Shows `settings` and saves them, the message being read kept in its
    /// place; `row` is the settings row that asked, if any.
    pub(crate) fn set_conversations(
        &mut self,
        settings: ConversationSettings,
        row: Option<RowId>,
        tx: &mut Tx,
    ) {
        let reading = self.reading_place();
        self.latest_rows.conversations = row;
        if let Some(next) = self.conversations.change(settings) {
            self.write_conversations(next, row, tx);
        }
        // Animate GIFs takes effect on the GIFs showing now, not at their next move.
        self.show_gifs();
        super::appearance::keep_place(reading, tx);
    }

    fn write_conversations(
        &mut self,
        settings: ConversationSettings,
        row: Option<RowId>,
        tx: &mut Tx,
    ) {
        let write = self.conversations.write_of(&settings);
        self.run_write(
            write,
            move |shell, ok, tx| shell.conversations_written(settings, row, ok, tx),
            tx,
        );
    }

    /// The write of `settings` landed, or did not: a failure takes them
    /// back, the message being read kept in its place, and shows under the
    /// row that asked.
    fn conversations_written(
        &mut self,
        settings: ConversationSettings,
        row: Option<RowId>,
        ok: bool,
        tx: &mut Tx,
    ) {
        let reading = (!ok).then(|| self.reading_place()).flatten();
        if !ok {
            log::warn!("a Conversations setting was not saved");
        }
        let next = self.conversations.written(settings, ok);
        if let (Some(panel), Some(row)) = (self.settings.as_mut(), row) {
            panel.saved(row, Target::Conversations(settings), ok);
        }
        self.show_gifs();
        super::appearance::keep_place(reading, tx);
        if let Some(next) = next {
            // The change that waited is the latest one: its row hears.
            self.write_conversations(next, self.latest_rows.conversations, tx);
        }
    }
}

/// `current` with `change` applied, when `change` is a Conversations row's.
pub(super) fn applied(
    current: &ConversationSettings,
    change: &Change,
) -> Option<ConversationSettings> {
    let mut next = *current;
    match *change {
        Change::Switch(RowId::EnterSends, on) => next.enter_sends = on,
        Change::Switch(RowId::ShowTimes, on) => next.show_times = on,
        Change::Switch(RowId::ShowAvatars, on) => next.show_avatars = on,
        Change::Switch(RowId::OpenAtNewest, on) => next.open_at_newest = on,
        Change::Switch(RowId::Hourglass, on) => next.unread_hourglass = on,
        Change::Switch(RowId::ChannelDescription, on) => next.channel_description = on,
        Change::Hue(RowId::MentionHue, hue) => next.mention_hue_degrees = hue.rem_euclid(360.0),
        Change::Hue(RowId::ReplyHue, hue) => next.reply_hue_degrees = hue.rem_euclid(360.0),
        Change::ImageSize(size) => next.image_size = size,
        Change::Switch(RowId::AnimateGifs, on) => next.animate_gifs = Some(on),
        Change::Slider {
            row: row @ (RowId::MediaMaxWidth | RowId::MediaMaxHeight),
            value,
            ..
        } => {
            let (width, height) = current.image_size.max_size_pt();
            // A slider's value is within its range, so it fits a u16.
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "a slider value between 200 and 720"
            )]
            let value = value.round().max(0.0) as u16;
            next.image_size = if row == RowId::MediaMaxWidth {
                papo_sdk_ui::MediaSizeSetting::custom(value, height)
            } else {
                papo_sdk_ui::MediaSizeSetting::custom(width, value)
            };
        }
        _ => return None,
    }
    Some(next)
}
