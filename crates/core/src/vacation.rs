// SPDX-License-Identifier: GPL-3.0-or-later

//! Counting vacation days and ages.

use chrono::{Datelike, Duration, NaiveDate, Weekday};

use crate::model::Entry;

/// Hours that make up one vacation day when time off is entered by the hour.
pub const HOURS_PER_DAY: f64 = 8.0;

/// Whether `day` counts as a vacation day.
pub fn counts(day: NaiveDate, saturday: bool, sunday: bool, holidays: &[NaiveDate]) -> bool {
    match day.weekday() {
        Weekday::Sat if !saturday => false,
        Weekday::Sun if !sunday => false,
        _ => !holidays.contains(&day),
    }
}

/// Vacation days an entry uses within `[from, to)`: whole days without
/// weekends (unless counted) and holidays, half days as 0.5, single hours
/// as a share of a working day.
pub fn days_used(entry: &Entry, from: NaiveDate, to: NaiveDate, holidays: &[NaiveDate]) -> f64 {
    let day = entry.start.date();
    if !entry.all_day {
        if day < from || day >= to {
            return 0.0;
        }
        let hours = (entry.end - entry.start).num_minutes() as f64 / 60.0;
        return (hours / HOURS_PER_DAY).min(1.0);
    }
    if entry.half.is_some() {
        return if day >= from && day < to { 0.5 } else { 0.0 };
    }
    let mut n = 0.0;
    let mut d = day.max(from);
    let end = entry.end.date().min(to);
    while d < end {
        if counts(d, entry.count_saturday, entry.count_sunday, holidays) {
            n += 1.0;
        }
        d += Duration::days(1);
    }
    n
}

/// Age on `date` for someone born in `birth_year` (birthdays are yearly entries).
pub fn age(birth_year: i32, date: NaiveDate) -> i32 {
    date.year() - birth_year
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{HalfDay, at, midnight};

    fn d(m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, day).unwrap()
    }

    fn whole(first: NaiveDate, last: NaiveDate, sat: bool, sun: bool) -> Entry {
        let mut e = Entry::new(
            1,
            "Urlaub",
            midnight(first),
            midnight(last + Duration::days(1)),
        );
        e.all_day = true;
        e.count_saturday = sat;
        e.count_sunday = sun;
        e
    }

    #[test]
    fn weekends_count_only_when_chosen() {
        // Monday 23.11. to Tuesday 1.12.2026
        let (from, to) = (d(11, 1), d(12, 31));
        assert_eq!(
            days_used(&whole(d(11, 23), d(12, 1), false, false), from, to, &[]),
            7.0
        );
        assert_eq!(
            days_used(&whole(d(11, 23), d(12, 1), true, true), from, to, &[]),
            9.0
        );
        assert_eq!(
            days_used(&whole(d(11, 23), d(12, 1), true, false), from, to, &[]),
            8.0
        );
    }

    #[test]
    fn holidays_are_skipped() {
        // Thu 24.12. to Mon 28.12., Fri 25.12. is a holiday
        let e = whole(d(12, 24), d(12, 28), false, false);
        assert_eq!(days_used(&e, d(12, 1), d(12, 31), &[d(12, 25)]), 2.0);
    }

    #[test]
    fn counting_whole_half_and_hours() {
        let mut whole = Entry::new(1, "Urlaub", midnight(d(11, 23)), midnight(d(12, 2)));
        whole.all_day = true;
        assert_eq!(days_used(&whole, d(1, 1), d(12, 31), &[]), 7.0);
        whole.count_saturday = true;
        assert_eq!(days_used(&whole, d(1, 1), d(12, 31), &[]), 8.0);
        // Only the part inside the range counts.
        assert_eq!(days_used(&whole, d(11, 30), d(12, 31), &[]), 2.0);

        let mut half = Entry::new(1, "Urlaub", midnight(d(11, 26)), midnight(d(11, 27)));
        half.all_day = true;
        half.half = Some(HalfDay::Afternoon);
        assert_eq!(days_used(&half, d(1, 1), d(12, 31), &[]), 0.5);

        let hours = Entry::new(1, "Urlaub", at(d(11, 26), 8, 0), at(d(11, 26), 11, 0));
        assert_eq!(days_used(&hours, d(1, 1), d(12, 31), &[]), 3.0 / 8.0);
    }

    #[test]
    fn ages() {
        assert_eq!(age(1996, d(11, 26)), 30);
    }
}
