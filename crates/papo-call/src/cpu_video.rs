//! Bounded CPU frame delivery and route/format validity; decoding remains in receive_video.

use gstreamer as gst;
use gstreamer_video as video;
use papo_sdk::{VideoDestination, VideoDestinationId, VideoSlotBinding, VideoTarget};
use std::{
    any::Any,
    ops::Deref,
    sync::{Arc, Mutex, MutexGuard},
};

/// Owned NV12 frame. Offsets and strides are in bytes and describe the actual buffer layout.
pub struct CpuVideoFrame {
    /// Width in pixels.
    pub width_px: u32,
    /// Height in pixels.
    pub height_px: u32,
    /// Byte offsets for the Y and UV planes.
    pub offsets: [usize; 2],
    /// Row strides for the Y and UV planes.
    pub strides: [i32; 2],
    /// GStreamer colorimetry, including range. The CPU path preserves source orientation.
    pub colorimetry: video::VideoColorimetry,
    /// Frame bytes including any per-plane padding.
    pub bytes: Vec<u8>,
}

#[derive(PartialEq, Eq)]
struct Format {
    width_px: u32,
    height_px: u32,
    colorimetry: video::VideoColorimetry,
    plane_bytes: [usize; 2],
    strides: [i32; 2],
}

#[derive(Default)]
struct Mailbox {
    generation: u64,
    revoked: bool,
    format_epoch: u64,
    format: Option<Format>,
    pending: Option<CpuVideoFrame>,
    in_flight: bool,
}

fn lock(mailbox: &Mutex<Mailbox>) -> MutexGuard<'_, Mailbox> {
    mailbox
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A participant-keyed, latest-wins destination with one outstanding frame lease.
pub struct CpuVideoDestination {
    id: VideoDestinationId,
    target: VideoTarget,
    mailbox: Arc<Mutex<Mailbox>>,
}

impl CpuVideoDestination {
    /// Creates an empty destination for a stable ID and immutable video target.
    pub fn new(id: VideoDestinationId, target: VideoTarget) -> Self {
        Self {
            id,
            target,
            mailbox: Arc::default(),
        }
    }

    /// Takes the newest pending frame if no lease is outstanding.
    pub fn take(&self) -> Option<CpuVideoFrameLease> {
        let mut state = lock(&self.mailbox);
        if state.revoked || state.in_flight {
            return None;
        }
        let frame = state.pending.take()?;
        state.in_flight = true;
        Some(CpuVideoFrameLease {
            frame,
            mailbox: Arc::clone(&self.mailbox),
            generation: state.generation,
            format_epoch: state.format_epoch,
        })
    }

    pub(crate) fn matches(&self, route: &VideoSlotBinding) -> bool {
        self.target.participant == route.participant && self.target.kind == route.kind
    }

    pub(crate) fn reset(&self) -> u64 {
        let mut state = lock(&self.mailbox);
        state.pending = None;
        match state.generation.checked_add(1) {
            Some(generation) => state.generation = generation,
            None => state.revoked = true,
        }
        state.generation
    }

    pub(crate) fn put(&self, generation: u64, sample: &gst::Sample) {
        {
            let state = lock(&self.mailbox);
            if state.revoked || state.generation != generation {
                return;
            }
        }
        self.copy_sample(generation, sample);
    }

    fn copy_sample(&self, generation: u64, sample: &gst::Sample) {
        use video::VideoFrameExt as _;
        let Some(info) = sample
            .caps()
            .and_then(|caps| video::VideoInfo::from_caps(caps).ok())
        else {
            return;
        };
        let Some(buffer) = sample.buffer() else {
            return;
        };
        let Ok(mapped) = video::VideoFrameRef::from_buffer_ref_readable(buffer, &info) else {
            return;
        };
        let strides = mapped.plane_stride();
        let (Some(&y_stride), Some(&uv_stride)) = (strides.first(), strides.get(1)) else {
            return;
        };
        let (Ok(y), Ok(uv)) = (mapped.plane_data(0), mapped.plane_data(1)) else {
            return;
        };
        let format = Format {
            width_px: info.width(),
            height_px: info.height(),
            colorimetry: info.colorimetry(),
            plane_bytes: [y.len(), uv.len()],
            strides: [y_stride, uv_stride],
        };
        let epoch = {
            let mut state = lock(&self.mailbox);
            if state.revoked || state.generation != generation {
                return;
            }
            if state.format.as_ref() != Some(&format) {
                let Some(epoch) = state.format_epoch.checked_add(1) else {
                    state.revoked = true;
                    state.pending = None;
                    return;
                };
                state.format_epoch = epoch;
                state.format = Some(format);
                state.pending = None;
            }
            state.format_epoch
        };
        let Some(size) = y.len().checked_add(uv.len()) else {
            return;
        };
        let mut bytes = Vec::new();
        if bytes.try_reserve_exact(size).is_err() {
            return;
        }
        bytes.extend_from_slice(y);
        bytes.extend_from_slice(uv);
        let frame = CpuVideoFrame {
            width_px: info.width(),
            height_px: info.height(),
            offsets: [0, y.len()],
            strides: [y_stride, uv_stride],
            colorimetry: info.colorimetry(),
            bytes,
        };
        let mut state = lock(&self.mailbox);
        if !state.revoked && state.generation == generation && state.format_epoch == epoch {
            state.pending = Some(frame);
        }
    }
}

impl VideoDestination for CpuVideoDestination {
    fn id(&self) -> VideoDestinationId {
        self.id
    }
    fn as_any(&self) -> &(dyn Any + Send + Sync) {
        self
    }
    fn revoke(&self) {
        let mut state = lock(&self.mailbox);
        state.revoked = true;
        state.pending = None;
    }
}

