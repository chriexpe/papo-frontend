//! Local dates and times of messages: the UTC offset at a message's own
//! time, its civil day and minute, and the labels the timeline shows for
//! them ("Today", "Tuesday, 18 November", "14:02").
//!
//! The SDK groups messages into days with the offsets given here; this
//! module only names them. The system's time zone is read through `chrono`;
//! tests give a fixed offset and a fixed "now" instead.

use std::sync::Arc;

use papo_sdk_ui::{UtcOffset, UtcOffsetProvider};

use crate::strings;

const MICROS_PER_SECOND: i64 = 1_000_000;
const SECONDS_PER_DAY: i64 = 86_400;

/// Where and when the reader is: the offsets to read message times in, and
/// the current time.
#[derive(Clone)]
pub(crate) struct LocalTime {
    pub(crate) offsets: Arc<dyn UtcOffsetProvider>,
    /// The current Unix time in microseconds.
    now_us: fn() -> i64,
}

impl LocalTime {
    /// The system's time zone and clock.
    pub(crate) fn system() -> LocalTime {
        LocalTime {
            offsets: Arc::new(system_offset),
            now_us: system_now_us,
        }
    }

    /// A fixed offset and a fixed "now", for tests.
    #[cfg(test)]
    pub(crate) fn fixed(offset: UtcOffset, now_us: fn() -> i64) -> LocalTime {
        LocalTime {
            offsets: Arc::new(move |_| offset),
            now_us,
        }
    }

    /// The current civil day, as signed days since 1970-01-01.
    pub(crate) fn today(&self) -> i64 {
        self.day_of((self.now_us)())
    }

    /// The civil day of a Unix time in microseconds, as the SDK counts it.
    pub(crate) fn day_of(&self, at_us: i64) -> i64 {
        self.local_seconds(at_us).div_euclid(SECONDS_PER_DAY)
    }

    /// "14:02": the local hour and minute of a Unix time in microseconds.
    pub(crate) fn time_label(&self, at_us: i64) -> String {
        let (hour, minute) = self.hour_minute(at_us);
        strings::time(&format!("{hour:02}"), &format!("{minute:02}"))
    }

    /// The local minute of a Unix time in microseconds, counted from
    /// 1970-01-01: two messages in the same minute share it.
    pub(crate) fn minute_of(&self, at_us: i64) -> i64 {
        self.local_seconds(at_us).div_euclid(60)
    }

    fn hour_minute(&self, at_us: i64) -> (i64, i64) {
        let second_of_day = self.local_seconds(at_us).rem_euclid(SECONDS_PER_DAY);
        (second_of_day / 3600, second_of_day % 3600 / 60)
    }

    fn local_seconds(&self, at_us: i64) -> i64 {
        let offset = i64::from(self.offsets.offset_at(at_us).seconds_east());
        at_us.div_euclid(MICROS_PER_SECOND) + offset
    }
}

/// A date separator's label for civil `day`, with `today` the current one:
/// "Today", "Yesterday", else the weekday and date, with the year only
/// outside the current year.
pub(crate) fn day_label(day: i64, today: i64) -> String {
    if day == today {
        return strings::today().to_owned();
    }
    if day == today - 1 {
        return strings::yesterday().to_owned();
    }
    let date = Civil::of(day);
    // 1970-01-01 was a Thursday (4, counting from Sunday).
    let weekday = u32::try_from((day + 4).rem_euclid(7)).unwrap_or_default();
    if date.year == Civil::of(today).year {
        strings::date(weekday, date.day, date.month)
    } else {
        strings::date_with_year(weekday, date.day, date.month, date.year)
    }
}

/// A proleptic Gregorian date.
#[derive(Debug, PartialEq, Eq)]
struct Civil {
    year: i64,
    /// 1 to 12.
    month: u32,
    /// 1 to 31.
    day: u32,
}

impl Civil {
    /// The date `days` days after 1970-01-01 (Howard Hinnant's
    /// `civil_from_days`).
    fn of(days: i64) -> Civil {
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let day_of_era = z.rem_euclid(146_097);
        let year_of_era =
            (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
        let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
        let shifted_month = (5 * day_of_year + 2) / 153;
        let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
        let month = if shifted_month < 10 {
            shifted_month + 3
        } else {
            shifted_month - 9
        };
        let year = year_of_era + era * 400 + i64::from(month <= 2);
        Civil {
            year,
            month: u32::try_from(month).unwrap_or(1),
            day: u32::try_from(day).unwrap_or(1),
        }
    }
}

/// The system time zone's offset at a Unix time in microseconds; UTC when
/// the time is out of range.
fn system_offset(at_us: i64) -> UtcOffset {
    use chrono::{Local, Offset, TimeZone};
    let Some(at) = chrono::DateTime::from_timestamp_micros(at_us) else {
        return UtcOffset::default();
    };
    let seconds = Local
        .offset_from_utc_datetime(&at.naive_utc())
        .fix()
        .local_minus_utc();
    UtcOffset::from_seconds_east(seconds).unwrap_or_default()
}

fn system_now_us() -> i64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    i64::try_from(now.as_micros()).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::{Civil, LocalTime, day_label};
    use papo_sdk_ui::UtcOffset;

    /// 2026-11-20 12:00:00 UTC, a Friday.
    const NOON_US: i64 = 1_795_176_000_000_000;

    fn noon() -> i64 {
        NOON_US
    }

    fn at(offset_s: i32) -> LocalTime {
        LocalTime::fixed(UtcOffset::from_seconds_east(offset_s).unwrap(), noon)
    }

    #[test]
    fn civil_dates_match_the_calendar() {
        assert_eq!(
            Civil::of(0),
            Civil {
                year: 1970,
                month: 1,
                day: 1
            }
        );
        assert_eq!(
            Civil::of(-1),
            Civil {
                year: 1969,
                month: 12,
                day: 31
            }
        );
        assert_eq!(
            Civil::of(11_016),
            Civil {
                year: 2000,
                month: 2,
                day: 29
            }
        );
        let today = at(0).today();
        assert_eq!(
            Civil::of(today),
            Civil {
                year: 2026,
                month: 11,
                day: 20
            }
        );
    }

    #[test]
    fn days_and_times_follow_the_offset_at_the_message() {
        // Three hours west, noon UTC is nine in the morning, the same day.
        let west = at(-3 * 3600);
        assert_eq!(west.time_label(NOON_US), "09:00");
        assert_eq!(west.day_of(NOON_US), at(0).day_of(NOON_US));
        // Thirteen hours east, it is already the next day.
        let east = at(13 * 3600);
        assert_eq!(east.time_label(NOON_US), "01:00");
        assert_eq!(east.day_of(NOON_US), at(0).day_of(NOON_US) + 1);
    }

    #[test]
    fn separators_name_today_yesterday_and_older_days() {
        let today = at(0).today();
        assert_eq!(day_label(today, today), "Today");
        assert_eq!(day_label(today - 1, today), "Yesterday");
        assert_eq!(day_label(today - 2, today), "Wednesday, 18 November");
        assert_eq!(day_label(today - 365, today), "Thursday, 20 November 2025");
    }
}
