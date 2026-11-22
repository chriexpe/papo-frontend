//! The bridge between the UI thread and the SDK client.
//!
//! Owns the bridge thread, which opens the SDK [`Client`] (the client owns
//! its Tokio runtime), drives the UI's requests on that runtime, and shuts the
//! client down when the window closes: the exit fence. Results come back to
//! the UI thread as callbacks posted through Rune's app handle ([`Post`]), and
//! a request's result is applied only while its [`Ticket`] is current.
//!
//! Account, server and message state stay in the SDK; this module carries
//! requests and results, and keeps no copy of them.

#[cfg(test)]
mod tests;
mod ticket;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::mpsc as std_mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use papo_sdk::secret::SecretStore;
use papo_sdk::{Client, ClientConfig, ClientError, ClientHandle, DataRoot, StorageShutdown};
use rune::{AppHandle, Tx};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinSet;

pub(crate) use ticket::{RequestSlot, Ticket};

/// A change to the UI state, run on the UI thread. It returns whether it
/// changed what the view reads, as `AppHandle::update_if` expects.
pub(crate) type Callback<S> = Box<dyn FnOnce(&mut S, &mut Tx) -> bool + Send>;

/// Where the bridge sends callbacks: Rune's app handle in the app, a mailbox
/// in headless tests.
pub(crate) trait Post<S>: Send + Sync {
    /// Queues `callback` for the UI thread. Callbacks run in posting order.
    fn post(&self, callback: Callback<S>);
}

impl<S: 'static> Post<S> for AppHandle<S> {
    fn post(&self, callback: Callback<S>) {
        self.update_if(callback);
    }
}

/// The UI state's side of the bridge: what it hears when the client opens.
pub(crate) trait BridgeHost: Sized + 'static {
    /// The client is open; `sdk` sends it requests from now on.
    fn sdk_opened(&mut self, sdk: Sdk<Self>);
    /// The client could not open; the app runs without it.
    fn sdk_failed(&mut self, failure: OpenFailure);
}

/// Why the SDK client did not open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OpenFailure {
    /// Another Papo process owns the profile.
    ProfileInUse,
    /// Storage or the runtime failed; the log has the category.
    Failed,
}

/// A request's work, given the client, run on the client's runtime.
type Job = Box<dyn FnOnce(ClientHandle) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send>;

/// A job on its way to the bridge, and what the exit fence does with it.
struct Queued {
    job: Job,
    /// The fence lets it finish, up to [`FINISH_DEADLINE`], rather than
    /// cancelling it.
    finish: bool,
}

/// How long the exit fence waits for the jobs sent with [`Sdk::finish`].
const FINISH_DEADLINE: Duration = Duration::from_secs(2);

/// The UI's way to send requests to the open client. Cloned freely; requests
/// sent after the exit fence are dropped.
pub(crate) struct Sdk<S> {
    jobs: mpsc::UnboundedSender<Queued>,
    post: Arc<dyn Post<S>>,
}

impl<S> Clone for Sdk<S> {
    fn clone(&self) -> Self {
        Sdk {
            jobs: self.jobs.clone(),
            post: Arc::clone(&self.post),
        }
    }
}

