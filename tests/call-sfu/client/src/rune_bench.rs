//! Event-driven Rune playback benchmark runner with `VK_EXT_present_timing` latch output.

use super::{
    Result,
    bench_audio::{AudioProbe, AudioStats},
    bench_profile::{LatchWriter, profile_spec},
    bench_report::{BenchReport, RunTimings, write_report},
    bench_session::receive,
    rune_proof::pack,
};
use papo_call::{CpuVideoDestination, telemetry};
use rune::native_surface::Format;
use rune::{App, AppSpec, Cx, El, NativeSurface, SurfaceFit, Tx, col, el, row, text};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{oneshot, watch};

const BASELINE_NS: u64 = 2_000_000_000;
const CONNECT_TIMEOUT_NS: u64 = 30_000_000_000;
const TAIL_DRAIN_NS: u64 = 500_000_000;

pub(super) fn run(address: String) -> Result {
    let profile_name = std::env::var("PAPO_CALL_BENCH_PROFILE")?;
    let spec = profile_spec(&profile_name)?;
    let warmup_secs = env_u64("PAPO_CALL_BENCH_WARMUP_SECS", 10)?;
    let measure_secs = env_u64("PAPO_CALL_BENCH_MEASURE_SECS", 60)?;
    let warmup_ns = warmup_secs
        .checked_mul(1_000_000_000)
        .ok_or("PAPO_CALL_BENCH_WARMUP_SECS overflows nanoseconds")?;
    let measure_ns = measure_secs
        .checked_mul(1_000_000_000)
        .ok_or("PAPO_CALL_BENCH_MEASURE_SECS overflows nanoseconds")?;
    let report_path = std::env::var("PAPO_CALL_BENCH_REPORT")?;
    let latch = std::env::var("PAPO_PT_LATCH_FILE")
        .ok()
        .filter(|s| !s.is_empty())
        .map(|p| LatchWriter::open(&p))
        .transpose()?
        .map(Arc::new);

    let audio_probe = Arc::new(AudioProbe::install()?);
    let _ = telemetry::monotonic_ns().ok_or("monotonic clock unavailable")?;
    let required_records = warmup_secs
        .saturating_add(measure_secs)
        .saturating_add(30)
        .saturating_mul(spec.targets.len() as u64)
        .saturating_mul(40);
    let ring_capacity = usize::try_from(required_records)
        .unwrap_or(usize::MAX)
        .max(telemetry::DEFAULT_RECORD_CAPACITY);
    telemetry::reset(ring_capacity);
    rune::orderly_exit();

    let (dest_tx, dest_rx) = watch::channel(Vec::new());
    let (start_tx, start_rx) = oneshot::channel();
    let (stop_tx, stop_rx) = oneshot::channel();
    let audio_ref = Arc::clone(&audio_probe);

    let app = App::new(AppSpec {
        poll: Some(State::poll),
        ..AppSpec::new("Papo 14c benchmark", spec.window_px, State::default, view)
    });
    let handle = app.handle();
    let runtime = tokio::runtime::Handle::current();
    let worker_handle = handle.clone();
    let worker = std::thread::Builder::new()
        .name("call-bench-rtp".into())
        .spawn(move || {
            runtime.block_on(receive(
                &address,
                spec,
                audio_ref,
                worker_handle,
                dest_tx,
                start_rx,
                stop_rx,
            ))
        })?;

    let shared = Arc::new(Mutex::new(SharedOutcome {
        worker: Some(worker),
        stop_tx: Some(stop_tx),
        audio_probe: Arc::clone(&audio_probe),
        audio_stats: None,
        timings: RunTimings::default(),
        nv12_uploads: 0,
        bgra_uploads: 0,
    }));
    let state_shared = Arc::clone(&shared);
    let tile_count = spec.targets.len();

    let win_res = app
        .init(move || State {
            input: Some(dest_rx),
            start_tx: Some(start_tx),
            tiles: (0..tile_count).map(|_| Tile::default()).collect(),
            latch,
            shared: Some(state_shared),
            audio_probe: Some(audio_probe),
            warmup_ns,
            measure_ns,
            ..State::default()
        })
        .on_close(State::close)
        .on_quit(State::close)
        .try_run();

    let report = finalize(&shared)?;
    win_res?;
    write_report(&report_path, spec.name, warmup_secs, measure_secs, &report)?;
    Ok(())
}

