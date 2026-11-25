// SPDX-License-Identifier: GPL-3.0-or-later

//! Two devices on this computer, talking over real TCP connections.

use std::net::{SocketAddr, TcpListener};
use std::sync::Mutex;
use std::thread;

use kalender_sync::Replica;
use serde_json::json;

use super::*;
use crate::pair::{Offer, pair, parse_qr};
use crate::sync::{Accepted, accept, sync, sync_limited};

struct Device {
    me: Identity,
    replica: Mutex<Replica>,
    peers: Mutex<Vec<PeerInfo>>,
    clock: Mutex<u64>,
}

impl Device {
    fn new(id: u64, name: &str, kind: DeviceKind) -> Self {
        Self {
            me: Identity::generate(id, name, kind).unwrap(),
            replica: Mutex::new(Replica::new(id)),
            peers: Mutex::new(Vec::new()),
            clock: Mutex::new(1_800_000_000_000),
        }
    }

    fn write(&self, uid: &str, title: &str) {
        let mut t = self.clock.lock().unwrap();
        *t += 1000;
        let mut r = self.replica.lock().unwrap();
        let old = r.object("entry", uid);
        r.record(
            "entry",
            uid,
            old.as_ref(),
            Some(&json!({ "title": title })),
            *t,
        );
    }

    fn title(&self, uid: &str) -> Option<String> {
        let r = self.replica.lock().unwrap();
        r.object("entry", uid)
            .and_then(|v| v["title"].as_str().map(String::from))
    }

    fn objects(&self) -> std::collections::BTreeMap<String, serde_json::Value> {
        self.replica.lock().unwrap().objects("entry")
    }
}

impl Calendar for Device {
    fn known(&self) -> Known {
        self.replica.lock().unwrap().known().clone()
    }
    fn changes_for(&self, theirs: &Known) -> Vec<Change> {
        self.replica.lock().unwrap().changes_for(theirs)
    }
    fn apply(&self, changes: &[Change]) -> anyhow::Result<bool> {
        Ok(!self.replica.lock().unwrap().apply(changes).is_empty())
    }
}

impl Peers for Device {
    fn peer(&self, device: DeviceId) -> Option<PeerInfo> {
        self.peers
            .lock()
            .unwrap()
            .iter()
            .find(|p| p.device == device)
            .cloned()
    }
}

/// Lets `host` answer exactly one connection in the background.
fn serve_once<'a>(
    scope: &'a thread::Scope<'a, '_>,
    host: &'a Device,
    offer: Option<&'a Offer>,
    yes: bool,
) -> (
    SocketAddr,
    thread::ScopedJoinHandle<'a, anyhow::Result<Accepted>>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = scope.spawn(move || {
        let (stream, _) = listener.accept()?;
        let mut confirm = |_: &[&'static str; 4], _: &PeerInfo| yes;
        accept(stream, &host.me, host, host, offer, &mut confirm)
    });
    (addr, handle)
}

fn paired() -> (Device, Device) {
    let pc = Device::new(1, "Arbeitszimmer-PC", DeviceKind::Desktop);
    let phone = Device::new(2, "Pixel 8", DeviceKind::Phone);
    pc.peers.lock().unwrap().push(phone.me.info());
    phone.peers.lock().unwrap().push(pc.me.info());
    (pc, phone)
}

#[test]
fn pairing_with_the_qr_code() {
    let pc = Device::new(1, "Arbeitszimmer-PC", DeviceKind::Desktop);
    let phone = Device::new(2, "Pixel 8", DeviceKind::Phone);
    let offer = Offer::new().unwrap();
    let mut words_pc = None;
    let result = thread::scope(|s| {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let qr = offer.qr(&pc.me, &[addr]);
        let (pc_ref, offer_ref, words_ref) = (&pc, &offer, &mut words_pc);
        let pc_side = s.spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut confirm = |w: &[&'static str; 4], p: &PeerInfo| {
                *words_ref = Some((*w, p.name.clone()));
                true
            };
            accept(
                stream,
                &pc_ref.me,
                pc_ref,
                pc_ref,
                Some(offer_ref),
                &mut confirm,
            )
            .unwrap()
        });
        let scanned = parse_qr(&qr).unwrap();
        assert_eq!(scanned.name, "Arbeitszimmer-PC");
        assert_eq!(scanned.addresses, vec![addr]);
        let mut words_phone = None;
        let mut confirm = |w: &[&'static str; 4], p: &PeerInfo| {
            words_phone = Some((*w, p.name.clone()));
            true
        };
        let got = pair(
            scanned.addresses[0],
            &phone.me,
            &scanned.code,
            Some(&scanned.public_key),
            &mut confirm,
        )
        .unwrap();
        (got, pc_side.join().unwrap(), words_phone)
    });
    let (got, pc_result, words_phone) = result;
    assert_eq!(got, Some(pc.me.info()));
    assert_eq!(pc_result, Accepted::Paired(phone.me.info()));
    let (w_pc, name_on_pc) = words_pc.unwrap();
    let (w_phone, name_on_phone) = words_phone.unwrap();
    assert_eq!(w_pc, w_phone, "both show the same words");
    assert_eq!(
        (name_on_pc.as_str(), name_on_phone.as_str()),
        ("Pixel 8", "Arbeitszimmer-PC")
    );
}