impl<S: 'static> Sdk<S> {
    /// Runs `work` on the client's runtime, then `apply`s its result on the UI
    /// thread, unless `slot` has issued a newer ticket than `ticket` by then:
    /// a superseded result is dropped, and the view does not redraw for it.
    /// `apply` returns whether it changed what the view reads.
    pub(crate) fn request<T, Work, Done>(
        &self,
        ticket: Ticket,
        slot: fn(&S) -> &RequestSlot,
        work: impl FnOnce(ClientHandle) -> Work + Send + 'static,
        apply: Done,
    ) where
        T: Send + 'static,
        Work: Future<Output = T> + Send + 'static,
        Done: FnOnce(&mut S, T) -> bool + Send + 'static,
    {
        let post = Arc::clone(&self.post);
        let job: Job = Box::new(move |client| {
            Box::pin(async move {
                let value = work(client).await;
                post.post(Box::new(move |state, _tx| {
                    if !slot(state).is_current(ticket) {
                        log::debug!("dropped the result of a superseded request");
                        return false;
                    }
                    apply(state, value)
                }));
            })
        });
        self.send(job, false);
    }

    /// Runs `work` on the client's runtime until it returns or the exit fence
    /// cancels it. Meanwhile it posts what it sees to the UI thread through
    /// [`Updates`], as often as it needs: a subscription, not a request.
    pub(crate) fn watch<Work>(
        &self,
        work: impl FnOnce(ClientHandle, Updates<S>) -> Work + Send + 'static,
    ) where
        Work: Future<Output = ()> + Send + 'static,
    {
        let updates = Updates {
            post: Arc::clone(&self.post),
        };
        self.send(
            Box::new(move |client| Box::pin(work(client, updates))),
            false,
        );
    }

    /// Runs `work` on the client's runtime for its effect alone: the UI waits
    /// for no answer, and the exit fence cancels it if it still runs.
    pub(crate) fn run<Work>(&self, work: impl FnOnce(ClientHandle) -> Work + Send + 'static)
    where
        Work: Future<Output = ()> + Send + 'static,
    {
        self.send(Box::new(move |client| Box::pin(work(client))), false);
    }

    /// [`Sdk::run`] for work that must land, such as a save: sent before
    /// the exit fence, the fence lets it finish (up to a deadline) instead
    /// of cancelling it.
    pub(crate) fn finish<Work>(&self, work: impl FnOnce(ClientHandle) -> Work + Send + 'static)
    where
        Work: Future<Output = ()> + Send + 'static,
    {
        self.send(Box::new(move |client| Box::pin(work(client))), true);
    }

    /// A way for a job sent with [`Sdk::run`] or [`Sdk::finish`] to post
    /// what it found back to the UI thread, when no ticket decides whether
    /// the answer still counts.
    pub(crate) fn updates(&self) -> Updates<S> {
        Updates {
            post: Arc::clone(&self.post),
        }
    }

    fn send(&self, job: Job, finish: bool) {
        if self.jobs.send(Queued { job, finish }).is_err() {
            log::debug!("dropped a request sent after the SDK shut down");
        }
    }
}

/// A watch's way to change the UI state: each change runs on the UI thread,
/// in the order sent.
pub(crate) struct Updates<S> {
    post: Arc<dyn Post<S>>,
}

impl<S: 'static> Updates<S> {
    /// Queues `change`, which returns whether it changed what the view reads.
    pub(crate) fn send(&self, change: impl FnOnce(&mut S) -> bool + Send + 'static) {
        self.post.post(Box::new(move |state, _tx| change(state)));
    }

    /// [`Updates::send`] for a change that also asks Rune for something,
    /// such as a scroll.
    pub(crate) fn send_tx(&self, change: impl FnOnce(&mut S, &mut Tx) -> bool + Send + 'static) {
        self.post.post(Box::new(change));
    }
}

impl<S> Clone for Updates<S> {
    fn clone(&self) -> Self {
        Updates {
            post: Arc::clone(&self.post),
        }
    }
}

/// What the bridge thread needs to open the client.
pub(crate) struct Profile {
    /// The SDK's data directory.
    pub(crate) root: DataRoot,
    /// Where credentials live.
    pub(crate) secrets: Arc<dyn SecretStore>,
    /// The client's limits.
    pub(crate) config: ClientConfig,
}

/// The running bridge. Dropping it, or [`Bridge::shutdown`], is the exit
/// fence: the client shuts down and the bridge thread is joined.
pub(crate) struct Bridge {
    stop: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
    ended: std_mpsc::Receiver<()>,
}

/// How long the exit fence waits for the bridge thread. Opening a profile can
/// wait on the keyring (an unlock prompt) and on each saved account's server,
/// up to the SDK's own deadlines per account; past this the process ends
/// without it, and the OS releases the profile lock.
const FENCE_DEADLINE: Duration = Duration::from_secs(10);

impl Bridge {
    /// Starts the bridge thread, which opens the client and then posts
    /// [`BridgeHost::sdk_opened`] or [`BridgeHost::sdk_failed`].
    ///
    /// # Errors
    ///
    /// The OS could not start a thread.
    pub(crate) fn start<S: BridgeHost>(
        profile: Profile,
        post: Arc<dyn Post<S>>,
    ) -> std::io::Result<Bridge> {
        let (stop, stopped) = oneshot::channel();
        let (ended_tx, ended) = std_mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("papo-sdk-bridge".to_owned())
            .spawn(move || {
                serve(profile, &post, stopped);
                // The fence may have stopped waiting; nobody to tell then.
                let _ = ended_tx.send(());
            })?;
        Ok(Bridge {
            stop: Some(stop),
            thread: Some(thread),
            ended,
        })
    }

    /// The exit fence: stops admitting requests, cancels those running, shuts
    /// the client down (it drains accepted writes and joins its threads) and
    /// joins the bridge thread. Returns once all of that has happened, or
    /// after [`FENCE_DEADLINE`] with the thread left behind.
    pub(crate) fn shutdown(mut self) {
        self.fence();
    }

