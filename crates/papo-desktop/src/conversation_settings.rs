//! The Conversations settings (`docs/design.md` §8, Conversations): how the
//! composer takes Enter and starts a reply, what a row shows (its time, its
//! avatar), how a timeline opens and what it shows of skipped messages, the
//! channel description on opening, the mention and reply highlights'
//! hues (the reply highlight is drawn once B-006 lands), and the Media group
//! (Image size, Video size, Embed size, Animate GIFs and emoji, Autoplay
//! videos, Keep playing when hidden), and Show link previews. Every
//! one is this device's: times and avatars could follow the account
//! (`display.show_timestamps`, `display.show_avatars`) only once the
//! backend can save one field without the others (B-020).
//!
//! Owns the values, their defaults and their keys in this device's
//! settings file, one key each. A change shows at once and is written off
//! the UI thread; a failed write takes the settings back to what the file
//! holds. What each one changes is its reader's (the composer, the
//! timeline, the look).

use papo_sdk_ui::{EmbedSizeSetting, MediaSizeSetting};
use rune::prefs::Prefs;
use serde_json::{Value, json};

use crate::design::{MENTION_HIGHLIGHT_HUE_DEGREES, REPLY_HIGHLIGHT_HUE_DEGREES};
use crate::device_settings::{SettingsFile, Stored, Write};
use crate::keep_playing::KeepPlaying;

mod saved_size;
use saved_size::{SavedSize, saved_size};

/// "Enter to send".
const ENTER_SENDS_KEY: &str = "enter_sends";
/// "Mention when replying" (no row until B-006; kept as it was saved).
const MENTION_REPLY_KEY: &str = "mention_when_replying";
/// "Show times".
const SHOW_TIMES_KEY: &str = "show_times";
/// "Show avatars".
const SHOW_AVATARS_KEY: &str = "show_avatars";
/// "Open at the newest message".
const OPEN_AT_NEWEST_KEY: &str = "open_at_newest";
/// "Unread hourglass".
const UNREAD_HOURGLASS_KEY: &str = "unread_hourglass";
/// "Description when opening a channel".
const CHANNEL_DESCRIPTION_KEY: &str = "channel_description";
/// Message highlights › Mentions, in OKLCH degrees.
const MENTION_HUE_KEY: &str = "mention_hue";
/// Message highlights › Replies, in OKLCH degrees.
const REPLY_HUE_KEY: &str = "reply_hue";
/// Media › Image size: "small", "medium", "large", or `{"custom": [width, height]}`.
const IMAGE_SIZE_KEY: &str = "image_size";
/// Media › Animate GIFs and emoji; absent until changed, when it follows the system's
/// reduced motion.
const ANIMATE_GIFS_KEY: &str = "animate_gifs";
/// Media › Video size, saved as Image size is.
const VIDEO_SIZE_KEY: &str = "video_size";
/// Media › Autoplay videos; absent until changed, when it follows the system's reduced motion.
const AUTOPLAY_VIDEOS_KEY: &str = "autoplay_videos";
/// Media › Keep playing when hidden: "nothing", "audio", "video" or "both".
const KEEP_PLAYING_KEY: &str = "keep_playing_hidden";
/// Media › Embed size, saved as Image size is.
const EMBED_SIZE_KEY: &str = "embed_size";
/// Show link previews.
const LINK_PREVIEWS_KEY: &str = "link_previews";

/// Everything the Conversations page holds.
#[derive(Clone, Copy, Debug, PartialEq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each is one of the page's independent switches"
)]
pub(crate) struct ConversationSettings {
    /// Enter sends and Shift+Enter adds a line; off, the reverse.
    pub(crate) enter_sends: bool,
    /// A reply's @ switch starts on.
    pub(crate) mention_when_replying: bool,
    /// Rows show their time; off, only the message menu gives it.
    pub(crate) show_times: bool,
    /// Rows show their author's avatar; off, every spacing draws Extra
    /// compact's shapes.
    pub(crate) show_avatars: bool,
    /// Every timeline opens at its newest message; off, it returns where
    /// the reader left it.
    pub(crate) open_at_newest: bool,
    /// ⌛ and the skipped block's edges show, and ↓ can clear the block.
    pub(crate) unread_hourglass: bool,
    /// A channel's topic shows in its title pill for six seconds when it
    /// opens.
    pub(crate) channel_description: bool,
    /// `highlight.mention`'s hue, in OKLCH degrees (0–360).
    pub(crate) mention_hue_degrees: f32,
    /// `highlight.reply`'s hue, in OKLCH degrees (0–360).
    pub(crate) reply_hue_degrees: f32,
    /// The largest box a picture takes (§14.9).
    pub(crate) image_size: MediaSizeSetting,
    /// GIFs animate while they show; off, they play while hovered or focused. Unset until
    /// changed: it then starts off when the system asks for reduced motion.
    pub(crate) animate_gifs: Option<bool>,
    /// The largest box a video takes (§14.9).
    pub(crate) video_size: MediaSizeSetting,
    /// Clips of a minute or less play muted while they show (§14.5). Unset until changed: it
    /// then starts off when the system asks for reduced motion.
    pub(crate) autoplay_videos: Option<bool>,
    /// Which sound a person started goes on while the window is off screen.
    pub(crate) keep_playing: KeepPlaying,
    /// The largest box a link card's picture or a provider's player takes (§15.7).
    pub(crate) embed_size: EmbedSizeSetting,
    /// Messages draw cards for their links; off, the links stay text and nothing is reserved.
    pub(crate) link_previews: bool,
}

