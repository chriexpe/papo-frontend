//! Direct's longer lists, headless: a long list of conversations, which
//! builds only the rows on screen, and the people the account blocks.

use std::sync::Arc;

use papo_sdk_ui::DirectMessageItem;

use super::direct::dm;
use super::navigation::{click, named, with_servers};
use super::replying::lia;
use super::*;
use crate::direct_pane::dm_row_key;

/// Direct with `count` conversations with people on Lumen.
fn many(count: u128) -> (Runtime<Shell>, Vec<Arc<DirectMessageItem>>) {
    let mut rt = with_servers();
    let rows: Vec<_> = (0..count)
        .map(|n| {
            dm(
                100 + n,
                Uuid::from_u128(0x500 + n),
                &format!("P{n}"),
                "Lumen",
            )
        })
        .collect();
    let posted = rows.clone();
    rt.update(move |shell, tx| {
        shell.direct_posted(posted.into_iter().collect(), tx);
        shell.select_direct(tx);
    });
    rt.frame(FRAME_S);
    (rt, rows)
}

#[test]
fn the_arrows_walk_a_long_list_past_the_rows_built_on_screen() {
    let (mut rt, rows) = many(40);
    let first = dm_row_key(&rows[0]);
    rt.update(move |_shell, tx| tx.focus(first, true));
    rt.frame(FRAME_S);
    for _ in 0..30 {
        press(&mut rt, Key::Down);
    }
    let focused = rt
        .elements()
        .into_iter()
        .find(|element| element.focused)
        .and_then(|element| element.key);
    assert_eq!(focused, Some(dm_row_key(&rows[30])));
}

#[test]
fn coming_back_to_direct_shows_the_picked_conversation_far_down() {
    let (mut rt, rows) = many(40);
    let picked = Arc::clone(&rows[35]);
    rt.update(move |shell, tx| {
        shell.pick_dm(picked, tx);
        shell.select_server(named("Lumen"), tx);
    });
    rt.frame(FRAME_S);
    rt.update(Shell::select_direct);
    for _ in 0..10 {
        rt.frame(FRAME_S);
    }
    let key = dm_row_key(&rows[35]);
    let shown = rt.elements().into_iter().any(|element| {
        element.key.as_deref() == Some(key.as_str())
            && element.visible[2] > 0.0
            && element.visible[3] > 0.0
    });
    assert!(shown, "its row is built and on screen");
}

#[test]
fn the_pills_blocked_switch_lists_the_people_blocked_with_unblock() {
    let mut rt = with_servers();
    rt.update(|shell, tx| {
        let mut servers = shell.navigation().servers().servers.clone();
        servers[0].blocked = vec![crate::server_list::BlockedPerson {
            id: lia(),
            name: Some("Lia".to_owned()),
        }];
        shell.servers_changed(
            crate::server_list::ServerList {
                servers,
                direct: crate::badges::Badge::default(),
            },
            tx,
        );
        shell.select_direct(tx);
    });
    rt.frame(FRAME_S);
    assert!(
        !tree(&rt).contains("Unblock Lia"),
        "the conversations first"
    );
    click(&mut rt, crate::direct_pane::blocked::BLOCKED_BUTTON_KEY);
    rt.frame(FRAME_S);
    let tree = tree(&rt);
    assert!(tree.contains("Lia, on Lumen"));
    assert!(tree.contains("Unblock Lia"));
}
