//! Reading the search field: ranges, name resolution, scopes and the problems that hold a search.

use super::*;
use crate::{Generation, ResourceSnapshot};
use papo_api::wire::{Channel, DirectConversation, UserSummary};

const OWN: u128 = 7;
const DM: u128 = 50;

fn person(id: u128, username: &str, nickname: &str) -> UserSummary {
    UserSummary {
        id: Some(Uuid::from_u128(id).to_string()),
        username: Some(username.into()),
        nickname: nickname.into(),
        ..Default::default()
    }
}

fn channel(id: u128, name: &str, kind: &str) -> Channel {
    Channel {
        id: Some(Uuid::from_u128(id).to_string()),
        name: Some(name.into()),
        r#type: Some(kind.into()),
        permissions: Some(vec![]),
        ..Default::default()
    }
}

/// Lia (@lia), Lia Moreira (@liamoreira), Rui (@rui); #dev, #devops, a category "devices"; a
/// DM with Rui.
fn resources() -> ResourceSnapshot {
    ResourceSnapshot::fixture(
        Generation(1),
        None,
        vec![
            channel(20, "dev", "text"),
            channel(21, "devops", "text"),
            channel(22, "devices", "category"),
        ],
        vec![],
        vec![
            person(1, "lia", "Lia"),
            person(2, "liamoreira", "Lia Moreira"),
            person(3, "rui", "Rui"),
        ],
        vec![],
        vec![DirectConversation {
            id: Uuid::from_u128(DM).to_string(),
            user: person(3, "rui", "Rui"),
            created_at: "2026-01-01T00:00:00Z".into(),
            last_message: None,
            last_read_message: None,
            last_read_at: None,
            unread_count: 0,
        }],
        vec![],
        Uuid::from_u128(OWN),
    )
    .unwrap()
}

fn read(raw: &str, scope: SearchScope) -> SearchDraft {
    let resources = resources();
    SearchDraft::read(
        raw,
        &SearchNames {
            resources: &resources,
        },
        scope,
    )
}

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

fn direct() -> SearchScope {
    SearchScope::Direct {
        channel: id(DM),
        account_user: id(OWN),
    }
}

#[test]
fn filters_and_words_keep_their_byte_ranges() {
    let draft = read("from:lia olá \"two words\" in:dev", SearchScope::Server);
    let from = &draft.filters[0];
    assert_eq!(from.keyword, SearchKeyword::From);
    assert_eq!(
        (from.keyword_range.clone(), from.value_range.clone()),
        (0..5, 5..8)
    );
    assert_eq!(draft.words, [9..13, 14..25]);
    assert_eq!(draft.filters[1].keyword_range, 26..29);
    assert_eq!(draft.filters[1].resolved, FilterValue::Channel(id(20)));
}

#[test]
fn a_name_prefix_matching_two_people_waits_for_a_pick_and_a_handle_binds() {
    let draft = read("from:lia", SearchScope::Server);
    assert_eq!(
        draft.filters[0].resolved,
        FilterValue::Ambiguous(vec![id(1), id(2)])
    );
    assert_eq!(draft.query(), Err(&SearchProblem::Unresolved(0)));

    let draft = read("from:@lia", SearchScope::Server);
    assert_eq!(draft.filters[0].resolved, FilterValue::Person(id(1)));
    assert_eq!(draft.query().unwrap().author, Some(id(1)));

    let draft = read("de:\"lia m\"", SearchScope::Server);
    assert_eq!(draft.filters[0].resolved, FilterValue::Person(id(2)));
}

#[test]
fn an_exact_channel_name_wins_over_longer_ones_and_categories_are_not_searched() {
    assert_eq!(
        read("in:dev", SearchScope::Server).filters[0].resolved,
        FilterValue::Channel(id(20))
    );
    assert_eq!(
        read("em:de", SearchScope::Server).filters[0].resolved,
        FilterValue::Ambiguous(vec![id(20), id(21)])
    );
    assert_eq!(
        read("in:#devi", SearchScope::Server).filters[0].resolved,
        FilterValue::Unknown
    );
}

#[test]
fn a_typed_channel_replaces_the_channel_scope() {
    let scope = SearchScope::Channel(id(21));
    assert_eq!(read("olá", scope).query().unwrap().channel, Some(id(21)));
    assert_eq!(
        read("in:dev olá", scope).query().unwrap().channel,
        Some(id(20))
    );
    assert_eq!(
        read("olá", SearchScope::Server).query().unwrap().channel,
        None
    );
}

