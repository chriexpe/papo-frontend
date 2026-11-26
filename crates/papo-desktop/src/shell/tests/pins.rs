//! The pins panel and the Escape order (`docs/design.md` §10 and §12),
//! headless: Ctrl+P opens and closes it, it waits while the pins are not
//! known, says when there are none, opens a pin with `highlight.selected`
//! and marks it, shrinks to that pin on a press outside and stretches back
//! on Ctrl+P, and Escape closes what is on top first.

use imbl::Vector;
use papo_sdk_ui::TimelineItem;
use rune::{Key, KeyEvent, Mods, Runtime};

use super::replying::{conversation, message_id};
use super::{FRAME_S, Shell, press};
use crate::timeline::rows::message_key;

fn ctrl_p(rt: &mut Runtime<Shell>) {
    rt.key(KeyEvent {
        key: Key::Command('p'),
        mods: Mods {
            cmd: true,
            ctrl: true,
            ..Mods::default()
        },
    });
    rt.frame(FRAME_S);
}

/// The shown timeline's pins: these of its messages, newest pin first.
fn pin(rt: &mut Runtime<Shell>, pinned: &[u128]) {
    let pinned = pinned.to_vec();
    rt.update(move |shell, _tx| {
        let open = shell.tabs.shown_mut().unwrap();
        let pins: Vector<_> = pinned
            .iter()
            .filter_map(|n| {
                open.items.iter().find_map(|item| match item.as_ref() {
                    TimelineItem::Message { id, message, .. } if *id == message_id(*n) => {
                        Some(std::sync::Arc::clone(message))
                    }
                    _ => None,
                })
            })
            .collect();
        let items = open.items.clone();
        open.pins.set(
            Some(pins),
            &items,
            &papo_sdk_ui::message_content::CustomEmojis::default(),
        );
    });
    rt.frame(FRAME_S);
}

fn shows(rt: &Runtime<Shell>, words: &str) -> bool {
    rt.elements().iter().any(|element| {
        element.text.as_deref() == Some(words) || element.label.as_deref() == Some(words)
    })
}

#[test]
fn ctrl_p_opens_the_panel_which_waits_then_says_there_are_none() {
    let mut rt = conversation("");
    ctrl_p(&mut rt);
    assert!(rt.state.shown_pins_panel().is_some());
    assert!(
        shows(&rt, "Loading pinned messages"),
        "the pins are not known yet"
    );
    pin(&mut rt, &[]);
    assert!(shows(&rt, "No pinned messages yet"));
    ctrl_p(&mut rt);
    assert!(
        rt.state.shown_pins_panel().is_none(),
        "Ctrl+P again closes it"
    );
}

#[test]
fn a_pin_opens_marked_and_leaving_the_panel_shrinks_it_to_that_pin() {
    let mut rt = conversation("");
    pin(&mut rt, &[3, 1]);
    ctrl_p(&mut rt);
    let key = rt.state.tabs.shown().unwrap().key;
    rt.update(move |shell, tx| shell.open_pin(key, message_id(3), false, tx));
    rt.frame(FRAME_S);
    assert_eq!(
        rt.state.tabs.shown().unwrap().flash.map(|(id, _)| id),
        Some(message_id(3)),
        "the pin wears highlight.selected in the conversation"
    );
    assert_eq!(
        rt.state.shown_pins_panel().and_then(|panel| panel.opened),
        Some(message_id(3))
    );
    rt.update(|shell, _tx| shell.pins_left());
    rt.frame(FRAME_S);
    assert!(rt.state.shown_pins_panel().is_some_and(|panel| panel.peek));
    ctrl_p(&mut rt);
    assert!(rt.state.shown_pins_panel().is_some_and(|panel| !panel.peek));
}

#[test]
fn with_nothing_opened_the_pointer_leaving_keeps_the_panel_whole() {
    let mut rt = conversation("");
    pin(&mut rt, &[1]);
    ctrl_p(&mut rt);
    rt.update(|shell, _tx| shell.pins_left());
    rt.frame(FRAME_S);
    assert!(rt.state.shown_pins_panel().is_some_and(|panel| !panel.peek));
}

