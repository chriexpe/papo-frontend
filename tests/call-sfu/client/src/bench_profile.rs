//! Benchmark profile specifications and VK_EXT_present_timing latch writer.

use super::{FIXTURE_BLUE, FIXTURE_GREEN, FIXTURE_RED, Result};
use papo_sdk::VideoKind;
use std::{
    fs::{File, OpenOptions},
    os::unix::fs::FileExt as _,
    sync::atomic::{AtomicU64, Ordering},
};

const LATCH_MAGIC: u64 = 0x5041_504F_5054_4C31;

#[derive(Clone, Copy)]
pub(super) struct ProfileSpec {
    pub name: &'static str,
    pub window_px: (f32, f32),
    pub targets: &'static [(&'static str, VideoKind)],
    pub real_audio: bool,
}

const TARGETS_1: &[(&str, VideoKind)] = &[(FIXTURE_RED, VideoKind::Camera)];
const TARGETS_4: &[(&str, VideoKind)] = &[
    (FIXTURE_RED, VideoKind::Camera),
    (FIXTURE_GREEN, VideoKind::Camera),
    (FIXTURE_BLUE, VideoKind::Camera),
    (FIXTURE_BLUE, VideoKind::Screen),
];

pub(super) fn profile_spec(name: &str) -> Result<ProfileSpec> {
    match name {
        "360p" => Ok(ProfileSpec {
            name: "360p",
            window_px: (640.0, 360.0),
            targets: TARGETS_1,
            real_audio: false,
        }),
        "720p" => Ok(ProfileSpec {
            name: "720p",
            window_px: (1280.0, 720.0),
            targets: TARGETS_1,
            real_audio: false,
        }),
        "1080p" => Ok(ProfileSpec {
            name: "1080p",
            window_px: (1920.0, 1080.0),
            targets: TARGETS_1,
            real_audio: false,
        }),
        "four-720p" => Ok(ProfileSpec {
            name: "four-720p",
            window_px: (1280.0, 720.0),
            targets: TARGETS_4,
            real_audio: true,
        }),
        _ => Err(format!("unknown PAPO_CALL_BENCH_PROFILE: {name}").into()),
    }
}

pub(super) struct LatchWriter {
    file: File,
    seq: AtomicU64,
}

impl LatchWriter {
    pub(super) fn open(path: &str) -> Result<Self> {
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        file.write_all_at(&LATCH_MAGIC.to_le_bytes(), 0)?;
        file.write_all_at(&0_u64.to_le_bytes(), 8)?;
        Ok(Self {
            file,
            seq: AtomicU64::new(0),
        })
    }

    pub(super) fn write_tiles(&self, tiles: &[Option<u64>]) {
        // Seqlock protocol matching papo_present_timing_layer.c: odd seq while writing, even when stable.
        let n = self.seq.fetch_add(1, Ordering::Relaxed);
        let odd = n.wrapping_mul(2).wrapping_add(1);
        let even = odd.wrapping_add(1);
        let _ = self.file.write_all_at(&odd.to_le_bytes(), 8);

        let count = u32::try_from(tiles.len().min(4)).unwrap_or(4);
        let mut valid_mask = 0_u32;
        let mut payload = [0_u8; 72];
        payload[0..4].copy_from_slice(&count.to_le_bytes());
        for (i, pts) in tiles.iter().take(4).enumerate() {
            let off = 8 + i * 16;
            payload[off..off + 8].copy_from_slice(&(i as u64).to_le_bytes());
            if let Some(pts_ns) = pts {
                valid_mask |= 1 << i;
                payload[off + 8..off + 16].copy_from_slice(&pts_ns.to_le_bytes());
            }
        }
        payload[4..8].copy_from_slice(&valid_mask.to_le_bytes());
        let _ = self.file.write_all_at(&payload, 16);
        let _ = self.file.write_all_at(&even.to_le_bytes(), 8);
    }
}
