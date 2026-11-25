// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::BTreeMap;

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, Weekday};
use serde::{Deserialize, Serialize};

pub type Id = i64;

/// The built-in kinds. Users may add their own kinds, which have no `Builtin`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Builtin {
    Event,
    Meeting,
    Task,
    Vacation,
    Birthday,
    Holiday,
}

impl Builtin {
    /// Automatic kinds are filled by the app, not by the user.
    pub fn is_automatic(self) -> bool {
        matches!(self, Builtin::Holiday)
    }
}

/// What kind of entry something is: Termin, Meeting, Aufgabe, Urlaub …
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Kind {
    pub id: Id,
    pub name: String,
    /// `#rrggbb`
    pub color: String,
    /// One of the icon names in the app's icon set.
    pub icon: String,
    pub builtin: Option<Builtin>,
    pub visible: bool,
    /// Set for a subscribed calendar: its entries come from this link and are read-only.
    #[serde(default)]
    pub source: Option<Source>,
}

/// Where a subscribed calendar comes from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Source {
    pub url: String,
    /// Last successful update (local time).
    #[serde(default)]
    pub updated: Option<NaiveDateTime>,
    /// Message of the last failed update, cleared on success.
    #[serde(default)]
    pub error: Option<String>,
}

/// A half day off. Which half does not matter for the calendar; new entries
/// use `Morning`, `Afternoon` is still read from older data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HalfDay {
    Morning,
    Afternoon,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Partstat {
    NeedsAction,
    Accepted,
    Declined,
    Tentative,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Participant {
    pub name: String,
    pub email: String,
    pub status: Partstat,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Freq {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecurEnd {
    Never,
    Until(NaiveDate),
    Count(u32),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Recurrence {
    pub freq: Freq,
    /// Every n-th day/week/month/year, at least 1.
    pub interval: u32,
    /// Monday = 0 … Sunday = 6. Only used for weekly rules; empty means the start's weekday.
    pub weekdays: Vec<u8>,
    pub end: RecurEnd,
}

/// One calendar entry. All-day entries run from `start` (midnight) to `end`
/// (midnight after the last day, exclusive), like iCalendar.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub id: Id,
    pub uid: String,
    pub kind: Id,
    pub title: String,
    pub start: NaiveDateTime,
    pub end: NaiveDateTime,
    pub all_day: bool,
    #[serde(default)]
    pub half: Option<HalfDay>,
    #[serde(default)]
    pub location: String,
    #[serde(default)]
    pub notes: String,
    /// Minutes before the start (or before leaving, if there is travel time).
    #[serde(default)]
    pub reminder: Option<i64>,
    /// Minutes kept free before the entry.
    #[serde(default)]
    pub travel: i64,
    /// Minutes kept free after the entry.
    #[serde(default)]
    pub buffer: i64,
    #[serde(default)]
    pub recurrence: Option<Recurrence>,
    /// Occurrence dates removed from a series.
    #[serde(default)]
    pub exceptions: Vec<NaiveDate>,
    #[serde(default)]
    pub subtasks: Vec<String>,
    /// Which subtasks are done, per occurrence date. Every occurrence of a
    /// recurring task starts with everything open.
    #[serde(default)]
    pub done: BTreeMap<NaiveDate, Vec<bool>>,
    #[serde(default)]
    pub participants: Vec<Participant>,
    /// Set when the entry came from someone else's invitation.
    #[serde(default)]
    pub organizer: Option<String>,
    #[serde(default)]
    pub sequence: u32,
    /// Birthdays: year of birth, if known (shows the age).
    #[serde(default)]
    pub birth_year: Option<i32>,
    /// Vacation over several days: whether Saturdays and Sundays count as vacation days.
    #[serde(default)]
    pub count_saturday: bool,
    #[serde(default)]
    pub count_sunday: bool,
}

impl Entry {
    pub fn new(kind: Id, title: &str, start: NaiveDateTime, end: NaiveDateTime) -> Self {
        Self {
            id: 0,
            uid: new_uid(),
            kind,
            title: title.to_owned(),
            start,
            end,
            all_day: false,
            half: None,
            location: String::new(),
            notes: String::new(),
            reminder: Some(15),
            travel: 0,
            buffer: 0,
            recurrence: None,
            exceptions: Vec::new(),
            subtasks: Vec::new(),
            done: BTreeMap::new(),
            participants: Vec::new(),
            organizer: None,
            sequence: 0,
            birth_year: None,
            count_saturday: false,
            count_sunday: false,
        }
    }

    pub fn duration(&self) -> Duration {
        self.end - self.start
    }

    pub fn has_tasks(&self) -> bool {
        !self.subtasks.is_empty()
    }

    /// Done flags of one occurrence, always as long as `subtasks`.
    pub fn done_on(&self, date: NaiveDate) -> Vec<bool> {
        let mut flags = self.done.get(&date).cloned().unwrap_or_default();
        flags.resize(self.subtasks.len(), false);
        flags
    }

    pub fn set_done(&mut self, date: NaiveDate, index: usize, done: bool) {
        let mut flags = self.done_on(date);
        if let Some(flag) = flags.get_mut(index) {
            *flag = done;
        }
        self.done.insert(date, flags);
    }

    /// (done, total) for one occurrence.
    pub fn progress(&self, date: NaiveDate) -> (usize, usize) {
        let flags = self.done_on(date);
        (flags.iter().filter(|d| **d).count(), flags.len())
    }

    pub fn all_done(&self, date: NaiveDate) -> bool {
        let (done, total) = self.progress(date);
        total > 0 && done == total
    }

    pub fn add_subtask(&mut self, text: &str) {
        self.subtasks.push(text.to_owned());
        for flags in self.done.values_mut() {
            flags.resize(self.subtasks.len(), false);
        }
    }

    pub fn remove_subtask(&mut self, index: usize) {
        if index >= self.subtasks.len() {
            return;
        }
        self.subtasks.remove(index);
        for flags in self.done.values_mut() {
            if index < flags.len() {
                flags.remove(index);
            }
        }
    }

    /// Number of days an all-day entry covers (at least 1).
    pub fn day_count(&self) -> i64 {
        ((self.end.date() - self.start.date()).num_days()).max(1)
    }
}

/// A saved template for tasks: title, duration and subtasks.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Routine {
    pub id: Id,
    pub name: String,
    pub minutes: i64,
    pub subtasks: Vec<String>,
}

/// One appearance of an entry in the calendar (series are expanded).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Occurrence {
    pub entry: Id,
    /// The day this occurrence belongs to, used as key for subtask state.
    pub date: NaiveDate,
    pub start: NaiveDateTime,
    pub end: NaiveDateTime,
}

pub fn new_uid() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let now = chrono::Utc::now();
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!(
        "{}-{}-{}@kalender.local",
        now.format("%Y%m%dT%H%M%S%f"),
        std::process::id(),
        n
    )
}

