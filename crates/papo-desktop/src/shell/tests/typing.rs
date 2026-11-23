//! The typing line, headless: who else types, as `live_watch` would post
//! it, over the shown conversation's composer.

use std::collections::BTreeMap;
use std::sync::Arc;

use papo_sdk::Generation;
use papo_sdk_ui::{AccountActivity, TypingItem};

use super::replying::{conversation, lia};
use super::*;
use crate::live_watch::Posted;

/// `account`'s people's activity with `typists` typing in `channel`.
fn typing(rt: &mut Runtime<Shell>, channel: Uuid, typists: &[(Uuid, Option<&str>)]) {
    let items = typists
        .iter()
        .map(|(user, phrase)| TypingItem {
            user: *user,
            nickname: None,
            phrase: phrase.map(str::to_owned),
        })
        .collect();
    let activity = AccountActivity {
        generation: Generation::for_test(1),
        live: true,
        presence: BTreeMap::new(),
        typing: BTreeMap::from([(channel, items)]),
    };
    let account = rt.state.tabs.shown().unwrap().key.account;
    rt.update(move |shell, tx| {
        shell.activity_posted(account, Posted::People(Arc::new(activity)), tx);
    });
    rt.frame(FRAME_S);
}

fn typing_line(rt: &Runtime<Shell>) -> Option<String> {
    let line = rt
        .elements()
        .into_iter()
        .position(|element| element.key.as_deref() == Some("typing-line") && !element.exiting)?;
    let elements = rt.elements();
    elements[line..]
        .iter()
        .find_map(|element| element.text.clone().filter(|text| text.contains("typing")))
}

#[test]
fn the_typing_line_names_who_else_types_in_the_shown_conversation_and_goes_when_they_stop() {
    let mut rt = conversation("");
    let channel = rt.state.tabs.shown().unwrap().key.channel;
    assert_eq!(typing_line(&rt), None);
    typing(&mut rt, channel, &[(lia(), None)]);
    // The fixture knows Lia by username only.
    assert_eq!(typing_line(&rt).as_deref(), Some("lia is typing…"));
    // Someone typing elsewhere is not this conversation's.
    typing(&mut rt, Uuid::from_u128(0x99), &[(lia(), None)]);
    assert_eq!(typing_line(&rt), None);
    assert!(settles(&mut rt), "no frames while nobody types");
}
