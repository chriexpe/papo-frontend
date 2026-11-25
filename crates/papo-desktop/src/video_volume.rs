//! The volume videos play at (`docs/design.md` §14.5: "The volume is remembered on the
//! device"), 0 to 1, in this device's settings file through `rune::prefs`: read at launch,
//! saved on each change. Muting is apart: it never changes the volume.
//!
//! Owns only the value; the players take it from `shell::videos`.

use rune::prefs::Prefs;

use crate::device_settings::{Remembered, SettingsFile, Write};

/// The videos' volume.
const VOLUME_KEY: &str = "video_volume";

/// The remembered volume, and where it is saved.
pub(crate) struct VideoVolume {
    volume: f32,
    remembered: Remembered,
}

impl Default for VideoVolume {
    /// Full, kept nowhere: before the settings file is read, and in tests.
    fn default() -> VideoVolume {
        VideoVolume::read(&Prefs::memory())
    }
}

impl VideoVolume {
    /// The volume in `prefs`, this device's settings file; a missing or unreadable value is
    /// full.
    pub(crate) fn read(prefs: &Prefs) -> VideoVolume {
        let volume = prefs
            .get::<f32>(VOLUME_KEY)
            .filter(|volume| volume.is_finite())
            .map_or(1.0, |volume| volume.clamp(0.0, 1.0));
        VideoVolume {
            volume,
            remembered: Remembered::new(VOLUME_KEY, SettingsFile::of(prefs)),
        }
    }

    /// 0 to 1.
    pub(crate) fn get(&self) -> f32 {
        self.volume
    }

    /// Sets the volume (clamped); the write that saves it, unless one is running (it then
    /// follows). A failed save keeps it for this run.
    pub(crate) fn set(&mut self, volume: f32) -> Option<Write> {
        self.volume = if volume.is_finite() {
            volume.clamp(0.0, 1.0)
        } else {
            self.volume
        };
        self.remembered
            .remember(serde_json::Value::from(f64::from(self.volume)))
    }

    /// The write waiting behind the running one, if any.
    pub(crate) fn pending_write(&self) -> Option<Write> {
        self.remembered.pending()
    }

    /// The running write landed, or did not; the one to run next.
    pub(crate) fn written(&mut self, ok: bool) -> Option<Write> {
        self.remembered.written(ok)
    }
}

#[cfg(test)]
mod tests {
    use rune::prefs::Prefs;

    use super::{VOLUME_KEY, VideoVolume};

    #[test]
    fn the_volume_is_full_until_set_and_then_remembered() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("prefs.json");
        let mut volume = VideoVolume::read(&Prefs::at(&path));
        assert!((volume.get() - 1.0).abs() < f32::EPSILON);
        volume.set(0.4).expect("a write").run().unwrap();
        assert_eq!(volume.written(true), None);
        let read = VideoVolume::read(&Prefs::at(&path));
        assert!((read.get() - 0.4).abs() < 1e-6);
        assert!(Prefs::at(&path).get::<f32>(VOLUME_KEY).is_some());
    }

    #[test]
    fn a_volume_out_of_range_comes_back_into_it() {
        let mut volume = VideoVolume::default();
        let _ = volume.set(3.0);
        assert!((volume.get() - 1.0).abs() < f32::EPSILON);
        let _ = volume.set(f32::NAN);
        assert!((volume.get() - 1.0).abs() < f32::EPSILON);
    }
}
