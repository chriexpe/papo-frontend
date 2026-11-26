//! Provider players' rules of §15.3, without a WebView.

use papo_sdk::ServerAddress;

use super::*;

fn account() -> AccountId {
    AccountId::new(
        ServerAddress::parse("https://players.example.invalid")
            .unwrap()
            .id(),
        Uuid::from_u128(1),
    )
}

fn key(n: u128) -> PlayerKey {
    PlayerKey {
        account: account(),
        channel: Uuid::from_u128(2),
        message: Uuid::from_u128(0x100 + n),
        embed: Uuid::from_u128(0x200 + n),
    }
}

fn player() -> ProviderPlayer {
    ProviderPlayer::from_embed_address("https://www.youtube.com/embed/dQw4w9WgXcQ").unwrap()
}

fn opened(commands: &[Command]) -> Vec<PageId> {
    commands
        .iter()
        .filter_map(|command| match command {
            Command::Open { page, .. } => Some(*page),
            _ => None,
        })
        .collect()
}

fn closed(commands: &[Command]) -> Vec<PageId> {
    commands
        .iter()
        .filter_map(|command| match command {
            Command::Close(page) => Some(*page),
            _ => None,
        })
        .collect()
}

#[test]
fn a_player_loads_then_plays_on_its_first_frame() {
    let mut store = PlayerStore::default();
    let commands = store.start(key(1), player(), false, 3.0);
    let page = opened(&commands)[0];
    assert_eq!(store.phase(key(1)), Some(Phase::Loading { since_s: 3.0 }));
    assert!(store.painted(page));
    assert_eq!(store.phase(key(1)), Some(Phase::Playing));
    assert!(!store.painted(page), "only the first frame counts");
}

#[test]
fn the_ask_holds_no_page_and_cancelling_brings_the_poster_back() {
    let mut store = PlayerStore::default();
    store.ask(key(1), player());
    assert_eq!(store.phase(key(1)), Some(Phase::Asking));
    assert_eq!(store.page(key(1)), None);
    assert_eq!(store.close(key(1)), Vec::new());
    assert_eq!(store.phase(key(1)), None);
}

#[test]
fn a_third_player_releases_the_one_touched_longest_ago() {
    let mut store = PlayerStore::default();
    let first = opened(&store.start(key(1), player(), false, 0.0))[0];
    let second = opened(&store.start(key(2), player(), false, 0.0))[0];
    store.touch(key(1));
    let commands = store.start(key(3), player(), false, 0.0);
    assert_eq!(closed(&commands), vec![second]);
    assert_eq!(store.phase(key(2)), None, "its poster returns");
    assert_eq!(store.page(key(1)), Some(first));
    assert_eq!(store.live_pages().len(), MAX_LIVE);
}

#[test]
fn a_player_out_of_view_for_ten_seconds_stops_and_a_quick_pass_costs_nothing() {
    let mut store = PlayerStore::default();
    let page = opened(&store.start(key(1), player(), false, 0.0))[0];
    store.painted(page);
    let gone = HashMap::new();
    let half = HashMap::from([(key(1), 0.5)]);
    store.shown(&gone, 1.0);
    assert_eq!(store.release_due(5.0), (Vec::new(), Some(11.0)));
    store.shown(&half, 6.0);
    assert_eq!(store.release_due(12.0), (Vec::new(), None), "back in view");
    store.shown(&gone, 20.0);
    let (commands, next) = store.release_due(30.0);
    assert_eq!(closed(&commands), vec![page]);
    assert_eq!(next, None);
    assert_eq!(store.phase(key(1)), Some(Phase::Stopped));
    let again = store.restart(key(1), false, 31.0);
    assert_eq!(
        opened(&again).len(),
        1,
        "play starts again from the beginning"
    );
}

#[test]
fn the_viewer_keeps_its_player_in_view_wherever_its_row_is() {
    let mut store = PlayerStore::default();
    let page = opened(&store.start(key(1), player(), false, 0.0))[0];
    store.set_viewer(Some(key(1)));
    store.shown(&HashMap::new(), 1.0);
    assert_eq!(store.release_due(60.0), (Vec::new(), None));
    store.set_viewer(None);
    assert_eq!(
        store.page(key(1)),
        Some(page),
        "closing the viewer keeps it live"
    );
}

#[test]
fn a_failed_page_closes_and_shows_failed_until_retried() {
    let mut store = PlayerStore::default();
    let page = opened(&store.start(key(1), player(), false, 0.0))[0];
    assert_eq!(closed(&store.failed(page)), vec![page]);
    assert_eq!(store.phase(key(1)), Some(Phase::Failed));
    assert_eq!(store.page(key(1)), None);
    assert_eq!(opened(&store.restart(key(1), false, 1.0)).len(), 1);
}

#[test]
fn one_sound_at_a_time_among_players_and_with_other_media() {
    let mut store = PlayerStore::default();
    let first = opened(&store.start(key(1), player(), false, 0.0))[0];
    let second = opened(&store.start(key(2), player(), false, 0.0))[0];
    let change = store.sound(second, true);
    assert!(change.quiet_others, "other media pause");
    assert_eq!(change.commands, vec![Command::Mute(first, true)]);
    // A video's sound in the conversation: every player goes quiet.
    assert_eq!(store.quiet_all(), vec![Command::Mute(second, true)]);
    // A quieted player sounds again when used, and quiets the rest.
    let change = store.unquiet(key(1));
    assert!(change.quiet_others);
    assert_eq!(change.commands, vec![Command::Mute(first, false)]);
    // Its own sound starting doesn't quiet it.
    assert_eq!(store.sound(first, true).commands, Vec::new());
}

#[test]
fn switching_players_off_releases_every_one() {
    let mut store = PlayerStore::default();
    let first = opened(&store.start(key(1), player(), false, 0.0))[0];
    store.ask(key(2), player());
    let mut pages = closed(&store.release_all());
    pages.sort_unstable();
    assert_eq!(pages, vec![first]);
    assert_eq!(store.phase(key(2)), None);
}
