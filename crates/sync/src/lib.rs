// SPDX-License-Identifier: GPL-3.0-or-later

//! Sync rules for Kalender (see planung/KONZEPT-ABGLEICH.md, section 5).
//!
//! All devices are equal; there is no main device. Every object (entry,
//! kind, routine, shared settings) is stored as a set of *fields*. Each field
//! keeps the value with the newest [`Stamp`]; stamps come from a hybrid
//! logical clock, so a wrong clock on one device can't make older changes
//! win over changes made after them. Devices exchange the fields the other
//! one has not seen yet ([`Replica::changes_for`]) and every device computes
//! the same result from the same changes, in any order.
//!
//! This crate has no storage and no network; `kalender-core` keeps the
//! fields in SQLite, the network part comes later.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

mod fields;

pub use fields::{flatten, unflatten};

/// A device, chosen at random once per installation.
pub type DeviceId = u64;

/// When and where a field was changed. Ordered by time, then counter, then
/// device, so every device orders all changes the same way.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct Stamp {
    /// Milliseconds since 1970 (logical: never goes backwards).
    pub millis: u64,
    /// Counts changes within the same millisecond.
    pub counter: u32,
    pub device: DeviceId,
}

/// Hybrid logical clock: wall time, but never behind anything already seen.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Clock {
    device: DeviceId,
    millis: u64,
    counter: u32,
}

impl Clock {
    pub fn new(device: DeviceId) -> Self {
        Self {
            device,
            millis: 0,
            counter: 0,
        }
    }

    /// A clock continuing from a saved state.
    pub fn resume(device: DeviceId, millis: u64, counter: u32) -> Self {
        Self {
            device,
            millis,
            counter,
        }
    }

    /// The stamp for a change made now; always newer than everything before.
    pub fn tick(&mut self, wall_millis: u64) -> Stamp {
        if wall_millis > self.millis {
            self.millis = wall_millis;
            self.counter = 0;
        } else {
            self.counter += 1;
        }
        Stamp {
            millis: self.millis,
            counter: self.counter,
            device: self.device,
        }
    }

    /// Takes note of a stamp from another device.
    pub fn observe(&mut self, stamp: Stamp) {
        if (stamp.millis, stamp.counter) > (self.millis, self.counter) {
            self.millis = stamp.millis;
            self.counter = stamp.counter;
        }
    }

    /// (millis, counter), for saving.
    pub fn state(&self) -> (u64, u32) {
        (self.millis, self.counter)
    }
}

/// One changed field, as stored and as sent to other devices.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Change {
    /// What kind of object: `entry`, `kind`, `routine`, `settings`.
    pub table: String,
    /// The object's uid, the same on every device.
    pub object: String,
    pub field: String,
    pub value: Value,
    pub stamp: Stamp,
}

/// Up to which stamp the changes of each device are known.
pub type Known = BTreeMap<DeviceId, Stamp>;

/// The field marking an object as deleted.
pub const DELETED: &str = "_deleted";

type Key = (String, String, String);

/// All fields one device knows, plus its clock.
#[derive(Clone, Debug, Default)]
pub struct Replica {
    device: DeviceId,
    clock: Clock,
    fields: BTreeMap<Key, (Value, Stamp)>,
    known: Known,
}

impl Replica {
    pub fn new(device: DeviceId) -> Self {
        Self {
            device,
            clock: Clock::new(device),
            fields: BTreeMap::new(),
            known: Known::new(),
        }
    }

    /// Rebuilds a replica from saved fields.
    pub fn restore(
        device: DeviceId,
        clock: (u64, u32),
        fields: impl IntoIterator<Item = Change>,
        known: Known,
    ) -> Self {
        let mut r = Self {
            device,
            clock: Clock::resume(device, clock.0, clock.1),
            fields: BTreeMap::new(),
            known,
        };
        for c in fields {
            r.fields
                .insert((c.table, c.object, c.field), (c.value, c.stamp));
        }
        r
    }

    pub fn device(&self) -> DeviceId {
        self.device
    }

    pub fn clock_state(&self) -> (u64, u32) {
        self.clock.state()
    }

    pub fn known(&self) -> &Known {
        &self.known
    }

    /// Records a local change of one field.
    pub fn edit(
        &mut self,
        table: &str,
        object: &str,
        field: &str,
        value: Value,
        wall_millis: u64,
    ) -> Change {
        let stamp = self.clock.tick(wall_millis);
        let change = Change {
            table: table.to_owned(),
            object: object.to_owned(),
            field: field.to_owned(),
            value,
            stamp,
        };
        self.fields.insert(
            (
                change.table.clone(),
                change.object.clone(),
                change.field.clone(),
            ),
            (change.value.clone(), stamp),
        );
        self.note(stamp);
        change
    }

