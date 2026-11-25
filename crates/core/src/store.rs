// SPDX-License-Identifier: GPL-3.0-or-later

//! Local storage in SQLite. Every object is stored as JSON next to its id;
//! a person's calendar is small enough to be held in memory as a whole.

use std::cell::RefCell;
use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Serialize, de::DeserializeOwned};

use crate::journal::{entry_value, kind_value, routine_value};
use crate::model::{Builtin, Entry, Id, Kind, Routine};

pub struct Store {
    pub(crate) conn: Connection,
    /// Field-wise record of all changes, for syncing devices; `None` while starting.
    pub(crate) journal: RefCell<Option<kalender_sync::Replica>>,
}

const TABLES: [&str; 3] = ["kinds", "entries", "routines"];

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        Self::init(conn)
    }

    pub fn in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        for table in TABLES {
            conn.execute(
                &format!("CREATE TABLE IF NOT EXISTS {table} (id INTEGER PRIMARY KEY, data TEXT NOT NULL)"),
                [],
            )?;
        }
        conn.execute(
            "CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
            [],
        )?;
        crate::journal::create_tables(&conn)?;
        crate::devices::create_tables(&conn)?;
        let store = Self {
            conn,
            journal: RefCell::new(None),
        };
        if store.kinds()?.is_empty() {
            for kind in default_kinds() {
                let mut kind = kind;
                store.save_kind(&mut kind)?;
            }
        }
        store.give_uids()?;
        store.start_journal()?;
        Ok(store)
    }

    /// Kinds and routines from before 25.11.2026 have no uid yet.
    fn give_uids(&self) -> Result<()> {
        for mut kind in self.kinds()? {
            if kind.uid.is_empty() {
                kind.uid = match kind.builtin {
                    Some(b) => builtin_uid(b),
                    None => crate::model::new_uid(),
                };
                self.save_kind(&mut kind)?;
            }
        }
        for mut routine in self.routines()? {
            if routine.uid.is_empty() {
                routine.uid = crate::model::new_uid();
                self.save_routine(&mut routine)?;
            }
        }
        Ok(())
    }

    fn all<T: DeserializeOwned>(&self, table: &str) -> Result<Vec<T>> {
        let mut stmt = self
            .conn
            .prepare(&format!("SELECT id, data FROM {table} ORDER BY id"))?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, Id>(0)?, r.get::<_, String>(1)?)))?;
        let mut out = Vec::new();
        for row in rows {
            let (id, data) = row?;
            match serde_json::from_str(&data) {
                Ok(v) => out.push(v),
                Err(e) => tracing::warn!("skipping unreadable row {id} in {table}: {e}"),
            }
        }
        Ok(out)
    }

    /// Inserts when `*id == 0` (and sets the new id), otherwise replaces.
    fn put<T: Serialize>(&self, table: &str, id: &mut Id, value: impl Fn(Id) -> T) -> Result<()> {
        if *id == 0 {
            self.conn
                .execute(&format!("INSERT INTO {table} (data) VALUES ('{{}}')"), [])?;
            *id = self.conn.last_insert_rowid();
        }
        let data = serde_json::to_string(&value(*id))?;
        self.conn.execute(
            &format!("INSERT OR REPLACE INTO {table} (id, data) VALUES (?1, ?2)"),
            params![*id, data],
        )?;
        Ok(())
    }

    fn remove(&self, table: &str, id: Id) -> Result<()> {
        self.conn
            .execute(&format!("DELETE FROM {table} WHERE id = ?1"), params![id])?;
        Ok(())
    }

    pub fn kinds(&self) -> Result<Vec<Kind>> {
        self.all("kinds")
    }

    pub fn save_kind(&self, kind: &mut Kind) -> Result<()> {
        if kind.uid.is_empty() {
            kind.uid = self
                .kinds()?
                .into_iter()
                .find(|k| k.id == kind.id && kind.id != 0)
                .map(|k| k.uid)
                .filter(|u| !u.is_empty())
                .unwrap_or_else(|| match kind.builtin {
                    Some(b) => builtin_uid(b),
                    None => crate::model::new_uid(),
                });
        }
        let mut id = kind.id;
        self.put("kinds", &mut id, |id| Kind { id, ..kind.clone() })?;
        kind.id = id;
        self.journal("kind", &kind.uid, kind_value(kind))
    }

    pub fn delete_kind(&self, id: Id) -> Result<()> {
        let uid = self
            .kinds()?
            .into_iter()
            .find(|k| k.id == id)
            .map(|k| k.uid);
        self.remove("kinds", id)?;
        match uid {
            Some(uid) => self.journal("kind", &uid, None),
            None => Ok(()),
        }
    }

    pub fn entries(&self) -> Result<Vec<Entry>> {
        self.all("entries")
    }

    pub fn save_entry(&self, entry: &mut Entry) -> Result<()> {
        self.put_entry(entry)?;
        if let Some(value) = entry_value(entry, &self.kinds()?) {
            self.journal("entry", &entry.uid, Some(value))?;
        }
        Ok(())
    }

    fn put_entry(&self, entry: &mut Entry) -> Result<()> {
        let mut id = entry.id;
        self.put("entries", &mut id, |id| Entry {
            id,
            ..entry.clone()
        })?;
        entry.id = id;
        Ok(())
    }

    pub fn delete_entry(&self, id: Id) -> Result<()> {
        let uid: Option<String> = self
            .conn
            .query_row(
                "SELECT json_extract(data, '$.uid') FROM entries WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .optional()?;
        self.remove("entries", id)?;
        match uid {
            Some(uid) => self.journal("entry", &uid, None),
            None => Ok(()),
        }
    }

    /// Replaces all entries of one kind in one go (used for subscribed calendars).
    pub fn replace_entries_of_kind(&self, kind: Id, entries: &mut [Entry]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "DELETE FROM entries WHERE json_extract(data, '$.kind') = ?1",
            params![kind],
        )?;
        // Subscribed entries are not journaled: every device loads them itself.
        for entry in entries.iter_mut() {
            entry.id = 0;
            self.put_entry(entry)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Writes a complete, consistent copy of the database to `path` (must not exist yet).
    pub fn backup_to(&self, path: &Path) -> Result<()> {
        let target = path.to_str().context("backup path is not valid UTF-8")?;
        self.conn
            .execute("VACUUM INTO ?1", params![target])
            .with_context(|| format!("writing backup {}", path.display()))?;
        Ok(())
    }

    /// Replaces everything (kinds, entries, routines, settings) with the content of a backup.
    pub fn restore_from(&self, path: &Path) -> Result<()> {
        let source = path.to_str().context("backup path is not valid UTF-8")?;
        // Check it really is one of our databases before touching anything.
        let check = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .with_context(|| format!("opening {}", path.display()))?;
        for table in TABLES.iter().chain(&["settings"]) {
            check
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| {
                    r.get::<_, i64>(0)
                })
                .context("this file is not a calendar backup")?;
        }
        drop(check);
        self.conn
            .execute("ATTACH DATABASE ?1 AS backup", params![source])?;
        let result = (|| -> Result<()> {
            let tx = self.conn.unchecked_transaction()?;
            for table in TABLES.iter().chain(&["settings"]) {
                tx.execute(&format!("DELETE FROM main.{table}"), [])?;
                tx.execute(
                    &format!("INSERT INTO main.{table} SELECT * FROM backup.{table}"),
                    [],
                )?;
            }
            tx.commit()?;
            Ok(())
        })();
        self.conn.execute("DETACH DATABASE backup", [])?;
        result?;
        // The restored state is new for all other devices.
        self.give_uids()?;
        self.resync_all()
    }

    pub fn routines(&self) -> Result<Vec<Routine>> {
        self.all("routines")
    }

    pub fn save_routine(&self, routine: &mut Routine) -> Result<()> {
        if routine.uid.is_empty() {
            routine.uid = self
                .routines()?
                .into_iter()
                .find(|r| r.id == routine.id && routine.id != 0)
                .map(|r| r.uid)
                .filter(|u| !u.is_empty())
                .unwrap_or_else(crate::model::new_uid);
        }
        let mut id = routine.id;
        self.put("routines", &mut id, |id| Routine {
            id,
            ..routine.clone()
        })?;
        routine.id = id;
        self.journal("routine", &routine.uid, routine_value(routine))
    }

    pub fn delete_routine(&self, id: Id) -> Result<()> {
        let uid = self
            .routines()?
            .into_iter()
            .find(|r| r.id == id)
            .map(|r| r.uid);
        self.remove("routines", id)?;
        match uid {
            Some(uid) => self.journal("routine", &uid, None),
            None => Ok(()),
        }
    }

    pub fn setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                params![key],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
            params![key, value],
        )?;
        self.journal_setting(key, value)
    }
}