#[test]
fn a_direct_conversation_names_only_its_two_people_and_takes_no_channel_or_mention() {
    let draft = read("from:li", direct());
    assert_eq!(draft.filters[0].resolved, FilterValue::Unknown);
    let draft = read("from:r in:dev", direct());
    assert_eq!(draft.filters[0].resolved, FilterValue::Person(id(3)));
    assert_eq!(draft.filters[1].resolved, FilterValue::Unavailable);
    assert_eq!(
        read("mentions:r", direct()).filters[0].resolved,
        FilterValue::Unavailable
    );
    let draft = read("from:r", direct());
    assert_eq!(draft.query().unwrap().channel, Some(id(DM)));
}

#[test]
fn nothing_typed_is_no_search_even_within_a_conversation() {
    assert_eq!(read("  ", direct()).query(), Err(&SearchProblem::Empty));
    assert_eq!(
        read("", SearchScope::Channel(id(20))).query(),
        Err(&SearchProblem::Empty)
    );
}

#[test]
fn a_repeated_keyword_an_unknown_kind_and_an_open_quote_hold_the_search() {
    assert_eq!(
        read("from:@lia from:@rui", SearchScope::Server).query(),
        Err(&SearchProblem::Repeated(1))
    );
    let draft = read("has:link tem:arquivo", SearchScope::Server);
    let query = draft.query().unwrap();
    assert!(query.has_link);
    assert_eq!(query.contains_attachment, Some(true));
    assert_eq!(
        read("has:image", SearchScope::Server).filters[0].resolved,
        FilterValue::Unknown
    );
    assert_eq!(
        read("\"open words", SearchScope::Server).query(),
        Err(&SearchProblem::UnclosedQuote)
    );
}

#[test]
fn the_filter_at_the_caret_includes_an_empty_value_and_a_link_is_a_word() {
    let draft = read("olá from: https://x.y", SearchScope::Server);
    assert_eq!(draft.filters.len(), 1);
    assert_eq!(draft.filters[0].resolved, FilterValue::Empty);
    assert_eq!(
        draft.filter_at(10).map(|f| f.keyword),
        Some(SearchKeyword::From)
    );
    assert!(draft.filter_at(3).is_none());
    assert_eq!(draft.words, [0..4, 11..22]);
}

#[test]
fn a_lone_at_or_hash_is_still_being_typed_and_a_long_field_is_too_long() {
    assert_eq!(
        read("from:@", SearchScope::Server).filters[0].resolved,
        FilterValue::Empty
    );
    assert_eq!(
        read("in:#", SearchScope::Server).filters[0].resolved,
        FilterValue::Empty
    );
    let long = "a".repeat(MAX_SEARCH_BYTES + 1);
    assert_eq!(
        read(&long, SearchScope::Server).query(),
        Err(&SearchProblem::TooLong)
    );
}

fn day(raw: &str) -> chrono::NaiveDate {
    chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d").unwrap()
}

#[test]
fn date_filters_are_days_of_the_readers_calendar() {
    let draft = read("antes:2026-11-20 after:2026-11-10", SearchScope::Server);
    let days = draft.query().unwrap().local_days.unwrap();
    assert_eq!(days.first, Some(day("2026-11-11")));
    assert_eq!(days.last, Some(day("2026-11-19")));
    let draft = read("durante:2026-11-20", SearchScope::Server);
    let days = draft.query().unwrap().local_days.unwrap();
    assert_eq!(
        (days.first, days.last),
        (Some(day("2026-11-20")), Some(day("2026-11-20")))
    );
}

#[test]
fn a_repeated_or_empty_date_range_and_a_bad_date_hold_the_search() {
    assert_eq!(
        read("during:2026-11-20 before:2026-11-25", SearchScope::Server).query(),
        Err(&SearchProblem::Repeated(1))
    );
    assert_eq!(
        read("after:2026-11-20 before:2026-11-21", SearchScope::Server).query(),
        Err(&SearchProblem::NoDays)
    );
    assert_eq!(
        read("before:20/11/2026", SearchScope::Server).filters[0].resolved,
        FilterValue::Unknown
    );
}

#[test]
fn local_days_never_reach_the_backend_unconverted() {
    let query = read("during:2026-11-20", SearchScope::Server)
        .query()
        .unwrap()
        .clone();
    assert!(query.into_request().is_err());
}
