// SPDX-License-Identifier: GPL-3.0-or-later

//! Tests for the rules in planung/KONZEPT-ABGLEICH.md, section 5.

use super::*;
use serde_json::json;

const T: &str = "entry";

fn entry(title: &str, location: &str) -> Value {
    json!({ "title": title, "location": location, "start": "2026-11-26T15:00:00", "subtasks": [], "done": {} })
}

/// Lets `a` and `b` exchange everything, in both directions.
fn sync(a: &mut Replica, b: &mut Replica) {
    let to_b = a.changes_for(b.known());
    let to_a = b.changes_for(a.known());
    b.apply(&to_b);
    a.apply(&to_a);
}

fn title(r: &Replica, uid: &str) -> Option<String> {
    r.object(T, uid)
        .and_then(|v| v["title"].as_str().map(String::from))
}

#[test]
fn clock_never_goes_back() {
    let mut c = Clock::new(1);
    let a = c.tick(1000);
    let b = c.tick(900); // wall clock went back
    assert!(b > a);
    c.observe(Stamp {
        millis: 5000,
        counter: 3,
        device: 2,
    });
    let d = c.tick(1000);
    assert!(
        d > Stamp {
            millis: 5000,
            counter: 3,
            device: 2
        }
    );
}

#[test]
fn fields_merge_separately() {
    let mut pc = Replica::new(1);
    let mut phone = Replica::new(2);
    pc.record(T, "z", None, Some(&entry("Zahnarzt", "")), 1000);
    sync(&mut pc, &mut phone);

    // Phone changes the title, PC the place, without seeing each other.
    phone.record(
        T,
        "z",
        Some(&entry("Zahnarzt", "")),
        Some(&entry("Zahnarzt Dr. Sommer", "")),
        2000,
    );
    pc.record(
        T,
        "z",
        Some(&entry("Zahnarzt", "")),
        Some(&entry("Zahnarzt", "Praxis am Markt")),
        2100,
    );
    sync(&mut pc, &mut phone);

    for r in [&pc, &phone] {
        let v = r.object(T, "z").unwrap();
        assert_eq!(v["title"], "Zahnarzt Dr. Sommer");
        assert_eq!(v["location"], "Praxis am Markt");
    }
}

#[test]
fn newer_change_of_the_same_field_wins_even_with_a_wrong_clock() {
    let mut pc = Replica::new(1);
    let mut phone = Replica::new(2);
    pc.record(T, "z", None, Some(&entry("A", "")), 10_000_000);
    sync(&mut pc, &mut phone);
    // The phone's clock is an hour behind, but it changes the title after
    // having seen the PC's version: its change must win.
    phone.record(
        T,
        "z",
        Some(&entry("A", "")),
        Some(&entry("B", "")),
        10_000_000 - 3_600_000,
    );
    sync(&mut pc, &mut phone);
    assert_eq!(title(&pc, "z").as_deref(), Some("B"));
    assert_eq!(title(&phone, "z").as_deref(), Some("B"));
}

