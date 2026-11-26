//! Filter suggestions and result pages, from synthetic rosters and hits.

use super::*;
use papo_sdk::{
    AccountId, Channel, DirectConversation, FilterValue, Generation, HasKind, ResourceSnapshot,
    SearchCursor, SearchDraft, SearchHit, SearchKeyword, SearchNames, SearchPage, SearchResult,
    SearchScope, ServerAddress, UserSummary, Uuid,
};

const OWN: u128 = 0;
const DM: u128 = 50;

fn id(n: u128) -> Uuid {
    Uuid::from_u128(0x2000 + n)
}

fn person(n: u128, username: &str, nickname: &str) -> UserSummary {
    UserSummary {
        id: Some(id(n).to_string()),
        username: Some(username.into()),
        nickname: nickname.into(),
        roles: Some(Vec::new()),
        ..UserSummary::default()
    }
}

fn channel(n: u128, name: &str, kind: &str) -> Channel {
    Channel {
        id: Some(id(n).to_string()),
        name: Some(name.into()),
        r#type: Some(kind.into()),
        permissions: Some(vec![]),
        ..Channel::default()
    }
}

/// You (@eu), Lia (@lia), Ana Lia (@ana), Rui (@rui); #geral, #dev chat, a category; a DM
/// with Rui.
fn snapshot() -> ResourceSnapshot {
    ResourceSnapshot::fixture(
        Generation::for_test(1),
        None,
        vec![
            channel(20, "geral", "text"),
            channel(21, "dev chat", "text"),
            channel(22, "general stuff", "category"),
        ],
        Vec::new(),
        vec![
            person(OWN, "eu", ""),
            person(1, "lia", "Lia"),
            person(2, "ana", "Ana Lia"),
            person(3, "rui", "Rui"),
        ],
        Vec::new(),
        vec![DirectConversation {
            id: id(DM).to_string(),
            user: person(3, "rui", "Rui"),
            created_at: "2026-01-01T00:00:00Z".into(),
            last_message: None,
            last_read_message: None,
            last_read_at: None,
            unread_count: 0,
        }],
        Vec::new(),
        id(OWN),
    )
    .unwrap()
}

fn bindings(suggestions: &[SearchSuggestion]) -> Vec<&str> {
    suggestions.iter().map(|s| s.binding.as_str()).collect()
}

#[test]
fn people_starting_with_the_value_come_before_people_containing_it() {
    let found = search_suggestions(&snapshot(), SearchKeyword::From, "@li", SearchScope::Server);
    assert_eq!(bindings(&found), ["@lia", "@ana"]);
    assert_eq!(found[1].name.as_deref(), Some("Ana Lia"));
    assert_eq!(found[1].value, SuggestedValue::Person(id(2)));
}

#[test]
fn a_direct_conversation_suggests_its_two_people_no_channels_and_no_mentions() {
    let scope = SearchScope::Direct {
        channel: id(DM),
        account_user: id(OWN),
    };
    let found = search_suggestions(&snapshot(), SearchKeyword::From, "", scope);
    assert_eq!(bindings(&found), ["@eu", "@rui"]);
    for keyword in [SearchKeyword::In, SearchKeyword::Mentions] {
        let found = search_suggestions(&snapshot(), keyword, "", scope);
        assert_eq!(bindings(&found), [""; 0]);
    }
}

#[test]
fn channels_leave_out_categories_and_a_name_with_spaces_binds_quoted() {
    let found = search_suggestions(&snapshot(), SearchKeyword::In, "#", SearchScope::Server);
    assert_eq!(bindings(&found), ["#geral", "\"#dev chat\""]);
}

#[test]
fn has_lists_the_kinds_the_backend_filters() {
    let found = search_suggestions(&snapshot(), SearchKeyword::Has, "", SearchScope::Server);
    let kinds: Vec<_> = found.iter().map(|s| s.value).collect();
    assert_eq!(
        kinds,
        [
            SuggestedValue::Has(HasKind::Link),
            SuggestedValue::Has(HasKind::File)
        ]
    );
    let found = search_suggestions(&snapshot(), SearchKeyword::Has, "fi", SearchScope::Server);
    assert_eq!(bindings(&found), ["file"]);
    // Portuguese spellings find their kind too.
    let found = search_suggestions(&snapshot(), SearchKeyword::Has, "arq", SearchScope::Server);
    assert_eq!(bindings(&found), ["file"]);
}

