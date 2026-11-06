//! Times and days in the chat, in the PC's own time zone.

use chrono::{DateTime, Datelike, Local, NaiveDate, TimeZone, Timelike};

/// "21:14" or, in 12-hour format, "9:14 PM".
pub fn time_text<Tz: TimeZone>(t: &DateTime<Tz>, h24: bool) -> String {
    if h24 {
        format!("{:02}:{:02}", t.hour(), t.minute())
    } else {
        let (pm, h) = t.hour12();
        format!("{}:{:02} {}", h, t.minute(), if pm { "PM" } else { "AM" })
    }
}

/// The text of the line that starts a day: Today, Yesterday, the weekday for the days before that in the
/// last week, then the date ("Mon 2 Nov", with the year when it is another year).
pub fn day_label(day: NaiveDate, today: NaiveDate) -> String {
    match (today - day).num_days() {
        0 => "Today".to_string(),
        1 => "Yesterday".to_string(),
        2..=6 => day.format("%A").to_string(),
        _ if day.year() == today.year() => day.format("%a %-d %b").to_string(),
        _ => day.format("%-d %b %Y").to_string(),
    }
}

/// "Monday 2 November 2026, 21:14" (the tooltip of a message).
pub fn full_text<Tz: TimeZone>(t: &DateTime<Tz>, h24: bool) -> String
where
    Tz::Offset: std::fmt::Display,
{
    format!("{}, {}", t.format("%A %-d %B %Y"), time_text(t, h24))
}

/// A time given as seconds since 1970 in this PC's time zone.
pub fn local(ts: f64) -> Option<DateTime<Local>> {
    DateTime::from_timestamp(ts as i64, 0).map(|t| t.with_timezone(&Local))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn at(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, min, 0).unwrap()
    }

    #[test]
    fn both_formats() {
        assert_eq!(time_text(&at(2026, 11, 2, 21, 14), true), "21:14");
        assert_eq!(time_text(&at(2026, 11, 2, 21, 14), false), "9:14 PM");
        assert_eq!(time_text(&at(2026, 11, 2, 0, 5), true), "00:05");
        assert_eq!(time_text(&at(2026, 11, 2, 0, 5), false), "12:05 AM");
        assert_eq!(time_text(&at(2026, 11, 2, 12, 0), false), "12:00 PM");
        assert_eq!(time_text(&at(2026, 11, 2, 9, 3), false), "9:03 AM");
    }

    #[test]
    fn day_labels() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 6).unwrap();
        let d = |y, m, dd| NaiveDate::from_ymd_opt(y, m, dd).unwrap();
        assert_eq!(day_label(d(2026, 11, 6), today), "Today");
        assert_eq!(day_label(d(2026, 11, 5), today), "Yesterday");
        assert_eq!(day_label(d(2026, 11, 4), today), "Wednesday");
        assert_eq!(day_label(d(2026, 10, 31), today), "Saturday");
        assert_eq!(day_label(d(2026, 10, 30), today), "Fri 30 Oct");
        assert_eq!(day_label(d(2025, 12, 24), today), "24 Dec 2025");
    }

    #[test]
    fn tooltip() {
        assert_eq!(full_text(&at(2026, 11, 2, 21, 14), true), "Monday 2 November 2026, 21:14");
    }
}
