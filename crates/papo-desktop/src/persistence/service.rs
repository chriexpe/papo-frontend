//! Off-main profile startup, serialized list operations and process ownership.
//!
//! One worker holds the profile lock. Accepted requests are completed even when
//! their foreground waiter disappears. Closing the queue stops admission but
//! drains accepted writes; the session shutdown reaper joins this worker before
//! exit. The empty shell gains no auth, server-list UI or failure presentation.

use std::{path::PathBuf, thread::JoinHandle};

use gpui::{App, AppContext as _, Entity, Global, Task};

use super::{Change, PersistenceError, Profile, ServerList};

struct Request {
    change: Option<Change>,
    reply: async_channel::Sender<Result<ServerList, PersistenceError>>,
}

/// Process-owned registry state. A failure is distinct from an empty list and
/// never authorizes an empty replacement. The first screens will present it.
struct ProfileState {
    snapshot: Option<ServerList>,
    failure: Option<PersistenceError>,
    worker: Option<ProfileWorker>,
    _foreground: Task<()>,
}

struct ApplicationProfile(Entity<ProfileState>);
impl Global for ApplicationProfile {}

/// The worker can be transferred to the off-main shutdown reaper for joining.
pub(crate) struct ProfileWorker {
    requests: async_channel::Sender<Request>,
    thread: Option<JoinHandle<()>>,
}

impl ProfileWorker {
    fn start(
        resolve: impl FnOnce() -> Result<PathBuf, PersistenceError> + Send + 'static,
    ) -> Result<Self, PersistenceError> {
        let (requests, incoming) = async_channel::bounded::<Request>(1);
        let thread = std::thread::Builder::new()
            .name("papo-profile".into())
            .spawn(move || {
                let mut profile = resolve().and_then(Profile::open);
                while let Ok(request) = incoming.recv_blocking() {
                    let result = match &mut profile {
                        Ok(profile) => match request.change {
                            Some(change) => profile.apply(change),
                            None => Ok(profile.snapshot().clone()),
                        },
                        Err(error) => Err(*error),
                    };
                    // A canceled waiter must not cancel an accepted disk write.
                    let _ = request.reply.try_send(result);
                }
            })
            .map_err(|_| PersistenceError::Worker)?;
        Ok(Self {
            requests,
            thread: Some(thread),
        })
    }

    fn request(
        &self,
        change: Option<Change>,
    ) -> Result<async_channel::Receiver<Result<ServerList, PersistenceError>>, PersistenceError>
    {
        let (reply, result) = async_channel::bounded(1);
        self.requests
            .try_send(Request { change, reply })
            .map_err(|error| match error {
                async_channel::TrySendError::Full(_) => PersistenceError::Busy,
                async_channel::TrySendError::Closed(_) => PersistenceError::Stopped,
            })?;
        Ok(result)
    }

    /// Stops admission; the queue still delivers all already-accepted work.
    fn stop(&self) {
        self.requests.close();
    }

    /// Joins off-main after the queue was closed. No foreground progress is needed.
    pub(crate) fn join(mut self) {
        self.stop();
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            log::error!("persistence: profile worker panicked");
        }
    }
}

impl Drop for ProfileWorker {
    fn drop(&mut self) {
        self.stop();
        // Normal shutdown transfers this object to the reaper first. A failed
        // reaper spawn must still not detach an in-flight registry replacement.
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            log::error!("persistence: profile worker panicked during cleanup");
        }
    }
}

/// Starts disk access on one background thread and retains the loaded state
/// independently of windows. Errors remain typed, not a fake empty profile.
pub(crate) fn install(cx: &mut App) {
    install_worker(ProfileWorker::start(Profile::data_directory), cx);
}

fn install_worker(worker: Result<ProfileWorker, PersistenceError>, cx: &mut App) {
    let initial = worker
        .as_ref()
        .map_err(|error| *error)
        .and_then(|worker| worker.request(None));
    let failure = initial.as_ref().err().copied();
    if let Some(error) = failure {
        log::error!("persistence: {error}");
    }
    let state = cx.new(|cx| {
        let foreground = cx.spawn(async move |this, cx| {
            let Ok(initial) = initial else { return };
            let result = initial
                .recv()
                .await
                .unwrap_or(Err(PersistenceError::Stopped));
            let _ = this.update(cx, |state: &mut ProfileState, cx| {
                // Shutdown owns the worker now; a late load must not reactivate it.
                if state.worker.is_none() {
                    return;
                }
                match result {
                    Ok(snapshot) => state.snapshot = Some(snapshot),
                    Err(error) => {
                        log::error!("persistence: {error}");
                        state.failure = Some(error);
                    }
                }
                cx.notify();
            });
        });
        ProfileState {
            snapshot: None,
            failure,
            worker: worker.ok(),
            _foreground: foreground,
        }
    });
    cx.set_global(ApplicationProfile(state));
}

