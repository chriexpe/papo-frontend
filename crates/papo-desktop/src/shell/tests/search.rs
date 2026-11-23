//! The search panel (`docs/design.md` §10, Search and `PanelPeek`),
//! headless: Ctrl+F opens it with the keyboard in its field and the
//! filters listed, a keyword lists its values and a pick binds one, the
//! text is searched once it rests, results show their count and channel
//! lines, input outside shrinks or folds it, one header panel is open at a
//! time, and a search goes with the conversation it does not belong to.

use std::sync::Arc;

use papo_sdk::{
    AccountId, Channel, FilterValue, Generation, ResourceSnapshot, SearchCursor, SearchError,
    SearchHit, SearchPage, SearchResult, UserSummary, Uuid,
};
use papo_sdk_ui::SearchResults;
use rune::{Key, KeyEvent, Mods, Runtime};

use super::navigation::focused;
use super::replying::{conversation, lia};
use super::{FRAME_S, Shell, press, type_text};
use crate::shell::search::{Found, Paging, SEARCH_FIELD_KEY, Scope, Showing};
use crate::shell::search_run::RUN_AFTER_S;
use crate::timeline::TimelineKey;

fn ctrl(rt: &mut Runtime<Shell>, letter: char) {
    rt.key(KeyEvent {
        key: Key::Command(letter),
        mods: Mods {
            cmd: true,
            ctrl: true,
            ..Mods::default()
        },
    });
    rt.frame(FRAME_S);
}

fn shows(rt: &Runtime<Shell>, words: &str) -> bool {
    rt.elements().iter().any(|element| {
        element.text.as_deref() == Some(words) || element.label.as_deref() == Some(words)
    })
}

fn text(rt: &Runtime<Shell>) -> String {
    rt.state.search.as_ref().unwrap().edit.text.clone()
}

fn showing(rt: &Runtime<Shell>) -> Option<Showing> {
    rt.state.search.as_ref().map(|search| search.showing)
}

fn key(rt: &Runtime<Shell>) -> TimelineKey {
    rt.state.tabs.shown().unwrap().key
}

/// Results for the field's current text: `hits` (number, channel name),
/// with more pages when `more`.
fn results(rt: &mut Runtime<Shell>, hits: &[(u128, &str)], more: bool) {
    let key = key(rt);
    let hits: Vec<SearchHit> = hits
        .iter()
        .map(|(n, channel)| {
            SearchHit::fixture(
                key.account,
                Generation::for_test(1),
                SearchResult {
                    r#type: Some("message".into()),
                    id: Some(Uuid::from_u128(0x5000 + n).to_string()),
                    content: Some(format!("hit {n}")),
                    channel_id: Some(Uuid::from_u128(0x6000 + n).to_string()),
                    channel_name: Some((*channel).to_owned()),
                    author_id: Some(lia().to_string()),
                    author_username: Some("lia".into()),
                    created_at: Some(format!("2026-11-{:02}T10:00:00Z", 20 - n)),
                    ..SearchResult::default()
                },
            )
        })
        .collect();
    let page = SearchPage {
        account: key.account,
        generation: Generation::for_test(1),
        hits,
        next: more.then(|| SearchCursor {
            created_at: "2026-11-01T10:00:00Z".into(),
            id: Uuid::from_u128(0x5000).to_string(),
        }),
    };
    rt.update(move |shell, _tx| {
        let asked = shell
            .search_draft()
            .and_then(|draft| draft.query().ok().cloned());
        let search = shell.search.as_mut().unwrap();
        search.run_due_s = None;
        search.asked = asked;
        search.found = Found::Ready(SearchResults::first(&page));
    });
    rt.frame(FRAME_S);
}

#[test]
fn ctrl_f_opens_the_panel_with_the_keyboard_in_its_field_and_the_filters_listed() {
    let mut rt = conversation("");
    ctrl(&mut rt, 'f');
    assert_eq!(showing(&rt), Some(Showing::Open));
    assert_eq!(focused(&rt).as_deref(), Some(SEARCH_FIELD_KEY));
    assert!(shows(&rt, "Filters"));
    for keyword in ["from:", "in:", "has:", "mentions:"] {
        assert!(shows(&rt, keyword), "{keyword} is listed");
    }
    assert!(
        shows(&rt, "link, file"),
        "has: names what the backend filters"
    );
}