#[test]
fn with_nothing_opened_a_press_outside_closes_the_panel() {
    let mut rt = conversation("");
    pin(&mut rt, &[1]);
    ctrl_p(&mut rt);
    rt.update(|shell, _tx| {
        shell.worked_outside();
    });
    rt.frame(FRAME_S);
    assert!(rt.state.shown_pins_panel().is_none());
}

#[test]
fn escape_closes_the_panel_then_folds_the_strip_then_goes_back_to_writing() {
    let mut rt = conversation("");
    rt.update(|shell, tx| {
        let account = shell.tabs.shown().unwrap().key.account;
        let quick = ["🧀", "👍", "😂"].map(str::to_owned).to_vec();
        shell.activity_posted(account, crate::live_watch::Posted::Quick(quick), tx);
    });
    pin(&mut rt, &[]);
    rt.update(|_shell, tx| tx.focus(message_key(message_id(3)), true));
    rt.frame(FRAME_S);
    press(&mut rt, Key::Text("+".to_owned()));
    // Opened from the message: the panel takes the keyboard, and gives it
    // back when it closes.
    ctrl_p(&mut rt);
    assert!(rt.state.shown_pins_panel().is_some());
    press(&mut rt, Key::Escape);
    assert!(rt.state.shown_pins_panel().is_none(), "the panel first");
    assert!(rt.state.reacting().stretched.is_some(), "the strip stays");
    let focused = rt
        .elements()
        .into_iter()
        .find(|element| element.focused)
        .and_then(|element| crate::timeline::content::copy::message_of(&element.path));
    assert_eq!(
        focused,
        Some(message_id(3)),
        "the keyboard is back on the message"
    );
    press(&mut rt, Key::Escape);
    assert!(rt.state.reacting().stretched.is_none(), "then the strip");
    press(&mut rt, Key::Escape);
    let focused = rt
        .elements()
        .into_iter()
        .find(|element| element.focused)
        .and_then(|element| element.key);
    assert_eq!(focused.as_deref(), Some(crate::composer::COMPOSER_KEY));
}

#[test]
fn the_pointer_leaving_the_panel_for_the_conversation_shrinks_it() {
    let mut rt = conversation("");
    pin(&mut rt, &[3, 1]);
    ctrl_p(&mut rt);
    let key = rt.state.tabs.shown().unwrap().key;
    rt.update(move |shell, tx| shell.open_pin(key, message_id(3), false, tx));
    rt.frame(FRAME_S);
    let panel = rt
        .elements()
        .into_iter()
        .find(|element| element.key.as_deref() == Some(super::super::pins::PINS_PANEL_KEY))
        .expect("the panel")
        .rect;
    rt.pointer_move(panel[0] + 20.0, panel[1] + panel[3] / 2.0);
    rt.frame(FRAME_S);
    assert!(rt.state.shown_pins_panel().is_some_and(|panel| !panel.peek));
    rt.pointer_move(panel[0] - 40.0, panel[1] + panel[3] / 2.0);
    rt.frame(FRAME_S);
    assert!(rt.state.shown_pins_panel().is_some_and(|panel| panel.peek));
}

#[test]
fn the_pointer_coming_back_onto_the_shrunk_panel_stretches_it() {
    let mut rt = conversation("");
    pin(&mut rt, &[3]);
    ctrl_p(&mut rt);
    let key = rt.state.tabs.shown().unwrap().key;
    rt.update(move |shell, tx| shell.open_pin(key, message_id(3), false, tx));
    rt.update(|shell, _tx| shell.pins_left());
    rt.frame(FRAME_S);
    assert!(rt.state.shown_pins_panel().is_some_and(|panel| panel.peek));
    rt.update(|shell, tx| {
        shell.pins_hovered(tx);
    });
    rt.frame(FRAME_S);
    assert!(rt.state.shown_pins_panel().is_some_and(|panel| !panel.peek));
}