impl ConversationSettings {
    /// Whether GIFs animate, the system asking for reduced motion or not.
    pub(crate) fn animates_gifs(&self, reduced_motion: bool) -> bool {
        self.animate_gifs.unwrap_or(!reduced_motion)
    }

    /// Whether short videos play muted while they show, the system asking for reduced motion or
    /// not.
    pub(crate) fn autoplays_videos(&self, reduced_motion: bool) -> bool {
        self.autoplay_videos.unwrap_or(!reduced_motion)
    }
}

impl Default for ConversationSettings {
    /// The design's defaults: Enter sends, replies mention, times and
    /// avatars show, a timeline returns where it was left, the hourglass
    /// and the description show, mentions in amber and replies in teal.
    fn default() -> ConversationSettings {
        ConversationSettings {
            enter_sends: true,
            mention_when_replying: true,
            show_times: true,
            show_avatars: true,
            open_at_newest: false,
            unread_hourglass: true,
            channel_description: true,
            mention_hue_degrees: MENTION_HIGHLIGHT_HUE_DEGREES,
            reply_hue_degrees: REPLY_HIGHLIGHT_HUE_DEGREES,
            image_size: MediaSizeSetting::Medium,
            animate_gifs: None,
            video_size: MediaSizeSetting::Medium,
            autoplay_videos: None,
            keep_playing: KeepPlaying::default(),
            embed_size: EmbedSizeSetting::Medium,
            link_previews: true,
        }
    }
}

/// The Conversations settings and where they are saved.
pub(crate) struct ConversationStore {
    stored: Stored<ConversationSettings>,
    file: SettingsFile,
}

impl Default for ConversationStore {
    /// The defaults, kept nowhere: before the settings file is read, and in
    /// tests.
    fn default() -> ConversationStore {
        ConversationStore::read(&Prefs::memory())
    }
}

impl ConversationStore {
    /// The settings in `prefs`, this device's settings file; a missing or
    /// unreadable value takes its default.
    pub(crate) fn read(prefs: &Prefs) -> ConversationStore {
        let defaults = ConversationSettings::default();
        let switch = |key: &str, default: bool| prefs.get(key).unwrap_or(default);
        let hue = |key: &str, default: f32| {
            prefs
                .get::<f32>(key)
                .filter(|hue| hue.is_finite())
                .map_or(default, |hue| hue.rem_euclid(360.0))
        };
        let settings = ConversationSettings {
            enter_sends: switch(ENTER_SENDS_KEY, defaults.enter_sends),
            mention_when_replying: switch(MENTION_REPLY_KEY, defaults.mention_when_replying),
            show_times: switch(SHOW_TIMES_KEY, defaults.show_times),
            show_avatars: switch(SHOW_AVATARS_KEY, defaults.show_avatars),
            open_at_newest: switch(OPEN_AT_NEWEST_KEY, defaults.open_at_newest),
            unread_hourglass: switch(UNREAD_HOURGLASS_KEY, defaults.unread_hourglass),
            channel_description: switch(CHANNEL_DESCRIPTION_KEY, defaults.channel_description),
            mention_hue_degrees: hue(MENTION_HUE_KEY, defaults.mention_hue_degrees),
            reply_hue_degrees: hue(REPLY_HUE_KEY, defaults.reply_hue_degrees),
            image_size: saved_size(prefs, IMAGE_SIZE_KEY)
                .map_or(defaults.image_size, SavedSize::media),
            animate_gifs: prefs.get(ANIMATE_GIFS_KEY),
            video_size: saved_size(prefs, VIDEO_SIZE_KEY)
                .map_or(defaults.video_size, SavedSize::media),
            autoplay_videos: prefs.get(AUTOPLAY_VIDEOS_KEY),
            keep_playing: prefs
                .get::<String>(KEEP_PLAYING_KEY)
                .and_then(|saved| KeepPlaying::of_saved(&saved))
                .unwrap_or(defaults.keep_playing),
            embed_size: saved_size(prefs, EMBED_SIZE_KEY)
                .map_or(defaults.embed_size, SavedSize::embed),
            link_previews: switch(LINK_PREVIEWS_KEY, defaults.link_previews),
        };
        ConversationStore {
            stored: Stored::new(settings),
            file: SettingsFile::of(prefs),
        }
    }