#[test]
fn a_keyword_lists_its_values_and_enter_binds_the_pick() {
    let mut rt = conversation("");
    ctrl(&mut rt, 'f');
    // The caret counts UTF-8 bytes: "olá" is four.
    type_text(&mut rt, "olá from:li");
    assert!(shows(&rt, "lia"), "Lia is offered");
    press(&mut rt, Key::Enter);
    assert_eq!(text(&rt), "olá from:@lia ");
    let draft = rt.state.search_draft().unwrap();
    assert_eq!(draft.filters[0].resolved, FilterValue::Person(lia()));
}

#[test]
fn the_text_is_searched_once_it_rests() {
    let mut rt = conversation("");
    rt.poll = Some(Shell::poll);
    ctrl(&mut rt, 'f');
    type_text(&mut rt, "kubernetes");
    assert!(matches!(
        rt.state.search.as_ref().unwrap().found,
        Found::Nothing
    ));
    rt.frame(RUN_AFTER_S);
    assert!(
        matches!(rt.state.search.as_ref().unwrap().found, Found::Loading),
        "searched once the text rested"
    );
}

#[test]
fn results_show_their_count_a_floor_while_more_pages_exist_and_channel_lines() {
    let mut rt = conversation("");
    ctrl(&mut rt, 'f');
    type_text(&mut rt, "hit");
    results(
        &mut rt,
        &[(1, "general"), (2, "general"), (3, "random")],
        true,
    );
    assert!(shows(&rt, "3+ results"));
    assert!(
        shows(&rt, "This server"),
        "a server search offers the scope"
    );
    assert!(shows(&rt, "#general"));
    assert!(shows(&rt, "#random"));
    results(&mut rt, &[(1, "general")], false);
    assert!(shows(&rt, "1 result"));
}

#[test]
fn nothing_found_says_so_and_offers_to_clear_the_filters() {
    let mut rt = conversation("");
    ctrl(&mut rt, 'f');
    type_text(&mut rt, "from:@lia kubernetes");
    results(&mut rt, &[], false);
    assert!(shows(&rt, "No results for “from:@lia kubernetes”"));
    assert!(shows(&rt, "Clear filters"));
    rt.update(Shell::search_clear_filters);
    rt.frame(FRAME_S);
    assert_eq!(text(&rt), "kubernetes");
}

#[test]
fn with_a_result_opened_work_outside_shrinks_it_and_ctrl_f_stretches_it() {
    let mut rt = conversation("");
    ctrl(&mut rt, 'f');
    type_text(&mut rt, "hit");
    results(&mut rt, &[(1, "general")], false);
    rt.update(|shell, _tx| {
        shell.search.as_mut().unwrap().opened = Some(Uuid::from_u128(0x5001));
    });
    rt.update(|shell, _tx| {
        shell.worked_outside();
    });
    rt.frame(FRAME_S);
    assert_eq!(showing(&rt), Some(Showing::Peek));
    assert!(
        shows(&rt, "hit 1"),
        "the shrunk panel keeps the opened result"
    );
    ctrl(&mut rt, 'f');
    assert_eq!(showing(&rt), Some(Showing::Open));
}

#[test]
fn with_nothing_opened_work_outside_folds_it_keeping_the_query() {
    let mut rt = conversation("");
    ctrl(&mut rt, 'f');
    type_text(&mut rt, "kubernetes");
    rt.update(|shell, _tx| {
        shell.worked_outside();
    });
    rt.frame(FRAME_S);
    assert_eq!(showing(&rt), Some(Showing::Folded));
    assert!(rt.state.shown_search().is_none());
    ctrl(&mut rt, 'f');
    assert_eq!(showing(&rt), Some(Showing::Open));
    assert_eq!(text(&rt), "kubernetes");
}

#[test]
fn escape_folds_it_and_gives_the_keyboard_back() {
    let mut rt = conversation("");
    let before = focused(&rt);
    ctrl(&mut rt, 'f');
    press(&mut rt, Key::Escape);
    assert_eq!(showing(&rt), Some(Showing::Folded));
    assert_eq!(focused(&rt), before);
}

