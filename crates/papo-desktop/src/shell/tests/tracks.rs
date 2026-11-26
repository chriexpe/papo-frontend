//! A provider's track (`docs/design.md` §15.4), headless and without WPE: its cover is spoken as
//! a track, it asks and plays like a video but has no viewer, and it plays on scrolled away
//! until another conversation shows.

use std::sync::Arc;

use papo_sdk::Message;
use papo_sdk_ui::TimelineItem;
use papo_sdk_ui::message_content::MessageContent;
use rune::{Key, Runtime};
use serde_json::json;

use super::players::{
    calls, embed_id, focus_message, opened, play_once, player_key, settle, with_youtube,
};
use super::replying::message_id;
use super::{Shell, press};
use crate::provider_players::Phase;

/// The conversation with message 3 linking a Spotify track, as the server's oEmbed crawl sends
/// it: a card with its cover and no player of its own.
fn with_track() -> Runtime<Shell> {
    let mut rt = with_youtube();
    rt.update(move |shell, _tx| {
        let id = message_id(3);
        let message: Message = serde_json::from_value(json!({
            "id": id.to_string(),
            "channel_id": shell.tabs.shown().unwrap().key.channel.to_string(),
            "author_id": papo_sdk::Uuid::from_u128(0x4002).to_string(),
            "content": "https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT",
            "created_at": "2026-11-20T12:03:00Z",
            "embeds": [{
                "id": embed_id().to_string(),
                "fetch_method": "oembed",
                "provider": "Spotify",
                "url": "https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT",
                "title": "Never Gonna Give You Up",
            }],
        }))
        .unwrap();
        shell.tabs.shown_mut().unwrap().items.set(
            2,
            Arc::new(TimelineItem::Message {
                id,
                message: Arc::new(message),
                content: Arc::new(MessageContent::parse(
                    "https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT",
                )),
                reply: None,
                pending: None,
            }),
        );
    });
    settle(&mut rt);
    rt
}

#[test]
fn a_track_asks_and_plays_spotifys_player_with_no_viewer() {
    let mut rt = with_track();
    let tree = rt.accessibility_json().to_string();
    assert!(
        tree.contains("Track from Spotify, Never Gonna Give You Up"),
        "{tree}"
    );
    play_once(&mut rt);
    assert!(
        opened().iter().any(|call| call
            .contains("spotify https://open.spotify.com/embed/track/4cOdK2wGLETKBW3PvgPWqT")),
        "{:?}",
        calls()
    );
    focus_message(&mut rt);
    press(&mut rt, Key::Text("o".to_owned()));
    settle(&mut rt);
    assert_eq!(
        rt.state.players.store.viewer(),
        None,
        "a track has no viewer"
    );
}

#[test]
fn a_track_plays_on_out_of_view_and_stops_when_its_conversation_is_left() {
    let mut rt = with_track();
    let key = player_key(&rt);
    play_once(&mut rt);
    rt.update(|shell, tx| {
        shell.players_drawn(&std::collections::HashSet::new());
        shell.poll_players(100.0, tx);
        shell.poll_players(130.0, tx);
    });
    assert!(
        rt.state.players.store.page(key).is_some(),
        "scrolled away, still playing"
    );
    rt.update(|shell, tx| {
        shell.tabs.show(None, false);
        shell.poll_players(131.0, tx);
    });
    assert_eq!(rt.state.players.store.page(key), None);
    assert_eq!(rt.state.players.store.phase(key), Some(Phase::Stopped));
}

#[test]
fn o_on_a_resting_track_leaves_it_and_plays_nothing() {
    let mut rt = with_track();
    let key = player_key(&rt);
    focus_message(&mut rt);
    press(&mut rt, Key::Text("o".to_owned()));
    settle(&mut rt);
    assert_eq!(rt.state.players.store.phase(key), None, "the cover stays");
    assert!(opened().is_empty(), "{:?}", calls());
}

#[test]
fn a_tracks_page_opens_at_the_strips_size() {
    let mut rt = with_track();
    play_once(&mut rt);
    let call = opened()
        .into_iter()
        .find(|call| call.contains("spotify"))
        .expect("the track's page opened");
    assert!(call.contains("x152"), "{call}");
}
