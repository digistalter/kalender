// SPDX-License-Identifier: GPL-3.0-or-later

//! Expanding entries and series into the occurrences of a time range.

use chrono::{Datelike, Duration, Months, NaiveDate, NaiveDateTime};

use crate::model::{Entry, Freq, Occurrence, RecurEnd, weekday_index};

/// Upper bound for generated candidates, so a broken rule can never hang the UI.
const MAX_STEPS: usize = 20_000;

/// All occurrences of `entry` that overlap `[from, to)`.
pub fn occurrences(entry: &Entry, from: NaiveDateTime, to: NaiveDateTime) -> Vec<Occurrence> {
    let duration = entry.duration();
    let make = |start: NaiveDateTime| Occurrence {
        entry: entry.id,
        date: start.date(),
        start,
        end: start + duration,
    };
    let overlaps =
        |o: &Occurrence| o.start < to && (o.end > from || (o.start >= from && o.end == o.start));

    let Some(rule) = &entry.recurrence else {
        let o = make(entry.start);
        return if overlaps(&o) { vec![o] } else { Vec::new() };
    };

    let mut out = Vec::new();
    let mut count = 0u32;
    let interval = rule.interval.max(1);
    let time = entry.start.time();
    let first = entry.start.date();
    let past_end = |date: NaiveDate, count: u32| match rule.end {
        RecurEnd::Never => false,
        RecurEnd::Until(until) => date > until,
        RecurEnd::Count(n) => count >= n,
    };

    let push = |date: NaiveDate, out: &mut Vec<Occurrence>, count: &mut u32| -> bool {
        if past_end(date, *count) {
            return false;
        }
        *count += 1;
        let o = make(date.and_time(time));
        if !entry.exceptions.contains(&date) && overlaps(&o) {
            out.push(o);
        }
        true
    };

    match rule.freq {
        Freq::Daily => {
            for step in 0..MAX_STEPS {
                let date = first + Duration::days(step as i64 * interval as i64);
                if date.and_time(time) >= to || !push(date, &mut out, &mut count) {
                    break;
                }
            }
        }
        Freq::Weekly => {
            let mut days: Vec<u8> = if rule.weekdays.is_empty() {
                vec![weekday_index(first.weekday())]
            } else {
                rule.weekdays.clone()
            };
            days.sort_unstable();
            days.dedup();
            let monday = crate::model::week_start(first);
            'weeks: for week in 0..MAX_STEPS {
                let base = monday + Duration::weeks(week as i64 * interval as i64);
                for &day in &days {
                    let date = base + Duration::days(day as i64);
                    if date < first {
                        continue;
                    }
                    if date.and_time(time) >= to || !push(date, &mut out, &mut count) {
                        break 'weeks;
                    }
                }
            }
        }
        Freq::Monthly | Freq::Yearly => {
            let months = if rule.freq == Freq::Monthly {
                interval
            } else {
                interval * 12
            };
            for step in 0..MAX_STEPS {
                let Some(date) = first.checked_add_months(Months::new(months * step as u32)) else {
                    break;
                };
                // 31st in a short month or 29 February in a common year: skip, like iCalendar.
                if date.day() != first.day() {
                    continue;
                }
                if date.and_time(time) >= to || !push(date, &mut out, &mut count) {
                    break;
                }
            }
        }
    }
    out
}

/// Occurrences of many entries in `[from, to)`, sorted by start.
pub fn expand<'a>(
    entries: impl IntoIterator<Item = &'a Entry>,
    from: NaiveDateTime,
    to: NaiveDateTime,
) -> Vec<Occurrence> {
    let mut all: Vec<Occurrence> = entries
        .into_iter()
        .flat_map(|e| occurrences(e, from, to))
        .collect();
    all.sort_by_key(|o| (o.start, o.entry));
    all
}

