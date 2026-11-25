// SPDX-License-Identifier: GPL-3.0-or-later

//! Everything the window shows, loaded from the local database.

use std::cell::RefCell;
use std::rc::Rc;

use anyhow::Result;
use chrono::{Datelike, Duration, Local, NaiveDate, NaiveDateTime};
use kalender_core::model::{Builtin, Entry, Id, Kind, Occurrence, Routine, midnight};
use kalender_core::store::Store;
use kalender_core::{holidays, recur};

use crate::i18n::tr;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Week,
    Month,
}

#[derive(Clone, Debug)]
pub struct Settings {
    pub week_numbers: bool,
    pub dim_past: bool,
    pub start_view: View,
    pub default_minutes: i64,
    pub default_reminder: Option<i64>,
    pub default_buffer: i64,
    pub own_name: String,
    pub own_email: String,
    /// Federal state for holidays, "" = nationwide only, "-" = no holidays.
    pub holiday_state: String,
    pub run_in_background: bool,
    /// "", "de" or "en".
    pub language: String,
    /// Vacation days per year; 0 = not counted.
    pub vacation_per_year: u32,
    /// Daily copy of all data.
    pub backup: bool,
    /// Folder for the copies; "" = the default folder (see `backup::folder`).
    pub backup_dir: String,
}

impl Settings {
    fn load(store: &Store) -> Self {
        let get = |k: &str| store.setting(k).ok().flatten();
        let flag = |k: &str, d: bool| get(k).map(|v| v == "1").unwrap_or(d);
        let num = |k: &str, d: i64| get(k).and_then(|v| v.parse().ok()).unwrap_or(d);
        Self {
            week_numbers: flag("week_numbers", false),
            dim_past: flag("dim_past", true),
            start_view: if get("start_view").as_deref() == Some("month") {
                View::Month
            } else {
                View::Week
            },
            default_minutes: num("default_minutes", 60),
            default_reminder: match num("default_reminder", 15) {
                n if n < 0 => None,
                n => Some(n),
            },
            default_buffer: num("default_buffer", 0),
            own_name: get("own_name").unwrap_or_default(),
            own_email: get("own_email").unwrap_or_default(),
            holiday_state: get("holiday_state").unwrap_or_else(|| "NW".into()),
            run_in_background: flag("run_in_background", true),
            language: get("language").unwrap_or_default(),
            vacation_per_year: num("vacation_per_year", 0).max(0) as u32,
            backup: flag("backup", true),
            backup_dir: get("backup_dir").unwrap_or_default(),
        }
    }

    pub fn save(&self, store: &Store) -> Result<()> {
        let b = |v: bool| if v { "1" } else { "0" };
        store.set_setting("week_numbers", b(self.week_numbers))?;
        store.set_setting("dim_past", b(self.dim_past))?;
        store.set_setting(
            "start_view",
            if self.start_view == View::Month {
                "month"
            } else {
                "week"
            },
        )?;
        store.set_setting("default_minutes", &self.default_minutes.to_string())?;
        store.set_setting(
            "default_reminder",
            &self.default_reminder.unwrap_or(-1).to_string(),
        )?;
        store.set_setting("default_buffer", &self.default_buffer.to_string())?;
        store.set_setting("own_name", &self.own_name)?;
        store.set_setting("own_email", &self.own_email)?;
        store.set_setting("holiday_state", &self.holiday_state)?;
        store.set_setting("run_in_background", b(self.run_in_background))?;
        store.set_setting("language", &self.language)?;
        store.set_setting("vacation_per_year", &self.vacation_per_year.to_string())?;
        store.set_setting("backup", b(self.backup))?;
        store.set_setting("backup_dir", &self.backup_dir)?;
        Ok(())
    }
}

/// One occurrence together with its entry and kind, ready to draw.
#[derive(Clone, Debug)]
pub struct Item {
    pub occ: Occurrence,
    pub entry: Entry,
    pub kind: Kind,
}

impl Item {
    /// The title as shown, with the age for birthdays.
    pub fn title(&self) -> String {
        display_title(&self.entry, self.occ.date)
    }

    pub fn progress(&self) -> Option<(usize, usize)> {
        self.entry
            .has_tasks()
            .then(|| self.entry.progress(self.occ.date))
    }

    pub fn done(&self) -> bool {
        self.entry.all_done(self.occ.date)
    }

    pub fn is_virtual(&self) -> bool {
        self.entry.id < 0
    }
}

pub struct State {
    pub store: Store,
    pub kinds: Vec<Kind>,
    pub entries: Vec<Entry>,
    pub routines: Vec<Routine>,
    pub settings: Settings,
    /// The day the views are centred on.
    pub selected: NaiveDate,
    pub view: View,
}