#[test]
fn with_nothing_opened_a_press_on_a_message_closes_the_panel_and_leaving_does_not() {
    let mut rt = conversation("");
    pin(&mut rt, &[1]);
    ctrl_p(&mut rt);
    rt.update(|shell, _tx| shell.pins_left());
    rt.frame(FRAME_S);
    assert!(
        rt.state.shown_pins_panel().is_some(),
        "leaving it alone keeps it"
    );
    let row = rt
        .elements()
        .into_iter()
        .find(|element| {
            element.path.ends_with(&format!(
                "{}/{}",
                message_key(message_id(2)),
                crate::timeline::rows::message_part_key(message_id(2))
            ))
        })
        .expect("message 2's row")
        .rect;
    // The row's left edge, past its avatar and text: nothing in it takes the press.
    let (x, y) = (row[0] + row[2] - 4.0, row[1] + row[3] - 2.0);
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(FRAME_S);
    assert!(
        rt.state.shown_pins_panel().is_none(),
        "a press on a message closes it"
    );
}

#[test]
fn a_key_on_a_message_typing_or_a_wheel_on_the_conversation_is_work_outside() {
    let mut rt = conversation("");
    pin(&mut rt, &[1]);
    ctrl_p(&mut rt);
    rt.update(|_shell, tx| tx.focus(message_key(message_id(3)), true));
    rt.frame(FRAME_S);
    press(&mut rt, Key::Down);
    assert!(rt.state.shown_pins_panel().is_none(), "a key on a message");
    ctrl_p(&mut rt);
    let key = rt.state.tabs.shown().unwrap().key;
    rt.update(move |shell, _tx| {
        crate::timeline::TimelineHost::timeline_wheeled(shell, key);
    });
    assert!(
        rt.state.shown_pins_panel().is_none(),
        "a wheel on the conversation"
    );
}

#[test]
fn the_app_acting_on_its_own_leaves_the_panel_alone() {
    let mut rt = conversation("");
    pin(&mut rt, &[1]);
    ctrl_p(&mut rt);
    let key = rt.state.tabs.shown().unwrap().key;
    // New rows reported visible (an arrival, a load, a resize) are no input.
    rt.update(move |shell, _tx| {
        crate::timeline::TimelineHost::rows_visible(shell, key, 0..3);
    });
    rt.frame(FRAME_S);
    assert!(rt.state.shown_pins_panel().is_some());
}

#[test]
fn an_opened_pin_that_goes_is_forgotten() {
    let mut rt = conversation("");
    pin(&mut rt, &[3, 1]);
    ctrl_p(&mut rt);
    let key = rt.state.tabs.shown().unwrap().key;
    rt.update(move |shell, tx| shell.open_pin(key, message_id(3), false, tx));
    rt.update(|shell, _tx| {
        shell.worked_outside();
    });
    assert!(rt.state.shown_pins_panel().is_some_and(|panel| panel.peek));
    pin(&mut rt, &[1]);
    rt.update(move |shell, _tx| shell.pins_changed(key));
    let panel = rt.state.shown_pins_panel().unwrap();
    assert_eq!((panel.opened, panel.peek), (None, false));
}

/// Message `n`'s hover bar's opacity, once its fade is done.
fn bar_opacity(rt: &mut Runtime<Shell>, n: u128) -> Option<f32> {
    for _ in 0..30 {
        rt.frame(FRAME_S);
    }
    let row = format!("/{}/", message_key(message_id(n)));
    rt.elements()
        .into_iter()
        .find(|element| element.path.contains(&row) && element.path.ends_with("/bar"))
        .map(|element| element.opacity)
}