/// A checked-out frame. Dropping it permits the next checkout.
pub struct CpuVideoFrameLease {
    frame: CpuVideoFrame,
    mailbox: Arc<Mutex<Mailbox>>,
    generation: u64,
    format_epoch: u64,
}

impl CpuVideoFrameLease {
    /// Whether the frame still belongs to the active route and format.
    pub fn is_current(&self) -> bool {
        let state = lock(&self.mailbox);
        !state.revoked
            && state.generation == self.generation
            && state.format_epoch == self.format_epoch
    }

    /// Nonzero, monotonically increasing format epoch for this destination.
    pub fn format_epoch(&self) -> u64 {
        self.format_epoch
    }
}

impl Deref for CpuVideoFrameLease {
    type Target = CpuVideoFrame;
    fn deref(&self) -> &Self::Target {
        &self.frame
    }
}

impl Drop for CpuVideoFrameLease {
    fn drop(&mut self) {
        lock(&self.mailbox).in_flight = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use papo_sdk::{Uuid, VideoKind};

    fn destination() -> CpuVideoDestination {
        CpuVideoDestination::new(
            VideoDestinationId::from_raw(1),
            VideoTarget {
                participant: Uuid::nil(),
                kind: VideoKind::Camera,
            },
        )
    }

    fn sample(width: i32, value: u8) -> gst::Sample {
        sample_at_offset(width, value, 0)
    }

    fn sample_at_offset(width: i32, value: u8, prefix: usize) -> gst::Sample {
        gst::init().unwrap();
        let caps = gst::Caps::builder("video/x-raw")
            .field("format", "NV12")
            .field("width", width)
            .field("height", 4_i32)
            .field("framerate", gst::Fraction::new(30, 1))
            .build();
        let info = video::VideoInfo::from_caps(&caps).unwrap();
        let mut buffer = gst::Buffer::with_size(info.size() + prefix).unwrap();
        if prefix != 0 {
            let offsets = [info.offset()[0] + prefix, info.offset()[1] + prefix];
            video::VideoMeta::add_full(
                buffer.get_mut().unwrap(),
                video::VideoFrameFlags::empty(),
                video::VideoFormat::Nv12,
                info.width(),
                info.height(),
                &offsets,
                &info.stride()[..2],
            )
            .unwrap();
        }
        buffer
            .get_mut()
            .unwrap()
            .map_writable()
            .unwrap()
            .as_mut_slice()
            .fill(value);
        gst::Sample::builder().caps(&caps).buffer(&buffer).build()
    }

    #[test]
    fn slow_consumer_gets_latest_after_dropping_checkout() {
        let dest = destination();
        let generation = dest.reset();
        dest.put(generation, &sample(4, 1));
        let first = dest.take().unwrap();
        dest.put(generation, &sample(4, 2));
        dest.put(generation, &sample(4, 3));
        assert!(dest.take().is_none());
        assert_eq!(first.bytes[0], 1);
        drop(first);
        assert_eq!(dest.take().unwrap().bytes[0], 3);
    }

    #[test]
    fn detach_and_reattach_invalidate_old_frames_and_keep_checkout_bounded() {
        let dest = destination();
        let generation = dest.reset();
        dest.put(generation, &sample(4, 1));
        let old = dest.take().unwrap();
        let next = dest.reset();
        assert!(!old.is_current());
        dest.put(generation, &sample(4, 2));
        dest.put(next, &sample(4, 3));
        assert!(dest.take().is_none());
        drop(old);
        let current = dest.take().unwrap();
        assert!(current.is_current());
        assert_eq!(current.bytes[0], 3);
    }

    #[test]
    fn format_change_invalidates_lease_and_advances_epoch() {
        let dest = destination();
        let generation = dest.reset();
        dest.put(generation, &sample(4, 1));
        let old = dest.take().unwrap();
        let epoch = old.format_epoch();
        assert_ne!(epoch, 0);
        dest.put(generation, &sample(8, 2));
        assert!(!old.is_current());
        drop(old);
        let next = dest.take().unwrap();
        assert!(next.format_epoch() > epoch);
        assert!(next.is_current());
    }

    #[test]
    fn source_plane_offset_changes_do_not_invalidate_a_delivered_format() {
        let dest = destination();
        let generation = dest.reset();
        dest.put(generation, &sample(4, 1));
        let first = dest.take().unwrap();
        let epoch = first.format_epoch();
        dest.put(generation, &sample_at_offset(4, 2, 64));
        assert!(first.is_current());
        drop(first);
        let next = dest.take().unwrap();
        assert_eq!(next.format_epoch(), epoch);
        assert_eq!(next.bytes[0], 2);
    }

    #[test]
    fn wrong_target_is_rejected() {
        let dest = destination();
        let mut route = VideoSlotBinding {
            track_id: String::new(),
            participant: Uuid::nil(),
            kind: VideoKind::Camera,
            fence: None,
            destination: None,
        };
        assert!(dest.matches(&route));
        route.kind = VideoKind::Screen;
        assert!(!dest.matches(&route));
        route.kind = VideoKind::Camera;
        route.participant = Uuid::from_u128(1);
        assert!(!dest.matches(&route));
    }

    #[test]
    fn revocation_invalidates_lease_and_rejects_new_samples() {
        let dest = destination();
        let generation = dest.reset();
        dest.put(generation, &sample(4, 1));
        let lease = dest.take().unwrap();
        dest.revoke();
        assert!(!lease.is_current());
        dest.put(generation, &sample(4, 2));
        drop(lease);
        assert!(dest.take().is_none());
    }
}