pub type Shared = Rc<RefCell<State>>;

impl State {
    pub fn backup_to(&self, path: &std::path::Path) -> Result<()> {
        self.store.backup_to(path)
    }

    /// Replaces all data with a backup and reloads it.
    pub fn restore_from(&mut self, path: &std::path::Path) -> Result<()> {
        self.store.restore_from(path)?;
        self.kinds = self.store.kinds()?;
        self.entries = self.store.entries()?;
        self.routines = self.store.routines()?;
        self.settings = Settings::load(&self.store);
        Ok(())
    }

    pub fn load(store: Store) -> Result<Self> {
        let settings = Settings::load(&store);
        Ok(Self {
            kinds: store.kinds()?,
            entries: store.entries()?,
            routines: store.routines()?,
            view: settings.start_view,
            settings,
            store,
            selected: today(),
        })
    }

    pub fn kind(&self, id: Id) -> Option<&Kind> {
        self.kinds.iter().find(|k| k.id == id)
    }

    pub fn builtin(&self, b: Builtin) -> Option<&Kind> {
        self.kinds.iter().find(|k| k.builtin == Some(b))
    }

    pub fn entry(&self, id: Id) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// Kinds the user can pick for a new entry.
    pub fn pickable_kinds(&self) -> Vec<Kind> {
        self.kinds
            .iter()
            .filter(|k| !k.builtin.is_some_and(Builtin::is_automatic) && k.source.is_none())
            .cloned()
            .collect()
    }

    /// Entries of subscribed calendars can only be looked at.
    pub fn is_read_only(&self, entry: Id) -> bool {
        self.entry(entry)
            .and_then(|e| self.kind(e.kind))
            .is_some_and(|k| k.source.is_some())
    }

    /// Puts the freshly loaded entries of a subscribed calendar in place of the old ones.
    pub fn replace_subscribed(&mut self, kind: Id, mut entries: Vec<Entry>) -> Result<()> {
        self.store.replace_entries_of_kind(kind, &mut entries)?;
        self.entries.retain(|e| e.kind != kind);
        self.entries.extend(entries);
        Ok(())
    }

    /// All visible occurrences overlapping `[from, to)`, including holidays.
    pub fn items(&self, from: NaiveDateTime, to: NaiveDateTime) -> Vec<Item> {
        let mut items: Vec<Item> = recur::expand(self.entries.iter(), from, to)
            .into_iter()
            .filter_map(|occ| {
                let entry = self.entry(occ.entry)?.clone();
                let kind = self.kind(entry.kind)?.clone();
                kind.visible.then_some(Item { occ, entry, kind })
            })
            .collect();
        if let Some(kind) = self.builtin(Builtin::Holiday)
            && kind.visible
            && self.settings.holiday_state != "-"
        {
            let from_day = from.date();
            let to_day = to.date() + Duration::days(1);
            for (day, name) in
                holidays::holidays_between(from_day, to_day, &self.settings.holiday_state)
            {
                let mut entry = Entry::new(
                    kind.id,
                    name,
                    midnight(day),
                    midnight(day + Duration::days(1)),
                );
                entry.id = -(day.num_days_from_ce() as Id);
                entry.all_day = true;
                entry.reminder = None;
                let occ = Occurrence {
                    entry: entry.id,
                    date: day,
                    start: entry.start,
                    end: entry.end,
                };
                if occ.start < to && occ.end > from {
                    items.push(Item {
                        occ,
                        entry,
                        kind: kind.clone(),
                    });
                }
            }
        }
        items.sort_by_key(|i| (!i.entry.all_day, i.occ.start, i.entry.id));
        items
    }

    pub fn items_on(&self, day: NaiveDate) -> Vec<Item> {
        self.items(midnight(day), midnight(day + Duration::days(1)))
    }

    pub fn save_entry(&mut self, mut entry: Entry) -> Result<Id> {
        self.store.save_entry(&mut entry)?;
        let id = entry.id;
        match self.entries.iter_mut().find(|e| e.id == id) {
            Some(existing) => *existing = entry,
            None => self.entries.push(entry),
        }
        Ok(id)
    }

    pub fn delete_entry(&mut self, id: Id) -> Result<Option<Entry>> {
        self.store.delete_entry(id)?;
        let pos = self.entries.iter().position(|e| e.id == id);
        Ok(pos.map(|p| self.entries.remove(p)))
    }

    pub fn save_kind(&mut self, mut kind: Kind) -> Result<Id> {
        self.store.save_kind(&mut kind)?;
        let id = kind.id;
        match self.kinds.iter_mut().find(|k| k.id == id) {
            Some(existing) => *existing = kind,
            None => self.kinds.push(kind),
        }
        Ok(id)
    }

