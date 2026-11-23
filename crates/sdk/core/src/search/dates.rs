//! The dates a search's date filters take (`before:`, `after:`, `during:`),
//! read the way people write them, against the reader's own calendar:
//!
//! - **Relative:** `2d`, `3w` (or `3s`, semana), `1m`, `1y` (or `1a`, ano):
//!   that long before today.
//! - **Words:** `today` / `hoje`, `yesterday` / `ontem`.
//! - **Days:** `21/11/2026`, `21/11/26`, `21-11`, `21.11` (the most recent
//!   one not after today), `21112026`, `211126`, `2026-11-21`; day before
//!   month or month before day by the calendar's order.
//! - **Months and years:** `nov`, `novembro`, `nov26`, `nov/2026`,
//!   `11/2026`, `2026-11`, `2025`. A month without a year is the most
//!   recent one not after today.
//!
//! English and Portuguese words are both read, whatever the client's
//! language. Owns the reading only; the client supplies today, the order
//! and the language (it owns the clock and the locale), and turns the days
//! into instants with its UTC offsets (`papo_sdk_ui::localize_search`).

use chrono::{Datelike, Months, NaiveDate};

/// Which comes first in a numeric date.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DateOrder {
    /// 21/11/2026, as Brazil writes it.
    DayFirst,
    /// 11/21/2026, as the United States writes it.
    MonthFirst,
}

/// The language suggested dates are written in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DateWords {
    /// today, yesterday, november.
    English,
    /// hoje, ontem, novembro.
    Portuguese,
}

/// The reader's calendar: today, as a local date, and how they write dates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchCalendar {
    /// Today in the reader's own zone.
    pub today: NaiveDate,
    /// The order of a numeric date's day and month.
    pub order: DateOrder,
    /// The language suggestions are written in.
    pub words: DateWords,
}

/// A date filter's value: one day, one month or one year.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchDate {
    /// One day.
    Day(NaiveDate),
    /// One month, 1 to 12, of a year.
    Month {
        /// The year.
        year: i32,
        /// The month, 1 to 12.
        month: u32,
    },
    /// One year.
    Year(i32),
}

impl SearchDate {
    /// Its first day; none outside the calendar's range.
    #[must_use]
    pub fn first_day(self) -> Option<NaiveDate> {
        match self {
            Self::Day(day) => Some(day),
            Self::Month { year, month } => NaiveDate::from_ymd_opt(year, month, 1),
            Self::Year(year) => NaiveDate::from_ymd_opt(year, 1, 1),
        }
    }

    /// Its last day; none outside the calendar's range.
    #[must_use]
    pub fn last_day(self) -> Option<NaiveDate> {
        match self {
            Self::Day(day) => Some(day),
            Self::Month { .. } => self
                .first_day()?
                .checked_add_months(Months::new(1))?
                .pred_opt(),
            Self::Year(year) => NaiveDate::from_ymd_opt(year, 12, 31),
        }
    }
}

/// The longest relative count read (`9999d`).
const MAX_RELATIVE: u32 = 9999;

/// Months by name, English and Portuguese, full and abbreviated, lowercase.
const MONTH_NAMES: &[(&str, u32)] = &[
    ("january", 1),
    ("janeiro", 1),
    ("jan", 1),
    ("february", 2),
    ("fevereiro", 2),
    ("feb", 2),
    ("fev", 2),
    ("march", 3),
    ("março", 3),
    ("marco", 3),
    ("mar", 3),
    ("april", 4),
    ("abril", 4),
    ("apr", 4),
    ("abr", 4),
    ("may", 5),
    ("maio", 5),
    ("mai", 5),
    ("june", 6),
    ("junho", 6),
    ("jun", 6),
    ("july", 7),
    ("julho", 7),
    ("jul", 7),
    ("august", 8),
    ("agosto", 8),
    ("aug", 8),
    ("ago", 8),
    ("september", 9),
    ("setembro", 9),
    ("sept", 9),
    ("sep", 9),
    ("set", 9),
    ("october", 10),
    ("outubro", 10),
    ("oct", 10),
    ("out", 10),
    ("november", 11),
    ("novembro", 11),
    ("nov", 11),
    ("december", 12),
    ("dezembro", 12),
    ("dec", 12),
    ("dez", 12),
];