fn env_u64(key: &str, default: u64) -> Result<u64> {
    match std::env::var(key) {
        Ok(val) if !val.is_empty() => Ok(val.parse()?),
        _ => Ok(default),
    }
}

struct SharedOutcome {
    worker: Option<std::thread::JoinHandle<std::result::Result<(), String>>>,
    stop_tx: Option<oneshot::Sender<()>>,
    audio_probe: Arc<AudioProbe>,
    audio_stats: Option<AudioStats>,
    timings: RunTimings,
    nv12_uploads: u64,
    bgra_uploads: u64,
}

fn finalize(shared: &Mutex<SharedOutcome>) -> Result<BenchReport> {
    let (probe, already_stats, stop) = {
        let mut guard = shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (
            Arc::clone(&guard.audio_probe),
            guard.audio_stats.take(),
            guard.stop_tx.take(),
        )
    };
    let audio_stats = already_stats.unwrap_or_else(|| probe.snapshot());
    if let Some(stop) = stop {
        let _ = stop.send(());
    }

    let (worker, timings, nv12_uploads, bgra_uploads) = {
        let mut guard = shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (
            guard.worker.take(),
            guard.timings,
            guard.nv12_uploads,
            guard.bgra_uploads,
        )
    };

    if let Some(worker) = worker {
        match worker.join() {
            Ok(Ok(())) => {}
            Ok(Err(err)) => return Err(err.into()),
            Err(_) => return Err("benchmark media worker panicked".into()),
        }
    }
    Ok(BenchReport {
        timings,
        nv12_uploads,
        bgra_uploads,
        audio_stats,
        telemetry: telemetry::take_snapshot(),
    })
}

#[derive(Default)]
struct Tile {
    surface: Option<NativeSurface>,
    pts_ns: Option<u64>,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Phase {
    #[default]
    Baseline,
    Connecting,
    Running,
    Done,
}

#[derive(Default)]
pub(super) struct State {
    input: Option<watch::Receiver<Vec<Arc<CpuVideoDestination>>>>,
    start_tx: Option<oneshot::Sender<()>>,
    tiles: Vec<Tile>,
    latch: Option<Arc<LatchWriter>>,
    shared: Option<Arc<Mutex<SharedOutcome>>>,
    audio_probe: Option<Arc<AudioProbe>>,
    phase: Phase,
    connecting_start_ns: u64,
    timings: RunTimings,
    warmup_ns: u64,
    measure_ns: u64,
    upload_seq: u64,
    nv12_uploads: u64,
    bgra_uploads: u64,
}

impl State {
    fn save_outcome(&self) {
        let Some(shared) = &self.shared else {
            return;
        };
        let audio_snapshot = self.audio_probe.as_ref().map(|probe| probe.snapshot());
        let stop = {
            let mut s = shared
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            s.timings = self.timings;
            s.nv12_uploads = self.nv12_uploads;
            s.bgra_uploads = self.bgra_uploads;
            if s.audio_stats.is_none() {
                s.audio_stats = audio_snapshot;
            }
            s.stop_tx.take()
        };
        if let Some(stop) = stop {
            let _ = stop.send(());
        }
    }

    fn close(&mut self, _: &mut Tx) {
        self.phase = Phase::Done;
        self.save_outcome();
    }

