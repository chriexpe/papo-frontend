//! Receiver gain follows the server's track-to-user routing, never speaker-list order.
//! A slot is reused by the SFU; every routing update reapplies settings to that slot.
use gst::prelude::*;
use gstreamer as gst;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Playback {
    pub volume_percent: u16,
    pub muted: bool,
}
impl Default for Playback {
    fn default() -> Self {
        Self {
            volume_percent: 100,
            muted: false,
        }
    }
}
#[derive(Default)]
pub(crate) struct AudioMix {
    routes: HashMap<String, String>,
    settings: HashMap<String, Playback>,
    receivers: HashMap<String, gst::Element>,
}
impl AudioMix {
    pub fn route(&mut self, routes: impl IntoIterator<Item = (String, String)>) {
        self.routes = routes.into_iter().collect();
        self.apply();
    }
    pub fn playback(&mut self, user: String, mut playback: Playback) {
        playback.volume_percent = playback.volume_percent.min(200);
        self.settings.insert(user, playback);
        self.apply();
    }
    pub fn attach(&mut self, track: String, volume: gst::Element) {
        self.receivers.insert(track, volume);
        self.apply();
    }
    fn effective(&self, track: &str) -> Playback {
        match self.routes.get(track) {
            Some(user) => self.settings.get(user).copied().unwrap_or_default(),
            // While identity metadata catches up, an unidentified slot must not bypass a mute.
            None if self
                .settings
                .values()
                .any(|value| *value != Playback::default()) =>
            {
                Playback {
                    muted: true,
                    ..Default::default()
                }
            }
            None => Playback::default(),
        }
    }
    fn apply(&self) {
        for (track, volume) in &self.receivers {
            let setting = self.effective(track);
            volume.set_property("volume", f64::from(setting.volume_percent) / 100.0);
            volume.set_property("mute", setting.muted);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reused_slot_follows_its_new_owner_and_retains_independent_mute_gain() {
        let mut mix = AudioMix::default();
        mix.playback(
            "ana".into(),
            Playback {
                volume_percent: 175,
                muted: true,
            },
        );
        mix.route([("papo-audio-0".into(), "ana".into())]);
        assert_eq!(
            mix.effective("papo-audio-0"),
            Playback {
                volume_percent: 175,
                muted: true
            }
        );
        mix.route([("papo-audio-0".into(), "bruno".into())]);
        assert_eq!(mix.effective("papo-audio-0"), Playback::default());
        mix.route([("papo-audio-0".into(), "ana".into())]);
        assert!(mix.effective("papo-audio-0").muted);
    }
    #[test]
    fn unknown_tracks_cannot_bypass_overrides_and_gain_is_bounded() {
        let mut mix = AudioMix::default();
        mix.playback(
            "ana".into(),
            Playback {
                volume_percent: 600,
                muted: false,
            },
        );
        assert!(mix.effective("unknown").muted);
        mix.route([("slot".into(), "ana".into())]);
        assert_eq!(mix.effective("slot").volume_percent, 200);
    }
    #[test]
    fn actual_volume_element_receives_gain_and_mute_when_route_changes() {
        gst::init().unwrap();
        let volume = gst::ElementFactory::make("volume").build().unwrap();
        let mut mix = AudioMix::default();
        mix.attach("slot".into(), volume.clone());
        mix.playback(
            "ana".into(),
            Playback {
                volume_percent: 150,
                muted: true,
            },
        );
        mix.route([("slot".into(), "ana".into())]);
        assert_eq!(volume.property::<f64>("volume"), 1.5);
        assert!(volume.property::<bool>("mute"));
        mix.route([("slot".into(), "bruno".into())]);
        assert_eq!(volume.property::<f64>("volume"), 1.0);
        assert!(!volume.property::<bool>("mute"));
    }
}
