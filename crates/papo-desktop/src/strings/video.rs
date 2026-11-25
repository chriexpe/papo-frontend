//! Video's words (`docs/design-strings-d7.md`, In the conversation, The viewer, Settings): the
//! player's controls and times, a video's spoken name, its failures, Video size and Autoplay.

use super::{I18N, Key, catalog};

/// A time on a player's clock: "0:12", "12:48", "1:02:03".
pub(crate) fn clock(ms: u64) -> String {
    let seconds = ms / 1000;
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

/// A video, spoken: "Video, 1:04", with ", muted" while it plays without sound.
pub(crate) fn media_video(duration_ms: Option<u64>, muted: bool) -> String {
    let duration = duration_ms.map(clock).unwrap_or_default();
    if muted {
        template!("media.video_muted", "duration" => duration)
    } else {
        template!("media.video", "duration" => duration)
    }
}

/// Play.
pub(crate) fn video_play() -> &'static str {
    text!("video.play")
}

/// Pause.
pub(crate) fn video_pause() -> &'static str {
    text!("video.pause")
}

/// Turn sound on.
pub(crate) fn video_unmute() -> &'static str {
    text!("video.unmute")
}

/// Turn sound off.
pub(crate) fn video_mute() -> &'static str {
    text!("video.mute")
}

/// The volume slider.
pub(crate) fn video_volume() -> &'static str {
    text!("video.volume")
}

/// The seek bar.
pub(crate) fn video_seek() -> &'static str {
    text!("video.seek")
}

/// The control bar's time: "0:12 / 1:04".
pub(crate) fn video_time(current_ms: u64, total_ms: Option<u64>) -> String {
    let total = total_ms.map(clock).unwrap_or_default();
    template!("video.time", "current" => clock(current_ms), "total" => total)
}

/// The control bar's full screen button: it opens the viewer.
pub(crate) fn video_fullscreen() -> &'static str {
    text!("video.fullscreen")
}

/// A video that can't play.
pub(crate) fn video_failed() -> &'static str {
    text!("video.failed")
}

/// A video whose format can't play here, as what happened and the pressable "Save".
pub(crate) fn video_unsupported() -> (String, String) {
    super::media::split_action(text!("video.unsupported"))
}

/// The media menu's Save video.
pub(crate) fn menu_save_video() -> &'static str {
    text!("menu.save_video")
}

/// The viewer on a video, spoken: "onda.mp4, video 4 of 5", or "onda.mp4, video" alone.
pub(crate) fn viewer_label_video(name: &str, place: usize, count: usize) -> String {
    if count < 2 {
        return template!("viewer.label_video_one", "name" => name);
    }
    let i = super::integer(i64::try_from(place).unwrap_or(i64::MAX));
    let n = super::integer(i64::try_from(count).unwrap_or(i64::MAX));
    template!("viewer.label_video", "name" => name, "i" => i, "n" => n)
}

/// The shortcuts sheet's video rows: on a focused message, and in the viewer.
pub(crate) fn shortcuts_video_play() -> &'static str {
    text!("shortcuts.video_play")
}

/// Its video's sound on / off.
pub(crate) fn shortcuts_video_sound() -> &'static str {
    text!("shortcuts.video_sound")
}

/// Its video in full screen.
pub(crate) fn shortcuts_video_full_screen() -> &'static str {
    text!("shortcuts.video_full_screen")
}

/// The viewer's Play / pause.
pub(crate) fn shortcuts_play_pause() -> &'static str {
    text!("shortcuts.play_pause")
}

/// The viewer's Back / forward 10 s.
pub(crate) fn shortcuts_seek_10() -> &'static str {
    text!("shortcuts.seek_10")
}

/// The viewer's Sound on / off.
pub(crate) fn shortcuts_sound() -> &'static str {
    text!("shortcuts.sound")
}

/// Video size.
pub(crate) fn settings_video_size() -> &'static str {
    text!("settings.video_size")
}

/// Autoplay videos.
pub(crate) fn settings_autoplay() -> &'static str {
    text!("settings.autoplay")
}

/// Autoplay videos' summary.
pub(crate) fn settings_autoplay_sub() -> &'static str {
    text!("settings.autoplay_sub")
}

#[cfg(test)]
mod tests {
    use super::clock;

    #[test]
    fn clocks_read_as_players_write_them() {
        assert_eq!(clock(0), "0:00");
        assert_eq!(clock(12_400), "0:12");
        assert_eq!(clock(64_000), "1:04");
        assert_eq!(clock(768_000), "12:48");
        assert_eq!(clock(3_723_000), "1:02:03");
    }
}