/// Every binding, written after its keyword, reads back as exactly its own value.
#[test]
fn every_binding_reads_back_as_its_own_value() {
    let snapshot = snapshot();
    let names = SearchNames {
        resources: &snapshot,
    };
    for (keyword, spelling) in [
        (SearchKeyword::From, "from:"),
        (SearchKeyword::In, "in:"),
        (SearchKeyword::Has, "has:"),
    ] {
        for suggestion in search_suggestions(&snapshot, keyword, "", SearchScope::Server) {
            let field = format!("{spelling}{}", suggestion.binding);
            let calendar = papo_sdk::SearchCalendar {
                today: local_day("2026-11-23"),
                order: papo_sdk::DateOrder::DayFirst,
                words: papo_sdk::DateWords::Portuguese,
            };
            let draft = SearchDraft::read(&field, &names, SearchScope::Server, calendar);
            let expected = match suggestion.value {
                SuggestedValue::Person(id) => FilterValue::Person(id),
                SuggestedValue::Channel(id) => FilterValue::Channel(id),
                SuggestedValue::Has(kind) => FilterValue::Has(kind),
            };
            assert_eq!(draft.filters[0].resolved, expected, "{field}");
        }
    }
}

/// Two handles differing only in case bind by UUID, so a pick never stays ambiguous.
#[test]
fn handles_equal_but_for_case_bind_by_id() {
    let snapshot = ResourceSnapshot::fixture(
        Generation::for_test(1),
        None,
        vec![
            channel(20, "General", "text"),
            channel(21, "general", "text"),
        ],
        Vec::new(),
        vec![
            person(OWN, "eu", ""),
            person(1, "Lia", ""),
            person(2, "lia", ""),
        ],
        Vec::new(),
        Vec::new(),
        Vec::new(),
        id(OWN),
    )
    .unwrap();
    let people = search_candidates(
        &snapshot,
        SearchKeyword::From,
        &[id(1), id(2)],
        SearchScope::Server,
    );
    assert_eq!(bindings(&people), [id(1).to_string(), id(2).to_string()]);
    let channels = search_suggestions(&snapshot, SearchKeyword::In, "gen", SearchScope::Server);
    assert_eq!(
        bindings(&channels),
        [id(20).to_string(), id(21).to_string()]
    );
}

#[test]
fn candidates_keep_their_order_and_drop_what_the_snapshot_lost() {
    let found = search_candidates(
        &snapshot(),
        SearchKeyword::From,
        &[id(2), id(9), id(1)],
        SearchScope::Server,
    );
    assert_eq!(bindings(&found), ["@ana", "@lia"]);
}

fn account() -> AccountId {
    AccountId::new(
        ServerAddress::parse("https://example.invalid")
            .unwrap()
            .id(),
        id(OWN),
    )
}

fn hit(n: u128, at: &str) -> SearchHit {
    SearchHit::fixture(
        account(),
        Generation::for_test(1),
        SearchResult {
            r#type: Some("message".into()),
            id: Some(id(100 + n).to_string()),
            content: Some(format!("**hit** {n}")),
            channel_id: Some(id(20).to_string()),
            channel_name: Some("geral".into()),
            author_id: Some(id(1).to_string()),
            author_username: Some("lia".into()),
            created_at: Some(at.into()),
            ..SearchResult::default()
        },
    )
}

fn page(hits: Vec<SearchHit>, more: bool) -> SearchPage {
    let next = more.then(|| SearchCursor {
        created_at: "2026-01-01T00:00:00Z".into(),
        id: id(100).to_string(),
    });
    SearchPage {
        account: account(),
        generation: Generation::for_test(1),
        hits,
        next,
    }
}

#[test]
fn the_count_is_a_floor_until_the_last_page_is_in() {
    let mut results = SearchResults::first(&page(
        vec![
            hit(2, "2026-01-02T00:00:00Z"),
            hit(1, "2026-01-01T00:00:00Z"),
        ],
        true,
    ));
    assert_eq!(results.count(), SearchCount::AtLeast(2));
    assert!(results.next().is_some());
    // A repeated hit is listed once; a malformed one not at all.
    let mut malformed = hit(3, "not a time");
    malformed = SearchHit::fixture(account(), Generation::for_test(1), {
        let mut result = (*malformed.result).clone();
        result.created_at = Some("yesterday".into());
        result
    });
    results.append(&page(
        vec![
            hit(1, "2026-01-01T00:00:00Z"),
            malformed,
            hit(0, "2025-12-31T00:00:00Z"),
        ],
        false,
    ));
    assert_eq!(results.count(), SearchCount::Exact(3));
    assert!(results.next().is_none());
    let first = &results.items()[0];
    assert_eq!(first.id(), id(102));
    assert_eq!(first.channel, id(20));
    assert_eq!(first.message.content.as_deref(), Some("**hit** 2"));
    assert!(first.message.attachments.is_none());
    assert!(!first.content.is_empty());
}

