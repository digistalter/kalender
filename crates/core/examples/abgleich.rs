// SPDX-License-Identifier: GPL-3.0-or-later

//! A small test tool for syncing between two real programs (or computers).
//!
//! ```text
//! cargo run -p kalender-core --example abgleich -- DB serve            # shows code, answers
//! cargo run -p kalender-core --example abgleich -- DB pair "kalender:pair?…"
//! cargo run -p kalender-core --example abgleich -- DB pair IP:PORT 123456
//! cargo run -p kalender-core --example abgleich -- DB sync             # with every paired device
//! cargo run -p kalender-core --example abgleich -- DB add "Titel"      # an entry for today
//! cargo run -p kalender-core --example abgleich -- DB list
//! ```

use std::net::{SocketAddr, TcpListener};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use kalender_core::devices::Peer;
use kalender_core::model::{Entry, at};
use kalender_core::store::Store;
use kalender_link::pair::{Offer, pair, parse_qr};
use kalender_link::sync::{Accepted, accept, sync};
use kalender_link::{DeviceKind, PeerInfo, discover};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (db, cmd) = (
        PathBuf::from(args.first().context("DB missing")?),
        args.get(1).map(String::as_str),
    );
    let store = Store::open(&db)?;
    let name = db
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let me = store.identity(&name, DeviceKind::Desktop)?;
    let mut ask = |words: &[&'static str; 4], peer: &PeerInfo| {
        println!(
            "Prüfwörter: {} – „{}“ wird gekoppelt",
            words.join(" · "),
            peer.name
        );
        true
    };
    match cmd {
        Some("serve") => {
            let listener = TcpListener::bind("0.0.0.0:0")?;
            let port = listener.local_addr()?.port();
            let _announce = discover::announce(me.device, port)?;
            let offer = Offer::new()?;
            let here: SocketAddr = format!("127.0.0.1:{port}").parse()?;
            println!("QR: {}", offer.qr(&me, &[here]));
            println!("Zahl: {} · Port {port}", offer.number);
            for stream in listener.incoming() {
                let open = (!offer.used_up()).then_some(&offer);
                match accept(stream?, &me, &store, &store, open, &mut ask) {
                    Ok(Accepted::Paired(info)) => {
                        println!("gekoppelt mit {}", info.name);
                        store.save_peer(&Peer {
                            info,
                            address: None,
                            synced: None,
                        })?;
                    }
                    Ok(Accepted::Synced { peer, report }) => println!(
                        "abgeglichen mit {}: {} bekommen, {} geschickt",
                        peer.name, report.received, report.sent
                    ),
                    Ok(Accepted::NotPaired) => println!("nicht gekoppelt"),
                    Err(e) => println!("abgelehnt: {e:#}"),
                }
            }
        }
        Some("pair") => {
            let (addr, code, key) = match (args.get(2), args.get(3)) {
                (Some(qr), None) => {
                    let s = parse_qr(qr)?;
                    (s.addresses[0], s.code, Some(s.public_key))
                }
                (Some(addr), Some(number)) => (addr.parse()?, number.clone(), None),
                _ => bail!("pair QR or pair IP:PORT NUMBER"),
            };
            match pair(addr, &me, &code, key.as_deref(), &mut ask)? {
                Some(info) => {
                    println!("gekoppelt mit {}", info.name);
                    store.save_peer(&Peer {
                        info,
                        address: Some(addr.to_string()),
                        synced: None,
                    })?;
                }
                None => println!("nicht gekoppelt"),
            }
        }
        Some("sync") => {
            let found = discover::browse(Duration::from_secs(3))?;
            for peer in store.peers()? {
                let mut addrs: Vec<SocketAddr> =
                    peer.address.iter().filter_map(|a| a.parse().ok()).collect();
                if let Some(f) = found.iter().find(|f| f.device == peer.info.device) {
                    addrs.extend(&f.addresses);
                }
                let result = addrs
                    .iter()
                    .find_map(|a| sync(*a, &me, &peer.info, &store).ok().map(|r| (a, r)));
                match result {
                    Some((a, r)) => println!(
                        "{} ({a}): {} bekommen, {} geschickt",
                        peer.info.name, r.received, r.sent
                    ),
                    None => println!("{}: nicht erreichbar", peer.info.name),
                }
            }
        }
        Some("add") => {
            let kind = store.kinds()?[0].id;
            let today = chrono::Local::now().date_naive();
            let mut e = Entry::new(
                kind,
                args.get(2).context("title")?,
                at(today, 9, 0),
                at(today, 10, 0),
            );
            store.save_entry(&mut e)?;
            println!("hinzugefügt");
        }
        Some("list") => {
            for e in store.entries()? {
                println!("{}", e.title);
            }
        }
        _ => bail!("serve | pair | sync | add | list"),
    }
    Ok(())
}