    fn fence(&mut self) {
        // The thread may have ended already (the client failed to open).
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        let Some(thread) = self.thread.take() else {
            return;
        };
        match self.ended.recv_timeout(FENCE_DEADLINE) {
            Ok(()) | Err(std_mpsc::RecvTimeoutError::Disconnected) => {
                if thread.join().is_err() {
                    log::error!("the SDK bridge thread panicked");
                }
            }
            Err(std_mpsc::RecvTimeoutError::Timeout) => {
                log::warn!("the SDK was still opening or shutting down at exit; leaving it");
            }
        }
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        self.fence();
    }
}

/// The bridge thread: open, serve requests until stopped, shut down.
fn serve<S: BridgeHost>(profile: Profile, post: &Arc<dyn Post<S>>, stopped: oneshot::Receiver<()>) {
    let Profile {
        root,
        secrets,
        config,
    } = profile;
    let mut client = match Client::open(root, secrets, config) {
        Ok(client) => client,
        Err(error) => {
            let failure = open_failure(&error);
            log::warn!("the SDK client did not open: {error}");
            post.post(Box::new(move |state, _tx| {
                state.sdk_failed(failure);
                true
            }));
            return;
        }
    };
    log::info!("the SDK client is open");
    let (jobs, queue) = mpsc::unbounded_channel();
    let sdk = Sdk {
        jobs,
        post: Arc::clone(post),
    };
    post.post(Box::new(move |state, _tx| {
        state.sdk_opened(sdk);
        true
    }));
    let handle = client.handle();
    if let Err(error) = client.block_on(run_jobs(handle, queue, stopped)) {
        log::error!("the SDK bridge could not run on the client's runtime: {error}");
    }
    match client.shutdown() {
        Ok(report) => log_shutdown(&report),
        Err(error) => log::error!("the SDK client did not shut down cleanly: {error}"),
    }
}

/// Runs every job on the client's runtime until the fence stops the bridge.
/// Then the jobs sent with [`Sdk::finish`], queued ones included, get up to
/// [`FINISH_DEADLINE`] to land, and the rest are cancelled.
async fn run_jobs(
    client: ClientHandle,
    mut queue: mpsc::UnboundedReceiver<Queued>,
    mut stopped: oneshot::Receiver<()>,
) {
    let mut running = JoinSet::new();
    let mut finishing = JoinSet::new();
    loop {
        tokio::select! {
            biased;
            // A dropped `Bridge` stops the bridge as `shutdown` does.
            _ = &mut stopped => break,
            queued = queue.recv() => {
                let Some(queued) = queued else {
                    // The UI dropped every `Sdk`; nothing more can arrive.
                    let _ = (&mut stopped).await;
                    break;
                };
                spawn(&client, queued, &mut running, &mut finishing);
            }
            Some(done) = running.join_next(), if !running.is_empty() => report(done),
            Some(done) = finishing.join_next(), if !finishing.is_empty() => report(done),
        }
    }
    // Sent before the fence, so still owed: the window's last saves.
    while let Ok(queued) = queue.try_recv() {
        if queued.finish {
            spawn(&client, queued, &mut running, &mut finishing);
        }
    }
    let landed = tokio::time::timeout(FINISH_DEADLINE, async {
        while let Some(done) = finishing.join_next().await {
            report(done);
        }
    })
    .await;
    if landed.is_err() {
        log::warn!("the exit fence cancelled work that did not finish in time");
    }
    finishing.shutdown().await;
    running.shutdown().await;
}

fn spawn(
    client: &ClientHandle,
    queued: Queued,
    running: &mut JoinSet<()>,
    finishing: &mut JoinSet<()>,
) {
    let work = (queued.job)(client.clone());
    if queued.finish {
        finishing.spawn(work);
    } else {
        running.spawn(work);
    }
}

fn report(done: Result<(), tokio::task::JoinError>) {
    if done.is_err_and(|error| error.is_panic()) {
        log::error!("an SDK request panicked; its result never arrives");
    }
}

fn open_failure(error: &ClientError) -> OpenFailure {
    match error {
        ClientError::ProfileInUse => OpenFailure::ProfileInUse,
        _ => OpenFailure::Failed,
    }
}

/// Logs what the SDK's shutdown could not persist. Counts only: the report
/// names no content.
fn log_shutdown(report: &StorageShutdown) {
    let unpersisted = report.state.unpersisted.len() + report.shared.unpersisted.len();
    let failures = report.state.failures.len() + report.shared.failures.len();
    let deadline = report.state.deadline_reached || report.shared.deadline_reached;
    if unpersisted == 0 && failures == 0 && !deadline && report.reads_drained {
        log::info!("the SDK client shut down cleanly");
    } else {
        log::warn!(
            "the SDK client shut down with {unpersisted} unpersisted writes and \
             {failures} database failures (deadline reached: {deadline}, reads drained: {})",
            report.reads_drained
        );
    }
}
