//! Connects received Opus tracks to one communication output, with per-route gain before mixing.

use crate::{
    audio::AudioMix,
    mic::pcm_caps,
    pipeline::{Branch, MediaResult, RequestedPad, element, filter, queue},
};
use gst::prelude::*;
use gstreamer as gst;
use papo_sdk::{AudioRoute, PlaybackSettings, Uuid};
use std::collections::HashMap;

pub(crate) struct ReceiveAudio {
    branches: HashMap<gst::Pad, (String, Branch)>,
    mix: AudioMix,
    mixer: gst::Element,
    revision: u64,
    _output: Branch,
}

impl ReceiveAudio {
    pub fn new(graph: &gst::Pipeline, sink: gst::Element) -> MediaResult<Self> {
        let mixer = element("audiomixer")?;
        mixer.set_property("ignore-inactive-pads", true);
        let output = Branch::attach(
            graph,
            vec![
                mixer.clone(),
                element("audioconvert")?,
                element("audioresample")?,
                queue(0, 120_000_000)?,
                sink,
            ],
            None,
            None,
        )?;
        Ok(Self {
            branches: HashMap::new(),
            mix: AudioMix::default(),
            mixer,
            revision: 0,
            _output: output,
        })
    }

    pub fn attach(
        &mut self,
        graph: &gst::Pipeline,
        source: &gst::Pad,
        track: String,
    ) -> MediaResult<()> {
        let decoder = element("opusdec")?;
        decoder.set_property("plc", true);
        decoder.set_property("use-inband-fec", true);
        let volume = element("volume")?;
        self.mix.attach(track.clone(), volume.clone());
        let result = Branch::attach(
            graph,
            vec![
                queue(0, 120_000_000)?,
                element("rtpopusdepay")?,
                decoder,
                element("audioconvert")?,
                element("audioresample")?,
                filter(&pcm_caps())?,
                volume,
            ],
            Some(source),
            Some(RequestedPad::new(&self.mixer, "sink_%u")?),
        );
        match result {
            Ok(branch) => {
                self.branches.insert(source.clone(), (track, branch));
            }
            Err(error) => {
                self.mix.detach(&track);
                return Err(error);
            }
        }
        Ok(())
    }

    pub fn detach(&mut self, pad: &gst::Pad) {
        if let Some((track, _)) = self.branches.remove(pad) {
            self.mix.detach(&track);
        }
    }

    pub fn route(&mut self, revision: u64, routes: Vec<AudioRoute>) {
        if revision >= self.revision {
            self.revision = revision;
            self.mix
                .route(routes.into_iter().map(|route| (route.track_id, route.user)));
        }
    }

    pub fn playback(&mut self, user: Uuid, settings: PlaybackSettings) {
        self.mix.playback(user, settings);
    }

    pub fn drain(&mut self) {
        self.branches.clear();
    }
}
