//! JSON report serialization for 14c desktop playback budget runs.

use super::{FIXTURE_BLUE, FIXTURE_GREEN, FIXTURE_RED, Result, bench_audio::AudioStats};
use papo_call::telemetry::TelemetrySnapshot;
use papo_sdk::{Uuid, VideoKind};
use std::fmt::Write as _;

#[derive(Clone, Copy, Default)]
pub(super) struct RunTimings {
    pub baseline_start_ns: u64,
    pub baseline_end_ns: u64,
    pub warmup_start_ns: u64,
    pub measure_start_ns: u64,
    pub measure_end_ns: u64,
}

pub(super) struct BenchReport {
    pub timings: RunTimings,
    pub nv12_uploads: u64,
    pub bgra_uploads: u64,
    pub audio_stats: AudioStats,
    pub telemetry: TelemetrySnapshot,
}

struct TileUuids {
    red: Uuid,
    green: Uuid,
    blue: Uuid,
}

impl TileUuids {
    fn tile_index(&self, participant: Uuid, kind: VideoKind) -> Option<usize> {
        if participant == self.red && kind == VideoKind::Camera {
            Some(0)
        } else if participant == self.green && kind == VideoKind::Camera {
            Some(1)
        } else if participant == self.blue && kind == VideoKind::Camera {
            Some(2)
        } else if participant == self.blue && kind == VideoKind::Screen {
            Some(3)
        } else {
            None
        }
    }
}

fn json_string(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 2);
    out.push('"');
    for ch in raw.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

pub(super) fn write_report(
    path: &str,
    profile: &str,
    warmup_secs: u64,
    measure_secs: u64,
    report: &BenchReport,
) -> Result {
    let uuids = TileUuids {
        red: FIXTURE_RED.parse()?,
        green: FIXTURE_GREEN.parse()?,
        blue: FIXTURE_BLUE.parse()?,
    };
    let t = &report.timings;
    let a = &report.audio_stats;
    let tel = &report.telemetry;
    let mut out = String::with_capacity(4096);
    writeln!(out, "{{")?;
    writeln!(out, "  \"profile\": {},", json_string(profile))?;
    writeln!(out, "  \"warmup_secs\": {warmup_secs},")?;
    writeln!(out, "  \"measure_secs\": {measure_secs},")?;
    writeln!(out, "  \"baseline_start_ns\": {},", t.baseline_start_ns)?;
    writeln!(out, "  \"baseline_end_ns\": {},", t.baseline_end_ns)?;
    writeln!(out, "  \"warmup_start_ns\": {},", t.warmup_start_ns)?;
    writeln!(out, "  \"measure_start_ns\": {},", t.measure_start_ns)?;
    writeln!(out, "  \"measure_end_ns\": {},", t.measure_end_ns)?;
    writeln!(out, "  \"nv12_uploads\": {},", report.nv12_uploads)?;
    writeln!(out, "  \"bgra_uploads\": {},", report.bgra_uploads)?;
    writeln!(
        out,
        "  \"fence_accepted_packets\": {},",
        tel.fence_accepted_packets
    )?;
    writeln!(
        out,
        "  \"fence_rejected_packets\": {},",
        tel.fence_rejected_packets
    )?;
    writeln!(out, "  \"decoded_frames\": {},", tel.decoded_frames)?;
    writeln!(
        out,
        "  \"decoder_factory\": {},",
        tel.decoder_factory
            .as_deref()
            .map_or_else(|| "null".into(), json_string)
    )?;
    writeln!(out, "  \"telemetry_overflowed\": {},", tel.overflowed)?;
    writeln!(
        out,
        "  \"telemetry_clock_unavailable\": {},",
        tel.clock_unavailable
    )?;
    writeln!(out, "  \"audio\": {{")?;
    writeln!(
        out,
        "    \"sink_factory\": {},",
        json_string(&a.sink_factory)
    )?;
    writeln!(
        out,
        "    \"current_device\": {},",
        a.current_device
            .as_deref()
            .map_or_else(|| "null".into(), json_string)
    )?;
    writeln!(
        out,
        "    \"clock_name\": {},",
        a.clock_name
            .as_deref()
            .map_or_else(|| "null".into(), json_string)
    )?;
    writeln!(out, "    \"rendered\": {},", a.rendered)?;
    writeln!(out, "    \"dropped\": {},", a.dropped)?;
    writeln!(out, "    \"underruns_warmup\": {},", a.underruns_warmup)?;
    writeln!(out, "    \"underruns_measured\": {}", a.underruns_measured)?;
    writeln!(out, "  }},")?;
    writeln!(out, "  \"jitter_records\": [")?;
    let mut first = true;
    for rec in &tel.jitter_records {
        let Some(tile) = uuids.tile_index(rec.participant, rec.kind) else {
            continue;
        };
        if !first {
            writeln!(out, ",")?;
        }
        first = false;
        let pts = rec.pts_ns.map_or_else(|| "null".into(), |p| p.to_string());
        write!(
            out,
            "    [{tile},{},{},{pts},{}]",
            rec.assignment_id, rec.rtp_timestamp, rec.jitter_exit_ns
        )?;
    }
    writeln!(out, "\n  ]")?;
    writeln!(out, "}}")?;
    std::fs::write(path, out)?;
    Ok(())
}
