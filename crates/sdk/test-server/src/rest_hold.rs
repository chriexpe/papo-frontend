//! Owns deterministic one-request response barriers; leaves route payloads and TCP serving outside.

use tokio::sync::oneshot;

use crate::{Error, TestServer};

pub(crate) struct PendingHold {
    pub(crate) method: String,
    pub(crate) path: String,
    started: oneshot::Sender<()>,
    released: oneshot::Receiver<()>,
}

/// A one-request barrier that captures a REST response before it is delivered.
///
/// Await [`Self::wait_started`] before mutating fixture state or emitting journaled events.
/// Dropping the hold releases the request, so cancellation cannot strand a connection.
/// No state lock is held while the response waits.
pub struct RestHold {
    started: Option<oneshot::Receiver<()>>,
    release: oneshot::Sender<()>,
}

impl RestHold {
    /// Waits until the matching request has captured its response.
    /// # Errors
    /// Returns [`Error::Connection`] if the server or request disappeared first, or
    /// [`Error::Configuration`] if this wait already completed. Canceling the wait is safe.
    pub async fn wait_started(&mut self) -> Result<(), Error> {
        let started = self.started.as_mut().ok_or(Error::Configuration)?;
        let result = started.await;
        self.started = None;
        result.map_err(|_| Error::Connection)
    }

    /// Releases the captured response. Dropping this handle also releases it.
    pub fn release(self) {
        let _ = self.release.send(());
    }
}

impl PendingHold {
    pub(crate) async fn wait(self) {
        let _ = self.started.send(());
        let _ = self.released.await;
    }
}

impl TestServer {
    /// Holds the next matching response after snapshot capture until released.
    ///
    /// Matches the HTTP method and path exactly, excluding the query string. Holds
    /// are consumed in insertion order per matching route and work with existing delays.
    /// # Errors
    /// Returns [`Error::Configuration`] for an invalid method or non-absolute path.
    pub async fn hold_next_rest(&self, method: &str, path: &str) -> Result<RestHold, Error> {
        if hyper::Method::from_bytes(method.as_bytes()).is_err() || !path.starts_with('/') {
            return Err(Error::Configuration);
        }
        let (started, observed) = oneshot::channel();
        let (release, released) = oneshot::channel();
        self.state.lock().await.holds.push(PendingHold {
            method: method.to_owned(),
            path: path.to_owned(),
            started,
            released,
        });
        Ok(RestHold {
            started: Some(observed),
            release,
        })
    }
}
