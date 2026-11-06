//! Process-owned sessions and the GPUI foreground end of the wake bridge.
//!
//! Each server has one session thread. Only immutable projection deltas reach
//! GPUI; every delta must still match its server, account and incarnation.
//! Replacing a session invalidates its outstanding work immediately. Windows
//! never own runtimes. The application stops all threads before quitting.
//! Credentials and data-folder selection are deliberately left to later slices.

mod actor;
mod hydration;
mod shutdown;

#[cfg(test)]
#[path = "../../tests/session/mod.rs"]
mod tests;

use std::{collections::HashMap, sync::Arc};

use gpui::{App, AppContext as _, Context, Entity, Global, Task};
use papo_core::{
    api::net::Wake, cache::ClientDb, notification::NotificationCoordinator, runtime::ServerRuntime,
    storage::MemorySecretStore,
};

use crate::projection::Projection;
use actor::{Publication, SessionThread};

/// A failure at the desktop session boundary. No input or secret is formatted.
#[derive(Debug, thiserror::Error)]
pub(crate) enum SessionError {
    /// The OS refused to start the session owner.
    #[error("could not start the session thread")]
    Spawn(#[source] std::io::Error),
    /// The session owner unwound. Its runtime guard still joins the transport.
    #[error("the session thread panicked")]
    Panicked,
    /// Shutdown has begun; no new session may outlive the last window.
    #[error("the application is stopping")]
    Stopping,
}

/// The identity captured when asynchronous work begins, including replacement.
#[derive(Clone)]
pub(crate) struct SessionIdentity {
    server: Arc<str>,
    account: Option<Arc<str>>,
    incarnation: Arc<()>,
}

impl SessionIdentity {
    /// A new incarnation never matches work from an earlier login, even of the same account.
    pub(crate) fn new(server: Arc<str>, account: Option<Arc<str>>) -> Self {
        Self {
            server,
            account,
            incarnation: Arc::new(()),
        }
    }

    fn matches(&self, other: &Self) -> bool {
        self.server == other.server
            && self.account == other.account
            && Arc::ptr_eq(&self.incarnation, &other.incarnation)
    }
}

struct Session {
    identity: SessionIdentity,
    projection: Projection,
    owner: SessionThread,
    _foreground: Task<()>,
}

/// Foreground projection owners, independent of any window or render callback.
#[derive(Default)]
pub(crate) struct Sessions {
    servers: HashMap<Arc<str>, Session>,
    stopping: bool,
    shutdown: shutdown::Shutdown,
}

/// One retained entity keeps sessions alive across window changes.
struct ApplicationSessions(Entity<Sessions>);
impl Global for ApplicationSessions {}

impl Sessions {
    /// Replaces a server/account incarnation before any new result can be accepted.
    /// Opening and all cache work are performed by the new owner thread.
    ///
    /// # Errors
    /// Returns `SessionError::Spawn` if the OS refuses to create its thread,
    /// or `SessionError::Stopping` after application shutdown has begun.
    pub(crate) fn replace(
        &mut self,
        identity: SessionIdentity,
        open: impl FnOnce(Wake) -> ServerRuntime + Send + 'static,
        cx: &mut Context<Self>,
    ) -> Result<(), SessionError> {
        if self.stopping {
            return Err(SessionError::Stopping);
        }
        let previous = self.servers.remove(&identity.server).map(|session| {
            session.owner.stop();
            session.owner
        });
        let owner = SessionThread::start(identity.clone(), move |wake| {
            // The replacement cannot open a second runtime for this server
            // until its predecessor has joined and flushed.
            if let Some(previous) = previous {
                join(previous);
            }
            open(wake)
        })?;
        let publications = owner.publications();
        let foreground = cx.spawn(async move |this, cx| {
            while let Ok(publication) = publications.recv().await {
                if this
                    .update(cx, |sessions, cx| {
                        if sessions.apply(publication) {
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        self.servers.insert(
            Arc::clone(&identity.server),
            Session {
                identity,
                projection: Projection::default(),
                owner,
                _foreground: foreground,
            },
        );
        Ok(())
    }

    fn apply(&mut self, publication: Publication) -> bool {
        let Some(session) = self.servers.get_mut(&publication.identity.server) else {
            return false;
        };
        if self.stopping || !session.identity.matches(&publication.identity) {
            return false;
        }
        let start = std::time::Instant::now();
        let changed = session.projection.apply(publication.update);
        if changed {
            log::debug!(
                "session: foreground projection changed, processed={}, rows={}, apply_us={}",
                publication.processed,
                session.projection.order().len(),
                start.elapsed().as_micros()
            );
        }
        changed
    }

    /// Cancels all owners first; the returned work joins and flushes off the main thread.
    fn stop(&mut self) -> Vec<SessionThread> {
        self.stopping = true;
        for session in self.servers.values() {
            session.owner.stop();
        }
        self.servers
            .drain()
            .map(|(_, session)| session.owner)
            .collect()
    }
}

fn join(owner: SessionThread) {
    if let Err(error) = owner.join() {
        log::error!("session: {error}");
    }
}

/// Installs process ownership. The opt-in probe uses no files and no live server.
pub(crate) fn install(cx: &mut App) {
    let sessions = cx.new(|_| Sessions::default());
    cx.set_global(ApplicationSessions(sessions.clone()));
    if std::env::var_os("PAPO_DESKTOP_SESSION_PROBE").is_some() {
        sessions.update(cx, |sessions, cx| {
            let url = "http://127.0.0.1:9".to_owned();
            let identity = SessionIdentity::new(papo_core::server_key(&url).into(), None);
            if let Err(error) = sessions.replace(
                identity,
                move |wake| {
                    let cache = Arc::new(ClientDb::open(None));
                    let notification =
                        Arc::new(NotificationCoordinator::new(Arc::clone(&cache), None));
                    ServerRuntime::open(
                        url,
                        wake,
                        Arc::new(MemorySecretStore::default()),
                        cache,
                        notification,
                    )
                },
                cx,
            ) {
                log::error!("session: {error}");
            }
        });
    }
    cx.on_app_quit(|cx| {
        begin_shutdown(cx);
        let sessions = cx.global::<ApplicationSessions>().0.clone();
        // GPUI invokes this body before its 200 ms quit grace starts
        // (gpui-fast 0.1.13, app.rs, App::shutdown). AppKit's terminate:
        // need not return through Application::run, so the native boundary
        // must wait here. Runtime/Net/cache work stays on the reaper.
        sessions.update(cx, |sessions, _| sessions.shutdown.wait());
        async {}
    })
    .detach();
}

/// Stops owners once and returns the same completion for native and explicit quit.
fn begin_shutdown(cx: &mut App) -> async_channel::Receiver<()> {
    let sessions = cx.global::<ApplicationSessions>().0.clone();
    sessions.update(cx, |sessions, _| {
        let owners = sessions.stop();
        sessions.shutdown.begin(move || {
            for owner in owners {
                join(owner);
            }
        })
    })
}

/// Keeps GPUI alive until every runtime has stopped and every transport has joined.
pub(crate) fn quit_after_shutdown(cx: &mut App) {
    if cx.global::<ApplicationSessions>().0.read(cx).stopping {
        return;
    }
    let completion = begin_shutdown(cx);
    cx.spawn(async move |cx| {
        // Closure, not a consumed message, releases every quit waiter. Dropping
        // this foreground task cannot cancel the independent reaper.
        let _ = completion.recv().await;
        cx.update(|cx| cx.quit());
    })
    .detach();
}
