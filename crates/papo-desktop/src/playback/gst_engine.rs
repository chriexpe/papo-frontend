//! The GStreamer engine: probes (`probe`) and players, each player a pipeline (`pipeline`)
//! driven by a thread of its own, which builds it, applies the interface's commands, follows
//! its bus (first frame, end, looping, errors) and lets it go. The interface's calls only queue
//! a command and wake that thread, so none of them waits.

use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};

use gstreamer as gst;
use gstreamer::prelude::*;

use super::pipeline::{self, Graph, Mode, UNSUPPORTED, frame_caps};
use super::{
    Changed, Engine, FrameSink, PlaybackFailure, Player, PlayerSetup, PlayerState, PlayerStatus,
    Probe, SeekMode, fitted_frame,
};

/// The application message that wakes a player's thread for its commands.
const WAKE: &str = "papo-wake";

/// GStreamer, started on first use.
pub(super) struct GstEngine;

impl Engine for GstEngine {
    fn probe(&self, bytes: Arc<[u8]>, poster_px: [u32; 2]) -> Result<Probe, PlaybackFailure> {
        super::probe::probe(bytes, poster_px)
    }

    fn open(
        &self,
        bytes: Arc<[u8]>,
        setup: PlayerSetup,
        sink: Arc<dyn FrameSink>,
        changed: Changed,
    ) -> Box<dyn Player> {
        let (commands, inbox) = mpsc::channel();
        let shared = Arc::new(Mutex::new(Shared {
            state: PlayerState::Loading,
            duration_ms: None,
            position_ms: setup.start_ms,
            pipeline: None,
        }));
        let player = GstPlayer {
            commands,
            shared: Arc::clone(&shared),
        };
        let worker = Worker {
            setup,
            shared,
            changed,
            inbox,
        };
        let spawned = std::thread::Builder::new()
            .name("papo-player".into())
            .spawn(move || worker.run(bytes, sink));
        if spawned.is_err() {
            player.fail(PlaybackFailure::Unavailable);
        }
        Box::new(player)
    }
}

/// Starts GStreamer for this process (once; later calls return at once).
pub(super) fn init() -> Result<(), PlaybackFailure> {
    papo_call::init_gstreamer().map_err(|_| PlaybackFailure::Unavailable)
}

/// What a pipeline's error means for the person.
pub(super) fn failure_of(error: &gst::message::Error) -> PlaybackFailure {
    let error = error.error();
    let unsupported = matches!(
        error.kind::<gst::StreamError>(),
        Some(
            gst::StreamError::CodecNotFound
                | gst::StreamError::TypeNotFound
                | gst::StreamError::WrongType
        )
    ) || error.kind::<gst::CoreError>() == Some(gst::CoreError::MissingPlugin);
    if unsupported {
        PlaybackFailure::Unsupported
    } else if error.kind::<gst::ResourceError>().is_some() {
        // The audio device (or another resource) refused.
        PlaybackFailure::Unavailable
    } else {
        PlaybackFailure::Broken
    }
}

/// What the interface reads, kept by the player's thread.
struct Shared {
    state: PlayerState,
    duration_ms: Option<u64>,
    /// Where it was last known to be (before the pipeline answers, or once it ended).
    position_ms: u64,
    /// The pipeline, once built: the interface asks it for the position.
    pipeline: Option<gst::Pipeline>,
}

/// What the interface asks a player's thread to do.
enum Command {
    Play,
    Pause,
    Seek(u64, SeekMode),
    Looping(bool),
    Muted(bool),
    Volume(f32),
    FrameBox([u32; 2]),
    Stop,
}

/// The interface's handle: commands to the thread, state from it.
struct GstPlayer {
    commands: mpsc::Sender<Command>,
    shared: Arc<Mutex<Shared>>,
}

impl GstPlayer {
    fn send(&self, command: Command) {
        if self.commands.send(command).is_err() {
            return;
        }
        let pipeline = self
            .shared
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pipeline
            .clone();
        // Before the pipeline exists the thread reads its commands as soon as it builds it.
        if let Some(pipeline) = pipeline {
            let _ = pipeline.post_message(gst::message::Application::new(
                gst::Structure::new_empty(WAKE),
            ));
        }
    }

    fn fail(&self, failure: PlaybackFailure) {
        self.shared
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .state = PlayerState::Failed(failure);
    }
}

impl Player for GstPlayer {
    fn play(&self) {
        self.send(Command::Play);
    }

    fn pause(&self) {
        self.send(Command::Pause);
    }

    fn seek(&self, position_ms: u64, mode: SeekMode) {
        self.send(Command::Seek(position_ms, mode));
    }