fn local_day(raw: &str) -> chrono::NaiveDate {
    chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d").unwrap()
}

fn instant_us(raw: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(raw)
        .unwrap()
        .timestamp_micros()
}

fn days_query(first: Option<&str>, last: Option<&str>) -> papo_sdk::SearchQuery {
    papo_sdk::SearchQuery {
        text: Some("openssl".into()),
        local_days: Some(papo_sdk::LocalDays {
            first: first.map(local_day),
            last: last.map(local_day),
        }),
        ..papo_sdk::SearchQuery::default()
    }
}

/// Brazil: three hours west of UTC.
fn brasilia(_at_us: i64) -> crate::UtcOffset {
    crate::UtcOffset::from_seconds_east(-3 * 3600).unwrap()
}

#[test]
fn a_local_day_asks_every_utc_day_it_touches_and_keeps_its_own_hours() {
    let (query, window) = localize_search(
        days_query(Some("2026-11-20"), Some("2026-11-20")),
        &brasilia,
    )
    .unwrap();
    assert_eq!(query.local_days, None);
    assert_eq!(query.date_start.as_deref(), Some("2026-11-20"));
    assert_eq!(query.date_end.as_deref(), Some("2026-11-21"));
    assert_eq!(window.start_us, Some(instant_us("2026-11-20T03:00:00Z")));
    assert_eq!(window.end_us, Some(instant_us("2026-11-21T03:00:00Z")));
    // 22:30 local on the 20th is the 21st in UTC, and is kept.
    assert!(window.holds(instant_us("2026-11-21T01:30:00Z")));
    assert!(!window.holds(instant_us("2026-11-20T02:59:59Z")));
}

#[test]
fn before_alone_ends_at_the_local_midnight_and_starts_nowhere() {
    let (query, window) = localize_search(days_query(None, Some("2026-11-19")), &brasilia).unwrap();
    assert_eq!(query.date_start, None);
    assert_eq!(query.date_end.as_deref(), Some("2026-11-20"));
    assert_eq!(window.start_us, None);
    assert_eq!(window.end_us, Some(instant_us("2026-11-20T03:00:00Z")));
}

#[test]
fn a_midnight_after_a_clock_change_takes_the_new_offset() {
    // UTC+1 until 2026-03-29T01:00Z, UTC+2 after: the 30th starts at
    // 22:00 UTC on the 29th.
    let change_us = instant_us("2026-03-29T01:00:00Z");
    let europe = move |at_us: i64| {
        let hours = if at_us < change_us { 1 } else { 2 };
        crate::UtcOffset::from_seconds_east(hours * 3600).unwrap()
    };
    let (_, window) = localize_search(days_query(Some("2026-03-30"), None), &europe).unwrap();
    assert_eq!(window.start_us, Some(instant_us("2026-03-29T22:00:00Z")));
}

#[test]
fn results_keep_to_the_window_and_want_the_next_page_when_one_kept_nothing() {
    let window = SearchWindow {
        start_us: Some(instant_us("2026-11-20T03:00:00Z")),
        end_us: Some(instant_us("2026-11-21T03:00:00Z")),
        oldest_first: false,
    };
    let results =
        SearchResults::first_within(&page(vec![hit(1, "2026-11-21T04:00:00Z")], true), window);
    assert!(results.wants_next(), "the whole page was past the window");
    let mut results = results;
    results.append(&page(
        vec![
            hit(2, "2026-11-21T01:30:00Z"),
            hit(3, "2026-11-20T02:00:00Z"),
        ],
        false,
    ));
    assert_eq!(results.count(), SearchCount::Exact(1));
    assert_eq!(results.items()[0].id(), id(102));
}