/// The occurrence of `entry` that starts on `date`, if there is one.
pub fn occurrence_on(entry: &Entry, date: NaiveDate) -> Option<Occurrence> {
    let from = crate::model::midnight(date);
    occurrences(entry, from, from + Duration::days(1))
        .into_iter()
        .find(|o| o.date == date)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Recurrence, at, midnight};

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn entry(start: NaiveDateTime, rule: Option<Recurrence>) -> Entry {
        let mut e = Entry::new(1, "x", start, start + Duration::hours(1));
        e.id = 7;
        e.recurrence = rule;
        e
    }

    fn rule(freq: Freq, interval: u32, weekdays: Vec<u8>, end: RecurEnd) -> Option<Recurrence> {
        Some(Recurrence {
            freq,
            interval,
            weekdays,
            end,
        })
    }

    #[test]
    fn single_entry_in_and_out_of_range() {
        let e = entry(at(d(2026, 11, 24), 9, 0), None);
        let week = midnight(d(2026, 11, 23));
        assert_eq!(occurrences(&e, week, week + Duration::days(7)).len(), 1);
        assert!(occurrences(&e, week + Duration::days(7), week + Duration::days(14)).is_empty());
    }

    #[test]
    fn weekly_on_two_days_with_count() {
        // Tue 24.11.2026, every week on Tue and Thu, 4 times.
        let e = entry(
            at(d(2026, 11, 24), 9, 0),
            rule(Freq::Weekly, 1, vec![1, 3], RecurEnd::Count(4)),
        );
        let from = midnight(d(2026, 11, 1));
        let dates: Vec<_> = occurrences(&e, from, from + Duration::days(60))
            .iter()
            .map(|o| o.date)
            .collect();
        assert_eq!(
            dates,
            vec![
                d(2026, 11, 24),
                d(2026, 11, 26),
                d(2026, 12, 1),
                d(2026, 12, 3)
            ]
        );
    }

    #[test]
    fn every_two_weeks_until_date_with_exception() {
        let mut e = entry(
            at(d(2026, 11, 2), 8, 0),
            rule(Freq::Weekly, 2, vec![], RecurEnd::Until(d(2026, 12, 31))),
        );
        e.exceptions.push(d(2026, 11, 30));
        let from = midnight(d(2026, 11, 1));
        let dates: Vec<_> = occurrences(&e, from, from + Duration::days(120))
            .iter()
            .map(|o| o.date)
            .collect();
        assert_eq!(
            dates,
            vec![
                d(2026, 11, 2),
                d(2026, 11, 16),
                d(2026, 12, 14),
                d(2026, 12, 28)
            ]
        );
    }

    #[test]
    fn monthly_skips_short_months() {
        let e = entry(
            at(d(2026, 1, 31), 10, 0),
            rule(Freq::Monthly, 1, vec![], RecurEnd::Count(3)),
        );
        let from = midnight(d(2026, 1, 1));
        let dates: Vec<_> = occurrences(&e, from, from + Duration::days(200))
            .iter()
            .map(|o| o.date)
            .collect();
        assert_eq!(dates, vec![d(2026, 1, 31), d(2026, 3, 31), d(2026, 5, 31)]);
    }

    #[test]
    fn yearly_birthday_far_in_the_future() {
        let mut e = entry(
            midnight(d(1990, 11, 26)),
            rule(Freq::Yearly, 1, vec![], RecurEnd::Never),
        );
        e.all_day = true;
        e.end = midnight(d(1990, 11, 27));
        let from = midnight(d(2026, 11, 23));
        let o = occurrences(&e, from, from + Duration::days(7));
        assert_eq!(o.len(), 1);
        assert_eq!(o[0].date, d(2026, 11, 26));
        assert_eq!(o[0].end, midnight(d(2026, 11, 27)));
    }

    #[test]
    fn daily_overlapping_range_start() {
        let mut e = entry(
            at(d(2026, 11, 20), 23, 0),
            rule(Freq::Daily, 1, vec![], RecurEnd::Never),
        );
        e.end = e.start + Duration::hours(2);
        let from = midnight(d(2026, 11, 22));
        let o = occurrences(&e, from, from + Duration::days(1));
        // The one starting the evening before reaches into the range.
        assert_eq!(
            o.iter().map(|o| o.date).collect::<Vec<_>>(),
            vec![d(2026, 11, 21), d(2026, 11, 22)]
        );
    }

    #[test]
    fn occurrence_on_finds_series_day() {
        let e = entry(
            at(d(2026, 11, 24), 9, 0),
            rule(Freq::Weekly, 1, vec![], RecurEnd::Never),
        );
        assert!(occurrence_on(&e, d(2026, 12, 8)).is_some());
        assert!(occurrence_on(&e, d(2026, 12, 9)).is_none());
    }
}
