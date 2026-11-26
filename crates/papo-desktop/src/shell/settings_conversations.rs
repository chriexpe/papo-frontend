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
use crate::settings::{Change, RowId, SizeChoice, SizeSetting, SizedMedia, Target};

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
        Change::MediaSize(SizeSetting::Images(size)) => next.image_size = size,
        Change::MediaSize(SizeSetting::Videos(size)) => next.video_size = size,
        Change::MediaSize(SizeSetting::Embeds(size)) => next.embed_size = size,
        Change::Switch(RowId::AnimateGifs, on) => next.animate_gifs = Some(on),
        Change::Switch(RowId::AutoplayVideos, on) => next.autoplay_videos = Some(on),
        Change::Switch(RowId::LinkPreviews, on) => next.link_previews = on,
        Change::KeepPlaying(choice) => next.keep_playing = choice,
        Change::Slider { row, value, .. } => {
            let media = SizedMedia::of_slider(row)?;
            let current = match media {
                SizedMedia::Images => SizeSetting::Images(next.image_size),
                SizedMedia::Videos => SizeSetting::Videos(next.video_size),
                SizedMedia::Embeds => SizeSetting::Embeds(next.embed_size),
            };
            let (width, height) = current.max_size_pt();
            // A slider's value is within its range, so it fits a u16.
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "a slider value between 160 and 720"
            )]
            let value = value.round().max(0.0) as u16;
            let (width, height) = if row == media.slider_rows().0 {
                (value, height)
            } else {
                (width, value)
            };
            match current.with(SizeChoice::Custom, width, height) {
                SizeSetting::Images(size) => next.image_size = size,
                SizeSetting::Videos(size) => next.video_size = size,
                SizeSetting::Embeds(size) => next.embed_size = size,
            }
        }
        _ => return None,
    }
    Some(next)
}
