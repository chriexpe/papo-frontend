//! One bounded request path from core workers to GPUI's native credential tasks.
//!
//! Shutdown closes the outstanding reply synchronously. It never needs GPUI to
//! poll a canceled task before a core worker can join. No operation is logged.

use std::sync::{Arc, Mutex};

use super::{CredentialError, CredentialOperation, generation::Generation};
use papo_core::storage::Secret;

type Reply = Result<Option<String>, CredentialError>;

pub(super) enum Operation {
    Read,
    Write(String),
    Delete,
}

impl Operation {
    pub(super) fn kind(&self) -> CredentialOperation {
        match self {
            Self::Read => CredentialOperation::Read,
            Self::Write(_) => CredentialOperation::Write,
            Self::Delete => CredentialOperation::Delete,
        }
    }
}

pub(super) struct Request {
    pub(super) key: String,
    pub(super) server: String,
    pub(super) secret: Secret,
    pub(super) operation: Operation,
    pub(super) generation: Arc<Generation>,
    reply: async_channel::Sender<Reply>,
}

impl Request {
    pub(super) fn is_canceled(&self) -> bool {
        self.reply.is_closed()
    }

    pub(super) fn complete(self, result: Reply) {
        // Cancellation can close the reply before the OS operation finishes.
        let _ = self.reply.try_send(result);
    }
}

impl Drop for Request {
    fn drop(&mut self) {
        self.reply.close();
    }
}

#[derive(Default)]
struct Pending {
    stopped: bool,
    reply: Option<async_channel::Sender<Reply>>,
}

pub(super) struct Bridge {
    requests: async_channel::Sender<Request>,
    pending: Mutex<Pending>,
    foreground: std::thread::ThreadId,
}

impl Bridge {
    pub(super) fn new() -> (Arc<Self>, async_channel::Receiver<Request>) {
        // The store serializes operations, so at most one request is outstanding.
        let (requests, receiver) = async_channel::bounded(1);
        (
            Arc::new(Self {
                requests,
                pending: Mutex::default(),
                foreground: std::thread::current().id(),
            }),
            receiver,
        )
    }

    pub(super) fn check_worker(&self) -> Result<(), CredentialError> {
        if self.foreground == std::thread::current().id() {
            Err(CredentialError::ForegroundCall)
        } else {
            Ok(())
        }
    }

    /// Called only under the store's operation lock, never on GPUI's thread.
    pub(super) fn request(
        &self,
        generation: &Arc<Generation>,
        server: &str,
        secret: Secret,
        operation: Operation,
    ) -> Reply {
        let (reply, receiver) = async_channel::bounded(1);
        {
            let mut pending = self.pending.lock().map_err(|_| CredentialError::Internal)?;
            if pending.stopped {
                return Err(CredentialError::Stopped);
            }
            pending.reply = Some(reply.clone());
            let request = Request {
                key: key(server, secret),
                server: server.to_owned(),
                secret,
                operation,
                generation: Arc::clone(generation),
                reply,
            };
            if self.requests.try_send(request).is_err() {
                pending.reply = None;
                return Err(CredentialError::Stopped);
            }
        }
        let result = receiver
            .recv_blocking()
            .unwrap_or(Err(CredentialError::Stopped));
        self.pending
            .lock()
            .map_err(|_| CredentialError::Internal)?
            .reply = None;
        result
    }

    pub(super) fn is_running(&self) -> bool {
        !self.requests.is_closed()
    }

    pub(super) fn stop(&self) {
        // This lock never covers I/O, waits, user callbacks or memory-store work.
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        pending.stopped = true;
        self.requests.close();
        if let Some(reply) = pending.reply.take() {
            reply.close();
        }
    }
}

/// Hex is injective even for arbitrary keys; separators cannot alias a secret kind.
fn key(server: &str, secret: Secret) -> String {
    use std::fmt::Write as _;
    let mut encoded = String::with_capacity(server.len() * 2);
    for byte in server.bytes() {
        // Formatting into String cannot fail.
        let _ = write!(encoded, "{byte:02x}");
    }
    format!(
        "io.github.chriexpe.Papo.Desktop/credentials/v1/{encoded}/{}",
        secret.key()
    )
}