    fn set_looping(&self, looping: bool) {
        self.send(Command::Looping(looping));
    }

    fn set_muted(&self, muted: bool) {
        self.send(Command::Muted(muted));
    }

    fn set_volume(&self, volume: f32) {
        self.send(Command::Volume(volume));
    }

    fn set_frame_box(&self, box_px: [u32; 2]) {
        self.send(Command::FrameBox(box_px));
    }

    fn status(&self) -> PlayerStatus {
        let (state, known_ms, duration_ms, pipeline) = {
            let shared = self.shared.lock().unwrap_or_else(PoisonError::into_inner);
            (
                shared.state,
                shared.position_ms,
                shared.duration_ms,
                shared.pipeline.clone(),
            )
        };
        // Asked of the pipeline outside the lock: the player's thread never waits on it.
        let live = matches!(state, PlayerState::Playing | PlayerState::Paused);
        let position_ms = pipeline
            .filter(|_| live)
            .and_then(|pipeline| pipeline.query_position::<gst::ClockTime>())
            .map_or(known_ms, gst::ClockTime::mseconds);
        PlayerStatus {
            state,
            position_ms,
            duration_ms,
        }
    }
}

impl Drop for GstPlayer {
    fn drop(&mut self) {
        self.send(Command::Stop);
    }
}

/// A player's thread.
struct Worker {
    setup: PlayerSetup,
    shared: Arc<Mutex<Shared>>,
    changed: Changed,
    inbox: mpsc::Receiver<Command>,
}

/// What the thread is steering the pipeline towards.
#[expect(
    clippy::struct_excessive_bools,
    reason = "each is one of the player's independent conditions"
)]
struct Steering {
    playing: bool,
    looping: bool,
    muted: bool,
    volume: f32,
    /// A seek waiting for the first frame (a start position, or a seek asked while loading).
    pending_seek: Option<(u64, SeekMode)>,
    /// The pipeline has its first frame.
    ready: bool,
    /// The state last asked of the pipeline.
    asked: gst::State,
    ended: bool,
}

impl Worker {
    fn run(self, bytes: Arc<[u8]>, sink: Arc<dyn FrameSink>) {
        let built = init()
            .and_then(|()| pipeline::build(bytes, Mode::Play, self.setup.frame_px, Some(sink)));
        let graph = match built {
            Ok(graph) => graph,
            Err(failure) => {
                self.set_state(PlayerState::Failed(failure));
                return;
            }
        };
        let Some(bus) = graph.pipeline.bus() else {
            self.set_state(PlayerState::Failed(PlaybackFailure::Unavailable));
            return;
        };
        self.lock().pipeline = Some(graph.pipeline.clone());
        let mut steering = Steering {
            playing: self.setup.playing,
            looping: self.setup.looping,
            muted: self.setup.muted,
            volume: self.setup.volume,
            pending_seek: (self.setup.start_ms > 0)
                .then_some((self.setup.start_ms, SeekMode::Accurate)),
            ready: false,
            ended: false,
            asked: gst::State::Paused,
        };
        set_loudness(&graph, &steering);
        if graph.pipeline.set_state(gst::State::Paused).is_err() {
            self.set_state(PlayerState::Failed(PlaybackFailure::Broken));
        }
        'running: loop {
            if !self.commands(&graph, &mut steering) {
                break 'running;
            }
            let Some(message) = bus.timed_pop(gst::ClockTime::NONE) else {
                continue;
            };
            if !self.message(&graph, &mut steering, &message) {
                break 'running;
            }
        }
        if let Some(relay) = graph
            .relay
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
        {
            relay.close();
        }
        let _ = graph.pipeline.set_state(gst::State::Null);
        self.lock().pipeline = None;
        drop(graph);
        super::memory::return_freed_memory();
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn set_state(&self, state: PlayerState) {
        let changed = {
            let mut shared = self.lock();
            let changed = shared.state != state;
            shared.state = state;
            changed
        };
        if changed {
            (self.changed)();
        }
    }

    /// Applies the queued commands; false once asked to stop.
    fn commands(&self, graph: &Graph, steering: &mut Steering) -> bool {
        // Seeks come fast from a dragged seek bar: only the last of a batch is made.
        let mut seek_to = None;
        while let Ok(command) = self.inbox.try_recv() {
            match command {
                Command::Stop => return false,
                Command::Play => {
                    steering.playing = true;
                    if steering.ended {
                        steering.ended = false;
                        seek(graph, 0, SeekMode::Accurate);
                    }
                }
                Command::Pause => steering.playing = false,
                Command::Seek(position_ms, mode) => seek_to = Some((position_ms, mode)),
                Command::Looping(looping) => steering.looping = looping,
                Command::Muted(muted) => {
                    steering.muted = muted;
                    set_loudness(graph, steering);
                }
                Command::Volume(volume) => {
                    steering.volume = volume;
                    set_loudness(graph, steering);
                }
                Command::FrameBox(box_px) => refit(graph, box_px),
            }
        }
        if let Some((position_ms, mode)) = seek_to {
            if steering.ready {
                steering.ended = false;
                seek(graph, position_ms, mode);
            } else {
                steering.pending_seek = Some((position_ms, mode));
            }
            self.lock().position_ms = position_ms;
        }
        self.steer(graph, steering);
        true
    }