#[test]
fn pairing_with_the_six_digits() {
    let pc = Device::new(1, "PC", DeviceKind::Desktop);
    let tablet = Device::new(3, "Tab", DeviceKind::Tablet);
    let offer = Offer::new().unwrap();
    assert_eq!(offer.number.len(), 6);
    thread::scope(|s| {
        let (addr, pc_side) = serve_once(s, &pc, Some(&offer), true);
        let mut yes = |_: &[&'static str; 4], _: &PeerInfo| true;
        let got = pair(addr, &tablet.me, &offer.number, None, &mut yes).unwrap();
        assert_eq!(got.map(|p| p.device), Some(1));
        assert!(matches!(
            pc_side.join().unwrap().unwrap(),
            Accepted::Paired(_)
        ));
    });
    assert!(offer.used_up(), "an offer pairs only once");
}

#[test]
fn wrong_code_and_used_up_offer() {
    let pc = Device::new(1, "PC", DeviceKind::Desktop);
    let phone = Device::new(2, "Handy", DeviceKind::Phone);
    let offer = Offer::new().unwrap();
    for _ in 0..5 {
        thread::scope(|s| {
            let (addr, pc_side) = serve_once(s, &pc, Some(&offer), true);
            let mut yes = |_: &[&'static str; 4], _: &PeerInfo| true;
            assert!(pair(addr, &phone.me, "000000", None, &mut yes).is_err());
            assert!(pc_side.join().unwrap().is_err());
        });
    }
    // Five wrong tries: even the right code no longer works.
    assert!(offer.used_up());
    thread::scope(|s| {
        let (addr, pc_side) = serve_once(s, &pc, Some(&offer), true);
        let mut yes = |_: &[&'static str; 4], _: &PeerInfo| true;
        assert!(pair(addr, &phone.me, &offer.number, None, &mut yes).is_err());
        assert!(pc_side.join().unwrap().is_err());
    });
}

#[test]
fn one_side_declines() {
    let pc = Device::new(1, "PC", DeviceKind::Desktop);
    let phone = Device::new(2, "Handy", DeviceKind::Phone);
    let offer = Offer::new().unwrap();
    thread::scope(|s| {
        let (addr, pc_side) = serve_once(s, &pc, Some(&offer), false);
        let mut yes = |_: &[&'static str; 4], _: &PeerInfo| true;
        assert_eq!(
            pair(
                addr,
                &phone.me,
                &offer.code,
                Some(&pc.me.public_key),
                &mut yes
            )
            .unwrap(),
            None
        );
        assert_eq!(pc_side.join().unwrap().unwrap(), Accepted::NotPaired);
    });
}

#[test]
fn qr_code_from_another_device_is_refused() {
    let pc = Device::new(1, "PC", DeviceKind::Desktop);
    let other = Device::new(9, "Fremd", DeviceKind::Desktop);
    let phone = Device::new(2, "Handy", DeviceKind::Phone);
    let offer = Offer::new().unwrap();
    thread::scope(|s| {
        let (addr, pc_side) = serve_once(s, &pc, Some(&offer), true);
        let mut yes = |_: &[&'static str; 4], _: &PeerInfo| true;
        // The phone expects the key of `other`, but `pc` answers.
        assert!(
            pair(
                addr,
                &phone.me,
                &offer.code,
                Some(&other.me.public_key),
                &mut yes
            )
            .is_err()
        );
        let _ = pc_side.join();
    });
}

#[test]
fn paired_devices_sync_both_ways() {
    let (pc, phone) = paired();
    pc.write("a", "Zahnarzt");
    phone.write("b", "Elternabend");
    thread::scope(|s| {
        let (addr, pc_side) = serve_once(s, &pc, None, true);
        let report = sync(addr, &phone.me, &pc.me.info(), &phone).unwrap();
        assert!(report.changed);
        match pc_side.join().unwrap().unwrap() {
            Accepted::Synced { peer, report } => {
                assert_eq!(peer.device, 2);
                assert!(report.changed);
            }
            other => panic!("{other:?}"),
        }
    });
    assert_eq!(pc.objects(), phone.objects());
    assert_eq!(phone.title("a").as_deref(), Some("Zahnarzt"));
    assert_eq!(pc.title("b").as_deref(), Some("Elternabend"));
}

#[test]
fn many_changes_go_in_batches() {
    let (pc, phone) = paired();
    for i in 0..1000 {
        phone.write(&format!("e{i}"), &"x".repeat(200));
    }
    thread::scope(|s| {
        let (addr, pc_side) = serve_once(s, &pc, None, true);
        let report = sync(addr, &phone.me, &pc.me.info(), &phone).unwrap();
        assert_eq!(report.sent, 1000);
        pc_side.join().unwrap().unwrap();
    });
    assert_eq!(pc.objects().len(), 1000);
}

#[test]
fn unknown_devices_get_nothing() {
    let (pc, _phone) = paired();
    pc.write("a", "Geheim");
    let stranger = Device::new(7, "Fremd", DeviceKind::Phone);
    // A stranger pretending to be the paired phone (id 2) but with its own key.
    let impostor = Identity {
        device: 2,
        ..stranger.me.clone()
    };
    thread::scope(|s| {
        let (addr, pc_side) = serve_once(s, &pc, None, true);
        assert!(sync(addr, &impostor, &pc.me.info(), &stranger).is_err());
        assert!(pc_side.join().unwrap().is_err());
    });
    thread::scope(|s| {
        let (addr, pc_side) = serve_once(s, &pc, None, true);
        assert!(sync(addr, &stranger.me, &pc.me.info(), &stranger).is_err());
        assert!(pc_side.join().unwrap().is_err());
    });
    assert_eq!(stranger.title("a"), None);
}

#[test]
fn a_broken_connection_loses_nothing() {
    let (pc, phone) = paired();
    for i in 0..650 {
        phone.write(&format!("e{i}"), &format!("Eintrag {i}"));
    }
    // The connection breaks after two of four batches.
    thread::scope(|s| {
        let (addr, pc_side) = serve_once(s, &pc, None, true);
        let report = sync_limited(addr, &phone.me, &pc.me.info(), &phone, Some(2)).unwrap();
        assert_eq!(report.sent, 400);
        assert!(pc_side.join().unwrap().is_err(), "the PC notices the break");
    });
    assert_eq!(pc.objects().len(), 400, "what arrived stays");
    // Meanwhile both change things; the next meeting completes everything.
    pc.write("e5", "auf dem PC geändert");
    phone.write("e600", "auf dem Handy geändert");
    thread::scope(|s| {
        let (addr, pc_side) = serve_once(s, &pc, None, true);
        sync(addr, &phone.me, &pc.me.info(), &phone).unwrap();
        pc_side.join().unwrap().unwrap();
    });
    assert_eq!(pc.objects(), phone.objects());
    assert_eq!(phone.title("e5").as_deref(), Some("auf dem PC geändert"));
    assert_eq!(pc.objects().len(), 650);
}

#[test]
fn only_home_networks() {
    for ok in [
        "192.168.178.20",
        "10.0.0.5",
        "172.16.1.1",
        "127.0.0.1",
        "169.254.3.4",
        "fe80::1",
        "fd00::5",
        "::1",
    ] {
        assert!(is_private(ok.parse().unwrap()), "{ok}");
    }
    for no in ["8.8.8.8", "100.64.1.1", "2a00:1450::1", "1.1.1.1"] {
        assert!(!is_private(no.parse().unwrap()), "{no}");
    }
}

#[test]
fn check_words_are_stable() {
    assert_eq!(
        check_words(&[0, 0, 0]),
        ["Apfel", "Apfel", "Apfel", "Apfel"]
    );
    assert_ne!(check_words(&[1, 2, 3]), check_words(&[3, 2, 1]));
}

/// Needs a network with multicast; run with `cargo test -- --ignored`.
#[test]
#[ignore]
fn devices_find_each_other() {
    let _a = discover::announce(4242, 47001).unwrap();
    let found = discover::browse(std::time::Duration::from_secs(4)).unwrap();
    assert!(
        found
            .iter()
            .any(|f| f.device == 4242 && !f.addresses.is_empty()),
        "{found:?}"
    );
}
