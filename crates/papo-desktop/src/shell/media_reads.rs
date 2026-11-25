//! The one media read at a time (`docs/papo-desktop.md` §5): a video's probe and an audio's
//! read each fetch a whole original through the SDK, which runs two fetches at once, and decode
//! it. One at a time leaves the other fetch for pictures. Videos go first, since their first
//! frame is what a row shows; then audio, in the order it shows.
//!
//! Owns the slot and the order; what a read does is `videos`' and `audio`'s.

use std::collections::VecDeque;

use super::Shell;
use crate::video_store::ProbeJob;

/// The longest a read (its fetch, up to 100 MiB, then its probe or decode) may take before it
/// fails with Retry and the next one starts.
pub(super) const READ_DEADLINE: std::time::Duration = std::time::Duration::from_secs(120);

/// The read slot.
#[derive(Default)]
pub(crate) struct MediaReads {
    /// A read is running.
    busy: bool,
    /// Video probes waiting for the slot.
    videos: VecDeque<ProbeJob>,
}

impl Shell {
    /// Video probes to run when the slot frees.
    pub(super) fn queue_video_probes(&mut self, jobs: Vec<ProbeJob>) {
        self.media_reads.videos.extend(jobs);
        self.pump_media_reads();
    }

    /// The running read ended: the next one starts.
    pub(super) fn media_read_done(&mut self) {
        self.media_reads.busy = false;
        self.pump_media_reads();
    }

    /// Starts the next read when the slot is free: a video's probe, else an audio's read.
    pub(super) fn pump_media_reads(&mut self) {
        if self.media_reads.busy {
            return;
        }
        while let Some(job) = self.media_reads.videos.pop_front() {
            if self.start_video_probe(job) {
                self.media_reads.busy = true;
                return;
            }
        }
        if self.sdk.is_none() {
            return;
        }
        if let Some(job) = self.audio.store.next_read() {
            self.start_audio_read(job);
            self.media_reads.busy = true;
        }
    }
}
