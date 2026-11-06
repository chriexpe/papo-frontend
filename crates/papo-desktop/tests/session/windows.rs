//! Application lifetime integration without a real display or GPU.
//!
//! These tests exercise production window-close and native-quit callbacks.
//! Worker storage lifetimes prove both runtime and Net have released ownership.

use super::super::{ApplicationSessions, begin_shutdown, quit_after_shutdown};
use super::{identity, open};
use gpui::{AppContext as _, TestAppContext, WindowOptions};
use std::sync::mpsc;

#[gpui_kit::test]
async fn closing_the_last_window_stops_every_runtime_and_joins_its_net_thread(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    cx.update(gpui_kit::init);
    cx.update(crate::app::install);
    let first = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
                cx.new(|_| crate::shell::Shell)
            })
        })
        .unwrap()
        .0;
    let last = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
                cx.new(|_| crate::shell::Shell)
            })
        })
        .unwrap()
        .0;
    let sessions = cx.update(|cx| cx.global::<ApplicationSessions>().0.clone());
    let (finished, received) = mpsc::channel();
    sessions.update(cx, |sessions, cx| {
        for server in ["server-1", "server-2"] {
            let finished = finished.clone();
            sessions
                .replace(
                    identity(server, "account-1"),
                    move |wake| open(wake, finished),
                    cx,
                )
                .unwrap();
        }
    });
    cx.update_window(first, |_, window, _| window.remove_window())
        .unwrap();
    sessions.read_with(cx, |sessions, _| assert!(!sessions.stopping));
    assert!(received.try_recv().is_err());
    cx.update_window(last, |_, window, _| window.remove_window())
        .unwrap();
    sessions.read_with(cx, |sessions, _| {
        assert!(sessions.stopping);
        assert!(sessions.servers.is_empty());
    });
    let completion = cx.update(begin_shutdown);
    assert!(completion.recv().await.is_err());
    assert_eq!(received.try_iter().count(), 2);
}

#[gpui_kit::test]
fn native_quit_reuses_an_in_progress_explicit_shutdown_and_waits_without_gpui_progress(
    cx: &mut TestAppContext,
) {
    cx.update(crate::app::install);
    let sessions = cx.update(|cx| cx.global::<ApplicationSessions>().0.clone());
    let (finished, received) = mpsc::channel();
    let (entered, opened) = mpsc::channel();
    let (release, released) = mpsc::channel();
    sessions.update(cx, |sessions, cx| {
        sessions
            .replace(
                identity("server-1", "account-1"),
                move |wake| {
                    entered.send(()).unwrap();
                    released.recv().unwrap();
                    open(wake, finished)
                },
                cx,
            )
            .unwrap();
    });
    opened.recv().unwrap();
    cx.update(quit_after_shutdown);
    // A repeated Quit must not lose the already-running shutdown completion.
    cx.update(quit_after_shutdown);
    let completion = cx.update(begin_shutdown);
    assert!(!completion.is_closed());
    let release = std::thread::spawn(move || release.send(()).unwrap());
    // App::shutdown invokes the actual installed native callback synchronously.
    // No foreground executor is pumped while it waits for the reaper.
    cx.update(gpui::App::shutdown);
    release.join().unwrap();
    assert!(completion.is_closed());
    assert!(received.try_recv().is_ok());
}
