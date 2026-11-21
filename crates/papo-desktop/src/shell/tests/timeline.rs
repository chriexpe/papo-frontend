//! The shown timeline in the shell: moving the reader arms the save, which
//! fires once the place has rested, and then the window asks for nothing
//! but the next local midnight.

use std::sync::Arc;

use imbl::Vector;
use papo_sdk::{AccountId, Message, Uuid};
use papo_sdk_ui::message_content::MessageContent;
use papo_sdk_ui::{TimelineItem, UtcOffset};
use rune::Runtime;

use super::{FRAME_S, Shell, app, server_id};
use crate::timeline::local_time::LocalTime;
use crate::timeline::{OpenTimeline, TimelineKey};

/// 2026-11-20 12:00 UTC.
fn noon_us() -> i64 {
    1_795_176_000_000_000
}

fn messages(count: u128) -> Vector<Arc<TimelineItem>> {
    (0..count)
        .map(|n| {
            let id = Uuid::from_u128(n + 1);
            let seconds = u32::try_from(n).unwrap() * 60;
            let at = format!(
                "2026-11-20T{:02}:{:02}:00Z",
                seconds / 3600 % 24,
                seconds / 60 % 60
            );
            Arc::new(TimelineItem::Message {
                id,
                message: Arc::new(Message {
                    id: Some(id.to_string()),
                    author_id: Some("ana".to_owned()),
                    content: Some("a line of words".to_owned()),
                    created_at: Some(at),
                    ..Message::default()
                }),
                content: Arc::new(MessageContent::parse("a line of words")),
            })
        })
        .collect()
}

/// The app showing a timeline of 200 messages, polled as the window is.
fn showing_a_timeline() -> Runtime<Shell> {
    let mut rt = app();
    rt.poll = Some(Shell::poll);
    rt.update(|shell, _tx| {
        shell.local_time = LocalTime::fixed(UtcOffset::from_seconds_east(0).unwrap(), noon_us);
        let key = TimelineKey {
            account: AccountId::new(server_id(), Uuid::nil()),
            channel: Uuid::from_u128(7),
        };
        shell.tabs.show(Some(key));
        *shell.tabs.shown_mut().unwrap() = OpenTimeline::loaded(key, messages(200));
    });
    for _ in 0..30 {
        rt.frame(FRAME_S);
    }
    rt
}

#[test]
fn a_moved_place_is_saved_once_it_rests_and_then_the_window_rests() {
    let mut rt = showing_a_timeline();
    for _ in 0..6 {
        rt.wheel(700.0, 300.0, 300.0);
        rt.frame(FRAME_S);
    }
    assert!(rt.state.save_due_s.is_some(), "the move arms the save");
    // Two seconds after the last move, and a little more.
    for _ in 0..150 {
        rt.frame(FRAME_S);
    }
    assert!(rt.state.save_due_s.is_none(), "the save fired");
    assert!(!rt.frame(FRAME_S), "no frame asked for");
    let clock_s = rt.state.day_ends_s.expect("the day's end is known");
    assert!(
        rt.wake_deadline()
            .is_none_or(|due_s| due_s >= clock_s - 1.0),
        "only midnight is awaited: {:?}",
        rt.wake_deadline()
    );
}
