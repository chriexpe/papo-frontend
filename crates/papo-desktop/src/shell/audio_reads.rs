//! Reading audio (`docs/design.md` §14.5; Open, "Waveform peaks"): an audio near the view is
//! fetched (or, unsent, read from staging, `audio_sources`) and read in `media_reads`' slot,
//! its length and, up to ten minutes, its waveform; long audio decodes its waveform while it
//! plays. A received audio's finished waveform goes to the SDK's `WaveformCache`, so an audio
//! read again needs only its length.
//!
//! A read is let go when its audio leaves the view: its fetch is cancelled, its decode stopped,
//! and the slot goes on. A read that runs out of time fails (Retry), so one slow audio never
//! holds the slot again and again. Each decode is numbered and answers by its number, so a late
//! answer from a stopped one is dropped, and one whose audio was sent meanwhile lands under the
//! server's attachment.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use papo_sdk_ui::{Waveform, WaveformKey};
use tokio::sync::Notify;

use super::Shell;
use crate::audio_store::{AudioKey, ReadJob};
use crate::playback::{AudioRead, Engine, PlaybackFailure};

/// A fetch and decode running off the UI thread, and how to end it early.
pub(crate) struct Running {
    /// Its audio; moves to the server's attachment when the audio is sent.
    pub(super) key: AudioKey,
    /// Numbers it among the decodes (`Audio::serial`).
    serial: u64,
    /// Read by the decode between buffers; also set when it runs out of time.
    stop: Arc<AtomicBool>,
    /// Wakes the fetch, which then gives up.
    cancel: Arc<Notify>,
    /// It was let go (`stop`): what it answers is not what the audio is.
    cancelled: AtomicBool,
}

impl Running {
    pub(super) fn new(key: AudioKey, serial: u64) -> Running {
        Running {
            key,
            serial,
            stop: Arc::new(AtomicBool::new(false)),
            cancel: Arc::new(Notify::new()),
            cancelled: AtomicBool::new(false),
        }
    }

    /// Ends it early: a fetch still running gives up, a decode stops at its next buffer.
    pub(super) fn stop(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
        self.stop.store(true, Ordering::Relaxed);
        // A permit is kept when nothing waits yet: a fetch starting later gives up at once.
        self.cancel.notify_one();
    }

    fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
}

/// A long audio's waveform decode to run once its bytes are read.
pub(super) struct Fill {
    serial: u64,
    stop: Arc<AtomicBool>,
}

/// The SDK cache's key for audio `key`: only a received audio has one.
pub(super) fn cache_key(key: AudioKey) -> Option<WaveformKey> {
    match key {
        AudioKey::Received(key) => Some(WaveformKey {
            account: key.account,
            attachment: key.attachment,
        }),
        AudioKey::Unsent { .. } => None,
    }
}

impl Shell {
    /// Starts `job`, which takes the read slot: it fetches the audio through the SDK's cache
    /// (an unsent one from staging) and reads it off the UI thread, then lands in the store. A
    /// waveform the SDK's cache holds is used, and only the length is read. Returns whether it
    /// started; an audio with nothing to read from fails at once (the pump checks the SDK
    /// first).
    pub(super) fn start_audio_read(&mut self, job: ReadJob) -> bool {
        let Some(sdk) = self.sdk.clone() else {
            return false;
        };
        let Some(source) = self.audio_source(job.key) else {
            self.audio
                .store
                .read(job.key, Err(PlaybackFailure::Unavailable));
            return false;
        };
        let cached = cache_key(job.key).and_then(|key| self.audio.waveforms.get(key));
        let ahead_max_ms = if cached.is_some() {
            Some(0)
        } else {
            job.ahead_max_ms
        };
        if let Some(waveform) = cached {
            self.audio.store.filled(job.key, waveform);
        }
        self.audio.serial += 1;
        let serial = self.audio.serial;
        let running = Running::new(job.key, serial);
        let stop = Arc::clone(&running.stop);
        let cancel = Arc::clone(&running.cancel);
        self.audio.reading = Some(running);
        let engine = self.videos.engine();
        let updates = sdk.updates();
        sdk.run(move |client| async move {
            let reading_stop = Arc::clone(&stop);
            let read = async move {
                let fetch = source.bytes(&client, false);
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
                shell.audio_read(serial, outcome);
                true
            });
        });
        true
    }

    /// Read `serial` ended. One let go (it left the view, or its audio was sent) waits for its
    /// turn again, unless it learned what stands anyway (its whole waveform, or that nothing
    /// here plays its format); a read that ran out of time fails.
    pub(super) fn audio_read(&mut self, serial: u64, outcome: Result<AudioRead, PlaybackFailure>) {
        let Some(running) = self
            .audio
            .reading
            .take_if(|reading| reading.serial == serial)
        else {
            return;
        };
        let key = running.key;
        let cancelled = running.cancelled();
        let kept = matches!(
            &outcome,
            Ok(AudioRead {
                waveform: Some(_),
                ..
            }) | Err(PlaybackFailure::Unsupported)
        );
        if cancelled && !kept {
            self.audio.store.read_dropped(key);
        } else {
            if let Ok(AudioRead {
                waveform: Some(waveform),
                ..
            }) = &outcome
                && let Some(cache) = cache_key(key)
            {
                self.audio.waveforms.insert(cache, waveform.clone());
            }
            self.audio.store.read(key, outcome);
        }
        self.media_read_done();
        self.reconcile_audio();
    }

    /// Audio `key` is opening to play: when it lacks its finished waveform, a decode of it
    /// (stopping another audio's) to run alongside. None when it has it, or one already runs.
    pub(super) fn start_fill(&mut self, key: AudioKey) -> Option<Fill> {
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
            serial: running.serial,
            stop: Arc::clone(&running.stop),
        };
        self.audio.filling = Some(running);
        Some(fill)
    }

    /// A snapshot of fill `serial`'s waveform. Returns whether views change.
    fn audio_filled(&mut self, serial: u64, waveform: Waveform) -> bool {
        let Some(key) = self.filling(serial) else {
            return false;
        };
        self.audio.store.filled(key, waveform);
        true
    }

    /// Fill `serial` ended: its whole waveform (and the length it decoded) when it got there.
    fn audio_fill_done(&mut self, serial: u64, read: Option<AudioRead>) -> bool {
        let Some(key) = self.filling(serial) else {
            return false;
        };
        self.audio.filling = None;
        let Some(read) = read else {
            return false;
        };
        if let Some(waveform) = &read.waveform
            && let Some(cache) = cache_key(key)
        {
            self.audio.waveforms.insert(cache, waveform.clone());
        }
        self.audio.store.fill_done(key, read);
        true
    }

    /// The audio fill `serial` decodes, while it is the one running.
    fn filling(&self, serial: u64) -> Option<AudioKey> {
        self.audio
            .filling
            .as_ref()
            .filter(|filling| filling.serial == serial)
            .map(|filling| filling.key)
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
    let Fill { serial, stop } = fill;
    // Not awaited: the player opens meanwhile.
    drop(tokio::task::spawn_blocking(move || {
        let progress = move |waveform| {
            progress_updates.send(move |shell: &mut Shell| shell.audio_filled(serial, waveform));
        };
        let read = engine
            .read_audio(bytes, None, &progress, &stop)
            .ok()
            .filter(|read| read.waveform.is_some());
        done_updates.send(move |shell: &mut Shell| shell.audio_fill_done(serial, read));
    }));
}
