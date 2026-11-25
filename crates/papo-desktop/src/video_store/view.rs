//! What views read of videos: each one's reading, whether it plays, with sound or not, and
//! where it is. A cheap snapshot; the rules are `video_store`'s.

use std::collections::HashMap;
use std::sync::Arc;

use super::Reading;
use crate::media_store::MediaKey;

/// One video as views draw it.
#[expect(
    clippy::struct_excessive_bools,
    reason = "each is one of the video's independent conditions"
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct VideoShown {
    pub(crate) reading: Reading,
    /// It plays now.
    pub(crate) playing: bool,
    /// Its sound is off (an autoplay always).
    pub(crate) muted: bool,
    /// The person pressed it: its controls stay as they left them, not autoplay's marks.
    pub(crate) pressed: bool,
    /// It reached its end with sound.
    pub(crate) ended: bool,
    /// Where it is, in milliseconds.
    pub(crate) position_ms: u64,
}

impl VideoShown {
    /// How long it plays, once read.
    pub(crate) fn duration_ms(&self) -> Option<u64> {
        match self.reading {
            Reading::Read(info) => info.duration_ms,
            _ => None,
        }
    }
}

/// Every video views may draw.
#[derive(Clone, Default)]
pub(crate) struct VideoView {
    shown: Arc<HashMap<MediaKey, VideoShown>>,
}

impl VideoView {
    pub(super) fn of(shown: impl Iterator<Item = (MediaKey, VideoShown)>) -> VideoView {
        VideoView {
            shown: Arc::new(shown.collect()),
        }
    }

    /// Video `key`, once a view wanted it.
    pub(crate) fn get(&self, key: MediaKey) -> Option<&VideoShown> {
        self.shown.get(&key)
    }

    /// Live players' positions over what the store remembers.
    pub(crate) fn at_positions(mut self, positions: &HashMap<MediaKey, u64>) -> VideoView {
        if positions.is_empty() {
            return self;
        }
        let shown = Arc::make_mut(&mut self.shown);
        for (key, position_ms) in positions {
            if let Some(video) = shown.get_mut(key) {
                video.position_ms = *position_ms;
            }
        }
        self
    }
}