fn november_23() -> papo_sdk::SearchCalendar {
    papo_sdk::SearchCalendar {
        today: local_day("2026-11-23"),
        order: papo_sdk::DateOrder::DayFirst,
        words: papo_sdk::DateWords::Portuguese,
    }
}

fn date_bindings(typed: &str, calendar: papo_sdk::SearchCalendar) -> Vec<String> {
    search_date_suggestions(typed, calendar)
        .into_iter()
        .map(|suggestion| suggestion.binding)
        .collect()
}

#[test]
fn an_empty_date_offers_the_common_ones_in_the_readers_language() {
    assert_eq!(
        date_bindings("", november_23()),
        ["hoje", "ontem", "7d", "novembro", "outubro", "2026"]
    );
    let january = papo_sdk::SearchCalendar {
        today: local_day("2027-01-05"),
        words: papo_sdk::DateWords::English,
        ..november_23()
    };
    assert_eq!(
        date_bindings("", january),
        ["today", "yesterday", "7d", "january", "12/2026", "2027"],
        "last month of another year carries its year"
    );
}

#[test]
fn a_number_offers_each_unit_and_a_word_its_completions() {
    let found = search_date_suggestions("2", november_23());
    let bindings: Vec<&str> = found.iter().map(|s| s.binding.as_str()).collect();
    assert_eq!(bindings, ["2d", "2s", "2m", "2a"]);
    assert_eq!(
        found[0].date,
        papo_sdk::SearchDate::Day(local_day("2026-11-21"))
    );
    assert_eq!(date_bindings("no", november_23()), ["novembro"]);
    assert_eq!(
        date_bindings("21/11", november_23()),
        ["21/11"],
        "a read value shows itself"
    );
    assert_eq!(date_bindings("zz", november_23()), [""; 0]);
}

#[test]
fn the_first_page_starts_at_the_windows_end_and_a_hit_before_it_ends_the_list() {
    let (_, window) = localize_search(
        days_query(Some("2026-11-20"), Some("2026-11-20")),
        &brasilia,
    )
    .unwrap();
    let cursor = window.first_cursor().unwrap();
    assert_eq!(cursor.created_at, "2026-11-21T03:00:00.000000Z");
    assert_eq!(cursor.id, Uuid::nil().to_string());
    let mut results =
        SearchResults::first_within(&page(vec![hit(1, "2026-11-20T12:00:00Z")], true), window);
    results.append(&page(
        vec![
            hit(2, "2026-11-20T04:00:00Z"),
            hit(3, "2026-11-20T02:00:00Z"),
        ],
        true,
    ));
    assert_eq!(
        results.count(),
        SearchCount::Exact(2),
        "older than the window: the end"
    );
    assert!(results.next().is_none());
}

#[test]
fn a_midnight_the_clock_skips_starts_the_day_at_the_change() {
    // UTC−4 until 2026-09-06T04:00Z, when local 00:00 becomes 01:00 (UTC−3).
    let change_us = instant_us("2026-09-06T04:00:00Z");
    let chile = move |at_us: i64| {
        let hours = if at_us < change_us { -4 } else { -3 };
        crate::UtcOffset::from_seconds_east(hours * 3600).unwrap()
    };
    let (_, window) = localize_search(days_query(Some("2026-09-06"), None), &chile).unwrap();
    assert_eq!(window.start_us, Some(change_us));
}

#[test]
fn results_resolve_custom_emoji_and_follow_a_new_catalog() {
    let mut emoji_hit = (*hit(1, "2026-01-01T00:00:00Z").result).clone();
    emoji_hit.content = Some(":gato:".into());
    let emoji_hit = SearchHit::fixture(account(), Generation::for_test(1), emoji_hit);
    let mut results = SearchResults::first(&page(vec![emoji_hit], false));
    let resolved = |results: &SearchResults| {
        matches!(
            results.items()[0].content.blocks(),
            [crate::message_content::Block::Paragraph(inlines)]
                if matches!(inlines.as_slice(), [crate::message_content::Inline::CustomEmoji { .. }])
        )
    };
    assert!(!resolved(&results), "no catalog yet: the text stays");
    let catalog = papo_sdk::EmojiCatalog {
        revision: 2,
        emojis: vec![papo_sdk::CustomEmoji {
            id: id(900),
            name: "gato".into(),
            format: "png".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
        }],
    };
    results.set_custom_emojis(&crate::message_content::CustomEmojis::new(&catalog));
    assert!(resolved(&results));
}
