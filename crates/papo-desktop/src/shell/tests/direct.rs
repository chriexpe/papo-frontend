//! Direct, headless: the list is posted as `direct_list` would post it, and
//! the people's activity as `live_watch` would.

use std::collections::BTreeMap;
use std::sync::Arc;

use papo_sdk::{Generation, Presence, PresenceStatus};
use papo_sdk_ui::{AccountActivity, DirectMessageId, DirectMessageItem, TypingItem};

use super::navigation::{ana, click, entry, named, with_servers};
use super::replying::{conversation, lia, resources_owned_by};
use super::*;
use crate::direct_list::DirectRows;
use crate::direct_pane::dm_row_key;
use crate::live_watch::Posted;
use crate::rail::DIRECT_KEY;

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
    assert_eq!(
        row_label(&rt, &row),
        "Lia, on Lumen",
        "no presence while not live"
    );
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
    assert_eq!(row_label(&rt, &row), "Lia, Away, on Lumen");
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

#[test]
fn a_conversation_a_card_opened_is_picked_once_the_list_holds_it() {
    let mut rt = with_servers();
    let row = dm(7, lia(), "Lia", "Lumen");
    let id = row.id.clone();
    rt.update(move |shell, tx| shell.await_dm(id, tx));
    assert!(
        rt.state.navigation().picked_dm().is_none(),
        "not in the list yet"
    );
    rt.update(move |shell, tx| {
        shell.direct_posted([row].into_iter().collect(), tx);
    });
    let picked = rt
        .state
        .navigation()
        .picked_dm()
        .map(|picked| picked.id.clone());
    assert_eq!(picked, Some(dm(7, lia(), "Lia", "Lumen").id.clone()));
    assert!(rt.state.navigation().direct_shown());
}

#[test]
fn a_conversation_the_backend_refuses_says_so_in_the_text_pills_place() {
    use papo_sdk::operations::social::DirectAccess;
    let mut rt = direct();
    let row = dm(1, lia(), "Lia", "Lumen");
    click(&mut rt, &dm_row_key(&row));
    assert!(rt.state.picked_dm_access().is_none(), "open while listed");
    // It leaves the list; the backend says it refuses it.
    rt.update(|shell, tx| {
        shell.direct_posted(DirectRows::new(), tx);
    });
    let id = row.id.clone();
    rt.update(move |shell, tx| {
        shell
            .dm_access
            .insert(id.clone(), crate::shell::direct::Asked::Pending);
        shell.dm_access_answered(id, Ok(DirectAccess::Refused), tx);
    });
    rt.frame(FRAME_S);
    assert_eq!(rt.state.picked_dm_access(), Some(DirectAccess::Refused));
    assert!(tree(&rt).contains("You can't message Lia."));
    // Back in the list, it is open again.
    rt.update(move |shell, tx| {
        shell.direct_posted([row].into_iter().collect(), tx);
    });
    assert!(rt.state.picked_dm_access().is_none());
}

#[test]
fn ctrl_u_shows_and_hides_a_dms_profile_pane() {
    let mut rt = direct();
    let row = dm(1, lia(), "Lia", "Lumen");
    click(&mut rt, &dm_row_key(&row));
    // The SDK would open its timeline; the fixture shows it with Lia known.
    let key = crate::timeline::TimelineKey {
        account: row.id.account,
        channel: row.id.channel,
    };
    rt.update(move |shell, _tx| {
        let mut open = crate::timeline::OpenTimeline::loaded(key, imbl::Vector::new());
        open.resources = Some(Arc::new(resources_owned_by(key, Uuid::from_u128(0x4001))));
        shell.tabs.show(Some(key), false);
        *shell.tabs.shown_mut().unwrap() = open;
    });
    rt.frame(FRAME_S);
    let pane = |rt: &Runtime<Shell>| {
        rt.elements()
            .into_iter()
            .any(|element| element.key.as_deref() == Some("profile-pane") && !element.exiting)
    };
    assert!(!pane(&rt));
    rt.key(KeyEvent {
        key: Key::Command('u'),
        mods: Mods {
            ctrl: true,
            cmd: true,
            ..Mods::default()
        },
    });
    rt.frame(FRAME_S);
    assert!(pane(&rt));
    assert!(
        tree(&rt).contains("@lia"),
        "the person, as the card shows them"
    );
    click(&mut rt, crate::shell::profile_pane::PROFILE_BUTTON_KEY);
    rt.frame(FRAME_S);
    assert!(!pane(&rt));
}

#[test]
fn an_answer_that_comes_after_the_conversation_is_back_in_the_list_is_dropped() {
    use papo_sdk::operations::social::DirectAccess;
    let mut rt = direct();
    let row = dm(1, lia(), "Lia", "Lumen");
    click(&mut rt, &dm_row_key(&row));
    let id = row.id.clone();
    rt.update(move |shell, tx| {
        shell.direct_posted(DirectRows::new(), tx);
        shell
            .dm_access
            .insert(id.clone(), crate::shell::direct::Asked::Pending);
        shell.direct_posted([row].into_iter().collect(), tx);
        shell.dm_access_answered(id, Ok(DirectAccess::Refused), tx);
    });
    assert!(rt.state.picked_dm_access().is_none());
}

#[test]
fn an_account_switch_drops_the_other_accounts_conversation() {
    let mut rt = direct();
    let row = dm(1, lia(), "Lia", "Lumen");
    click(&mut rt, &dm_row_key(&row));
    assert!(rt.state.navigation().picked_dm().is_some());
    // Lumen now shows another account: Lia's conversation was Ana's.
    rt.update(|shell, tx| {
        let mut lumen = entry("Lumen", &["general"]);
        let other = crate::server_list::AccountEntry {
            id: AccountId::new(named("Lumen"), Uuid::from_u128(0x77)),
            name: "Ana Two".to_owned(),
            username: "ana2".to_owned(),
        };
        lumen.account = Some(other.clone());
        lumen.accounts.push(other);
        shell.servers_changed(
            crate::server_list::ServerList {
                servers: vec![lumen, entry("Mirante", &["avisos"])],
                direct: crate::badges::Badge::default(),
            },
            tx,
        );
    });
    assert!(rt.state.navigation().picked_dm_row().is_none());
}

#[test]
fn a_conversation_the_account_is_no_longer_in_closes() {
    use papo_sdk::operations::social::DirectAccess;
    let mut rt = direct();
    let row = dm(1, lia(), "Lia", "Lumen");
    click(&mut rt, &dm_row_key(&row));
    let id = row.id.clone();
    rt.update(move |shell, tx| {
        shell.direct_posted(DirectRows::new(), tx);
        shell
            .dm_access
            .insert(id.clone(), crate::shell::direct::Asked::Pending);
        shell.dm_access_answered(id, Ok(DirectAccess::Gone), tx);
    });
    assert!(rt.state.navigation().picked_dm().is_none());
}

#[test]
fn a_notice_shows_in_direct_with_no_conversation_open() {
    let mut rt = direct();
    rt.update(|shell, tx| shell.refuse(crate::notice::Refusal::CloseDm, tx));
    rt.frame(FRAME_S);
    let notice = rt
        .elements()
        .into_iter()
        .find(|element| element.key.as_deref() == Some(crate::notice::NOTICE_KEY))
        .expect("the notice");
    let [x, y, w, h] = notice.rect;
    assert!(
        w > 0.0 && h > 0.0 && x >= 0.0 && y >= 0.0,
        "drawn in the window"
    );
    assert!(tree(&rt).contains("Couldn't close the conversation."));
}