    /// The settings that apply now.
    pub(crate) fn settings(&self) -> &ConversationSettings {
        self.stored.shown()
    }

    /// Shows `settings` at once; the settings to write now, unless a write
    /// is running (they then follow it) or nothing changed.
    pub(crate) fn change(
        &mut self,
        settings: ConversationSettings,
    ) -> Option<ConversationSettings> {
        self.stored.change(settings)
    }

    /// The write of `settings` landed (`ok`) or did not, which takes them
    /// back to what the file holds; the settings to write next.
    pub(crate) fn written(
        &mut self,
        settings: ConversationSettings,
        ok: bool,
    ) -> Option<ConversationSettings> {
        self.stored.written(settings, ok)
    }

    /// Sets the settings as if the file held them, for tests.
    #[cfg(test)]
    pub(crate) fn set_for_test(&mut self, change: impl FnOnce(&mut ConversationSettings)) {
        let mut settings = *self.settings();
        change(&mut settings);
        self.stored = Stored::new(settings);
    }

    /// The write of the change waiting behind the running one, if any.
    pub(crate) fn pending_write(&self) -> Option<Write> {
        self.stored
            .pending()
            .map(|settings| self.write_of(settings))
    }

    /// The write that saves `settings`: the keys that differ from what the
    /// file holds (`rune::prefs` rewrites the file once per key).
    pub(crate) fn write_of(&self, settings: &ConversationSettings) -> Write {
        let saved = entries(self.stored.saved());
        let entries = entries(settings)
            .into_iter()
            .filter(|entry| !saved.contains(entry))
            .collect();
        self.file.write(entries)
    }
}

/// Every key of `settings`, with its value.
fn entries(settings: &ConversationSettings) -> Vec<(&'static str, Value)> {
    vec![
        (ENTER_SENDS_KEY, json!(settings.enter_sends)),
        (MENTION_REPLY_KEY, json!(settings.mention_when_replying)),
        (SHOW_TIMES_KEY, json!(settings.show_times)),
        (SHOW_AVATARS_KEY, json!(settings.show_avatars)),
        (OPEN_AT_NEWEST_KEY, json!(settings.open_at_newest)),
        (UNREAD_HOURGLASS_KEY, json!(settings.unread_hourglass)),
        (CHANNEL_DESCRIPTION_KEY, json!(settings.channel_description)),
        (MENTION_HUE_KEY, json!(settings.mention_hue_degrees)),
        (REPLY_HUE_KEY, json!(settings.reply_hue_degrees)),
        (
            IMAGE_SIZE_KEY,
            SavedSize::of_media(settings.image_size).value(),
        ),
        (ANIMATE_GIFS_KEY, json!(settings.animate_gifs)),
        (
            VIDEO_SIZE_KEY,
            SavedSize::of_media(settings.video_size).value(),
        ),
        (AUTOPLAY_VIDEOS_KEY, json!(settings.autoplay_videos)),
        (KEEP_PLAYING_KEY, json!(settings.keep_playing.saved())),
        (
            EMBED_SIZE_KEY,
            SavedSize::of_embed(settings.embed_size).value(),
        ),
        (LINK_PREVIEWS_KEY, json!(settings.link_previews)),
    ]
}

#[cfg(test)]
mod tests {
    use rune::prefs::Prefs;

    use super::{
        ConversationSettings, ConversationStore, EMBED_SIZE_KEY, ENTER_SENDS_KEY, IMAGE_SIZE_KEY,
        MENTION_HUE_KEY, OPEN_AT_NEWEST_KEY, UNREAD_HOURGLASS_KEY,
    };
    use papo_sdk_ui::{EmbedSizeSetting, MediaSizeSetting};