#[test]
fn one_header_panel_is_open_at_a_time() {
    let mut rt = conversation("");
    ctrl(&mut rt, 'f');
    ctrl(&mut rt, 'p');
    assert!(rt.state.shown_pins_panel().is_some());
    assert_eq!(showing(&rt), Some(Showing::Folded));
    ctrl(&mut rt, 'f');
    assert!(rt.state.shown_pins_panel().is_none());
    assert_eq!(showing(&rt), Some(Showing::Open));
}

/// Shows another channel of the same account, as moving there would.
fn show_other_channel(rt: &mut Runtime<Shell>, account: AccountId) {
    rt.update(move |shell, _tx| {
        let other = TimelineKey {
            account,
            channel: Uuid::from_u128(0x7777),
        };
        shell.tabs.show(Some(other), false);
        shell.conversation_left();
    });
    rt.frame(FRAME_S);
}

#[test]
fn a_this_channel_search_goes_when_another_channel_shows_a_server_one_stays() {
    let mut rt = conversation("");
    let account = key(&rt).account;
    ctrl(&mut rt, 'f');
    rt.update(|shell, _tx| shell.search.as_mut().unwrap().scope = Scope::Channel);
    show_other_channel(&mut rt, account);
    assert!(rt.state.search.is_none());

    let mut rt = conversation("");
    ctrl(&mut rt, 'f');
    show_other_channel(&mut rt, account);
    assert!(rt.state.search.is_some(), "a server search stays");
}

#[test]
fn a_search_goes_when_another_account_shows() {
    let mut rt = conversation("");
    ctrl(&mut rt, 'f');
    let other = AccountId::new(key(&rt).account.server(), Uuid::from_u128(0x9999));
    show_other_channel(&mut rt, other);
    assert!(rt.state.search.is_none());
}

#[test]
fn an_ambiguous_name_asks_which_and_a_pick_binds_it() {
    let mut rt = conversation("");
    rt.update(|shell, _tx| {
        let open = shell.tabs.shown_mut().unwrap();
        open.resources = Some(Arc::new(two_lias(open.key)));
    });
    ctrl(&mut rt, 'f');
    type_text(&mut rt, "from:lia olá");
    assert!(shows(&rt, "Which lia?"));
    assert!(shows(&rt, "Lia Moreira"));
    press(&mut rt, Key::Down);
    press(&mut rt, Key::Enter);
    let draft = rt.state.search_draft().unwrap();
    assert_eq!(
        draft.filters[0].resolved,
        FilterValue::Person(Uuid::from_u128(0x2002))
    );
}

/// The account, Lia (@lia) and Lia Moreira (@liamoreira).
fn two_lias(key: TimelineKey) -> ResourceSnapshot {
    let person = |id: Uuid, username: &str, nickname: &str| UserSummary {
        id: Some(id.to_string()),
        username: Some(username.to_owned()),
        nickname: nickname.into(),
        roles: Some(Vec::new()),
        ..UserSummary::default()
    };
    ResourceSnapshot::fixture(
        Generation::for_test(1),
        None,
        vec![Channel {
            id: Some(key.channel.to_string()),
            name: Some("general".to_owned()),
            r#type: Some("text".to_owned()),
            permissions: Some(Vec::new()),
            ..Channel::default()
        }],
        Vec::new(),
        vec![
            person(key.account.user(), "me", ""),
            person(lia(), "lia", "Lia"),
            person(Uuid::from_u128(0x2002), "liamoreira", "Lia Moreira"),
        ],
        Vec::new(),
        Vec::new(),
        Vec::new(),
        key.account.user(),
    )
    .unwrap()
}

#[test]
fn a_kind_picked_after_its_keyword_is_followed_by_a_space() {
    let mut rt = conversation("");
    ctrl(&mut rt, 'f');
    press(&mut rt, Key::Down);
    press(&mut rt, Key::Down);
    press(&mut rt, Key::Enter);
    assert_eq!(text(&rt), "has:", "the filter list's third is has:");
    press(&mut rt, Key::Enter);
    assert_eq!(text(&rt), "has:link ");
    type_text(&mut rt, "openssl");
    assert_eq!(text(&rt), "has:link openssl");
}

#[test]
fn tab_and_a_press_pick_as_enter_does() {
    let mut rt = conversation("");
    ctrl(&mut rt, 'f');
    type_text(&mut rt, "has:");
    press(&mut rt, Key::Tab);
    assert_eq!(text(&rt), "has:link ");
    rt.update(Shell::clear_search);
    rt.frame(FRAME_S);
    type_text(&mut rt, "has:");
    rt.update(|shell, tx| shell.search_picked(1, tx));
    rt.frame(FRAME_S);
    assert_eq!(text(&rt), "has:file ");
}

