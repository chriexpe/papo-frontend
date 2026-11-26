//! GIF favorites, headless (`docs/design.md` §16.4): S on a focused message with a GIPHY GIF
//! stars it and again unstars it; its menu says which; a picker tile's Ctrl+D stars the
//! keyboard's GIF, which is then spoken as a favorite; a message without a GIPHY GIF takes none.

use std::sync::Arc;

use papo_sdk::{Gif, Message};
use papo_sdk_ui::TimelineItem;
use rune::{Key, KeyEvent, Mods, Runtime};

use super::replying::{conversation, message_id};
use super::{FRAME_S, Shell, press};
use crate::gif_picker::GifPicker;
use crate::menu::Entry;
use crate::picker::{OpenPicker, PICKER_KEY};
use crate::timeline::rows::message_key;

const GIPHY_ID: &str = "ReocLd5xHpvanCOQ7Y";

/// Message 3 becomes a GIPHY GIF link the server made a `gif` embed.
fn with_gif(rt: &mut Runtime<Shell>) {
    rt.update(|shell, _tx| {
        let open = shell.tabs.shown_mut().unwrap();
        let at = open
            .items
            .iter()
            .position(|item| matches!(item.as_ref(), TimelineItem::Message { id, .. } if *id == message_id(3)))
            .unwrap();
        let TimelineItem::Message {
            id,
            message,
            content,
            reply,
            pending,
        } = open.items[at].as_ref().clone()
        else {
            unreachable!()
        };
        let embeds = serde_json::from_value(serde_json::json!([{
            "id": "00000000-0000-4000-8000-000000000006",
            "fetch_method": "gif",
            "url": format!("https://giphy.com/gifs/{GIPHY_ID}"),
            "title": "Football",
            "thumbnail": { "mime_type": "image/gif", "width": 200, "height": 150 },
        }]))
        .unwrap();
        let message = Message {
            content: Some(format!("https://media1.giphy.com/media/{GIPHY_ID}/giphy.gif")),
            embeds: Some(embeds),
            ..Message::clone(&message)
        };
        open.items.set(
            at,
            Arc::new(TimelineItem::Message {
                id,
                message: Arc::new(message),
                content,
                reply,
                pending,
            }),
        );
    });
    rt.frame(FRAME_S);
}

fn favorite(rt: &Runtime<Shell>) -> bool {
    let account = rt.state.tabs.shown().unwrap().key.account;
    rt.state.favorite_gifs(account).contains(GIPHY_ID)
}

fn focus_message(rt: &mut Runtime<Shell>, n: u128) {
    rt.update(move |_shell, tx| tx.focus(message_key(message_id(n)), true));
    rt.frame(FRAME_S);
}

fn menu_labels(rt: &Runtime<Shell>) -> Vec<String> {
    rt.state
        .nav
        .menu
        .iter()
        .flat_map(|menu| menu.entries.iter())
        .filter_map(|entry| match entry {
            Entry::Item(item) => Some(item.label.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn s_on_a_gif_message_stars_it_and_again_unstars_it_and_its_menu_says_which() {
    let mut rt = conversation("");
    with_gif(&mut rt);
    focus_message(&mut rt, 3);
    press(&mut rt, Key::Text("s".to_owned()));
    assert!(favorite(&rt));
    press(&mut rt, Key::Enter);
    assert!(
        menu_labels(&rt)
            .iter()
            .any(|label| label == "Remove from favorites"),
        "{:?}",
        menu_labels(&rt)
    );
    press(&mut rt, Key::Escape);
    focus_message(&mut rt, 3);
    press(&mut rt, Key::Text("s".to_owned()));
    assert!(!favorite(&rt));
}

#[test]
fn a_message_without_a_giphy_gif_takes_no_star() {
    let mut rt = conversation("");
    focus_message(&mut rt, 2);
    press(&mut rt, Key::Text("s".to_owned()));
    let account = rt.state.tabs.shown().unwrap().key.account;
    assert!(rt.state.favorite_gifs(account).is_empty());
    press(&mut rt, Key::Enter);
    assert!(
        !menu_labels(&rt)
            .iter()
            .any(|label| label.contains("favorites")),
        "{:?}",
        menu_labels(&rt)
    );
}

#[test]
fn ctrl_d_in_the_picker_stars_the_keyboards_gif_which_is_then_spoken_as_a_favorite() {
    let mut rt = conversation("");
    rt.update(|shell, _tx| {
        let gif = Gif {
            id: GIPHY_ID.to_owned(),
            url: format!("https://media.giphy.com/media/{GIPHY_ID}/giphy.gif"),
            title: "Football".to_owned(),
            ..Gif::default()
        };
        let mut picker = GifPicker::new(true, vec![gif], Vec::new());
        picker.step(1);
        shell.picker = Some(OpenPicker::Gif(picker));
    });
    rt.update(|_shell, tx| tx.focus(crate::gif_picker::view::GIF_SEARCH_KEY, false));
    rt.frame(FRAME_S);
    rt.key(KeyEvent {
        key: Key::Command('d'),
        mods: Mods {
            cmd: true,
            ctrl: true,
            ..Mods::default()
        },
    });
    rt.frame(FRAME_S);
    assert!(favorite(&rt));
    let options: Vec<String> = rt
        .elements()
        .into_iter()
        .filter(|element| element.path.contains(PICKER_KEY) && element.role == "option")
        .filter_map(|element| element.label)
        .collect();
    assert_eq!(options, ["GIF, Football, favorite"]);
}
