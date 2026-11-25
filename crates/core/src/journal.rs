// SPDX-License-Identifier: GPL-3.0-or-later

//! The change journal for syncing devices (planung/KONZEPT-ABGLEICH.md).
//!
//! Every save is also recorded field by field in a [`Replica`]; the fields
//! live in the tables `sync_fields` and `sync_state` next to the calendar.
//! Only what is the same on all devices goes in: local ids, the visibility
//! of kinds, entries of subscribed calendars and device settings stay out.
//! Entries refer to their kind by the kind's uid, not by the local id.

use std::collections::BTreeMap;

use anyhow::Result;
use kalender_sync::{Change, DeviceId, Known, Replica, Stamp};
use rusqlite::{OptionalExtension, params};
use serde_json::Value;

use crate::model::{Entry, Kind, Routine};
use crate::store::Store;

/// Settings that are the same on all devices; the others stay per device.
pub const SHARED_SETTINGS: [&str; 7] = [
    "vacation_per_year",
    "holiday_state",
    "own_name",
    "own_email",
    "default_minutes",
    "default_reminder",
    "default_buffer",
];

const SETTINGS_OBJECT: &str = "shared";

pub(crate) fn create_tables(conn: &rusqlite::Connection) -> Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS sync_fields (tbl TEXT NOT NULL, object TEXT NOT NULL, field TEXT NOT NULL,
            value TEXT NOT NULL, millis INTEGER NOT NULL, counter INTEGER NOT NULL, device INTEGER NOT NULL,
            PRIMARY KEY (tbl, object, field))",
        [],
    )?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS sync_state (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
        [],
    )?;
    Ok(())
}

fn wall_millis() -> u64 {
    chrono::Utc::now().timestamp_millis().max(0) as u64
}

fn random_device() -> DeviceId {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_i64(chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default());
    h.write_u32(std::process::id());
    // SQLite stores signed 64-bit numbers; stay below that.
    h.finish() >> 1
}

/// An entry as all devices see it; `None` for entries of subscribed calendars.
pub(crate) fn entry_value(entry: &Entry, kinds: &[Kind]) -> Option<Value> {
    let kind = kinds.iter().find(|k| k.id == entry.kind);
    if kind.is_some_and(|k| k.source.is_some()) {
        return None;
    }
    let mut v = serde_json::to_value(entry).ok()?;
    let map = v.as_object_mut()?;
    map.remove("id");
    map.insert(
        "kind".into(),
        Value::String(kind.map(|k| k.uid.clone()).unwrap_or_default()),
    );
    Some(v)
}

pub(crate) fn kind_value(kind: &Kind) -> Option<Value> {
    let mut v = serde_json::to_value(kind).ok()?;
    let map = v.as_object_mut()?;
    map.remove("id");
    map.remove("visible");
    // Of a subscription only the link is shared; each device loads it itself.
    match &kind.source {
        Some(s) => map.insert("source".into(), serde_json::json!({ "url": s.url })),
        None => map.remove("source"),
    };
    Some(v)
}

pub(crate) fn routine_value(routine: &Routine) -> Option<Value> {
    let mut v = serde_json::to_value(routine).ok()?;
    v.as_object_mut()?.remove("id");
    Some(v)
}