/// Closes admission before transferring disk work to the shared quit reaper.
/// Does nothing in tests/probes which did not install a disk profile.
pub(crate) fn stop_before_session_join(cx: &mut App) -> Option<ProfileWorker> {
    let entity = cx.try_global::<ApplicationProfile>()?.0.clone();
    entity.update(cx, |state, _| {
        let worker = state.worker.take()?;
        worker.stop();
        Some(worker)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui_kit::test]
    async fn startup_loads_committed_state_off_main_and_native_quit_releases_lock(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.executor().allow_parking();
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("papo-desktop");
        let mut profile = Profile::open(root.clone()).unwrap();
        profile
            .apply(Change::Add {
                url: "https://saved.example.invalid".into(),
            })
            .unwrap();
        drop(profile);
        let foreground = std::thread::current().id();
        let (resolved, resolution) = async_channel::bounded(1);
        let worker = ProfileWorker::start({
            let root = root.clone();
            move || {
                resolved.try_send(std::thread::current().id()).unwrap();
                Ok(root)
            }
        });
        let state = cx.update(|cx| {
            crate::session::install(cx);
            install_worker(worker, cx);
            cx.global::<ApplicationProfile>().0.clone()
        });
        assert_ne!(resolution.recv().await.unwrap(), foreground);
        cx.condition(&state, |state, _| state.snapshot.is_some())
            .await;
        cx.update(|cx| {
            assert_eq!(state.read(cx).snapshot.as_ref().unwrap().servers().len(), 1);
            assert_eq!(state.read(cx).failure, None);
            assert_eq!(
                Profile::open(root.clone()).err(),
                Some(PersistenceError::Locked)
            );
            // The headless platform does not drive a native event loop from
            // App::quit. Invoke the same termination-observer boundary directly.
            cx.shutdown();
        });
        assert_eq!(Profile::open(root).unwrap().snapshot().servers().len(), 1);
    }

    #[gpui_kit::test]
    async fn late_startup_completion_does_not_reactivate_a_stopped_profile(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.executor().allow_parking();
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("papo-desktop");
        let (entered, started) = async_channel::bounded(1);
        let (release, released) = std::sync::mpsc::channel();
        let worker = ProfileWorker::start({
            let root = root.clone();
            move || {
                entered.try_send(()).unwrap();
                released.recv().unwrap();
                Ok(root)
            }
        });
        let state = cx.update(|cx| {
            install_worker(worker, cx);
            cx.global::<ApplicationProfile>().0.clone()
        });
        started.recv().await.unwrap();
        let worker = cx.update(stop_before_session_join).unwrap();
        release.send(()).unwrap();
        worker.join();
        cx.run_until_parked();
        cx.update(|cx| {
            assert!(state.read(cx).worker.is_none());
            assert!(state.read(cx).snapshot.is_none());
        });
        assert!(Profile::open(root).is_ok());
    }

    #[test]
    fn bounded_queue_reports_busy_without_accepting_a_second_pending_request() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("papo-desktop");
        let (release, released) = std::sync::mpsc::channel();
        let worker = ProfileWorker::start(move || {
            released.recv().unwrap();
            Ok(root)
        })
        .unwrap();
        let first = worker.request(None).unwrap();
        assert_eq!(worker.request(None).err(), Some(PersistenceError::Busy));
        release.send(()).unwrap();
        assert!(first.recv_blocking().unwrap().is_ok());
        worker.join();
    }

    #[test]
    fn shutdown_drains_accepted_write_and_releases_profile_lock() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("papo-desktop");
        let worker = ProfileWorker::start({
            let root = root.clone();
            move || Ok(root)
        })
        .unwrap();
        worker
            .request(None)
            .unwrap()
            .recv_blocking()
            .unwrap()
            .unwrap();
        let reply = worker
            .request(Some(Change::Add {
                url: "https://saved.example.invalid".into(),
            }))
            .unwrap();
        worker.stop();
        assert_eq!(worker.request(None).err(), Some(PersistenceError::Stopped));
        worker.join();
        assert_eq!(reply.recv_blocking().unwrap().unwrap().servers().len(), 1);
        let profile = Profile::open(root).unwrap();
        assert_eq!(profile.snapshot().servers().len(), 1);
    }

    #[test]
    fn canceled_reply_does_not_cancel_accepted_disk_change() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("papo-desktop");
        let worker = ProfileWorker::start({
            let root = root.clone();
            move || Ok(root)
        })
        .unwrap();
        worker
            .request(None)
            .unwrap()
            .recv_blocking()
            .unwrap()
            .unwrap();
        drop(
            worker
                .request(Some(Change::Add {
                    url: "https://saved.example.invalid".into(),
                }))
                .unwrap(),
        );
        worker.join();
        assert_eq!(Profile::open(root).unwrap().snapshot().servers().len(), 1);
    }

    #[test]
    fn failed_open_never_authorizes_a_write_or_overwrites_existing_document() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().to_path_buf();
        std::fs::write(root.join("servers.json"), b"not a registry").unwrap();
        let worker = ProfileWorker::start({
            let root = root.clone();
            move || Ok(root)
        })
        .unwrap();
        assert_eq!(
            worker.request(None).unwrap().recv_blocking().unwrap().err(),
            Some(PersistenceError::InvalidDocument)
        );
        assert_eq!(
            worker
                .request(Some(Change::Add {
                    url: "https://saved.example.invalid".into()
                }))
                .unwrap()
                .recv_blocking()
                .unwrap()
                .err(),
            Some(PersistenceError::InvalidDocument)
        );
        worker.join();
        assert_eq!(
            std::fs::read(root.join("servers.json")).unwrap(),
            b"not a registry"
        );
    }
}
