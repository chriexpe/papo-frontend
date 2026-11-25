//! Reading audio (`docs/design.md` §14.5; Open, "Waveform peaks"): an audio near the view is
//! fetched and read in `media_reads`' slot, its length and, up to ten minutes, its waveform;
//! long audio decodes its waveform while it plays. Finished waveforms go to the SDK's
//! `WaveformCache`, so an audio read again needs only its length.
//!
//! A read is let go when its audio leaves the view: its fetch is cancelled, its decode stopped,
//! and the slot goes on. A read that runs out of time fails (Retry), so one slow audio never
//! holds the slot again and again. Each decode is numbered: a late answer from a stopped one is
//! dropped.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use papo_sdk::MediaSource;
use papo_sdk_ui::{Waveform, WaveformKey};
use tokio::sync::Notify;

use super::Shell;
use crate::audio_store::ReadJob;
use crate::media_store::MediaKey;
use crate::playback::{AudioRead, Engine, PlaybackFailure};

/// A fetch and decode running off the UI thread, and how to end it early.
pub(crate) struct Running {
    pub(super) key: MediaKey,
    /// Numbers it among the decodes (`Audio::serial`).
    serial: u64,
    /// Read by the decode between buffers.
    stop: Arc<AtomicBool>,
    /// Wakes the fetch, which then gives up.
    cancel: Arc<Notify>,
}

impl Running {
    pub(super) fn new(key: MediaKey, serial: u64) -> Running {
        Running {
            key,
            serial,
            stop: Arc::new(AtomicBool::new(false)),
            cancel: Arc::new(Notify::new()),
        }
    }

    /// Ends it early: a fetch still running gives up, a decode stops at its next buffer.
    pub(super) fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        // A permit is kept when nothing waits yet: a fetch starting later gives up at once.
        self.cancel.notify_one();
    }

    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }
}

/// A long audio's waveform decode to run once its bytes are read.
pub(super) struct Fill {
    key: MediaKey,
    serial: u64,
    stop: Arc<AtomicBool>,
}

/// The SDK cache's key for audio `key`.
fn cache_key(key: MediaKey) -> WaveformKey {
    WaveformKey {
        account: key.account,
        attachment: key.attachment,
    }
}

impl Shell {
    /// Starts `job` (the read slot is its): it fetches the audio through the SDK's cache and
    /// reads it off the UI thread, then lands in the store. A waveform the SDK's cache holds is
    /// used, and only the length is read.
    pub(super) fn start_audio_read(&mut self, job: ReadJob) {
        let Some(sdk) = self.sdk.clone() else {
            return;
        };
        let cached = self.audio.waveforms.get(cache_key(job.key));
        let ahead_max_ms = if cached.is_some() {
            Some(0)
        } else {
            job.ahead_max_ms
        };
        if let Some(waveform) = cached {
            self.audio.store.filled(job.key, waveform);
        }
        self.audio.serial += 1;
        let running = Running::new(job.key, self.audio.serial);
        let stop = Arc::clone(&running.stop);
        let cancel = Arc::clone(&running.cancel);
        self.audio.reading = Some(running);
        let engine = self.videos.engine();
        let updates = sdk.updates();
        sdk.run(move |client| async move {
            let reading_stop = Arc::clone(&stop);
            let read = async move {
                let fetch = async {
                    match client.session(job.key.account) {
                        Ok(session) => session
                            .media()
                            .fetch(MediaSource::Attachment(job.key.attachment), false)
                            .await
                            .ok()
                            .map(|data| data.value.bytes),
                        Err(_) => None,
                    }
                };
                let bytes = tokio::select! {
                    bytes = fetch => bytes,
                    () = cancel.notified() => {
                        return Ok(AudioRead {
                            duration_ms: None,
                            waveform: None,
                        });
                    }
                };
                let Some(bytes) = bytes else {
                    return Err(PlaybackFailure::Broken);
                };
                tokio::task::spawn_blocking(move || {
                    engine.read_audio(bytes, ahead_max_ms, &|_| {}, &reading_stop)
                })
                .await
                .unwrap_or(Err(PlaybackFailure::Unavailable))
            };
            let timed = tokio::time::timeout(super::media_reads::READ_DEADLINE, read).await;
            let outcome = timed.unwrap_or_else(|_| {
                // Its decode ends at the flag; the audio is shown as failed, with Retry.
                stop.store(true, Ordering::Relaxed);
                Err(PlaybackFailure::Broken)
            });
            updates.send(move |shell| {
                shell.audio_read(job.key, outcome);
                true
            });
        });
    }