    fn poll(&mut self, _: f64, tx: &mut Tx) -> bool {
        if self.phase == Phase::Done {
            return false;
        }
        if self
            .input
            .as_ref()
            .is_some_and(|input| input.has_changed().is_err())
        {
            self.phase = Phase::Done;
            self.save_outcome();
            tx.close_window();
            return false;
        }
        let Some(now) = telemetry::monotonic_ns() else {
            self.phase = Phase::Done;
            self.save_outcome();
            tx.close_window();
            return false;
        };
        tx.wake_after(Duration::from_millis(200));
        match self.phase {
            Phase::Baseline => {
                if self.timings.baseline_start_ns == 0 {
                    self.timings.baseline_start_ns = now;
                } else if now >= self.timings.baseline_start_ns + BASELINE_NS {
                    self.timings.baseline_end_ns = now;
                    self.connecting_start_ns = now;
                    if let Some(start) = self.start_tx.take() {
                        let _ = start.send(());
                    }
                    self.phase = Phase::Connecting;
                }
                return false;
            }
            Phase::Connecting => {
                if self.connecting_start_ns != 0
                    && now.saturating_sub(self.connecting_start_ns) >= CONNECT_TIMEOUT_NS
                {
                    self.phase = Phase::Done;
                    self.save_outcome();
                    tx.close_window();
                    return false;
                }
            }
            Phase::Running => {}
            Phase::Done => return false,
        }
        let mut changed = false;
        if let Some(input) = &self.input {
            let destinations = input.borrow().clone();
            for (idx, dest) in destinations.iter().enumerate() {
                if let Some(lease) = dest.take() {
                    if !lease.is_current() {
                        continue;
                    }
                    let Some(bytes) = pack(&lease) else {
                        continue;
                    };
                    let pts_ns = lease.pts_ns;
                    let is_bgra = matches!(bytes.format, Format::Bgra);
                    let still_current = lease.is_current();
                    drop(lease);
                    if !still_current {
                        continue;
                    }
                    self.upload_seq += 1;
                    let surface = NativeSurface::new();
                    if surface.push_bytes(bytes, self.upload_seq).is_ok()
                        && let Some(tile) = self.tiles.get_mut(idx)
                    {
                        if is_bgra {
                            self.bgra_uploads += 1;
                        } else {
                            self.nv12_uploads += 1;
                        }
                        tile.surface = Some(surface);
                        tile.pts_ns = pts_ns;
                        changed = true;
                        if self.phase == Phase::Connecting {
                            self.phase = Phase::Running;
                            self.timings.warmup_start_ns = now;
                            self.timings.measure_start_ns = now + self.warmup_ns;
                            self.timings.measure_end_ns =
                                self.timings.measure_start_ns + self.measure_ns;
                        }
                    }
                }
            }
        }
        if self.phase == Phase::Running {
            let measured =
                now >= self.timings.measure_start_ns && now <= self.timings.measure_end_ns;
            if let Some(probe) = &self.audio_probe {
                probe.set_measured(measured);
            }
            if now >= self.timings.measure_end_ns + TAIL_DRAIN_NS {
                self.phase = Phase::Done;
                self.save_outcome();
                tx.close_window();
                return true;
            }
        }
        changed
    }
}

fn tile_el(tile: &Tile, theme: &rune::tok::Theme) -> El {
    let box_el = el()
        .w_full()
        .h_full()
        .grow(1.0)
        .min_h(0.0)
        .min_w(0.0)
        .center();
    match &tile.surface {
        Some(surface) => box_el.native_surface(surface, SurfaceFit::Fit),
        None => box_el.child(text("Waiting").color(theme.text)),
    }
}

fn view(state: &State, cx: &mut Cx<State>) -> El {
    if let Some(latch) = &state.latch {
        let pts: Vec<Option<u64>> = state.tiles.iter().map(|t| t.pts_ns).collect();
        latch.write_tiles(&pts);
    }
    let theme = rune::tok::theme(cx.system_dark());
    let body = if state.tiles.len() == 4 {
        col()
            .w_full()
            .h_full()
            .child(
                row()
                    .w_full()
                    .grow(1.0)
                    .min_h(0.0)
                    .child(tile_el(&state.tiles[0], &theme))
                    .child(tile_el(&state.tiles[1], &theme)),
            )
            .child(
                row()
                    .w_full()
                    .grow(1.0)
                    .min_h(0.0)
                    .child(tile_el(&state.tiles[2], &theme))
                    .child(tile_el(&state.tiles[3], &theme)),
            )
    } else if let Some(first) = state.tiles.first() {
        tile_el(first, &theme)
    } else {
        el().w_full().h_full()
    };
    col().w_full().h_full().fill(theme.bg).child(body)
}
