//! Date filters in the reader's own calendar (`before:`, `after:`,
//! `during:`) over a backend that counts UTC days. A query's local days
//! become the exact instants they span, read with the client's UTC offsets
//! (its time zone, daylight saving included). The backend is asked for
//! every UTC day those instants touch, and results are trimmed to the
//! window (`SearchResults`). Exact, at the cost of up to one partial UTC
//! day fetched at each end.
//!
//! Owns the conversion only; reading the field is `papo_sdk::SearchDraft`'s
//! and the client's clock and zone are its own (`UtcOffsetProvider`).

use chrono::{DateTime, NaiveDate};
use papo_sdk::{HistoryOrder, SearchCursor, SearchQuery, Uuid};

use crate::UtcOffsetProvider;

/// Microseconds in a second.
const US_PER_S: i64 = 1_000_000;

/// The instants a search keeps, in Unix microseconds: from `start_us`
/// (inclusive) to `end_us` (exclusive); an open end is unbounded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SearchWindow {
    /// The first instant kept.
    pub start_us: Option<i64>,
    /// The first instant past the window.
    pub end_us: Option<i64>,
    /// Results come oldest first (`HistoryOrder::Asc`), not newest first.
    pub oldest_first: bool,
}

impl SearchWindow {
    /// Whether instant `at_us` is in the window.
    #[must_use]
    pub fn holds(&self, at_us: i64) -> bool {
        self.start_us.is_none_or(|start_us| at_us >= start_us)
            && self.end_us.is_none_or(|end_us| at_us < end_us)
    }

    /// Whether instant `at_us` lies past the window's far end in the
    /// results' order: every later result does too, so the list ends.
    #[must_use]
    pub fn passed(&self, at_us: i64) -> bool {
        if self.oldest_first {
            self.end_us.is_some_and(|end_us| at_us >= end_us)
        } else {
            self.start_us.is_some_and(|start_us| at_us < start_us)
        }
    }

    /// The cursor the first page starts from: at the window's near end,
    /// so the backend skips the hours of a UTC day outside it instead of
    /// paging through them. None for an open near end.
    #[must_use]
    pub fn first_cursor(&self) -> Option<SearchCursor> {
        // The backend's cursor is exclusive: newest first, results before
        // `end_us`; oldest first, results after the instant before
        // `start_us` (one microsecond, the backend's precision).
        let (at_us, id) = if self.oldest_first {
            (self.start_us? - 1, Uuid::max())
        } else {
            (self.end_us?, Uuid::nil())
        };
        let at = DateTime::from_timestamp_micros(at_us)?;
        Some(SearchCursor {
            created_at: at.to_rfc3339_opts(chrono::SecondsFormat::Micros, true),
            id: id.to_string(),
        })
    }
}

/// `query` as the backend can take it: its local days turned into the UTC
/// days they touch (`date_start`, `date_end`), and the window its results
/// are trimmed to. A query without local days comes back as it was, with
/// an unbounded window; one whose days have no instant (out of range) has
/// none.
#[must_use]
pub fn localize_search(
    mut query: SearchQuery,
    offsets: &dyn UtcOffsetProvider,
) -> Option<(SearchQuery, SearchWindow)> {
    let Some(days) = query.local_days.take() else {
        return Some((query, SearchWindow::default()));
    };
    // An end that has no instant (a date out of range) is no search.
    let start_us = match days.first {
        Some(first) => Some(local_midnight_us(first, offsets)?),
        None => None,
    };
    let end_us = match days.last {
        Some(last) => Some(local_midnight_us(last.succ_opt()?, offsets)?),
        None => None,
    };
    query.date_start = match start_us {
        Some(start_us) => Some(utc_day(start_us)?),
        None => None,
    };
    // The window ends before `end_us`: its last UTC day holds the instant
    // just before.
    query.date_end = match end_us {
        Some(end_us) => Some(utc_day(end_us - 1)?),
        None => None,
    };
    let oldest_first = query.order == Some(HistoryOrder::Asc);
    Some((
        query,
        SearchWindow {
            start_us,
            end_us,
            oldest_first,
        },
    ))
}

/// The instant local `day` starts at, in Unix microseconds. The offset is
/// the one in force at that instant, found from the offset at the same
/// wall time in UTC, then corrected once (a zone's offset changes at most
/// once around a midnight). A midnight the clock skips (00:00 straight to
/// 01:00) starts the day at the change itself.
fn local_midnight_us(day: NaiveDate, offsets: &dyn UtcOffsetProvider) -> Option<i64> {
    let wall_us = day.and_hms_opt(0, 0, 0)?.and_utc().timestamp_micros();
    let offset_us = |at_us: i64| i64::from(offsets.offset_at(at_us).seconds_east()) * US_PER_S;
    let guess_us = wall_us - offset_us(wall_us);
    let corrected_us = wall_us - offset_us(guess_us);
    if offset_us(corrected_us) == offset_us(guess_us) {
        return Some(corrected_us);
    }
    // The offset changes between the two readings: midnight falls in the
    // skipped hour, and the later reading is where the day starts.
    Some(corrected_us.max(wall_us - offset_us(corrected_us)))
}

/// The UTC day of instant `at_us`, as the backend spells it.
fn utc_day(at_us: i64) -> Option<String> {
    let at = DateTime::from_timestamp_micros(at_us)?;
    Some(at.date_naive().format("%Y-%m-%d").to_string())
}