    /// Puts the pipeline in the state the interface asked for, once it can.
    fn steer(&self, graph: &Graph, steering: &mut Steering) {
        if !steering.ready || steering.ended {
            return;
        }
        let target = if steering.playing {
            gst::State::Playing
        } else {
            gst::State::Paused
        };
        if steering.asked == target {
            return;
        }
        steering.asked = target;
        if graph.pipeline.set_state(target).is_err() {
            self.set_state(PlayerState::Failed(PlaybackFailure::Unavailable));
        }
    }

    /// Follows one bus message; false once the player can't go on.
    fn message(&self, graph: &Graph, steering: &mut Steering, message: &gst::Message) -> bool {
        match message.view() {
            gst::MessageView::Error(error) => {
                log::info!("A media player failed");
                self.set_state(PlayerState::Failed(failure_of(error)));
                return false;
            }
            gst::MessageView::Application(application) => {
                let unsupported = application
                    .structure()
                    .is_some_and(|structure| structure.name() == UNSUPPORTED);
                if unsupported {
                    self.set_state(PlayerState::Failed(PlaybackFailure::Unsupported));
                    return false;
                }
            }
            gst::MessageView::AsyncDone(_) => {
                self.learn_duration(graph);
                if !steering.ready {
                    steering.ready = true;
                    if let Some((position_ms, mode)) = steering.pending_seek.take() {
                        seek(graph, position_ms, mode);
                    }
                }
                self.steer(graph, steering);
            }
            gst::MessageView::DurationChanged(_) => self.learn_duration(graph),
            gst::MessageView::StateChanged(change)
                if change.src() == Some(graph.pipeline.upcast_ref()) =>
            {
                match change.current() {
                    gst::State::Playing => self.set_state(PlayerState::Playing),
                    gst::State::Paused if steering.ready && !steering.ended => {
                        self.set_state(PlayerState::Paused);
                    }
                    _ => {}
                }
            }
            gst::MessageView::Eos(_) => {
                if steering.looping {
                    seek(graph, 0, SeekMode::Accurate);
                } else {
                    steering.ended = true;
                    steering.playing = false;
                    let end = self.lock().duration_ms.unwrap_or_default();
                    self.lock().position_ms = end;
                    steering.asked = gst::State::Paused;
                    let _ = graph.pipeline.set_state(gst::State::Paused);
                    self.set_state(PlayerState::Ended);
                }
            }
            _ => {}
        }
        true
    }

    fn learn_duration(&self, graph: &Graph) {
        let duration_ms = graph
            .pipeline
            .query_duration::<gst::ClockTime>()
            .map(gst::ClockTime::mseconds);
        let changed = {
            let mut shared = self.lock();
            let changed = duration_ms.is_some() && shared.duration_ms != duration_ms;
            if changed {
                shared.duration_ms = duration_ms;
            }
            changed
        };
        if changed {
            (self.changed)();
        }
    }
}

/// Moves `graph` to `position_ms`.
fn seek(graph: &Graph, position_ms: u64, mode: SeekMode) {
    let flags = match mode {
        SeekMode::Accurate => gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
        SeekMode::Keyframe => {
            gst::SeekFlags::FLUSH | gst::SeekFlags::KEY_UNIT | gst::SeekFlags::SNAP_NEAREST
        }
    };
    let _ = graph
        .pipeline
        .seek_simple(flags, gst::ClockTime::from_mseconds(position_ms));
}

/// The sound as `steering` says.
fn set_loudness(graph: &Graph, steering: &Steering) {
    graph
        .loudness
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .set(steering.muted, steering.volume);
}

/// A pipeline's frames fitted into `box_px` from now on; nothing before its video appeared.
fn refit(graph: &Graph, box_px: [u32; 2]) {
    let Some(source) = *graph
        .video_px
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
    else {
        return;
    };
    if let Some(filter) = graph
        .frame_caps
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
    {
        filter.set_property("caps", frame_caps(Some(fitted_frame(source, box_px))));
    }
}
