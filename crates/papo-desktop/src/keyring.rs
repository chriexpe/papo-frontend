//! The SDK's credential store on the OS keyring.
//!
//! Owns the keyring worker: one thread that runs every credential call, one
//! at a time, in the order the SDK submitted them. Which keyring answers is
//! the backend's business (`secret_service`); deadlines, cancellation and
//! uncertain outcomes are the SDK's (`papo_sdk::secret`).
//!
//! **Why one queue is the fence.** The SDK may drop a call's future (a
//! timeout, a cancelled sign-in) while the OS call goes on. A call that has
//! started runs to its end on the worker, so it stays within its own key, and
//! every later call waits behind it. A call whose caller gave up before it
//! started is skipped: it never touches the keyring, and an unanswered unlock
//! prompt does not come back once per abandoned call. A read therefore runs
//! after every mutation submitted before it has either finished or been
//! skipped, which is what [`SecretRead::settled`] promises.
//!
//! **Across a restart** the claim rests on an assumption: the earlier process
//! has exited (the profile lock admits one owner at a time), and the keyring
//! daemon does not apply a call after its caller's D-Bus connection closed.
//! GNOME Keyring and KWallet dismiss an open prompt with the connection; a
//! backend that blocks the call itself on a confirmation dialog (KeePassXC)
//! could still apply an old process's write after the new process read the
//! entry. A call still running at exit is left behind ([`Keyring::close`]),
//! which is when that can happen.
//!
//! A call never starts before its future is first polled: the request is
//! queued from inside the future.

mod secret_service;
#[cfg(test)]
mod tests;

use std::future::Future;
use std::sync::Mutex;
use std::sync::mpsc as std_mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use papo_sdk::secret::{
    SecretFuture, SecretKey, SecretRead, SecretStore, SecretStoreError, SecretValue,
};
use tokio::sync::{mpsc, oneshot};

pub(crate) use secret_service::SecretService;

/// How long the exit fence waits for a keyring call still running (an open
/// unlock prompt, a stalled daemon) before it leaves the thread behind.
const CLOSE_DEADLINE: Duration = Duration::from_secs(2);

/// A keyring that stores credential bytes under a [`SecretKey`]. Each method
/// finishes its OS effects before it returns.
pub(crate) trait Backend {
    /// The entry's bytes, or `None` when it does not exist.
    fn read(
        &mut self,
        key: SecretKey,
    ) -> impl Future<Output = Result<Option<Vec<u8>>, SecretStoreError>>;
    /// Creates the entry or replaces its bytes.
    fn write(
        &mut self,
        key: SecretKey,
        value: SecretValue,
    ) -> impl Future<Output = Result<(), SecretStoreError>>;
    /// Removes the entry; an absent entry is success.
    fn delete(&mut self, key: SecretKey) -> impl Future<Output = Result<(), SecretStoreError>>;
}

/// One credential call waiting for the worker, with where its answer goes.
enum Request {
    Read {
        key: SecretKey,
        reply: oneshot::Sender<Result<Option<Vec<u8>>, SecretStoreError>>,
    },
    Write {
        key: SecretKey,
        value: SecretValue,
        reply: oneshot::Sender<Result<(), SecretStoreError>>,
    },
    Delete {
        key: SecretKey,
        reply: oneshot::Sender<Result<(), SecretStoreError>>,
    },
}

impl Request {
    /// Whether the SDK stopped waiting for this call (it timed out or was
    /// cancelled).
    fn abandoned(&self) -> bool {
        match self {
            Request::Read { reply, .. } => reply.is_closed(),
            Request::Write { reply, .. } | Request::Delete { reply, .. } => reply.is_closed(),
        }
    }
}

/// The SDK's [`SecretStore`], answered by one keyring worker thread.
pub(crate) struct Keyring {
    /// `None` once [`Keyring::close`] has run: later calls fail as unavailable.
    requests: Mutex<Option<mpsc::UnboundedSender<Request>>>,
    worker: Mutex<Option<Worker>>,
}

/// The worker thread and the signal it sends when it ends.
struct Worker {
    thread: JoinHandle<()>,
    ended: std_mpsc::Receiver<()>,
}

