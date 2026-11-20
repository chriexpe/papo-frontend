//! The process around the window: one running Papo per user, and the signals
//! that end it.
//!
//! Owns the lifecycle thread. It claims Papo's name on the session bus before
//! the window exists (`single_instance`), holds it while the process runs,
//! turns later launches into window activations and SIGTERM or SIGINT into a
//! quit (`exit_signals`), and lets go of the name at the end of the exit
//! fence. The window and the quit itself are Rune's ([`AppControl`]); the
//! SDK's shutdown is `sdk_bridge`'s.

mod exit_signals;
mod single_instance;
#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc as std_mpsc;
use std::thread::JoinHandle;

use tokio::sync::oneshot;

use self::exit_signals::ExitSignals;
use self::single_instance::{Claim, DEADLINES, Instance, Launch};

/// What the lifecycle thread asks of the app.
pub(crate) trait AppControl: Send + Sync {
    /// Shows and focuses the window, with the launcher's token when a later
    /// launch forwarded one.
    fn activate(&self, token: Option<String>);
    /// Quits as the window closing would: the exit fence runs after.
    fn quit(&self);
}

impl<S: 'static> AppControl for rune::AppHandle<S> {
    fn activate(&self, token: Option<String>) {
        match token {
            Some(token) => rune::AppHandle::activate(self, token),
            None => self.open(),
        }
    }

    fn quit(&self) {
        rune::AppHandle::quit(self);
    }
}

/// How the launch starts.
pub(crate) enum Start {
    /// This process runs Papo.
    Running(Lifecycle),
    /// The running Papo took the launch and raises its window; this process
    /// exits without opening one.
    HandedOff,
}

/// The running lifecycle thread. [`Lifecycle::quitting`] and
/// [`Lifecycle::stop`] are its parts of the exit fence.
pub(crate) struct Lifecycle {
    attach: Option<oneshot::Sender<Arc<dyn AppControl>>>,
    /// Set once the app has quit: later launches are refused.
    quitting: Arc<AtomicBool>,
    stop: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Lifecycle {
    /// Starts the thread and claims Papo's name, handing this launch to the
    /// running instance when there is one. Blocks until that is decided: at
    /// most the handoff's deadlines.
    ///
    /// Without a session bus, or when the thread cannot start, Papo runs
    /// without a single instance and the SDK's profile lease decides.
    pub(crate) fn start() -> Start {
        let launch = Launch::from_env();
        let quitting = Arc::new(AtomicBool::new(false));
        let (claimed_tx, claimed) = std_mpsc::channel();
        let (attach, attached) = oneshot::channel();
        let (stop, stopped) = oneshot::channel();
        let thread_quitting = Arc::clone(&quitting);
        let spawned = std::thread::Builder::new()
            .name("papo-lifecycle".to_owned())
            .spawn(move || serve(&launch, thread_quitting, &claimed_tx, attached, stopped));
        let thread = match spawned {
            Ok(thread) => thread,
            Err(error) => {
                log::error!("the lifecycle thread did not start: {error}");
                return Start::Running(Lifecycle {
                    attach: None,
                    quitting,
                    stop: None,
                    thread: None,
                });
            }
        };
        // The thread reports once; it ends without a report only by panicking.
        if let Ok(Claimed::HandedOff) = claimed.recv() {
            let _ = thread.join();
            return Start::HandedOff;
        }
        Start::Running(Lifecycle {
            attach: Some(attach),
            quitting,
            stop: Some(stop),
            thread: Some(thread),
        })
    }

    /// Connects later launches and the exit signals to `app`. Until then a
    /// later launch's activation waits, and a signal ends the process as it
    /// would without Papo.
    pub(crate) fn attach(&mut self, app: Arc<dyn AppControl>) {
        if let Some(attach) = self.attach.take() {
            let _ = attach.send(app);
        }
    }

    /// The app has quit: later launches are refused from now on, so they wait
    /// for the name instead of handing off to a window that is gone. Rune's
    /// own `quitting` covers the time between the quit starting and the loop
    /// returning.
    pub(crate) fn quitting(&self) {
        self.quitting.store(true, Ordering::Release);
    }

    /// The end of the exit fence: lets go of Papo's name, so a launch waiting
    /// for it opens, and joins the thread. Call it after the SDK has released
    /// the profile.
    pub(crate) fn stop(mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            log::error!("the lifecycle thread panicked");
        }
    }
}

