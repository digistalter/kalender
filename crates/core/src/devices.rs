// SPDX-License-Identifier: GPL-3.0-or-later

//! This device's identity and the devices it is paired with, kept next to
//! the calendar. The private key never leaves this file.

use anyhow::{Context, Result};
use chrono::NaiveDateTime;
use kalender_link::{Calendar, DeviceKind, Identity, PeerInfo, Peers};
use kalender_sync::{Change, DeviceId, Known};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::store::Store;

/// A paired device and what this device remembers about it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Peer {
    pub info: PeerInfo,
    /// Where it was reached last ("192.168.178.20:47001").
    #[serde(default)]
    pub address: Option<String>,
    /// Last successful sync (local time).
    #[serde(default)]
    pub synced: Option<NaiveDateTime>,
}

pub(crate) fn create_tables(conn: &rusqlite::Connection) -> Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS sync_peers (device INTEGER PRIMARY KEY, data TEXT NOT NULL)",
        [],
    )?;
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Result<Vec<u8>> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2).unwrap_or("x"), 16).context("bad key"))
        .collect()
}

impl Store {
    fn state(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT value FROM sync_state WHERE key = ?1",
                params![key],
                |r| r.get(0),
            )
            .optional()?)
    }

    fn set_state(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO sync_state (key, value) VALUES (?1, ?2)",
            params![key, value],
        )?;
        Ok(())
    }

    /// This device's identity; made once, with `name` and `kind` as first values.
    pub fn identity(&self, name: &str, kind: DeviceKind) -> Result<Identity> {
        let device = self.device_id().context("journal not started")?;
        if let (Some(private), Some(public)) =
            (self.state("private_key")?, self.state("public_key")?)
        {
            return Ok(Identity {
                device,
                name: self
                    .state("device_name")?
                    .unwrap_or_else(|| name.to_owned()),
                kind: self
                    .state("device_kind")?
                    .and_then(|k| serde_json::from_str(&k).ok())
                    .unwrap_or(kind),
                private_key: unhex(&private)?,
                public_key: unhex(&public)?,
            });
        }
        let id = Identity::generate(device, name, kind)?;
        self.set_state("private_key", &hex(&id.private_key))?;
        self.set_state("public_key", &hex(&id.public_key))?;
        self.set_state("device_name", name)?;
        self.set_state("device_kind", &serde_json::to_string(&kind)?)?;
        Ok(id)
    }

    pub fn rename_device(&self, name: &str) -> Result<()> {
        self.set_state("device_name", name)
    }

    pub fn peers(&self) -> Result<Vec<Peer>> {
        let mut stmt = self
            .conn
            .prepare("SELECT data FROM sync_peers ORDER BY device")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for row in rows {
            if let Ok(p) = serde_json::from_str(&row?) {
                out.push(p);
            }
        }
        Ok(out)
    }

    pub fn save_peer(&self, peer: &Peer) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO sync_peers (device, data) VALUES (?1, ?2)",
            params![peer.info.device as i64, serde_json::to_string(peer)?],
        )?;
        Ok(())
    }

    /// Forgets a device: it can no longer sync with this one.
    pub fn remove_peer(&self, device: DeviceId) -> Result<()> {
        self.conn.execute(
            "DELETE FROM sync_peers WHERE device = ?1",
            params![device as i64],
        )?;
        Ok(())
    }
}

impl Calendar for Store {
    fn known(&self) -> Known {
        self.sync_known()
    }

    fn changes_for(&self, theirs: &Known) -> Vec<Change> {
        self.sync_changes_for(theirs)
    }

    fn apply(&self, changes: &[Change]) -> Result<bool> {
        self.sync_apply(changes)
    }
}

impl Peers for Store {
    fn peer(&self, device: DeviceId) -> Option<PeerInfo> {
        self.peers()
            .ok()?
            .into_iter()
            .find(|p| p.info.device == device)
            .map(|p| p.info)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Entry, at};
    use chrono::NaiveDate;
    use kalender_link::pair::{Offer, pair, parse_qr};
    use kalender_link::sync::{Accepted, accept, sync};
    use std::net::TcpListener;

    /// Two calendars in their own files, each in its own thread (like two
    /// programs), paired and synced over the network as devices at home would.
    #[test]
    fn two_calendars_pair_and_sync_over_the_network() {
        let dir = std::env::temp_dir().join(format!("kalender-devices-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let day = NaiveDate::from_ymd_opt(2026, 11, 26).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (qr_tx, qr_rx) = std::sync::mpsc::channel();
        let pc_dir = dir.clone();

        // The PC: offers pairing, then answers three connections.
        let pc_thread = std::thread::spawn(move || {
            let pc = Store::open(&pc_dir.join("pc.sqlite")).unwrap();
            let me = pc
                .identity("Arbeitszimmer-PC", DeviceKind::Desktop)
                .unwrap();
            assert_eq!(
                pc.identity("anders", DeviceKind::Phone).unwrap().public_key,
                me.public_key,
                "made once"
            );
            let kind = pc.kinds().unwrap()[0].id;
            let mut e = Entry::new(kind, "Zahnarzt", at(day, 15, 0), at(day, 16, 0));
            pc.save_entry(&mut e).unwrap();
            let offer = Offer::new().unwrap();
            qr_tx.send(offer.qr(&me, &[addr])).unwrap();
            let mut yes = |_: &[&'static str; 4], _: &PeerInfo| true;
            let mut results = Vec::new();
            for round in 0..3 {
                let (stream, _) = listener.accept().unwrap();
                let r = accept(
                    stream,
                    &me,
                    &pc,
                    &pc,
                    (round == 0).then_some(&offer),
                    &mut yes,
                );
                if let Ok(Accepted::Paired(info)) = &r {
                    pc.save_peer(&Peer {
                        info: info.clone(),
                        address: None,
                        synced: None,
                    })
                    .unwrap();
                }
                if round == 1 {
                    // After the first sync the PC forgets the phone.
                    for p in pc.peers().unwrap() {
                        pc.remove_peer(p.info.device).unwrap();
                    }
                }
                results.push(
                    r.map(|a| matches!(a, Accepted::Paired(_) | Accepted::Synced { .. }))
                        .unwrap_or(false),
                );
            }
            results
        });

        // The phone, in this thread with its own file.
        let phone = Store::open(&dir.join("phone.sqlite")).unwrap();
        let me = phone.identity("Pixel 8", DeviceKind::Phone).unwrap();
        let code = parse_qr(&qr_rx.recv().unwrap()).unwrap();
        let mut yes = |_: &[&'static str; 4], _: &PeerInfo| true;
        let pc_info = pair(
            code.addresses[0],
            &me,
            &code.code,
            Some(&code.public_key),
            &mut yes,
        )
        .unwrap()
        .unwrap();
        assert_eq!(pc_info.name, "Arbeitszimmer-PC");
        phone
            .save_peer(&Peer {
                info: pc_info.clone(),
                address: Some(addr.to_string()),
                synced: None,
            })
            .unwrap();

        let report = sync(addr, &me, &pc_info, &phone).unwrap();
        assert!(report.changed);
        let titles: Vec<String> = phone
            .entries()
            .unwrap()
            .into_iter()
            .map(|e| e.title)
            .collect();
        assert_eq!(titles, vec!["Zahnarzt"]);

        // The PC has forgotten the phone: no more syncing.
        assert!(sync(addr, &me, &pc_info, &phone).is_err());
        assert_eq!(pc_thread.join().unwrap(), vec![true, true, false]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