impl Store {
    /// Loads the journal, or starts it (first start, or data from before syncing existed).
    pub(crate) fn start_journal(&self) -> Result<()> {
        let device: Option<DeviceId> = self
            .conn
            .query_row(
                "SELECT value FROM sync_state WHERE key = 'device'",
                [],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .and_then(|v| v.parse().ok());
        match device {
            Some(device) => {
                let state = |key: &str| -> Result<Option<String>> {
                    Ok(self
                        .conn
                        .query_row(
                            "SELECT value FROM sync_state WHERE key = ?1",
                            params![key],
                            |r| r.get(0),
                        )
                        .optional()?)
                };
                let clock: (u64, u32) = state("clock")?
                    .and_then(|v| serde_json::from_str(&v).ok())
                    .unwrap_or_default();
                let known: Known = state("known")?
                    .and_then(|v| serde_json::from_str(&v).ok())
                    .unwrap_or_default();
                let mut stmt = self.conn.prepare(
                    "SELECT tbl, object, field, value, millis, counter, device FROM sync_fields",
                )?;
                let fields = stmt
                    .query_map([], |r| {
                        Ok(Change {
                            table: r.get(0)?,
                            object: r.get(1)?,
                            field: r.get(2)?,
                            value: serde_json::from_str(&r.get::<_, String>(3)?)
                                .unwrap_or(Value::Null),
                            stamp: Stamp {
                                millis: r.get::<_, i64>(4)? as u64,
                                counter: r.get::<_, i64>(5)? as u32,
                                device: r.get::<_, i64>(6)? as u64,
                            },
                        })
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                *self.journal.borrow_mut() = Some(Replica::restore(device, clock, fields, known));
            }
            None => {
                let device = random_device();
                self.conn.execute(
                    "INSERT OR REPLACE INTO sync_state (key, value) VALUES ('device', ?1)",
                    params![device.to_string()],
                )?;
                *self.journal.borrow_mut() = Some(Replica::new(device));
                self.resync_all()?;
            }
        }
        Ok(())
    }

    /// This installation's device id (once the journal runs).
    pub fn device_id(&self) -> Option<DeviceId> {
        self.journal.borrow().as_ref().map(Replica::device)
    }

    /// All recorded fields (for tests and, later, for syncing).
    pub fn journal_changes(&self) -> Vec<Change> {
        self.journal
            .borrow()
            .as_ref()
            .map(Replica::all_changes)
            .unwrap_or_default()
    }

    /// Records one object (`None` = deleted) and saves the changed fields.
    pub(crate) fn journal(&self, table: &str, uid: &str, value: Option<Value>) -> Result<()> {
        if uid.is_empty() {
            return Ok(());
        }
        let changes = {
            let mut journal = self.journal.borrow_mut();
            let Some(replica) = journal.as_mut() else {
                return Ok(());
            };
            replica.record(
                table,
                uid,
                Some(&Value::Null),
                value.as_ref(),
                wall_millis(),
            )
        };
        self.persist(&changes)
    }

    pub(crate) fn journal_setting(&self, key: &str, value: &str) -> Result<()> {
        if !SHARED_SETTINGS.contains(&key) {
            return Ok(());
        }
        let changes = {
            let mut journal = self.journal.borrow_mut();
            let Some(replica) = journal.as_mut() else {
                return Ok(());
            };
            let current = replica.object("settings", SETTINGS_OBJECT);
            if current
                .as_ref()
                .and_then(|c| c.get(key))
                .and_then(Value::as_str)
                == Some(value)
            {
                return Ok(());
            }
            vec![replica.edit(
                "settings",
                SETTINGS_OBJECT,
                key,
                Value::String(value.into()),
                wall_millis(),
            )]
        };
        self.persist(&changes)
    }

    /// Compares the whole calendar with the journal and records every
    /// difference. Used on the first start and after restoring a backup, so
    /// the restored state wins over what other devices knew before.
    pub fn resync_all(&self) -> Result<()> {
        let kinds = self.kinds()?;
        let mut current: BTreeMap<&str, BTreeMap<String, Value>> = BTreeMap::new();
        current.insert(
            "kind",
            kinds
                .iter()
                .filter_map(|k| Some((k.uid.clone(), kind_value(k)?)))
                .collect(),
        );
        current.insert(
            "entry",
            self.entries()?
                .iter()
                .filter_map(|e| Some((e.uid.clone(), entry_value(e, &kinds)?)))
                .collect(),
        );
        current.insert(
            "routine",
            self.routines()?
                .iter()
                .filter_map(|r| Some((r.uid.clone(), routine_value(r)?)))
                .collect(),
        );
        for (table, objects) in &current {
            let known: Vec<String> = self
                .journal
                .borrow()
                .as_ref()
                .map(|r| r.objects(table).into_keys().collect())
                .unwrap_or_default();
            for uid in known.iter().filter(|u| !objects.contains_key(*u)) {
                self.journal(table, uid, None)?;
            }
            for (uid, value) in objects {
                self.journal(table, uid, Some(value.clone()))?;
            }
        }
        for key in SHARED_SETTINGS {
            if let Some(value) = self.setting(key)? {
                self.journal_setting(key, &value)?;
            }
        }
        Ok(())
    }

    fn persist(&self, changes: &[Change]) -> Result<()> {
        if changes.is_empty() {
            return Ok(());
        }
        let tx = self.conn.unchecked_transaction()?;
        for c in changes {
            tx.execute(
                "INSERT OR REPLACE INTO sync_fields (tbl, object, field, value, millis, counter, device)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    c.table,
                    c.object,
                    c.field,
                    c.value.to_string(),
                    c.stamp.millis as i64,
                    c.stamp.counter as i64,
                    c.stamp.device as i64
                ],
            )?;
        }
        let (clock, known) = {
            let journal = self.journal.borrow();
            let r = journal.as_ref().expect("journal");
            (r.clock_state(), r.known().clone())
        };
        tx.execute(
            "INSERT OR REPLACE INTO sync_state (key, value) VALUES ('clock', ?1)",
            params![serde_json::to_string(&clock)?],
        )?;
        tx.execute(
            "INSERT OR REPLACE INTO sync_state (key, value) VALUES ('known', ?1)",
            params![serde_json::to_string(&known)?],
        )?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Source, at};
    use chrono::NaiveDate;

    fn day() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 26).unwrap()
    }

