// SPDX-License-Identifier: GPL-3.0-or-later

//! Turning an object (as JSON) into fields and back.
//!
//! Top-level keys become fields. Two lists are split further, so that
//! changes on different devices don't overwrite each other:
//!
//! - `subtasks` (`[{id, text}]`) → `subtasks/<id>` = `{text, pos}`
//! - `done` (`{date: [ids]}`) → `done/<date>/<id>` = `true`
//!
//! A removed field is stored as `null` and left out again on the way back,
//! so the object falls back to its defaults.

use serde_json::{Map, Value, json};

const SUBTASKS: &str = "subtasks";
const DONE: &str = "done";

/// The fields of an object.
pub fn flatten(object: &Value) -> Map<String, Value> {
    let mut out = Map::new();
    let Some(map) = object.as_object() else {
        return out;
    };
    for (key, value) in map {
        match (key.as_str(), value) {
            (SUBTASKS, Value::Array(items)) => {
                for (pos, item) in items.iter().enumerate() {
                    if let (Some(id), Some(text)) =
                        (item.get("id").and_then(Value::as_str), item.get("text"))
                    {
                        out.insert(
                            format!("{SUBTASKS}/{id}"),
                            json!({ "text": text, "pos": pos }),
                        );
                    }
                }
            }
            (DONE, Value::Object(days)) => {
                for (date, ids) in days {
                    for id in ids
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                    {
                        out.insert(format!("{DONE}/{date}/{id}"), Value::Bool(true));
                    }
                }
            }
            _ => {
                out.insert(key.clone(), value.clone());
            }
        }
    }
    out
}

/// The object made from its fields.
pub fn unflatten(fields: &Map<String, Value>) -> Value {
    let mut out = Map::new();
    let mut subtasks: Vec<(u64, String, Value)> = Vec::new();
    let mut done: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    let mut has_subtasks = false;
    let mut has_done = false;
    for (field, value) in fields {
        if let Some(id) = field.strip_prefix("subtasks/") {
            has_subtasks = true;
            if let Some(text) = value.get("text") {
                let pos = value.get("pos").and_then(Value::as_u64).unwrap_or(u64::MAX);
                subtasks.push((pos, id.to_owned(), text.clone()));
            }
        } else if let Some(rest) = field.strip_prefix("done/") {
            has_done = true;
            if value == &Value::Bool(true)
                && let Some((date, id)) = rest.split_once('/')
            {
                done.entry(date.to_owned()).or_default().push(id.to_owned());
            }
        } else if !value.is_null() {
            out.insert(field.clone(), value.clone());
        }
    }
    // Same position on two devices (both added at the end): order by id.
    subtasks.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
    if has_subtasks || out.contains_key(SUBTASKS) {
        out.insert(
            SUBTASKS.into(),
            Value::Array(
                subtasks
                    .into_iter()
                    .map(|(_, id, text)| json!({ "id": id, "text": text }))
                    .collect(),
            ),
        );
    }
    let done: Map<String, Value> = done
        .into_iter()
        .map(|(date, mut ids)| {
            ids.sort();
            (date, Value::from(ids))
        })
        .collect();
    if has_done {
        out.insert(DONE.into(), Value::Object(done));
    }
    out.into()
}
