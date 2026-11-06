//! A blocked factory cannot accumulate successor threads or obsolete openings.
//!
//! Channels hold and release the first opening without sleeps or a real clock.

use super::{SessionThread, identity, open};
use std::sync::mpsc;

#[test]
fn only_the_latest_of_50_replacements_opens_on_the_original_server_thread() {
    let (entered, opened) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let (finished, _) = mpsc::channel();
    let first_entered = entered.clone();
    let first_finished = finished.clone();
    let owner = SessionThread::start(identity("server-1", "account-0"), move |wake| {
        first_entered
            .send((0, std::thread::current().id()))
            .unwrap();
        released.recv().unwrap();
        open(wake, first_finished)
    })
    .unwrap();
    let (_, original_thread) = opened.recv().unwrap();
    for index in 1..=50 {
        let entered = entered.clone();
        let finished = finished.clone();
        owner
            .replace(
                identity("server-1", &format!("account-{index}")),
                move |wake| {
                    entered.send((index, std::thread::current().id())).unwrap();
                    open(wake, finished)
                },
            )
            .unwrap();
    }
    assert!(opened.try_recv().is_err());
    release.send(()).unwrap();
    let (latest, thread) = opened.recv().unwrap();
    assert_eq!(latest, 50);
    assert_eq!(thread, original_thread);
    assert!(opened.try_recv().is_err());
    owner.join().unwrap();
}

#[test]
fn stopped_owner_rejects_replacement_without_invoking_the_factory() {
    let (finished, _) = mpsc::channel();
    let owner = SessionThread::start(identity("server-1", "account-0"), move |wake| {
        open(wake, finished)
    })
    .unwrap();
    owner.stop();
    let result = owner.replace(identity("server-1", "account-1"), |_| {
        panic!("a stopped owner must not open replacements")
    });
    assert!(matches!(
        result,
        Err(super::super::SessionError::OwnerStopped)
    ));
    owner.join().unwrap();
}
