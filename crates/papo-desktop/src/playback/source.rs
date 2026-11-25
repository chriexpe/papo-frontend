//! Encoded bytes as a pipeline's source: an `appsrc` serving slices of the bytes in memory,
//! seekable, so a container with its index at the end plays without a file. The bytes come
//! from the SDK's media cache; this only reads them.

use std::sync::{Arc, Mutex, PoisonError};

use gstreamer as gst;
use gstreamer_app as gst_app;

/// The most one read hands the pipeline when it states no length.
const CHUNK_BYTES: u64 = 64 * 1024;

/// A slice of shared bytes that a buffer holds without copying.
struct Chunk {
    bytes: Arc<[u8]>,
    start: usize,
    end: usize,
}

impl AsRef<[u8]> for Chunk {
    fn as_ref(&self) -> &[u8] {
        &self.bytes[self.start..self.end]
    }
}

/// An `appsrc` reading `bytes` from wherever the pipeline seeks.
pub(super) fn byte_source(bytes: Arc<[u8]>) -> gst_app::AppSrc {
    let total = bytes.len() as u64;
    let source = gst_app::AppSrc::builder()
        .stream_type(gst_app::AppStreamType::RandomAccess)
        .format(gst::Format::Bytes)
        .size(i64::try_from(total).unwrap_or(i64::MAX))
        .build();
    let offset = Arc::new(Mutex::new(0_u64));
    let seek_to = Arc::clone(&offset);
    source.set_callbacks(
        gst_app::AppSrcCallbacks::builder()
            .need_data(move |source, length| {
                let mut at = offset.lock().unwrap_or_else(PoisonError::into_inner);
                if *at >= total {
                    let _ = source.end_of_stream();
                    return;
                }
                let wanted = if length == 0 {
                    CHUNK_BYTES
                } else {
                    u64::from(length)
                };
                let end = at.saturating_add(wanted).min(total);
                let (Ok(start), Ok(stop)) = (usize::try_from(*at), usize::try_from(end)) else {
                    let _ = source.end_of_stream();
                    return;
                };
                let mut buffer = gst::Buffer::from_slice(Chunk {
                    bytes: Arc::clone(&bytes),
                    start,
                    end: stop,
                });
                if let Some(buffer) = buffer.get_mut() {
                    buffer.set_offset(*at);
                }
                *at = end;
                drop(at);
                let _ = source.push_buffer(buffer);
            })
            .seek_data(move |_, to| {
                *seek_to.lock().unwrap_or_else(PoisonError::into_inner) = to.min(total);
                true
            })
            .build(),
    );
    source
}