    fn object(store: &Store, table: &str, uid: &str) -> Option<Value> {
        store.journal.borrow().as_ref().unwrap().object(table, uid)
    }

    #[test]
    fn saving_and_deleting_is_journaled_with_kind_uids() {
        let store = Store::in_memory().unwrap();
        // The built-in kinds are in the journal from the start.
        assert!(object(&store, "kind", "builtin-event").is_some());
        let event = store.kinds().unwrap()[0].clone();

        let mut e = Entry::new(event.id, "Zahnarzt", at(day(), 15, 0), at(day(), 16, 0));
        e.add_subtask("Unterlagen");
        store.save_entry(&mut e).unwrap();
        let v = object(&store, "entry", &e.uid).unwrap();
        assert_eq!(v["kind"], "builtin-event");
        assert_eq!(v["title"], "Zahnarzt");
        assert!(v.get("id").is_none());
        assert_eq!(v["subtasks"][0]["text"], "Unterlagen");

        let before = store.journal_changes().len();
        store.save_entry(&mut e).unwrap(); // nothing changed
        assert_eq!(store.journal_changes().len(), before);

        store.delete_entry(e.id).unwrap();
        assert_eq!(object(&store, "entry", &e.uid), None);
    }

    #[test]
    fn local_things_stay_out() {
        let store = Store::in_memory().unwrap();
        let mut kind = store.kinds().unwrap()[0].clone();
        let before = store.journal_changes();
        kind.visible = false; // shown or hidden is per device
        store.save_kind(&mut kind).unwrap();
        store.set_setting("language", "de").unwrap(); // per device
        assert_eq!(store.journal_changes(), before);

        store.set_setting("vacation_per_year", "30").unwrap();
        assert_eq!(
            object(&store, "settings", "shared").unwrap()["vacation_per_year"],
            "30"
        );

        // Entries of subscribed calendars are loaded by every device itself.
        let mut sub = Kind {
            id: 0,
            uid: String::new(),
            name: "Müll".into(),
            color: "#2a9db0".into(),
            icon: "globe".into(),
            builtin: None,
            visible: true,
            source: Some(Source {
                url: "https://example.org/m.ics".into(),
                updated: None,
                error: Some("x".into()),
            }),
        };
        store.save_kind(&mut sub).unwrap();
        let k = object(&store, "kind", &sub.uid).unwrap();
        assert_eq!(
            k["source"],
            serde_json::json!({ "url": "https://example.org/m.ics" })
        );
        let mut entries = vec![Entry::new(
            sub.id,
            "Biotonne",
            at(day(), 0, 0),
            at(day(), 0, 0),
        )];
        store.replace_entries_of_kind(sub.id, &mut entries).unwrap();
        assert_eq!(object(&store, "entry", &entries[0].uid), None);
    }

    #[test]
    fn journal_survives_a_restart_and_restoring_a_backup_counts_as_new() {
        let dir = std::env::temp_dir().join(format!("kalender-journal-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (db, backup) = (dir.join("k.sqlite"), dir.join("b.sqlite"));
        let _ = std::fs::remove_file(&db);
        let _ = std::fs::remove_file(&backup);

        let store = Store::open(&db).unwrap();
        let device = store.device_id().unwrap();
        let kind = store.kinds().unwrap()[0].id;
        let mut a = Entry::new(kind, "Alt", at(day(), 9, 0), at(day(), 10, 0));
        store.save_entry(&mut a).unwrap();
        store.backup_to(&backup).unwrap();
        let mut b = Entry::new(kind, "Neu", at(day(), 11, 0), at(day(), 12, 0));
        store.save_entry(&mut b).unwrap();
        drop(store);

        let store = Store::open(&db).unwrap();
        assert_eq!(store.device_id(), Some(device));
        assert!(object(&store, "entry", &b.uid).is_some());

        store.restore_from(&backup).unwrap();
        assert!(object(&store, "entry", &a.uid).is_some());
        assert_eq!(object(&store, "entry", &b.uid), None, "restored state wins");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
