//! Real clocked audio sink, observable pulsesink underflow counter, and starvation self-test.

use super::Result;
use gst::prelude::*;
use gstreamer as gst;
use gstreamer_app as app;
use papo_call::MediaDevices;
use papo_sdk::CameraFacing;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

#[derive(Clone, Debug, Default)]
pub(super) struct AudioStats {
    pub sink_factory: String,
    pub current_device: Option<String>,
    pub clock_name: Option<String>,
    pub rendered: u64,
    pub dropped: u64,
    pub underruns_warmup: u64,
    pub underruns_measured: u64,
}

pub(super) struct AudioProbe {
    in_measured: Arc<AtomicBool>,
    underruns_warmup: Arc<AtomicU64>,
    underruns_measured: Arc<AtomicU64>,
    sink: Arc<Mutex<Option<gst::Element>>>,
    log_handle: Option<gst::DebugLogFunction>,
}

impl AudioProbe {
    pub fn install() -> Result<Self> {
        gst::init()?;
        // Ensure libgstpulseaudio registers its "pulse" debug category before threshold setup.
        let _ = gst::ElementFactory::make("pulsesink").build();
        if let Some(cat) = gst::DebugCategory::get("pulse") {
            cat.set_threshold(gst::DebugLevel::Warning);
        }
        let in_measured = Arc::new(AtomicBool::new(false));
        let underruns_warmup = Arc::new(AtomicU64::new(0));
        let underruns_measured = Arc::new(AtomicU64::new(0));
        let flag = Arc::clone(&in_measured);
        let warmup = Arc::clone(&underruns_warmup);
        let measured = Arc::clone(&underruns_measured);
        let log_handle = gst::log::add_log_function(move |cat, _lvl, _f, _fn, _ln, _obj, msg| {
            if cat.name() == "pulse"
                && let Some(text) = msg.get()
                && text.as_str().contains("Got underflow")
            {
                if flag.load(Ordering::Relaxed) {
                    measured.fetch_add(1, Ordering::Relaxed);
                } else {
                    warmup.fetch_add(1, Ordering::Relaxed);
                }
            }
        });
        Ok(Self {
            in_measured,
            underruns_warmup,
            underruns_measured,
            sink: Arc::new(Mutex::new(None)),
            log_handle: Some(log_handle),
        })
    }

    pub fn set_measured(&self, active: bool) {
        self.in_measured.store(active, Ordering::Relaxed);
    }

    pub fn devices(&self, real_audio: bool) -> Arc<dyn MediaDevices> {
        Arc::new(BenchDevices {
            real_audio,
            sink_slot: Arc::clone(&self.sink),
        })
    }

    pub fn snapshot(&self) -> AudioStats {
        let mut stats = AudioStats {
            underruns_warmup: self.underruns_warmup.load(Ordering::Relaxed),
            underruns_measured: self.underruns_measured.load(Ordering::Relaxed),
            ..AudioStats::default()
        };
        let sink = self
            .sink
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let Some(sink) = sink else {
            return stats;
        };
        stats.sink_factory = sink
            .factory()
            .map_or_else(|| "unknown".into(), |f| f.name().to_string());
        if sink.has_property("current-device") {
            stats.current_device = sink.property::<Option<String>>("current-device");
        }
        if sink.has_property("stats") {
            let s = sink.property::<gst::Structure>("stats");
            stats.rendered = s.get::<u64>("rendered").unwrap_or(0);
            stats.dropped = s.get::<u64>("dropped").unwrap_or(0);
        }
        stats.clock_name = pipeline_clock_name(&sink);
        stats
    }
}

impl Drop for AudioProbe {
    fn drop(&mut self) {
        if let Some(handle) = self.log_handle.take() {
            gst::log::remove_log_function(handle);
        }
    }
}

fn pipeline_clock_name(element: &gst::Element) -> Option<String> {
    let mut cur = element.clone().upcast::<gst::Object>();
    while let Some(parent) = cur.parent() {
        cur = parent;
        if let Ok(pipeline) = cur.clone().downcast::<gst::Pipeline>() {
            return pipeline
                .clock()
                .map(|c| format!("{}:{}", c.name(), c.type_().name()));
        }
    }
    element
        .clock()
        .map(|c| format!("{}:{}", c.name(), c.type_().name()))
}

struct BenchDevices {
    real_audio: bool,
    sink_slot: Arc<Mutex<Option<gst::Element>>>,
}

impl MediaDevices for BenchDevices {
    fn microphone(&self) -> Option<gst::Element> {
        None
    }

    fn camera(&self, _: CameraFacing) -> Option<gst::Element> {
        None
    }

    fn speaker(&self) -> Option<gst::Element> {
        let sink = if self.real_audio {
            gst::ElementFactory::make("pulsesink")
                .property("sync", true)
                .property("provide-clock", true)
                .property("volume", 0.0_f64)
                .property("client-name", "papo-call-bench")
                .build()
                .ok()?
        } else {
            gst::ElementFactory::make("fakesink")
                .property("sync", false)
                .build()
                .ok()?
        };
        *self
            .sink_slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(sink.clone());
        Some(sink)
    }
}

/// Intentionally starves a live `pulsesink` to prove the observable underflow counter fires.
pub(super) fn run_audio_starvation_self_test() -> Result {
    let probe = AudioProbe::install()?;
    probe.set_measured(true);
    let sink = probe
        .devices(true)
        .speaker()
        .ok_or("pulsesink unavailable")?;
    let caps = gst::Caps::builder("audio/x-raw")
        .field("format", "S16LE")
        .field("layout", "interleaved")
        .field("rate", 48_000_i32)
        .field("channels", 2_i32)
        .build();
    let src = app::AppSrc::builder()
        .caps(&caps)
        .format(gst::Format::Time)
        .is_live(true)
        .build();
    let pipeline = gst::Pipeline::new();
    pipeline.add_many([src.upcast_ref(), &sink])?;
    let push_res: Result<AudioStats> = (|| {
        src.link(&sink)?;
        pipeline.set_state(gst::State::Playing)?;

        // Push 240 ms of silence (12 x 20 ms buffers) so the PA stream opens and plays, then starve.
        let samples_bytes = 48_000 * 2 * 2 / 50;
        for i in 0_u64..12 {
            let mut buf = gst::Buffer::with_size(samples_bytes)?;
            {
                let buf_mut = buf.get_mut().ok_or("buffer not unique")?;
                buf_mut.set_pts(gst::ClockTime::from_mseconds(i * 20));
                buf_mut.set_duration(gst::ClockTime::from_mseconds(20));
                buf_mut.map_writable()?.as_mut_slice().fill(0);
            }
            src.push_buffer(buf)?;
        }
        std::thread::sleep(Duration::from_secs(2));
        Ok(probe.snapshot())
    })();
    let _ = pipeline.set_state(gst::State::Null);
    let stats = push_res?;
    println!(
        "Audio underrun self-test: sink={} clock={:?} underruns={} rendered={} dropped={}",
        stats.sink_factory,
        stats.clock_name,
        stats.underruns_measured,
        stats.rendered,
        stats.dropped
    );
    if stats.underruns_measured == 0 {
        return Err("expected pulsesink underflow during intentional starvation, got 0".into());
    }
    Ok(())
}
