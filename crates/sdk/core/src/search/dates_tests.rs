//! Reading dates as people write them, against a fixed today.

use super::*;

fn day(raw: &str) -> NaiveDate {
    NaiveDate::parse_from_str(raw, "%Y-%m-%d").unwrap()
}

/// Sunday, 23 November 2026, written day first in Portuguese.
fn brazil() -> SearchCalendar {
    SearchCalendar {
        today: day("2026-11-23"),
        order: DateOrder::DayFirst,
        words: DateWords::Portuguese,
    }
}

fn read(raw: &str) -> Option<SearchDate> {
    parse_search_date(raw, brazil())
}

#[test]
fn relative_counts_go_back_from_today() {
    assert_eq!(read("2d"), Some(SearchDate::Day(day("2026-11-21"))));
    assert_eq!(read("1w"), Some(SearchDate::Day(day("2026-11-16"))));
    assert_eq!(read("1s"), read("1w"), "semana");
    assert_eq!(read("1m"), Some(SearchDate::Day(day("2026-10-23"))));
    assert_eq!(read("1a"), Some(SearchDate::Day(day("2025-11-23"))), "ano");
    assert_eq!(read("10000d"), None);
}

#[test]
fn words_in_either_language() {
    assert_eq!(read("Hoje"), Some(SearchDate::Day(day("2026-11-23"))));
    assert_eq!(read("yesterday"), Some(SearchDate::Day(day("2026-11-22"))));
}

#[test]
fn days_in_the_calendars_order_with_or_without_separators() {
    let expected = Some(SearchDate::Day(day("2026-11-21")));
    for raw in [
        "21/11/2026",
        "21/11/26",
        "21-11-2026",
        "21.11.26",
        "21112026",
        "211126",
        "2026-11-21",
        "20261121",
    ] {
        assert_eq!(read(raw), expected, "{raw}");
    }
    let us = SearchCalendar {
        order: DateOrder::MonthFirst,
        words: DateWords::English,
        ..brazil()
    };
    assert_eq!(parse_search_date("11/21/2026", us), expected);
    assert_eq!(read("31/02/2026"), None);
}

#[test]
fn a_day_or_month_without_a_year_is_the_most_recent_one_not_after_today() {
    assert_eq!(read("21/11"), Some(SearchDate::Day(day("2026-11-21"))));
    assert_eq!(read("25/12"), Some(SearchDate::Day(day("2025-12-25"))));
    assert_eq!(
        read("nov"),
        Some(SearchDate::Month {
            year: 2026,
            month: 11
        })
    );
    assert_eq!(
        read("dezembro"),
        Some(SearchDate::Month {
            year: 2025,
            month: 12
        })
    );
}

#[test]
fn months_and_years() {
    let november = Some(SearchDate::Month {
        year: 2026,
        month: 11,
    });
    for raw in [
        "novembro2026",
        "nov26",
        "nov-26",
        "nov/2026",
        "11/2026",
        "2026-11",
        "November",
    ] {
        assert_eq!(read(raw), november, "{raw}");
    }
    assert_eq!(
        read("março"),
        Some(SearchDate::Month {
            year: 2026,
            month: 3
        })
    );
    assert_eq!(read("2025"), Some(SearchDate::Year(2025)));
    assert_eq!(SearchDate::Year(2025).last_day(), Some(day("2025-12-31")));
    assert_eq!(
        SearchDate::Month {
            year: 2024,
            month: 2
        }
        .last_day(),
        Some(day("2024-02-29"))
    );
}

#[test]
fn what_is_no_date_reads_as_none() {
    for raw in ["", "2", "soon", "21/11/2026/1", "1x", "nov2026x"] {
        assert_eq!(read(raw), None, "{raw}");
    }
}

#[test]
fn eight_digits_prefer_the_reading_with_a_plausible_year() {
    assert_eq!(read("20101115"), Some(SearchDate::Day(day("2010-11-15"))));
    assert_eq!(read("21112026"), Some(SearchDate::Day(day("2026-11-21"))));
    let us = SearchCalendar {
        order: DateOrder::MonthFirst,
        ..brazil()
    };
    assert_eq!(
        parse_search_date("112126", us),
        Some(SearchDate::Day(day("2026-11-21")))
    );
}

#[test]
fn two_digit_years_are_the_most_recent_and_far_years_are_refused() {
    assert_eq!(read("01/01/99"), Some(SearchDate::Day(day("1999-01-01"))));
    assert_eq!(
        read("nov26"),
        Some(SearchDate::Month {
            year: 2026,
            month: 11
        })
    );
    assert_eq!(read("9999y"), None);
    assert_eq!(read("+2d"), None);
    assert_eq!(
        read("sept"),
        Some(SearchDate::Month {
            year: 2026,
            month: 9
        })
    );
}

#[test]
fn february_29_with_no_year_is_the_most_recent_leap_day() {
    assert_eq!(read("29/02"), Some(SearchDate::Day(day("2024-02-29"))));
}
