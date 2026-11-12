//! Owns payload-free read request counters and cancellation-safe inflight tracking; leaves upload policy outside.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use crate::TestServer;

/// Aggregate counters since fixture startup, including scripted failures and unauthorized requests.
/// No account, channel, message ID or body is retained by this instrumentation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ReadRequestCounts {
    /// Number of GET /channels requests admitted.
    pub channel_lists: usize,
    /// Number of GET /channels/{id}/messages requests admitted, including failures.
    pub history_pages: usize,
    /// Number of message rows returned by successful history responses.
    pub history_rows: usize,
    /// Number of GET /channels/{id}/pinned requests admitted.
    pub pin_lists: usize,
    /// Number of PUT /channels/{id}/read requests admitted.
    pub uploads: usize,
    /// Read uploads currently being handled, including delayed or held responses.
    pub inflight: usize,
    /// Largest simultaneous inflight upload count observed across the fixture.
    pub max_inflight: usize,
}

#[derive(Default)]
pub(crate) struct ReadRequests {
    channel_lists: AtomicUsize,
    history_pages: AtomicUsize,
    history_rows: AtomicUsize,
    pin_lists: AtomicUsize,
    uploads: AtomicUsize,
    inflight: AtomicUsize,
    max_inflight: AtomicUsize,
}

pub(crate) struct UploadGuard(Arc<ReadRequests>);

impl ReadRequests {
    pub(crate) fn record(self: &Arc<Self>, method: &str, path: &str) -> Option<UploadGuard> {
        if method == "GET" && path == "/channels" {
            self.channel_lists.fetch_add(1, Ordering::SeqCst);
        }
        if method == "GET"
            && path
                .strip_prefix("/channels/")
                .and_then(|tail| tail.strip_suffix("/messages"))
                .is_some_and(|id| !id.is_empty() && !id.contains('/'))
        {
            self.history_pages.fetch_add(1, Ordering::SeqCst);
        }
        if method == "GET"
            && path
                .strip_prefix("/channels/")
                .and_then(|tail| tail.strip_suffix("/pinned"))
                .is_some_and(|id| !id.is_empty() && !id.contains('/'))
        {
            self.pin_lists.fetch_add(1, Ordering::SeqCst);
        }
        if method != "PUT" || super::read_marks::channel_id(path).is_none() {
            return None;
        }
        self.uploads.fetch_add(1, Ordering::SeqCst);
        let inflight = self.inflight.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_inflight.fetch_max(inflight, Ordering::SeqCst);
        Some(UploadGuard(Arc::clone(self)))
    }

    pub(crate) fn history_rows(&self, rows: usize) {
        self.history_rows.fetch_add(rows, Ordering::SeqCst);
    }

    pub(crate) fn snapshot(&self) -> ReadRequestCounts {
        ReadRequestCounts {
            channel_lists: self.channel_lists.load(Ordering::SeqCst),
            history_pages: self.history_pages.load(Ordering::SeqCst),
            history_rows: self.history_rows.load(Ordering::SeqCst),
            pin_lists: self.pin_lists.load(Ordering::SeqCst),
            uploads: self.uploads.load(Ordering::SeqCst),
            inflight: self.inflight.load(Ordering::SeqCst),
            max_inflight: self.max_inflight.load(Ordering::SeqCst),
        }
    }
}

impl Drop for UploadGuard {
    fn drop(&mut self) {
        self.0.inflight.fetch_sub(1, Ordering::SeqCst);
    }
}

impl TestServer {
    /// Returns aggregate read-route counters without storing request data.
    /// Held/delayed responses remain inflight until released or canceled by server shutdown.
    pub async fn read_request_counts(&self) -> ReadRequestCounts {
        self.state.lock().await.read_requests.snapshot()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guards_track_overlapping_uploads_and_release_on_drop() {
        let requests = Arc::new(ReadRequests::default());
        let first = requests.record("PUT", "/channels/a/read");
        let second = requests.record("PUT", "/channels/b/read");
        drop(first);
        drop(second);
        assert_eq!(
            requests.snapshot(),
            ReadRequestCounts {
                uploads: 2,
                max_inflight: 2,
                ..ReadRequestCounts::default()
            }
        );
    }

    #[test]
    fn history_metrics_count_page_attempts_and_only_explicit_success_rows() {
        let requests = Arc::new(ReadRequests::default());
        requests.record("GET", "/channels/a/messages");
        requests.record("GET", "/channels/a/messages");
        requests.record("GET", "/channels/a/pinned");
        requests.history_rows(100);
        assert_eq!(
            requests.snapshot(),
            ReadRequestCounts {
                history_pages: 2,
                history_rows: 100,
                pin_lists: 1,
                ..ReadRequestCounts::default()
            }
        );
    }

    #[test]
    fn list_and_unrelated_routes_do_not_count_as_uploads() {
        let requests = Arc::new(ReadRequests::default());
        requests.record("GET", "/channels");
        requests.record("GET", "/channels/a/read");
        requests.record("PUT", "/channels/a/permissions");
        assert_eq!(
            requests.snapshot(),
            ReadRequestCounts {
                channel_lists: 1,
                ..ReadRequestCounts::default()
            }
        );
    }
}