#[test]
fn a_message_under_the_panel_does_not_see_the_pointer_on_the_panel() {
    let mut rt = conversation("");
    pin(&mut rt, &[3, 1]);
    ctrl_p(&mut rt);
    let panel = rt
        .elements()
        .into_iter()
        .find(|element| element.key.as_deref() == Some(super::super::pins::PINS_PANEL_KEY))
        .expect("the panel")
        .rect;
    let row_at = |rt: &Runtime<Shell>, y: f32| {
        (1..=5u128).find(|n| {
            let key = message_key(message_id(*n));
            rt.elements().into_iter().any(|element| {
                let [_, top, _, height] = element.rect;
                element.key.as_deref() == Some(key.as_str()) && y >= top && y < top + height
            })
        })
    };
    // The header, the padding and the middle: none lets a row under it hover.
    for (x, y) in [
        (panel[0] + 40.0, panel[1] + 14.0),
        (panel[0] + 3.0, panel[1] + panel[3] / 2.0),
        (panel[0] + panel[2] / 2.0, panel[1] + panel[3] / 2.0),
    ] {
        let Some(row) = row_at(&rt, y) else {
            continue;
        };
        rt.pointer_move(x, y);
        assert_eq!(
            bar_opacity(&mut rt, row),
            Some(0.0),
            "the panel covers message {row} at ({x}, {y})"
        );
        // Off the panel, over the same row, the bar shows.
        rt.pointer_move(panel[0] - 40.0, y);
        assert_eq!(bar_opacity(&mut rt, row), Some(1.0));
    }
}

#[test]
fn a_message_under_a_floating_pill_does_not_see_the_pointer_on_it() {
    let mut rt = conversation("");
    // Enough messages that rows sit under the header and the composer.
    rt.update(|shell, _tx| {
        let open = shell.tabs.shown_mut().unwrap();
        let items = (1..=40)
            .map(|n| {
                super::replying::item(n, super::replying::lia(), &format!("message {n}"), None)
            })
            .collect();
        let key = open.key;
        let resources = open.resources.clone();
        *open = crate::timeline::OpenTimeline::loaded(key, items);
        open.resources = resources;
    });
    for _ in 0..10 {
        rt.frame(FRAME_S);
    }
    let mut checked = 0;
    for key in [
        "title-pill",
        "actions-pill",
        crate::composer::COMPOSER_PILL_KEY,
    ] {
        let Some(pill) = rt
            .elements()
            .into_iter()
            .find(|element| element.key.as_deref() == Some(key))
            .map(|element| element.rect)
        else {
            panic!("no {key}");
        };
        let (x, y) = (pill[0] + 6.0, pill[1] + pill[3] / 2.0);
        let row = (1..=40u128).find(|n| {
            let row_key = message_key(message_id(*n));
            rt.elements().into_iter().any(|element| {
                let [_, top, _, height] = element.rect;
                element.key.as_deref() == Some(row_key.as_str()) && y >= top && y < top + height
            })
        });
        let Some(row) = row else {
            continue;
        };
        rt.pointer_move(x, y);
        assert_eq!(
            bar_opacity(&mut rt, row),
            Some(0.0),
            "{key} covers message {row}"
        );
        checked += 1;
    }
    assert!(checked >= 2, "rows sit under the pills: {checked} checked");
}

#[test]
fn a_press_on_pinned_media_or_a_pinned_card_keeps_the_panel_open() {
    use crate::message_media::received::{MediaAt, MediaHost, MediaPress, MediaSurface};
    let mut rt = conversation("");
    pin(&mut rt, &[1]);
    ctrl_p(&mut rt);
    let at = |surface| {
        move |shell: &Shell| {
            let key = shell.tabs.shown().unwrap().key;
            MediaAt {
                surface,
                account: key.account,
                channel: key.channel,
                message: message_id(1),
                index: 0,
            }
        }
    };
    let pinned = at(MediaSurface::Pins);
    rt.update(move |shell, tx| {
        let at = pinned(shell);
        shell.media_press(at, MediaPress::Open, tx);
        crate::timeline::TimelineHost::card_pressed(
            shell,
            at,
            crate::link_cards::CardPress::OpenInBrowser,
            tx,
        );
    });
    rt.frame(FRAME_S);
    assert!(
        rt.state.shown_pins_panel().is_some(),
        "a press inside the panel keeps it"
    );
    let conversation = at(MediaSurface::Timeline);
    rt.update(move |shell, tx| {
        let at = conversation(shell);
        shell.media_press(at, MediaPress::Open, tx);
    });
    rt.frame(FRAME_S);
    assert!(
        rt.state.shown_pins_panel().is_none(),
        "the conversation's media is outside it"
    );
}
