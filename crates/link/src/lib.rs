// SPDX-License-Identifier: GPL-3.0-or-later

//! Connecting Kalender devices at home (planung/KONZEPT-ABGLEICH.md, section 6).
//!
//! - [`pair`]: first contact. One device shows a QR code (or a six-digit
//!   number), the other connects with it. Both show the same four check words;
//!   only when both people confirm are the devices paired. Noise
//!   `XXpsk3`: the code is the pre-shared key, so nobody without it gets in.
//! - [`sync`]: later meetings. Noise `KK`: both know each other's key, so
//!   only paired devices can talk. Each side sends what the other is missing.
//! - [`discover`]: finding each other in the home network with mDNS.
//!
//! Connections are only accepted from private addresses (home network,
//! VPN to home). Everything is end-to-end encrypted; no server is involved.
//! Storage is not part of this crate: the calendar is reached through
//! [`Calendar`], paired devices through [`Peers`].

use anyhow::Result;
use kalender_sync::{Change, DeviceId, Known};
use serde::{Deserialize, Serialize};

pub mod discover;
pub mod pair;
pub mod sync;
mod wire;

pub use wire::is_private;

/// Version of the protocol; devices with a different major version don't talk.
pub const PROTOCOL: u32 = 1;

/// This device: its id, how it is called and its key pair.
#[derive(Clone, Debug)]
pub struct Identity {
    pub device: DeviceId,
    pub name: String,
    pub kind: DeviceKind,
    pub private_key: Vec<u8>,
    pub public_key: Vec<u8>,
}

impl Identity {
    /// A new identity with a fresh key pair.
    pub fn generate(device: DeviceId, name: &str, kind: DeviceKind) -> Result<Self> {
        let pair = wire::builder(wire::SYNC)?.generate_keypair()?;
        Ok(Self {
            device,
            name: name.to_owned(),
            kind,
            private_key: pair.private,
            public_key: pair.public,
        })
    }

    pub fn info(&self) -> PeerInfo {
        PeerInfo {
            device: self.device,
            name: self.name.clone(),
            kind: self.kind,
            public_key: self.public_key.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeviceKind {
    Desktop,
    Phone,
    Tablet,
}

/// A paired device as this device remembers it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerInfo {
    pub device: DeviceId,
    pub name: String,
    pub kind: DeviceKind,
    pub public_key: Vec<u8>,
}

/// The calendar of this device, as the sync protocol needs it.
pub trait Calendar {
    fn known(&self) -> Known;
    fn changes_for(&self, theirs: &Known) -> Vec<Change>;
    /// Applies one batch at once (all or nothing). Returns whether anything changed.
    fn apply(&self, changes: &[Change]) -> Result<bool>;
}

/// The devices this device is paired with.
pub trait Peers {
    fn peer(&self, device: DeviceId) -> Option<PeerInfo>;
}

/// What devices say to each other once the connection is encrypted.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum Message {
    Hello {
        protocol: u32,
        peer: PeerInfo,
        known: Option<Known>,
    },
    Changes(Vec<Change>),
    Done,
    Decision(bool),
}

/// Four words both people compare when pairing. They come from the
/// encrypted connection itself: if somebody sat in between, they differ.
pub fn check_words(handshake_hash: &[u8]) -> [&'static str; 4] {
    const WORDS: [&str; 64] = [
        "Apfel", "Birne", "Wolke", "Sonne", "Fluss", "Berg", "Wald", "Wiese", "Stern", "Mond",
        "Regen", "Schnee", "Blume", "Baum", "Vogel", "Fisch", "Katze", "Hund", "Pferd", "Igel",
        "Fuchs", "Biene", "Eule", "Rose", "Tulpe", "Kirsche", "Nuss", "Brot", "Kerze", "Glocke",
        "Anker", "Insel", "Hafen", "Brücke", "Turm", "Garten", "Feder", "Kompass", "Laterne",
        "Muschel", "Wind", "Welle", "Kiesel", "Moos", "Farn", "Eiche", "Linde", "Ahorn", "Sieben",
        "Acht", "Neun", "Zehn", "Elf", "Zwölf", "Rot", "Blau", "Grün", "Gelb", "Silber", "Gold",
        "Morgen", "Abend", "Sommer", "Winter",
    ];
    let mut bits: u32 = 0;
    for b in handshake_hash.iter().take(3) {
        bits = (bits << 8) | u32::from(*b);
    }
    [0, 1, 2, 3].map(|i| WORDS[((bits >> (18 - 6 * i)) & 63) as usize])
}

#[cfg(test)]
mod tests;
