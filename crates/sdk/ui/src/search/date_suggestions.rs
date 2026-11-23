//! What a date filter's value can be, as a client lists it under the field
//! so nobody types a date blind: with nothing typed, today, yesterday, a
//! week ago, this month, last month and this year; after a number, that
//! many days, weeks, months or years ago; after a word, the days and
//! months it starts; a value already read, itself, so its meaning shows.
//! Each comes with the date it means, for the client to name, and the text
//! that binds it, in the calendar's language.
//!
//! Reading dates is `papo_sdk::parse_search_date`'s; the list's look is the
//! client's.

use chrono::{Datelike, Months};
use papo_sdk::{DateWords, SearchCalendar, SearchDate, parse_search_date};

/// One value a date filter can take.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DateSuggestion {
    /// What it means.
    pub date: SearchDate,
    /// The value to write after the keyword's colon; it reads back as `date`.
    pub binding: String,
}

/// The date values `typed` (the value after a date keyword's colon) leads
/// to in `calendar`.
#[must_use]
pub fn search_date_suggestions(typed: &str, calendar: SearchCalendar) -> Vec<DateSuggestion> {
    let typed = typed.trim();
    let lowered = typed.to_lowercase();
    let bindings: Vec<String> = if lowered.is_empty() {
        defaults(calendar)
    } else if lowered.len() <= 3 && lowered.chars().all(|c| c.is_ascii_digit()) {
        let units = match calendar.words {
            DateWords::English => ["d", "w", "m", "y"],
            DateWords::Portuguese => ["d", "s", "m", "a"],
        };
        units
            .iter()
            .map(|unit| format!("{lowered}{unit}"))
            .collect()
    } else if parse_search_date(typed, calendar).is_some() {
        vec![typed.to_owned()]
    } else {
        words(calendar)
            .into_iter()
            .filter(|word| word.starts_with(&lowered))
            .collect()
    };
    let mut suggestions: Vec<DateSuggestion> = Vec::new();
    for binding in bindings {
        let Some(date) = parse_search_date(&binding, calendar) else {
            continue;
        };
        if suggestions.iter().all(|known| known.binding != binding) {
            suggestions.push(DateSuggestion { date, binding });
        }
    }
    suggestions
}

/// Today, yesterday, a week ago, this month, last month, this year.
fn defaults(calendar: SearchCalendar) -> Vec<String> {
    let today = calendar.today;
    let (now, before) = match calendar.words {
        DateWords::English => ("today", "yesterday"),
        DateWords::Portuguese => ("hoje", "ontem"),
    };
    let mut bindings = vec![now.to_owned(), before.to_owned(), "7d".to_owned()];
    bindings.push(month_word(today.month(), calendar.words).to_owned());
    if let Some(last) = today.checked_sub_months(Months::new(1)) {
        // A month of another year is written with its year.
        if last.year() == today.year() {
            bindings.push(month_word(last.month(), calendar.words).to_owned());
        } else {
            bindings.push(format!("{:02}/{}", last.month(), last.year()));
        }
    }
    bindings.push(today.year().to_string());
    bindings
}

/// The words a typed prefix can complete: the two days, then the months.
fn words(calendar: SearchCalendar) -> Vec<String> {
    let days = match calendar.words {
        DateWords::English => ["today", "yesterday"],
        DateWords::Portuguese => ["hoje", "ontem"],
    };
    days.iter()
        .map(|word| (*word).to_owned())
        .chain((1..=12).map(|month| month_word(month, calendar.words).to_owned()))
        .collect()
}

/// Month `month`'s name in `words`, lowercase, as a value is written.
fn month_word(month: u32, words: DateWords) -> &'static str {
    const ENGLISH: [&str; 12] = [
        "january",
        "february",
        "march",
        "april",
        "may",
        "june",
        "july",
        "august",
        "september",
        "october",
        "november",
        "december",
    ];
    const PORTUGUESE: [&str; 12] = [
        "janeiro",
        "fevereiro",
        "março",
        "abril",
        "maio",
        "junho",
        "julho",
        "agosto",
        "setembro",
        "outubro",
        "novembro",
        "dezembro",
    ];
    let names = match words {
        DateWords::English => &ENGLISH,
        DateWords::Portuguese => &PORTUGUESE,
    };
    let index = usize::try_from(month.clamp(1, 12) - 1).unwrap_or(0);
    names[index]
}