    /// Records what changed between two versions of an object (`None` = not
    /// there). Creating or restoring writes every field, deleting sets
    /// [`DELETED`]. Returns the recorded changes.
    pub fn record(
        &mut self,
        table: &str,
        object: &str,
        old: Option<&Value>,
        new: Option<&Value>,
        wall_millis: u64,
    ) -> Vec<Change> {
        let Some(new) = new else {
            return if old.is_some() && self.object(table, object).is_some() {
                vec![self.edit(table, object, DELETED, Value::Bool(true), wall_millis)]
            } else {
                Vec::new()
            };
        };
        let new_fields = flatten(new);
        let old_fields = match old {
            // Compare with what the replica holds, so a restored object
            // (e.g. undo after delete) is written again in full.
            Some(_) => {
                let mut f = self.object_fields(table, object).unwrap_or_default();
                f.retain(|_, v| !v.is_null());
                f
            }
            None => Map::new(),
        };
        let mut out = Vec::new();
        for (field, value) in &new_fields {
            if old_fields.get(field) != Some(value) {
                out.push(self.edit(table, object, field, value.clone(), wall_millis));
            }
        }
        for field in old_fields.keys() {
            if !new_fields.contains_key(field) {
                out.push(self.edit(table, object, field, Value::Null, wall_millis));
            }
        }
        if self.is_deleted(table, object) {
            out.push(self.edit(table, object, DELETED, Value::Bool(false), wall_millis));
        }
        out
    }

    /// Takes over changes from another device. Returns the objects whose
    /// content changed, as (table, object).
    pub fn apply(&mut self, changes: &[Change]) -> BTreeSet<(String, String)> {
        let mut touched = BTreeSet::new();
        for c in changes {
            self.clock.observe(c.stamp);
            let key = (c.table.clone(), c.object.clone(), c.field.clone());
            let newer = self.fields.get(&key).is_none_or(|(_, s)| c.stamp > *s);
            if newer {
                self.fields.insert(key, (c.value.clone(), c.stamp));
                touched.insert((c.table.clone(), c.object.clone()));
            }
            self.note(c.stamp);
        }
        touched
    }

    /// The current value of one field and when it was set.
    pub fn field(&self, table: &str, object: &str, field: &str) -> Option<(&Value, Stamp)> {
        self.fields
            .get(&(table.to_owned(), object.to_owned(), field.to_owned()))
            .map(|(v, s)| (v, *s))
    }

    /// Everything the other device has not seen yet, oldest first. Taking
    /// over only the beginning of the list (connection lost) is safe.
    pub fn changes_for(&self, theirs: &Known) -> Vec<Change> {
        let mut out: Vec<Change> = self
            .fields
            .iter()
            .filter(|(_, (_, stamp))| theirs.get(&stamp.device).is_none_or(|k| stamp > k))
            .map(|((table, object, field), (value, stamp))| Change {
                table: table.clone(),
                object: object.clone(),
                field: field.clone(),
                value: value.clone(),
                stamp: *stamp,
            })
            .collect();
        out.sort_by_key(|c| c.stamp);
        out
    }

    /// The current content of an object, or `None` if it is unknown or deleted.
    pub fn object(&self, table: &str, object: &str) -> Option<Value> {
        self.object_fields(table, object).map(|f| unflatten(&f))
    }

    /// All objects of one table that are not deleted, by uid.
    pub fn objects(&self, table: &str) -> BTreeMap<String, Value> {
        let uids: BTreeSet<&String> = self
            .fields
            .keys()
            .filter(|(t, _, _)| t == table)
            .map(|(_, o, _)| o)
            .collect();
        uids.into_iter()
            .filter_map(|uid| self.object(table, uid).map(|v| (uid.clone(), v)))
            .collect()
    }

    /// All stored fields, for saving.
    pub fn all_changes(&self) -> Vec<Change> {
        self.changes_for(&Known::new())
    }

    /// Deleted wins over older changes; a change after the deletion keeps
    /// the object (it is evidently still needed).
    fn is_deleted(&self, table: &str, object: &str) -> bool {
        let mut deleted = None;
        let mut newest_other = None;
        for ((t, o, f), (v, s)) in self.object_range(table, object) {
            debug_assert!(t == table && o == object);
            if f == DELETED {
                if v == &Value::Bool(true) {
                    deleted = Some(*s);
                }
            } else {
                newest_other = newest_other.max(Some(*s));
            }
        }
        match deleted {
            Some(d) => newest_other.is_none_or(|o| d > o),
            None => false,
        }
    }

    fn object_fields(&self, table: &str, object: &str) -> Option<Map<String, Value>> {
        if self.is_deleted(table, object) {
            return None;
        }
        let fields: Map<String, Value> = self
            .object_range(table, object)
            .filter(|((_, _, f), _)| f != DELETED)
            .map(|((_, _, f), (v, _))| (f.clone(), v.clone()))
            .collect();
        (!fields.is_empty()).then_some(fields)
    }

    fn object_range<'a>(
        &'a self,
        table: &str,
        object: &str,
    ) -> impl Iterator<Item = (&'a Key, &'a (Value, Stamp))> {
        let start = (table.to_owned(), object.to_owned(), String::new());
        let table = table.to_owned();
        let object = object.to_owned();
        self.fields
            .range(start..)
            .take_while(move |((t, o, _), _)| *t == table && *o == object)
    }

    fn note(&mut self, stamp: Stamp) {
        let k = self.known.entry(stamp.device).or_default();
        if stamp > *k {
            *k = stamp;
        }
    }
}

#[cfg(test)]
mod tests;