    /// Removes a user-made kind; its entries become plain appointments.
    pub fn delete_kind(&mut self, id: Id) -> Result<()> {
        if self.kind(id).is_some_and(|k| k.source.is_some()) {
            // A subscription: its entries go with it.
            self.replace_subscribed(id, Vec::new())?;
            self.store.delete_kind(id)?;
            self.kinds.retain(|k| k.id != id);
            return Ok(());
        }
        let fallback = self.builtin(Builtin::Event).map(|k| k.id);
        if let Some(fallback) = fallback {
            let moved: Vec<Entry> = self
                .entries
                .iter()
                .filter(|e| e.kind == id)
                .cloned()
                .map(|mut e| {
                    e.kind = fallback;
                    e
                })
                .collect();
            for e in moved {
                self.save_entry(e)?;
            }
        }
        self.store.delete_kind(id)?;
        self.kinds.retain(|k| k.id != id);
        Ok(())
    }

    pub fn save_routine(&mut self, mut routine: Routine) -> Result<Id> {
        self.store.save_routine(&mut routine)?;
        let id = routine.id;
        match self.routines.iter_mut().find(|r| r.id == id) {
            Some(existing) => *existing = routine,
            None => self.routines.push(routine),
        }
        Ok(id)
    }

    pub fn delete_routine(&mut self, id: Id) -> Result<()> {
        self.store.delete_routine(id)?;
        self.routines.retain(|r| r.id != id);
        Ok(())
    }

    pub fn save_settings(&self) -> Result<()> {
        self.settings.save(&self.store)
    }

    /// Past entries and finished tasks are drawn faded when the setting is on.
    /// Holidays between two days for the chosen federal state.
    pub fn holiday_dates(&self, from: NaiveDate, to: NaiveDate) -> Vec<NaiveDate> {
        if self.settings.holiday_state == "-" {
            return Vec::new();
        }
        holidays::holidays_between(from, to, &self.settings.holiday_state)
            .into_iter()
            .map(|(d, _)| d)
            .collect()
    }

    /// Vacation days taken (or planned) in `year`.
    pub fn vacation_used(&self, year: i32) -> f64 {
        let Some(kind) = self.builtin(Builtin::Vacation).map(|k| k.id) else {
            return 0.0;
        };
        let from = NaiveDate::from_ymd_opt(year, 1, 1).expect("valid date");
        let to = NaiveDate::from_ymd_opt(year + 1, 1, 1).expect("valid date");
        let holidays = self.holiday_dates(from, to);
        self.entries
            .iter()
            .filter(|e| e.kind == kind)
            .map(|e| kalender_core::vacation::days_used(e, from, to, &holidays))
            .sum()
    }

    pub fn faded(&self, item: &Item, now: NaiveDateTime) -> bool {
        self.settings.dim_past
            && (item.occ.end <= now && !(item.entry.all_day && item.occ.date == now.date())
                || item.done())
    }
}

pub fn today() -> NaiveDate {
    now().date()
}

/// The current time; KALENDER_NOW=2026-11-24T10:36 fixes it, e.g. for screenshots.
pub fn now() -> NaiveDateTime {
    thread_local! {
        static FIXED: Option<NaiveDateTime> = std::env::var("KALENDER_NOW")
            .ok()
            .and_then(|v| NaiveDateTime::parse_from_str(&v, "%Y-%m-%dT%H:%M").ok());
    }
    FIXED
        .with(|f| *f)
        .unwrap_or_else(|| Local::now().naive_local())
}

/// The shown name of a kind: built-in names are translated as long as the user kept them.
pub fn kind_label(kind: &Kind) -> String {
    if kind.builtin.is_none() {
        return kind.name.clone();
    }
    match kind.name.as_str() {
        "Appointment" => tr("Appointment"),
        "Meeting" => tr("Meeting"),
        "Task" => tr("Task"),
        "Vacation" => tr("Vacation"),
        "Birthday" => tr("Birthday"),
        "Holiday" => tr("Holiday"),
        other => other.to_owned(),
    }
}

/// The shown title: birthdays with a known year say how old the person turns.
pub fn display_title(entry: &Entry, date: NaiveDate) -> String {
    match entry.birth_year {
        Some(year) if kalender_core::vacation::age(year, date) > 0 => crate::i18n::trf(
            "{} turns {}",
            &[
                &entry.title,
                &kalender_core::vacation::age(year, date).to_string(),
            ],
        ),
        _ => entry.title.clone(),
    }
}
