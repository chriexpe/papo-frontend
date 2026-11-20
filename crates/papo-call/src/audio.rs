//! Receiver gain follows the server's track-to-user routing, never speaker-list order.
//! A slot is reused by the SFU; every routing update reapplies settings to that slot.
use gst::prelude::*;
use gstreamer as gst;
use papo_sdk::{PlaybackSettings, Uuid};
use std::collections::HashMap;

#[derive(Default)]
pub(crate) struct AudioMix {
    routes: HashMap<String, Uuid>,
    settings: HashMap<Uuid, PlaybackSettings>,
    receivers: HashMap<String, gst::Element>,
}
impl AudioMix {
    pub fn route(&mut self, routes: impl IntoIterator<Item = (String, Uuid)>) {
        self.routes = routes.into_iter().collect();
        self.apply();
    }
    pub fn playback(&mut self, user: Uuid, playback: PlaybackSettings) {
        let playback = PlaybackSettings::new(playback.gain_percent, playback.muted);
        self.settings.insert(user, playback);
        self.apply();
    }
    pub fn attach(&mut self, track: String, volume: gst::Element) {
        self.receivers.insert(track, volume);
        self.apply();
    }
    pub fn detach(&mut self, track: &str) {
        self.receivers.remove(track);
    }

    fn effective(&self, track: &str) -> PlaybackSettings {
        match self.routes.get(track) {
            Some(user) => self.settings.get(user).copied().unwrap_or_default(),
            // While identity metadata catches up, an unidentified slot must not bypass a mute.
            None if self
                .settings
                .values()
                .any(|value| *value != PlaybackSettings::default()) =>
            {
                PlaybackSettings {
                    muted: true,
                    ..Default::default()
                }
            }
            None => PlaybackSettings::default(),
        }
    }
    fn apply(&self) {
        for (track, volume) in &self.receivers {
            let setting = self.effective(track);
            volume.set_property("volume", f64::from(setting.gain_percent) / 100.0);
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
            Uuid::from_u128(1),
            PlaybackSettings {
                gain_percent: 175,
                muted: true,
            },
        );
        mix.route([("papo-audio-0".into(), Uuid::from_u128(1))]);
        assert_eq!(
            mix.effective("papo-audio-0"),
            PlaybackSettings {
                gain_percent: 175,
                muted: true
            }
        );
        mix.route([("papo-audio-0".into(), Uuid::from_u128(2))]);
        assert_eq!(mix.effective("papo-audio-0"), PlaybackSettings::default());
        mix.route([("papo-audio-0".into(), Uuid::from_u128(1))]);
        assert!(mix.effective("papo-audio-0").muted);
    }
    #[test]
    fn unknown_tracks_cannot_bypass_overrides_and_gain_is_bounded() {
        let mut mix = AudioMix::default();
        mix.playback(
            Uuid::from_u128(1),
            PlaybackSettings {
                gain_percent: 600,
                muted: false,
            },
        );
        assert!(mix.effective("unknown").muted);
        mix.route([("slot".into(), Uuid::from_u128(1))]);
        assert_eq!(mix.effective("slot").gain_percent, 200);
    }
    #[test]
    fn actual_volume_element_receives_gain_and_mute_when_route_changes() {
        gst::init().unwrap();
        let volume = gst::ElementFactory::make("volume").build().unwrap();
        let mut mix = AudioMix::default();
        mix.attach("slot".into(), volume.clone());
        mix.playback(
            Uuid::from_u128(1),
            PlaybackSettings {
                gain_percent: 150,
                muted: true,
            },
        );
        mix.route([("slot".into(), Uuid::from_u128(1))]);
        assert_eq!(volume.property::<f64>("volume"), 1.5);
        assert!(volume.property::<bool>("mute"));
        mix.route([("slot".into(), Uuid::from_u128(2))]);
        assert_eq!(volume.property::<f64>("volume"), 1.0);
        assert!(!volume.property::<bool>("mute"));
    }
}
