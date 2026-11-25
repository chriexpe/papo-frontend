//! Keep playing when hidden (Settings › Conversations › Media; Christian, 2026-11-25): which
//! sound a person started goes on while the window is off screen (minimised, hidden, covered
//! whole). Muted autoplays always stop then. Owns the value and its saved form; the stores apply
//! it (`video_store`, `audio_store`).

/// Which sound keeps playing while the window is off screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum KeepPlaying {
    /// Everything pauses.
    Nothing,
    /// An audio player goes on; a video with sound pauses.
    #[default]
    Audio,
    /// A video with sound goes on; an audio player pauses.
    Video,
    /// Both go on.
    Both,
}

impl KeepPlaying {
    /// Every choice, in the menu's order.
    pub(crate) const ALL: [KeepPlaying; 4] = [
        KeepPlaying::Nothing,
        KeepPlaying::Audio,
        KeepPlaying::Video,
        KeepPlaying::Both,
    ];

    /// Whether an audio player keeps playing.
    pub(crate) fn audio(self) -> bool {
        matches!(self, KeepPlaying::Audio | KeepPlaying::Both)
    }

    /// Whether a video playing with sound keeps playing.
    pub(crate) fn video(self) -> bool {
        matches!(self, KeepPlaying::Video | KeepPlaying::Both)
    }

    /// How it is saved.
    pub(crate) fn saved(self) -> &'static str {
        match self {
            KeepPlaying::Nothing => "nothing",
            KeepPlaying::Audio => "audio",
            KeepPlaying::Video => "video",
            KeepPlaying::Both => "both",
        }
    }

    /// The choice saved as `saved`, when it is one.
    pub(crate) fn of_saved(saved: &str) -> Option<KeepPlaying> {
        KeepPlaying::ALL
            .into_iter()
            .find(|choice| choice.saved() == saved)
    }
}

#[cfg(test)]
mod tests {
    use super::KeepPlaying;

    #[test]
    fn each_choice_reads_back_as_saved() {
        for choice in KeepPlaying::ALL {
            assert_eq!(KeepPlaying::of_saved(choice.saved()), Some(choice));
        }
        assert_eq!(KeepPlaying::of_saved("radio"), None);
    }

    #[test]
    fn audio_keeps_playing_unless_asked_otherwise() {
        assert!(KeepPlaying::default().audio());
        assert!(!KeepPlaying::default().video());
        assert!(KeepPlaying::Both.audio() && KeepPlaying::Both.video());
        assert!(!KeepPlaying::Nothing.audio() && !KeepPlaying::Nothing.video());
    }
}