/// What `raw` names in `calendar`; none when it is no date this reads, or
/// one outside years 1 to 9999.
#[must_use]
pub fn parse_search_date(raw: &str, calendar: SearchCalendar) -> Option<SearchDate> {
    let raw = raw.trim().to_lowercase();
    if raw.is_empty() {
        return None;
    }
    let today = calendar.today;
    let date = match raw.as_str() {
        "today" | "hoje" => Some(SearchDate::Day(today)),
        "yesterday" | "ontem" => today.pred_opt().map(SearchDate::Day),
        _ => relative(&raw, today)
            .or_else(|| named_month(&raw, today))
            .or_else(|| numeric(&raw, calendar)),
    }?;
    let in_range = |day: NaiveDate| (MIN_YEAR..=MAX_YEAR).contains(&day.year());
    (date.first_day().is_some_and(in_range) && date.last_day().is_some_and(in_range))
        .then_some(date)
}

/// The years a date may fall in: the backend's `YYYY-MM-DD`.
const MIN_YEAR: i32 = 1;
const MAX_YEAR: i32 = 9999;

/// How far back a day and month with no year looks for a year that has
/// it (29 February: the most recent leap year).
const YEARS_BACK: i32 = 8;

/// `2d`, `3w` / `3s`, `1m`, `1y` / `1a`: that long before today.
fn relative(raw: &str, today: NaiveDate) -> Option<SearchDate> {
    let unit = raw.chars().last()?;
    let digits = &raw[..raw.len() - unit.len_utf8()];
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let count: u32 = digits.parse().ok()?;
    if count > MAX_RELATIVE {
        return None;
    }
    let day = match unit {
        'd' => today.checked_sub_days(chrono::Days::new(count.into()))?,
        'w' | 's' => today.checked_sub_days(chrono::Days::new(u64::from(count) * 7))?,
        'm' => today.checked_sub_months(Months::new(count))?,
        'y' | 'a' => today.checked_sub_months(Months::new(count.checked_mul(12)?))?,
        _ => return None,
    };
    Some(SearchDate::Day(day))
}

/// The month a name starts `raw` with, if any, and what follows it.
fn month_name(raw: &str) -> Option<(u32, &str)> {
    // The longest name that starts it, so "marco" is not read as "mar" + "co".
    let named = MONTH_NAMES
        .iter()
        .copied()
        .filter(|(name, _)| raw.starts_with(name))
        .max_by_key(|(name, _)| name.len())?;
    Some((named.1, &raw[named.0.len()..]))
}

/// `nov`, `novembro`, `nov26`, `nov-26`, `nov/2026`.
fn named_month(raw: &str, today: NaiveDate) -> Option<SearchDate> {
    let (month, rest) = month_name(raw)?;
    let rest = rest.trim_start_matches(['-', '/', '.']);
    if rest.is_empty() {
        return Some(recent_month(month, today));
    }
    let year = year_of(rest, today)?;
    Some(SearchDate::Month { year, month })
}

/// Month `month` in the most recent year that puts it not after today.
fn recent_month(month: u32, today: NaiveDate) -> SearchDate {
    let year = if month > today.month() {
        today.year() - 1
    } else {
        today.year()
    };
    SearchDate::Month { year, month }
}

/// A four-digit year, or a two-digit one as the most recent such year not
/// after today's (in 2026, `26` is 2026 and `99` is 1999).
fn year_of(raw: &str, today: NaiveDate) -> Option<i32> {
    if !raw.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let year: i32 = raw.parse().ok()?;
    match raw.len() {
        2 => {
            let this_century = today.year() - today.year().rem_euclid(100) + year;
            Some(if this_century > today.year() {
                this_century - 100
            } else {
                this_century
            })
        }
        4 => Some(year),
        _ => None,
    }
}