/// Termin, Meeting, Aufgabe, Urlaub, Geburtstag, Feiertag – names are English
/// in the database and translated for display while unchanged.
/// The same uid on every device, so built-in kinds match up when syncing.
pub fn builtin_uid(builtin: Builtin) -> String {
    format!("builtin-{}", format!("{builtin:?}").to_lowercase())
}

pub fn default_kinds() -> Vec<Kind> {
    let kind = |name: &str, color: &str, icon: &str, builtin| Kind {
        id: 0,
        uid: builtin_uid(builtin),
        name: name.to_owned(),
        color: color.to_owned(),
        icon: icon.to_owned(),
        builtin: Some(builtin),
        visible: true,
        source: None,
    };
    vec![
        kind("Appointment", "#3b82c4", "calendar", Builtin::Event),
        kind("Meeting", "#7a6ff0", "people", Builtin::Meeting),
        kind("Task", "#2e9d76", "task", Builtin::Task),
        kind("Vacation", "#e8a33d", "sun", Builtin::Vacation),
        kind("Birthday", "#c2457a", "cake", Builtin::Birthday),
        kind("Holiday", "#8a8a85", "flag", Builtin::Holiday),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::at;
    use chrono::NaiveDate;

    #[test]
    fn seeds_kinds_and_stores_entries() {
        let store = Store::in_memory().unwrap();
        let kinds = store.kinds().unwrap();
        assert_eq!(kinds.len(), 6);
        assert!(kinds.iter().all(|k| k.id > 0));

        let day = NaiveDate::from_ymd_opt(2026, 11, 24).unwrap();
        let mut e = Entry::new(kinds[2].id, "Putzen", at(day, 15, 0), at(day, 17, 0));
        e.add_subtask("Staubsaugen");
        store.save_entry(&mut e).unwrap();
        assert!(e.id > 0);
        e.set_done(day, 0, true);
        store.save_entry(&mut e).unwrap();

        let loaded = store.entries().unwrap();
        assert_eq!(loaded, vec![e.clone()]);
        store.delete_entry(e.id).unwrap();
        assert!(store.entries().unwrap().is_empty());
    }

    #[test]
    fn settings_roundtrip() {
        let store = Store::in_memory().unwrap();
        assert_eq!(store.setting("x").unwrap(), None);
        store.set_setting("x", "1").unwrap();
        store.set_setting("x", "2").unwrap();
        assert_eq!(store.setting("x").unwrap().as_deref(), Some("2"));
    }

    #[test]
    fn routines_roundtrip() {
        let store = Store::in_memory().unwrap();
        let mut r = Routine {
            uid: String::new(),
            id: 0,
            name: "Putzen".into(),
            minutes: 120,
            subtasks: vec!["Bad".into()],
        };
        store.save_routine(&mut r).unwrap();
        assert_eq!(store.routines().unwrap(), vec![r.clone()]);
        store.delete_routine(r.id).unwrap();
        assert!(store.routines().unwrap().is_empty());
    }

    #[test]
    fn replace_entries_of_one_kind() {
        let store = Store::in_memory().unwrap();
        let day = NaiveDate::from_ymd_opt(2026, 11, 24).unwrap();
        let mut own = Entry::new(1, "Eigener", at(day, 9, 0), at(day, 10, 0));
        store.save_entry(&mut own).unwrap();
        let mut old = Entry::new(9, "Alt", at(day, 9, 0), at(day, 10, 0));
        store.save_entry(&mut old).unwrap();
        let mut fresh = vec![
            Entry::new(9, "Neu 1", at(day, 11, 0), at(day, 12, 0)),
            Entry::new(9, "Neu 2", at(day, 13, 0), at(day, 14, 0)),
        ];
        store.replace_entries_of_kind(9, &mut fresh).unwrap();
        let titles: Vec<String> = store
            .entries()
            .unwrap()
            .into_iter()
            .map(|e| e.title)
            .collect();
        assert_eq!(titles, vec!["Eigener", "Neu 1", "Neu 2"]);
    }

    #[test]
    fn backup_and_restore() {
        let dir = std::env::temp_dir().join(format!("kalender-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("backup.sqlite");
        let _ = std::fs::remove_file(&file);
        let day = NaiveDate::from_ymd_opt(2026, 11, 24).unwrap();

        let store = Store::in_memory().unwrap();
        let mut e = Entry::new(1, "Gesichert", at(day, 9, 0), at(day, 10, 0));
        store.save_entry(&mut e).unwrap();
        store.set_setting("own_name", "Lena").unwrap();
        store.backup_to(&file).unwrap();

        let other = Store::in_memory().unwrap();
        let mut x = Entry::new(1, "Wird ersetzt", at(day, 9, 0), at(day, 10, 0));
        other.save_entry(&mut x).unwrap();
        other.restore_from(&file).unwrap();
        let titles: Vec<String> = other
            .entries()
            .unwrap()
            .into_iter()
            .map(|e| e.title)
            .collect();
        assert_eq!(titles, vec!["Gesichert"]);
        assert_eq!(other.setting("own_name").unwrap().as_deref(), Some("Lena"));

        // A file that is no backup changes nothing.
        let junk = dir.join("junk.sqlite");
        std::fs::write(&junk, b"nope").unwrap();
        assert!(other.restore_from(&junk).is_err());
        assert_eq!(other.entries().unwrap().len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