#[test]
fn a_value_typed_out_in_full_takes_its_space_on_enter() {
    let mut rt = conversation("");
    ctrl(&mut rt, 'f');
    type_text(&mut rt, "has:link");
    press(&mut rt, Key::Enter);
    assert_eq!(text(&rt), "has:link ");
}

/// Types `words` and lets them rest: a search is asked (with no SDK, it
/// never answers). Returns its number.
fn asked(rt: &mut Runtime<Shell>, words: &str) -> u64 {
    type_text(rt, words);
    rt.frame(RUN_AFTER_S);
    rt.state.search.as_ref().unwrap().revision
}

fn empty_page(rt: &Runtime<Shell>) -> SearchPage {
    SearchPage {
        account: key(rt).account,
        generation: Generation::for_test(1),
        hits: Vec::new(),
        next: None,
    }
}

#[test]
fn an_answer_for_a_replaced_or_dropped_search_never_lands() {
    let mut rt = conversation("");
    rt.poll = Some(Shell::poll);
    ctrl(&mut rt, 'f');
    let first = asked(&mut rt, "open");
    let second = asked(&mut rt, "ssl");
    assert_ne!(first, second);
    let page = empty_page(&rt);
    rt.update(move |shell, _tx| {
        assert!(!shell.search_answered(first, Ok(page)));
    });
    assert!(matches!(
        rt.state.search.as_ref().unwrap().found,
        Found::Loading
    ));
    // Dropped with its account: a new search never takes its answer.
    let other = AccountId::new(key(&rt).account.server(), Uuid::from_u128(0x9999));
    show_other_channel(&mut rt, other);
    assert!(rt.state.search.is_none());
    let page = empty_page(&rt);
    rt.update(move |shell, _tx| {
        assert!(!shell.search_answered(second, Ok(page)));
    });
}

#[test]
fn a_current_search_the_sdk_cancelled_says_it_failed_instead_of_waiting() {
    let mut rt = conversation("");
    rt.poll = Some(Shell::poll);
    ctrl(&mut rt, 'f');
    let revision = asked(&mut rt, "open");
    rt.update(move |shell, _tx| {
        shell.search_answered(revision, Err(SearchError::Canceled));
    });
    rt.frame(FRAME_S);
    assert!(matches!(
        rt.state.search.as_ref().unwrap().found,
        Found::Failed
    ));
    assert!(shows(&rt, "Try again"));
}

#[test]
fn the_results_stay_while_the_next_search_loads() {
    let mut rt = conversation("");
    rt.poll = Some(Shell::poll);
    ctrl(&mut rt, 'f');
    type_text(&mut rt, "hit");
    results(&mut rt, &[(1, "general")], false);
    asked(&mut rt, " more");
    let search = rt.state.search.as_ref().unwrap();
    assert!(matches!(search.found, Found::Ready(_)));
    assert!(search.loading());
    assert!(
        shows(&rt, "hit 1"),
        "the panel never empties between searches"
    );
}

#[test]
fn a_failed_page_waits_for_its_try_again() {
    let mut rt = conversation("");
    rt.poll = Some(Shell::poll);
    ctrl(&mut rt, 'f');
    let revision = asked(&mut rt, "hit");
    results(&mut rt, &[(1, "general")], true);
    rt.update(move |shell, _tx| {
        let search = shell.search.as_mut().unwrap();
        search.answered = revision;
        search.paging = Paging::Loading;
        shell.search_paged(revision, Err(SearchError::InvalidData));
    });
    rt.frame(FRAME_S);
    assert_eq!(rt.state.search.as_ref().unwrap().paging, Paging::Failed);
    assert!(shows(&rt, "Try again"));
    rt.update(|shell, _tx| {
        assert!(!shell.search_rows_visible(0..1), "no page asked on its own");
    });
}

#[test]
fn another_conversation_showing_cancels_an_opening_on_its_way() {
    let mut rt = conversation("");
    let account = key(&rt).account;
    let before = rt.state.openings;
    show_other_channel(&mut rt, account);
    assert_ne!(rt.state.openings, before);
}
