//! Direct, headless: the list is posted as `direct_list` would post it, and
//! the people's activity as `live_watch` would.

use std::collections::BTreeMap;
use std::sync::Arc;

use papo_sdk::{Generation, Presence, PresenceStatus};
use papo_sdk_ui::{AccountActivity, DirectMessageId, DirectMessageItem, TypingItem};

use super::navigation::{ana, click, named, with_servers};
use super::*;
use crate::direct_list::DirectRows;
use crate::direct_pane::dm_row_key;
use crate::live_watch::Posted;
use crate::rail::DIRECT_KEY;

fn lia() -> Uuid {
    Uuid::from_u128(0x51)
}

fn dm(channel: u128, user: Uuid, name: &str, server: &str) -> Arc<DirectMessageItem> {
    Arc::new(DirectMessageItem {
        id: DirectMessageId {
            account: ana(server).id,
            channel: Uuid::from_u128(channel),
        },
        generation: Generation::for_test(1),
        user,
        username: Some(name.to_lowercase()),
        nickname: Some(name.to_owned()),
        activity_at: "2026-11-23T12:00:00Z".to_owned(),
    })
}

/// Two servers, Direct shown, and a conversation with Lia on Lumen and one
/// with Yuki on Mirante.
fn direct() -> Runtime<Shell> {
    let mut rt = with_servers();
    let rows = [
        dm(1, lia(), "Lia", "Lumen"),
        dm(2, Uuid::from_u128(0x52), "Yuki", "Mirante"),
    ];
    rt.update(move |shell, tx| {
        shell.direct_posted(rows.into_iter().collect(), tx);
        shell.select_direct(tx);
    });
    rt.frame(FRAME_S);
    rt
}

fn row_label(rt: &Runtime<Shell>, row: &DirectMessageItem) -> String {
    let key = dm_row_key(row);
    rt.elements()
        .into_iter()
        .find(|element| element.key.as_deref() == Some(key.as_str()))
        .and_then(|element| element.label)
        .unwrap_or_else(|| panic!("no row {key}"))
}

#[test]
fn direct_on_the_rail_lists_the_conversations_in_the_channel_pane_with_their_server() {
    let mut rt = with_servers();
    rt.update(|shell, tx| {
        shell.direct_posted([dm(1, lia(), "Lia", "Lumen")].into_iter().collect(), tx);
    });
    click(&mut rt, DIRECT_KEY);
    rt.frame(FRAME_S);
    assert!(rt.state.navigation().direct_shown());
    assert!(
        rt.state.navigation().selected().is_none(),
        "no server is selected"
    );
    let tree = tree(&rt);
    assert!(tree.contains("Direct messages"));
    assert!(tree.contains("Lumen"), "the server under the person");
}

#[test]
fn a_row_says_the_persons_presence_and_typing_while_they_type() {
    let mut rt = direct();
    let row = dm(1, lia(), "Lia", "Lumen");
    assert_eq!(row_label(&rt, &row), "Lia", "no presence while not live");
    let presence = BTreeMap::from([(
        lia(),
        Presence {
            user: lia(),
            status: PresenceStatus::Away,
            status_message: None,
            typing_phrase: None,
            nickname: None,
        },
    )]);
    let typing = BTreeMap::from([(
        row.id.channel,
        vec![TypingItem {
            user: lia(),
            nickname: None,
            phrase: None,
        }],
    )]);
    let activity = AccountActivity {
        generation: Generation::for_test(1),
        live: true,
        presence,
        typing,
    };
    let account = row.id.account;
    rt.update(move |shell, tx| {
        shell.activity_posted(account, Posted::People(Arc::new(activity)), tx);
    });
    rt.frame(FRAME_S);
    assert_eq!(row_label(&rt, &row), "Lia, Away");
    // Drawn in place of the server, never spoken: it would talk over the
    // focused row each time it came and went.
    let drawn = rt
        .elements()
        .into_iter()
        .any(|element| element.text.as_deref() == Some("typing…"));
    assert!(drawn);
}

#[test]
fn picking_a_conversation_shows_it_and_it_stays_when_it_leaves_the_list() {
    let mut rt = direct();
    let row = dm(1, lia(), "Lia", "Lumen");
    click(&mut rt, &dm_row_key(&row));
    let picked = |rt: &Runtime<Shell>| {
        rt.state
            .navigation()
            .picked_dm()
            .map(|picked| picked.id.clone())
    };
    assert_eq!(picked(&rt), Some(row.id.clone()));
    // A block or a hide elsewhere drops it from the list: it stays, so the
    // conversation can say why.
    rt.update(|shell, tx| {
        shell.direct_posted(DirectRows::new(), tx);
    });
    assert_eq!(picked(&rt), Some(row.id.clone()));
    // Choosing a server leaves Direct; coming back finds it picked.
    rt.update(|shell, tx| shell.select_server(named("Lumen"), tx));
    assert!(rt.state.navigation().picked_dm().is_none());
    rt.update(Shell::select_direct);
    assert_eq!(picked(&rt), Some(row.id.clone()));
}

#[test]
fn delete_on_a_row_closes_it_and_a_picked_one_is_no_longer_picked() {
    let mut rt = direct();
    let row = dm(1, lia(), "Lia", "Lumen");
    click(&mut rt, &dm_row_key(&row));
    let key = dm_row_key(&row);
    rt.update(move |_shell, tx| tx.focus(key, true));
    rt.frame(FRAME_S);
    press(&mut rt, Key::Delete);
    assert!(rt.state.navigation().picked_dm().is_none());
}
