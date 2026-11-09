//! Counts member endpoint attempts for bounded-refresh proofs; serving and resource payloads live elsewhere.

use crate::TestServer;

/// Cumulative member endpoint requests since this fixture started, including held or failed requests.
/// Only these three routes are counted; no payloads or user identifiers are retained.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MemberRequestCounts {
    /// `GET /users` list pages.
    pub list_requests: u64,
    /// `POST /users/user_summary_batch` requests.
    pub summary_requests: u64,
    /// `POST /users/profile_batch` requests.
    pub profile_requests: u64,
}

impl MemberRequestCounts {
    pub(crate) fn record(&mut self, method: &str, path: &str) {
        let count = match (method, path) {
            ("GET", "/users") => &mut self.list_requests,
            ("POST", "/users/user_summary_batch") => &mut self.summary_requests,
            ("POST", "/users/profile_batch") => &mut self.profile_requests,
            _ => return,
        };
        *count = count.saturating_add(1);
    }
}

impl TestServer {
    /// Reads member endpoint counts atomically with respect to request admission.
    pub async fn member_request_counts(&self) -> MemberRequestCounts {
        self.state.lock().await.member_requests
    }
}