#[test]
fn subtasks_added_and_ticked_on_two_devices_all_stay() {
    let base = json!({ "title": "Putzen", "subtasks": [{ "id": "a", "text": "Bad" }], "done": {} });
    let mut pc = Replica::new(1);
    let mut phone = Replica::new(2);
    pc.record(T, "p", None, Some(&base), 1000);
    sync(&mut pc, &mut phone);

    let on_pc = json!({ "title": "Putzen",
        "subtasks": [{ "id": "a", "text": "Bad" }, { "id": "b", "text": "Küche" }],
        "done": { "2026-11-24": ["a"] } });
    let on_phone = json!({ "title": "Putzen",
        "subtasks": [{ "id": "a", "text": "Bad" }, { "id": "c", "text": "Fenster" }],
        "done": { "2026-11-24": ["c"] } });
    pc.record(T, "p", Some(&base), Some(&on_pc), 2000);
    phone.record(T, "p", Some(&base), Some(&on_phone), 2000);
    sync(&mut pc, &mut phone);

    let v = pc.object(T, "p").unwrap();
    let texts: Vec<&str> = v["subtasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["text"].as_str().unwrap())
        .collect();
    assert_eq!(texts, vec!["Bad", "Küche", "Fenster"]);
    assert_eq!(v["done"]["2026-11-24"], json!(["a", "c"]));
    assert_eq!(pc.object(T, "p"), phone.object(T, "p"));
}

#[test]
fn removing_a_tick_and_a_subtask() {
    let with = json!({ "title": "P", "subtasks": [{ "id": "a", "text": "Bad" }, { "id": "b", "text": "Flur" }], "done": { "2026-11-24": ["a"] } });
    let without = json!({ "title": "P", "subtasks": [{ "id": "b", "text": "Flur" }], "done": {} });
    let mut r = Replica::new(1);
    r.record(T, "p", None, Some(&with), 1000);
    r.record(T, "p", Some(&with), Some(&without), 2000);
    let v = r.object(T, "p").unwrap();
    assert_eq!(v["subtasks"], json!([{ "id": "b", "text": "Flur" }]));
    assert_eq!(v["done"], json!({}));
}

#[test]
fn deleting_wins_over_older_changes() {
    let mut pc = Replica::new(1);
    let mut phone = Replica::new(2);
    pc.record(T, "z", None, Some(&entry("Zahnarzt", "")), 1000);
    sync(&mut pc, &mut phone);
    phone.record(
        T,
        "z",
        Some(&entry("Zahnarzt", "")),
        Some(&entry("Zahnarzt", "Markt")),
        2000,
    );
    pc.record(T, "z", Some(&entry("Zahnarzt", "")), None, 3000);
    sync(&mut pc, &mut phone);
    assert_eq!(pc.object(T, "z"), None);
    assert_eq!(phone.object(T, "z"), None);
}

#[test]
fn a_change_after_deleting_keeps_the_entry() {
    let mut pc = Replica::new(1);
    let mut phone = Replica::new(2);
    pc.record(T, "z", None, Some(&entry("Zahnarzt", "")), 1000);
    sync(&mut pc, &mut phone);
    pc.record(T, "z", Some(&entry("Zahnarzt", "")), None, 2000);
    phone.record(
        T,
        "z",
        Some(&entry("Zahnarzt", "")),
        Some(&entry("Zahnarzt", "Markt")),
        3000,
    );
    sync(&mut pc, &mut phone);
    assert_eq!(title(&pc, "z").as_deref(), Some("Zahnarzt"));
    assert_eq!(pc.object(T, "z"), phone.object(T, "z"));
}

#[test]
fn undo_after_delete_brings_everything_back() {
    let mut pc = Replica::new(1);
    let mut phone = Replica::new(2);
    let e = entry("Zahnarzt", "Markt");
    pc.record(T, "z", None, Some(&e), 1000);
    sync(&mut pc, &mut phone);
    pc.record(T, "z", Some(&e), None, 2000);
    sync(&mut pc, &mut phone);
    assert_eq!(phone.object(T, "z"), None);
    pc.record(T, "z", None, Some(&e), 3000); // undo saves it again
    sync(&mut pc, &mut phone);
    assert_eq!(phone.object(T, "z").unwrap()["location"], "Markt");
}

#[test]
fn changes_travel_through_a_third_device() {
    let (mut pc, mut phone, mut tablet) = (Replica::new(1), Replica::new(2), Replica::new(3));
    phone.record(T, "x", None, Some(&entry("Vom Handy", "")), 1000);
    sync(&mut phone, &mut pc);
    // The phone is away; the tablet meets only the PC.
    sync(&mut pc, &mut tablet);
    assert_eq!(title(&tablet, "x").as_deref(), Some("Vom Handy"));
}

#[test]
fn nothing_is_sent_twice() {
    let (mut pc, mut phone) = (Replica::new(1), Replica::new(2));
    pc.record(T, "x", None, Some(&entry("A", "")), 1000);
    sync(&mut pc, &mut phone);
    assert!(pc.changes_for(phone.known()).is_empty());
    assert!(phone.changes_for(pc.known()).is_empty());
}

/// Like serde's defaults in the app: missing lists are empty.
fn with_defaults(mut v: Value) -> Value {
    if v.get("subtasks").is_none() {
        v["subtasks"] = json!([]);
    }
    if v.get("done").is_none() {
        v["done"] = json!({});
    }
    v
}

/// A small deterministic random generator, so failures can be repeated.
struct Dice(u64);
impl Dice {
    fn next(&mut self, n: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) % n
    }
}