impl Keyring {
    /// Starts the worker. `backend` runs on the worker thread, so a backend
    /// that is not `Send` is fine; it connects to the OS on its first call.
    ///
    /// # Errors
    ///
    /// The OS could not start a thread.
    pub(crate) fn start<B: Backend + 'static>(
        backend: impl FnOnce() -> B + Send + 'static,
    ) -> std::io::Result<Keyring> {
        let (requests, queue) = mpsc::unbounded_channel();
        let (ended_tx, ended) = std_mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("papo-keyring".to_owned())
            .spawn(move || {
                serve(backend(), queue);
                // The fence may already have stopped waiting; nobody to tell then.
                let _ = ended_tx.send(());
            })?;
        Ok(Keyring {
            requests: Mutex::new(Some(requests)),
            worker: Mutex::new(Some(Worker { thread, ended })),
        })
    }

    /// The exit fence's part: refuses new calls, lets the call in progress
    /// and those queued finish, and joins the thread. A call still running
    /// after [`CLOSE_DEADLINE`] keeps its thread; the process is ending.
    pub(crate) fn close(&self) {
        drop(lock(&self.requests).take());
        let Some(worker) = lock(&self.worker).take() else {
            return;
        };
        match worker.ended.recv_timeout(CLOSE_DEADLINE) {
            Ok(()) | Err(std_mpsc::RecvTimeoutError::Disconnected) => {
                if worker.thread.join().is_err() {
                    log::warn!("the keyring worker panicked");
                }
            }
            Err(std_mpsc::RecvTimeoutError::Timeout) => {
                log::warn!("a keyring call was still running at exit; leaving it");
            }
        }
    }

    /// Queues `request`, or fails when the keyring is closed or its worker
    /// has ended.
    fn submit(&self, request: Request) -> Result<(), SecretStoreError> {
        let requests = lock(&self.requests);
        let sender = requests.as_ref().ok_or(SecretStoreError::Unavailable)?;
        sender
            .send(request)
            .map_err(|_| SecretStoreError::Unavailable)
    }
}

impl SecretStore for Keyring {
    fn read(&self, key: SecretKey) -> SecretFuture<'_, SecretRead> {
        Box::pin(async move {
            let (reply, answer) = oneshot::channel();
            self.submit(Request::Read { key, reply })?;
            let value = answer.await.map_err(|_| SecretStoreError::Unavailable)??;
            // Every mutation submitted before this read has finished: see the
            // module header.
            Ok(SecretRead::settled(value.map(SecretValue::new)))
        })
    }

    fn write(&self, key: SecretKey, value: SecretValue) -> SecretFuture<'_, ()> {
        Box::pin(async move {
            let (reply, answer) = oneshot::channel();
            self.submit(Request::Write { key, value, reply })?;
            answer.await.map_err(|_| SecretStoreError::Unavailable)?
        })
    }

    fn delete(&self, key: SecretKey) -> SecretFuture<'_, ()> {
        Box::pin(async move {
            let (reply, answer) = oneshot::channel();
            self.submit(Request::Delete { key, reply })?;
            answer.await.map_err(|_| SecretStoreError::Unavailable)?
        })
    }
}

/// The worker's loop: one request at a time, in submission order, until
/// every sender is gone.
fn serve<B: Backend>(mut backend: B, mut queue: mpsc::UnboundedReceiver<Request>) {
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        log::warn!("the keyring worker could not start its runtime");
        refuse_all(queue);
        return;
    };
    runtime.block_on(async move {
        while let Some(request) = queue.recv().await {
            if request.abandoned() {
                // The caller gave up before the call started: nothing has
                // touched the keyring, so skipping it keeps later reads fenced.
                continue;
            }
            match request {
                Request::Read { key, reply } => {
                    let _ = reply.send(backend.read(key).await);
                }
                Request::Write { key, value, reply } => {
                    let _ = reply.send(backend.write(key, value).await);
                }
                Request::Delete { key, reply } => {
                    let _ = reply.send(backend.delete(key).await);
                }
            }
        }
    });
}

/// Answers every request as unavailable, for a worker that cannot run.
fn refuse_all(mut queue: mpsc::UnboundedReceiver<Request>) {
    while let Some(request) = queue.blocking_recv() {
        match request {
            Request::Read { reply, .. } => {
                let _ = reply.send(Err(SecretStoreError::Unavailable));
            }
            Request::Write { reply, .. } | Request::Delete { reply, .. } => {
                let _ = reply.send(Err(SecretStoreError::Unavailable));
            }
        }
    }
}

/// Locks `mutex`, taking the value over from a thread that panicked holding
/// it: the guarded values stay valid at every step.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