    #[test]
    fn missing_settings_take_the_designs_defaults() {
        let store = ConversationStore::read(&Prefs::memory());
        assert_eq!(*store.settings(), ConversationSettings::default());
        let settings = store.settings();
        assert!(settings.enter_sends && settings.show_times && settings.show_avatars);
        assert!(!settings.open_at_newest && settings.unread_hourglass);
        assert!((settings.mention_hue_degrees - 75.0).abs() < f32::EPSILON);
    }

    #[test]
    fn settings_saved_before_this_page_are_read_as_they_were() {
        let mut prefs = Prefs::memory();
        prefs.set(ENTER_SENDS_KEY, &false).unwrap();
        prefs.set(OPEN_AT_NEWEST_KEY, &true).unwrap();
        prefs.set(UNREAD_HOURGLASS_KEY, "no").unwrap();
        let settings = *ConversationStore::read(&prefs).settings();
        assert!(!settings.enter_sends);
        assert!(settings.open_at_newest);
        assert!(settings.unread_hourglass, "a wrong type is ignored");
    }

    #[test]
    fn a_hue_out_of_range_comes_back_into_it_and_a_bad_one_is_the_default() {
        let mut prefs = Prefs::memory();
        prefs.set(MENTION_HUE_KEY, &400.0).unwrap();
        let hue = ConversationStore::read(&prefs)
            .settings()
            .mention_hue_degrees;
        assert!((hue - 40.0).abs() < 1e-3, "{hue}");
        prefs.set(MENTION_HUE_KEY, "amber").unwrap();
        let hue = ConversationStore::read(&prefs)
            .settings()
            .mention_hue_degrees;
        assert!((hue - 75.0).abs() < f32::EPSILON);
    }

    #[test]
    fn a_change_is_written_and_read_back() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("prefs.json");
        let mut store = ConversationStore::read(&Prefs::at(&path));
        let mut changed = *store.settings();
        changed.show_times = false;
        changed.show_avatars = false;
        changed.mention_hue_degrees = 196.0;
        changed.reply_hue_degrees = 320.0;
        changed.image_size = MediaSizeSetting::custom(504, 304);
        changed.animate_gifs = Some(false);
        changed.video_size = MediaSizeSetting::Large;
        changed.autoplay_videos = Some(true);
        changed.embed_size = EmbedSizeSetting::custom(304, 168);
        changed.link_previews = false;
        let written = store.change(changed).expect("a write");
        store.write_of(&written).run().unwrap();
        assert_eq!(store.written(written, true), None);
        let again = ConversationStore::read(&Prefs::at(&path));
        assert_eq!(*again.settings(), changed);
    }

    #[test]
    fn animate_gifs_follows_reduced_motion_until_changed() {
        let mut settings = ConversationSettings::default();
        assert!(settings.animates_gifs(false));
        assert!(!settings.animates_gifs(true));
        settings.animate_gifs = Some(true);
        assert!(settings.animates_gifs(true));
    }

    #[test]
    fn a_saved_image_size_is_read_and_a_wrong_one_is_medium() {
        let mut prefs = Prefs::memory();
        prefs.set(IMAGE_SIZE_KEY, "large").unwrap();
        assert_eq!(
            ConversationStore::read(&prefs).settings().image_size,
            MediaSizeSetting::Large
        );
        prefs
            .set(IMAGE_SIZE_KEY, &serde_json::json!({ "custom": [9999, 1] }))
            .unwrap();
        assert_eq!(
            ConversationStore::read(&prefs).settings().image_size,
            MediaSizeSetting::custom(720, 200)
        );
        prefs.set(IMAGE_SIZE_KEY, "huge").unwrap();
        assert_eq!(
            ConversationStore::read(&prefs).settings().image_size,
            MediaSizeSetting::Medium
        );
    }

    #[test]
    fn embed_size_is_saved_apart_and_custom_keeps_its_own_heights() {
        let mut prefs = Prefs::memory();
        prefs
            .set(EMBED_SIZE_KEY, &serde_json::json!({ "custom": [9999, 1] }))
            .unwrap();
        let settings = *ConversationStore::read(&prefs).settings();
        assert_eq!(settings.embed_size, EmbedSizeSetting::custom(720, 160));
        assert_eq!(settings.image_size, MediaSizeSetting::Medium);
        assert!(settings.link_previews, "on by default");
    }
}
