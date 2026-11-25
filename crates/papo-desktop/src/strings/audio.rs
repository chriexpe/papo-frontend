//! Audio's words (`docs/design-strings-d7.md`, In the conversation, The media menu): the
//! player's controls, an audio's spoken name, Save audio, and Keep playing when hidden (Settings
//! › Conversations › Media).

use super::{I18N, Key, catalog};

/// An audio, spoken: "Audio, demo-faixa-3-mix.flac".
pub(crate) fn media_audio(name: &str) -> String {
    template!("media.audio", "name" => name)
}

/// Play.
pub(crate) fn audio_play() -> &'static str {
    text!("audio.play")
}

/// Pause.
pub(crate) fn audio_pause() -> &'static str {
    text!("audio.pause")
}

/// The waveform as a slider, spoken: "Position, 0:12 of 3:12".
pub(crate) fn audio_seek(current_ms: u64, total_ms: Option<u64>) -> String {
    template!(
        "audio.seek",
        "current" => super::clock(current_ms),
        "total" => total_ms.map(super::clock).unwrap_or_default()
    )
}

/// The media menu's Save audio.
pub(crate) fn menu_save_audio() -> &'static str {
    text!("menu.save_audio")
}

/// Keep playing when hidden.
pub(crate) fn settings_keep_playing() -> &'static str {
    text!("settings.keep_playing")
}

/// Keep playing when hidden's summary.
pub(crate) fn settings_keep_playing_sub() -> &'static str {
    text!("settings.keep_playing_sub")
}

/// Keep playing when hidden: Nothing.
pub(crate) fn settings_keep_nothing() -> &'static str {
    text!("settings.keep_nothing")
}

/// Keep playing when hidden: Audio.
pub(crate) fn settings_keep_audio() -> &'static str {
    text!("settings.keep_audio")
}

/// Keep playing when hidden: Video.
pub(crate) fn settings_keep_video() -> &'static str {
    text!("settings.keep_video")
}

/// Keep playing when hidden: Audio and video.
pub(crate) fn settings_keep_both() -> &'static str {
    text!("settings.keep_both")
}