/// What the claim decided, as the thread reports it.
enum Claimed {
    Running,
    HandedOff,
}

/// The lifecycle thread: claim, report, then serve until stopped.
fn serve(
    launch: &Launch,
    quitting: Arc<AtomicBool>,
    claimed: &std_mpsc::Sender<Claimed>,
    attached: oneshot::Receiver<Arc<dyn AppControl>>,
    stopped: oneshot::Receiver<()>,
) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build();
    let runtime = match runtime {
        Ok(runtime) => runtime,
        Err(error) => {
            log::error!("the lifecycle thread has no runtime: {error}");
            let _ = claimed.send(Claimed::Running);
            return;
        }
    };
    let quitting_flag = Arc::clone(&quitting);
    runtime.block_on(async move {
        let instance = match claim(launch, quitting).await {
            Some(Claim::HandedOff) => {
                log::info!("Papo is already running; it was asked to show its window");
                let _ = claimed.send(Claimed::HandedOff);
                return;
            }
            Some(Claim::First(instance)) => Some(instance),
            None => None,
        };
        let _ = claimed.send(Claimed::Running);
        run(instance, quitting_flag, attached, stopped).await;
    });
}

/// Claims the name on the session bus; `None` when there is no bus.
async fn claim(launch: &Launch, quitting: Arc<AtomicBool>) -> Option<Claim> {
    single_instance::claim(
        zbus::connection::Builder::session(),
        launch,
        DEADLINES,
        quitting,
    )
    .await
    .inspect_err(|error| log::warn!("no single instance: the session bus failed: {error}"))
    .ok()
}

/// Waits for the app, then serves it until stopped. The exit signals are
/// installed only once there is an app to quit.
async fn run(
    instance: Option<Instance>,
    quitting: Arc<AtomicBool>,
    attached: oneshot::Receiver<Arc<dyn AppControl>>,
    mut stopped: oneshot::Receiver<()>,
) {
    let app = tokio::select! {
        app = attached => app.ok(),
        _ = &mut stopped => None,
    };
    let Some(app) = app else {
        return;
    };
    let signals = ExitSignals::install()
        .inspect_err(|error| log::warn!("SIGTERM and SIGINT keep their default action: {error}"))
        .ok();
    serve_app(app.as_ref(), instance, signals, &quitting, stopped).await;
}

/// Serves later launches and the exit signals until stopped. Holding
/// `instance` holds the name; it is dropped, letting go, when this returns.
async fn serve_app(
    app: &dyn AppControl,
    mut instance: Option<Instance>,
    mut signals: Option<ExitSignals>,
    quitting: &AtomicBool,
    mut stopped: oneshot::Receiver<()>,
) {
    let mut signalled = false;
    loop {
        tokio::select! {
            _ = &mut stopped => break,
            Some(activation) = next_activation(instance.as_mut()) => {
                log::info!("a later launch asked for the window");
                app.activate(activation.token);
            }
            name = next_signal(signals.as_mut()) => {
                if signalled {
                    // Asked twice: the person does not want to wait for the
                    // exit fence. SQLite recovers its WAL on the next open.
                    log::warn!("{name} again: ending without waiting for the SDK");
                    std::process::exit(EXIT_SIGNALLED_TWICE);
                }
                signalled = true;
                log::info!("{name}: quitting");
                // Refused from now, not from when the UI thread starts the
                // quit, so no launch hands off to a window about to go.
                quitting.store(true, Ordering::Release);
                app.quit();
            }
        }
    }
}

/// The exit code when a second signal ends the process before the fence.
const EXIT_SIGNALLED_TWICE: i32 = 1;

/// The next later launch's activation; never, without an instance.
async fn next_activation(instance: Option<&mut Instance>) -> Option<single_instance::Activation> {
    match instance {
        Some(instance) => instance.next_activation().await,
        None => std::future::pending().await,
    }
}

/// The next exit signal; never, when the handlers could not be installed.
async fn next_signal(signals: Option<&mut ExitSignals>) -> &'static str {
    match signals {
        Some(signals) => signals.next().await,
        None => std::future::pending().await,
    }
}