/// Three devices with wrong clocks make random changes and meet in random
/// pairs, sometimes losing the connection half way. In the end, after
/// everybody met everybody, all three must hold exactly the same calendar.
#[test]
fn three_devices_always_end_up_the_same() {
    for seed in 1..=40 {
        let mut dice = Dice(seed);
        let mut devices = [Replica::new(11), Replica::new(22), Replica::new(33)];
        let offsets: [i64; 3] = [0, -3_600_000, 90_000]; // one hour behind, 1.5 min ahead
        let uids = ["a", "b", "c", "d"];
        let mut wall: i64 = 1_800_000_000_000;
        for _ in 0..400 {
            wall += dice.next(5000) as i64;
            let who = dice.next(3) as usize;
            let now = (wall + offsets[who]) as u64;
            let r = &mut devices[who];
            let uid = uids[dice.next(4) as usize];
            let old = r.object(T, uid);
            match dice.next(10) {
                0..=1 => {
                    r.record(T, uid, old.as_ref(), None, now);
                }
                2..=4 => {
                    let mut v = with_defaults(old.clone().unwrap_or_else(|| entry("neu", "")));
                    v["title"] = json!(format!("t{}", dice.next(100)));
                    r.record(T, uid, old.as_ref(), Some(&v), now);
                }
                5..=6 => {
                    let mut v = with_defaults(old.clone().unwrap_or_else(|| entry("neu", "")));
                    v["location"] = json!(format!("o{}", dice.next(100)));
                    r.record(T, uid, old.as_ref(), Some(&v), now);
                }
                7..=8 => {
                    let mut v = with_defaults(old.clone().unwrap_or_else(|| entry("neu", "")));
                    let list = v["subtasks"].as_array_mut().unwrap();
                    if list.len() < 4 || dice.next(2) == 0 {
                        list.push(
                            json!({ "id": format!("{who}-{}", dice.next(1000)), "text": "x" }),
                        );
                    } else {
                        list.remove(dice.next(list.len() as u64) as usize);
                    }
                    r.record(T, uid, old.as_ref(), Some(&v), now);
                }
                _ => {
                    let mut v = with_defaults(old.clone().unwrap_or_else(|| entry("neu", "")));
                    let ids: Vec<String> = v["subtasks"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|s| s["id"].as_str().unwrap().to_owned())
                        .collect();
                    if let Some(id) = ids.get(dice.next(ids.len().max(1) as u64) as usize) {
                        let day = v["done"]
                            .as_object_mut()
                            .unwrap()
                            .entry("2026-11-24")
                            .or_insert(json!([]));
                        let list = day.as_array_mut().unwrap();
                        if let Some(p) = list.iter().position(|x| x == id) {
                            list.remove(p);
                        } else {
                            list.push(json!(id));
                        }
                    }
                    r.record(T, uid, old.as_ref(), Some(&v), now);
                }
            }
            // Sometimes two devices meet; sometimes the connection breaks.
            if dice.next(4) == 0 {
                let (x, y) = (dice.next(3) as usize, dice.next(3) as usize);
                if x != y {
                    let mut changes = devices[x].changes_for(devices[y].known());
                    if dice.next(3) == 0 {
                        changes.truncate(dice.next(changes.len() as u64 + 1) as usize);
                    }
                    devices[y].apply(&changes);
                }
            }
        }
        // Everybody meets everybody, twice.
        for _ in 0..2 {
            for (x, y) in [(0, 1), (1, 2), (2, 0)] {
                let (a, b) = if x < y {
                    let (l, r) = devices.split_at_mut(y);
                    (&mut l[x], &mut r[0])
                } else {
                    let (l, r) = devices.split_at_mut(x);
                    (&mut r[0], &mut l[y])
                };
                sync(a, b);
            }
        }
        let first = devices[0].objects(T);
        assert_eq!(first, devices[1].objects(T), "seed {seed}");
        assert_eq!(first, devices[2].objects(T), "seed {seed}");
        assert_eq!(
            devices[0].all_changes(),
            devices[1].all_changes(),
            "seed {seed}"
        );
    }
}