    /// Audio `key`'s read ended. One let go because it left the view waits for its turn
    /// again, unless it learned what stands anyway (its whole waveform, or that nothing here
    /// plays its format); a read that ran out of time while wanted fails.
    pub(super) fn audio_read(
        &mut self,
        key: MediaKey,
        outcome: Result<AudioRead, PlaybackFailure>,
    ) {
        let running = self.audio.reading.take_if(|reading| reading.key == key);
        let stopped = running.is_some_and(|reading| reading.stopped());
        let kept = matches!(
            &outcome,
            Ok(AudioRead {
                waveform: Some(_),
                ..
            }) | Err(PlaybackFailure::Unsupported)
        );
        if stopped && !self.audio.store.is_wanted(key) && !kept {
            self.audio.store.read_dropped(key);
        } else {
            if let Ok(AudioRead {
                waveform: Some(waveform),
                ..
            }) = &outcome
            {
                self.audio
                    .waveforms
                    .insert(cache_key(key), waveform.clone());
            }
            self.audio.store.read(key, outcome);
        }
        self.media_read_done();
        self.reconcile_audio();
    }

    /// Audio `key` is opening to play: when it lacks its finished waveform, a decode of it
    /// (stopping another audio's) to run alongside. None when it has it, or one already runs.
    pub(super) fn start_fill(&mut self, key: MediaKey) -> Option<Fill> {
        if !self.audio.store.needs_waveform(key) {
            return None;
        }
        if let Some(filling) = &self.audio.filling {
            if filling.key == key {
                return None;
            }
            filling.stop();
        }
        self.audio.serial += 1;
        let running = Running::new(key, self.audio.serial);
        let fill = Fill {
            key,
            serial: running.serial,
            stop: Arc::clone(&running.stop),
        };
        self.audio.filling = Some(running);
        Some(fill)
    }

    /// A snapshot of fill `serial`'s waveform. Returns whether views change.
    fn audio_filled(&mut self, key: MediaKey, serial: u64, waveform: Waveform) -> bool {
        if !self.filling_is(key, serial) {
            return false;
        }
        self.audio.store.filled(key, waveform);
        true
    }

    /// Fill `serial` ended: its whole waveform (and the length it decoded) when it got there.
    fn audio_fill_done(&mut self, key: MediaKey, serial: u64, read: Option<AudioRead>) -> bool {
        if !self.filling_is(key, serial) {
            return false;
        }
        self.audio.filling = None;
        let Some(read) = read else {
            return false;
        };
        if let Some(waveform) = &read.waveform {
            self.audio
                .waveforms
                .insert(cache_key(key), waveform.clone());
        }
        self.audio.store.fill_done(key, read);
        true
    }

    fn filling_is(&self, key: MediaKey, serial: u64) -> bool {
        self.audio
            .filling
            .as_ref()
            .is_some_and(|filling| filling.key == key && filling.serial == serial)
    }
}

/// Runs `fill` on `bytes` on a blocking worker, handing each snapshot and the end to the shell.
pub(super) fn fill_waveform(
    engine: &Arc<dyn Engine>,
    bytes: Arc<[u8]>,
    fill: Fill,
    updates: &crate::sdk_bridge::Updates<Shell>,
) {
    let engine = Arc::clone(engine);
    let progress_updates = updates.clone();
    let done_updates = updates.clone();
    let Fill { key, serial, stop } = fill;
    // Not awaited: the player opens meanwhile.
    drop(tokio::task::spawn_blocking(move || {
        let progress = move |waveform| {
            progress_updates
                .send(move |shell: &mut Shell| shell.audio_filled(key, serial, waveform));
        };
        let read = engine
            .read_audio(bytes, None, &progress, &stop)
            .ok()
            .filter(|read| read.waveform.is_some());
        done_updates.send(move |shell: &mut Shell| shell.audio_fill_done(key, serial, read));
    }));
}