/// Numeric dates, with `/`, `-` or `.` between their parts or none.
fn numeric(raw: &str, calendar: SearchCalendar) -> Option<SearchDate> {
    let parts: Vec<&str> = raw.split(['/', '-', '.']).collect();
    if parts
        .iter()
        .any(|part| part.is_empty() || !part.chars().all(|c| c.is_ascii_digit()))
    {
        return None;
    }
    match parts.as_slice() {
        [digits] => packed(digits, calendar),
        [first, second] => two_parts(first, second, calendar),
        [first, second, third] => three_parts(first, second, third, calendar),
        _ => None,
    }
}

/// A date with no separators: `2025` (a year), `211126`, `21112026`,
/// `20261121`. Eight digits read both ways; a reading whose year is
/// plausible (from 1970 to next year) wins, the calendar's order first.
fn packed(digits: &str, calendar: SearchCalendar) -> Option<SearchDate> {
    match digits.len() {
        4 => Some(SearchDate::Year(digits.parse().ok()?)),
        6 => three_parts(&digits[..2], &digits[2..4], &digits[4..], calendar),
        8 => {
            let ordered = three_parts(&digits[..2], &digits[2..4], &digits[4..], calendar);
            let year_first = three_parts(&digits[..4], &digits[4..6], &digits[6..], calendar);
            let plausible = |date: &SearchDate| {
                date.first_day()
                    .is_some_and(|day| (1970..=calendar.today.year() + 1).contains(&day.year()))
            };
            [ordered, year_first]
                .into_iter()
                .flatten()
                .find(plausible)
                .or(ordered)
                .or(year_first)
        }
        _ => None,
    }
}

/// `11/2026` and `2026-11` (a month), else a day and month in the
/// calendar's order: the most recent such day not after today.
fn two_parts(first: &str, second: &str, calendar: SearchCalendar) -> Option<SearchDate> {
    let month_of = |raw: &str| {
        raw.parse::<u32>()
            .ok()
            .filter(|month| (1..=12).contains(month))
    };
    if first.len() == 4 {
        return Some(SearchDate::Month {
            year: first.parse().ok()?,
            month: month_of(second)?,
        });
    }
    if second.len() == 4 {
        return Some(SearchDate::Month {
            year: second.parse().ok()?,
            month: month_of(first)?,
        });
    }
    let (day, month) = match calendar.order {
        DateOrder::DayFirst => (first, second),
        DateOrder::MonthFirst => (second, first),
    };
    let (day, month): (u32, u32) = (day.parse().ok()?, month.parse().ok()?);
    let today = calendar.today;
    (0..=YEARS_BACK)
        .filter_map(|back| NaiveDate::from_ymd_opt(today.year() - back, month, day))
        .find(|date| *date <= today)
        .map(SearchDate::Day)
}

/// `2026-11-21` (year first), else day, month and year in the calendar's
/// order, the year in two or four digits.
fn three_parts(
    first: &str,
    second: &str,
    third: &str,
    calendar: SearchCalendar,
) -> Option<SearchDate> {
    let (year, month, day) = if first.len() == 4 {
        (first, second, third)
    } else {
        match calendar.order {
            DateOrder::DayFirst => (third, second, first),
            DateOrder::MonthFirst => (third, first, second),
        }
    };
    if day.len() > 2 || month.len() > 2 {
        return None;
    }
    let year = year_of(year, calendar.today)?;
    let date = NaiveDate::from_ymd_opt(year, month.parse().ok()?, day.parse().ok()?)?;
    Some(SearchDate::Day(date))
}

#[cfg(test)]
#[path = "dates_tests.rs"]
mod tests;
