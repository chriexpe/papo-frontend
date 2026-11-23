//! The timeline's strings: date separators, message times, the loading and
//! failure rows, and what message content says of itself (spoilers, tasks,
//! code, "(edited)"). The catalog entries are `timeline.*`.

use super::{I18N, Key, catalog};

/// What assistive technology announces for the message list.
pub(crate) fn timeline_label() -> &'static str {
    text!("timeline.label")
}

/// The date separator of the current day.
pub(crate) fn today() -> &'static str {
    text!("timeline.today")
}

/// The date separator of the day before the current one.
pub(crate) fn yesterday() -> &'static str {
    text!("timeline.yesterday")
}

/// "Tuesday, 18 November". `weekday` counts from Sunday (0), `month` from
/// January (1).
pub(crate) fn date(weekday: u32, day: u32, month: u32) -> String {
    template!("timeline.date", "weekday" => weekday_name(weekday), "day" => day, "month" => month_name(month))
}

/// "Tuesday, 18 November 2025", for a day outside the current year.
pub(crate) fn date_with_year(weekday: u32, day: u32, month: u32, year: i64) -> String {
    template!(
        "timeline.date_with_year",
        "weekday" => weekday_name(weekday),
        "day" => day,
        "month" => month_name(month),
        "year" => year
    )
}

/// A message's local time, "14:02": both already two digits.
pub(crate) fn time(hour: &str, minute: &str) -> String {
    template!("timeline.time", "hour" => hour, "minute" => minute)
}

/// The author of a message whose author the account does not know.
pub(crate) fn someone() -> &'static str {
    text!("timeline.someone")
}

/// What assistive technology announces for a row standing in for messages
/// on their way.
pub(crate) fn loading_messages() -> &'static str {
    text!("timeline.loading")
}

/// A gap that failed to fill.
pub(crate) fn gap_failed() -> &'static str {
    text!("timeline.gap_failed")
}

/// Fills a failed gap again.
pub(crate) fn retry() -> &'static str {
    text!("timeline.retry")
}

/// "20 November", or "20 November 2025" when `year` is given.
pub(crate) fn day_month(day: u32, month: u32, year: Option<i64>) -> String {
    match year {
        None => template!("timeline.day_month", "day" => day, "month" => month_name(month)),
        Some(year) => template!(
            "timeline.day_month_year",
            "day" => day,
            "month" => month_name(month),
            "year" => year
        ),
    }
}

/// "20 November, 09:04".
pub(crate) fn date_time(date: &str, time: &str) -> String {
    template!("timeline.date_time", "date" => date, "time" => time)
}

/// "Lia — 09:04": what a copy of several messages puts before each.
pub(crate) fn copy_header(author: &str, time: &str) -> String {
    template!("timeline.copy_header", "author" => author, "time" => time)
}

/// "(edited)", after an edited message's text.
pub(crate) fn edited() -> &'static str {
    text!("timeline.edited")
}

/// "Edited on 21 November 2026, 13:02".
pub(crate) fn edited_on(date: &str, time: &str) -> String {
    template!("timeline.edited_on", "date" => date, "time" => time)
}

/// A mention of a person the server does not know.
pub(crate) fn unknown_person() -> &'static str {
    text!("timeline.unknown_person")
}

/// What assistive technology announces for a hidden spoiler.
pub(crate) fn spoiler_hidden() -> &'static str {
    text!("timeline.spoiler_hidden")
}

/// What assistive technology announces for a done task.
pub(crate) fn task_done() -> &'static str {
    text!("timeline.task_done")
}

/// What assistive technology announces for a task not done.
pub(crate) fn task_to_do() -> &'static str {
    text!("timeline.task_to_do")
}

/// A code block's Copy button.
pub(crate) fn code_copy() -> &'static str {
    text!("timeline.code_copy")
}

/// A code block's Copy button just after a press, after its ✓.
pub(crate) fn code_copied() -> &'static str {
    text!("timeline.code_copied")
}

/// What assistive technology announces for a code block's Copy button.
pub(crate) fn copy_code_label() -> &'static str {
    text!("timeline.code_copy_label")
}

fn weekday_name(weekday: u32) -> &'static str {
    match weekday % 7 {
        0 => text!("timeline.weekday.0"),
        1 => text!("timeline.weekday.1"),
        2 => text!("timeline.weekday.2"),
        3 => text!("timeline.weekday.3"),
        4 => text!("timeline.weekday.4"),
        5 => text!("timeline.weekday.5"),
        _ => text!("timeline.weekday.6"),
    }
}

pub(super) fn month_name(month: u32) -> &'static str {
    match month {
        1 => text!("timeline.month.1"),
        2 => text!("timeline.month.2"),
        3 => text!("timeline.month.3"),
        4 => text!("timeline.month.4"),
        5 => text!("timeline.month.5"),
        6 => text!("timeline.month.6"),
        7 => text!("timeline.month.7"),
        8 => text!("timeline.month.8"),
        9 => text!("timeline.month.9"),
        10 => text!("timeline.month.10"),
        11 => text!("timeline.month.11"),
        _ => text!("timeline.month.12"),
    }
}
