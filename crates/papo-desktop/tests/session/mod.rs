//! Session behavior across the actor, projection and application lifetime.
//!
//! Unit fixtures have no saved session, so Net publishes Session(None) without
//! making a request. No network or user data is involved.
//! Channels establish ordering; assertions do not depend on sleeps or a clock.

mod burst;
mod replacements;
mod windows;

use std::sync::{Arc, mpsc};

use gpui::{AppContext as _, TestAppContext};
use papo_core::{
    api::net::Wake,
    cache::ClientDb,
    notification::NotificationCoordinator,
    runtime::ServerRuntime,
    state::Store,
    storage::{Secret, SecretStore, StorageResult},
};

use super::{
    SessionIdentity, Sessions,
    actor::{Publication, SessionThread},
};
use crate::projection::ProjectionBuilder;

struct TrackedSecrets(mpsc::Sender<()>);
impl Drop for TrackedSecrets {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}
impl SecretStore for TrackedSecrets {
    fn load(&self, _: &str, _: Secret) -> StorageResult<Option<String>> {
        Ok(None)
    }
    fn store(&self, _: &str, _: Secret, _: &str) -> StorageResult<()> {
        Ok(())
    }
    fn remove(&self, _: &str, _: Secret) -> StorageResult<()> {
        Ok(())
    }
}

fn open(wake: Wake, finished: mpsc::Sender<()>) -> ServerRuntime {
    let cache = Arc::new(ClientDb::open(None));
    let notification = Arc::new(NotificationCoordinator::new(Arc::clone(&cache), None));
    ServerRuntime::open(
        "http://127.0.0.1:9".into(),
        wake,
        Arc::new(TrackedSecrets(finished)),
        cache,
        notification,
    )
}

fn identity(server: &str, account: &str) -> SessionIdentity {
    SessionIdentity::new(server.into(), Some(account.into()))
}

fn result(identity: SessionIdentity) -> Publication {
    let mut store = Store::default();
    store.screen = papo_core::state::Screen::Chat;
    Publication {
        identity,
        processed: 0,
        update: ProjectionBuilder::default().build(&store).unwrap(),
    }
}

#[gpui_kit::test]
fn replaced_server_drops_its_async_result(cx: &mut TestAppContext) {
    let sessions = cx.new(|_| Sessions::default());
    let old = identity("server-1", "account-1");
    let (finished, _) = mpsc::channel();
    sessions.update(cx, |sessions, cx| {
        sessions
            .replace(
                old.clone(),
                {
                    let finished = finished.clone();
                    move |wake| open(wake, finished)
                },
                cx,
            )
            .unwrap();
        sessions
            .replace(
                identity("server-1", "account-1"),
                move |wake| open(wake, finished),
                cx,
            )
            .unwrap();
        assert!(!sessions.apply(result(old)));
    });
    let owners = sessions.update(cx, |sessions, _| sessions.stop());
    for owner in owners {
        owner.join().unwrap();
    }
}

#[gpui_kit::test]
fn replaced_account_drops_its_async_result_from_ownership_handoff_section_3(
    cx: &mut TestAppContext,
) {
    let sessions = cx.new(|_| Sessions::default());
    let old = identity("server-1", "account-1");
    let (finished, _) = mpsc::channel();
    sessions.update(cx, |sessions, cx| {
        sessions
            .replace(
                old.clone(),
                {
                    let finished = finished.clone();
                    move |wake| open(wake, finished)
                },
                cx,
            )
            .unwrap();
        sessions
            .replace(
                identity("server-1", "account-2"),
                move |wake| open(wake, finished),
                cx,
            )
            .unwrap();
        assert!(!sessions.apply(result(old)));
    });
    let owners = sessions.update(cx, |sessions, _| sessions.stop());
    for owner in owners {
        owner.join().unwrap();
    }
}

#[test]
fn stopping_without_consuming_projection_joins_runtime_and_transport() {
    let (finished, received) = mpsc::channel();
    let (entered, opened) = mpsc::channel();
    let owner = SessionThread::start(identity("server-1", "account-1"), move |wake| {
        let runtime = open(wake, finished);
        entered.send(()).unwrap();
        runtime
    })
    .unwrap();
    opened.recv().unwrap();
    owner.stop();
    owner.join().unwrap();
    // Net keeps its own Arc to the storage until its papo-net thread exits.
    assert!(received.try_recv().is_ok());
}

#[gpui_kit::test]
async fn core_wake_reaches_the_gpui_foreground_without_a_frame(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let sessions = cx.new(|_| Sessions::default());
    let (finished, _) = mpsc::channel();
    let (applied, received) = async_channel::bounded(1);
    let _subscription = cx.update(|cx| {
        cx.observe(&sessions, move |_, _| {
            let _ = applied.try_send(());
        })
    });
    sessions.update(cx, |sessions, cx| {
        sessions
            .replace(
                identity("server-1", "account-1"),
                move |wake| open(wake, finished),
                cx,
            )
            .unwrap();
    });
    received.recv().await.unwrap();
    let owners = sessions.update(cx, |sessions, _| sessions.stop());
    cx.background_spawn(async move {
        for owner in owners {
            owner.join().unwrap();
        }
    })
    .await;
}

#[gpui_kit::test]
fn shutdown_rejects_new_sessions_before_invoking_their_factory(cx: &mut TestAppContext) {
    let sessions = cx.new(|_| Sessions::default());
    sessions.update(cx, |sessions, cx| {
        assert!(sessions.stop().is_empty());
        let result = sessions.replace(
            identity("server-1", "account-1"),
            |_| panic!("stopped sessions must not open"),
            cx,
        );
        assert!(matches!(result, Err(super::SessionError::Stopping)));
    });
}

#[test]
fn an_owner_panic_is_reported_at_join_instead_of_unwinding_the_foreground() {
    let (entered, opened) = mpsc::channel();
    let owner = SessionThread::start(identity("server-1", "account-1"), move |_| {
        entered.send(()).unwrap();
        panic!("synthetic owner failure")
    })
    .unwrap();
    opened.recv().unwrap();
    assert!(matches!(owner.join(), Err(super::SessionError::Panicked)));
}