pub fn weekday_index(day: Weekday) -> u8 {
    day.num_days_from_monday() as u8
}

pub fn at(date: NaiveDate, hour: u32, minute: u32) -> NaiveDateTime {
    date.and_time(NaiveTime::from_hms_opt(hour, minute, 0).unwrap_or(NaiveTime::MIN))
}

pub fn midnight(date: NaiveDate) -> NaiveDateTime {
    date.and_time(NaiveTime::MIN)
}

/// Monday of the week containing `date`.
pub fn week_start(date: NaiveDate) -> NaiveDate {
    date - Duration::days(date.weekday().num_days_from_monday() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn subtask_progress_per_date() {
        let mut e = Entry::new(
            1,
            "Putzen",
            at(d(2026, 11, 24), 15, 0),
            at(d(2026, 11, 24), 17, 0),
        );
        e.add_subtask("Staubsaugen");
        e.add_subtask("Bad");
        e.set_done(d(2026, 11, 24), 0, true);
        assert_eq!(e.progress(d(2026, 11, 24)), (1, 2));
        assert_eq!(e.progress(d(2026, 12, 1)), (0, 2));
        e.set_done(d(2026, 11, 24), 1, true);
        assert!(e.all_done(d(2026, 11, 24)));
        e.add_subtask("Küche");
        assert!(!e.all_done(d(2026, 11, 24)));
        e.remove_subtask(0);
        assert_eq!(e.done_on(d(2026, 11, 24)), vec![true, false]);
    }

    #[test]
    fn week_starts_on_monday() {
        assert_eq!(week_start(d(2026, 11, 26)), d(2026, 11, 23));
        assert_eq!(week_start(d(2026, 11, 23)), d(2026, 11, 23));
        assert_eq!(week_start(d(2026, 11, 29)), d(2026, 11, 23));
    }
}
