// SPDX-License-Identifier: GPL-3.0-or-later

//! Pairing: the first meeting of two devices.
//!
//! The device that is already set up makes an [`Offer`] and shows it as a
//! QR code ([`Offer::qr`]) or as a six-digit number. The new device connects
//! with [`pair`]. The code is the pre-shared key of the Noise handshake;
//! after five wrong tries the offer is used up. Then both devices show the
//! same [`check_words`](crate::check_words) and ask their person; only if
//! both confirm are the devices paired.

use std::net::{SocketAddr, TcpStream};
use std::sync::atomic::{AtomicU32, Ordering};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

use crate::wire::{self, Channel, PAIR};
use crate::{Identity, Message, PROTOCOL, PeerInfo, check_words};

/// Confirmation asked from the person: the check words and the other device.
pub type Confirm<'a> = &'a mut dyn FnMut(&[&'static str; 4], &PeerInfo) -> bool;

const MAX_TRIES: u32 = 5;
pub(crate) const MODE_QR: u8 = b'P';
pub(crate) const MODE_NUMBER: u8 = b'M';

/// An invitation to pair, valid until it is used or tried too often.
pub struct Offer {
    /// Long random code for the QR code.
    pub code: String,
    /// Six digits for devices without a camera.
    pub number: String,
    tries: AtomicU32,
}

impl Offer {
    pub fn new() -> Result<Self> {
        let random = wire::builder(PAIR)?.generate_keypair()?.private;
        let code: String = random[..16].iter().map(|b| format!("{b:02x}")).collect();
        let n = u32::from_be_bytes([random[16], random[17], random[18], random[19]]) % 1_000_000;
        Ok(Self {
            code,
            number: format!("{n:06}"),
            tries: AtomicU32::new(0),
        })
    }

    /// Text for the QR code: who we are, how to reach us, and the code.
    pub fn qr(&self, me: &Identity, addresses: &[SocketAddr]) -> String {
        let key: String = me.public_key.iter().map(|b| format!("{b:02x}")).collect();
        let addrs: Vec<String> = addresses.iter().map(ToString::to_string).collect();
        format!(
            "kalender:pair?v={PROTOCOL}&d={}&n={}&k={key}&c={}&a={}",
            me.device,
            escape(&me.name),
            self.code,
            escape(&addrs.join(","))
        )
    }

    pub fn used_up(&self) -> bool {
        self.tries.load(Ordering::SeqCst) >= MAX_TRIES
    }

    fn psk(&self, mode: u8) -> [u8; 32] {
        psk(if mode == MODE_QR {
            &self.code
        } else {
            &self.number
        })
    }
}

/// What a scanned QR code says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scanned {
    pub device: kalender_sync::DeviceId,
    pub name: String,
    pub public_key: Vec<u8>,
    pub code: String,
    pub addresses: Vec<SocketAddr>,
}

pub fn parse_qr(text: &str) -> Result<Scanned> {
    let query = text
        .strip_prefix("kalender:pair?")
        .context("not a Kalender code")?;
    let get = |key: &str| {
        query
            .split('&')
            .find_map(|p| p.strip_prefix(key).and_then(|r| r.strip_prefix('=')))
            .map(unescape)
    };
    if get("v").as_deref() != Some(&PROTOCOL.to_string()) {
        bail!("this code is from another version of Kalender");
    }
    let key = get("k").context("no key")?;
    let public_key = (0..key.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(key.get(i..i + 2).unwrap_or("x"), 16))
        .collect::<Result<Vec<u8>, _>>()
        .context("bad key")?;
    Ok(Scanned {
        device: get("d").context("no device")?.parse()?,
        name: get("n").unwrap_or_default(),
        public_key,
        code: get("c").context("no code")?,
        addresses: get("a")
            .unwrap_or_default()
            .split(',')
            .filter_map(|a| a.parse().ok())
            .collect(),
    })
}

fn psk(code: &str) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"kalender-pair-v1:");
    h.update(code.trim().as_bytes());
    h.finalize().into()
}

fn escape(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '%' | '&' | '=' | '?' | ' ' | '#' => format!("%{:02X}", c as u32),
            c => c.to_string(),
        })
        .collect()
}

fn unescape(s: &str) -> String {
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(Ok(b)) = s.get(i + 1..i + 3).map(|h| u8::from_str_radix(h, 16))
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// New device: pairs with the device at `address`, using the code from the
/// QR code (`expected_key` from it too) or the six digits (`expected_key` = None).
/// Returns the other device if both people confirmed.
pub fn pair(
    address: SocketAddr,
    me: &Identity,
    code: &str,
    expected_key: Option<&[u8]>,
    confirm: Confirm,
) -> Result<Option<PeerInfo>> {
    let mut stream = TcpStream::connect_timeout(&address, wire::TIMEOUT)?;
    wire::prepare(&stream)?;
    let mode = if expected_key.is_some() {
        MODE_QR
    } else {
        MODE_NUMBER
    };
    wire::write_frame(&mut stream, &[mode])?;
    let key = psk(code);
    let hs = wire::builder(PAIR)?
        .local_private_key(&me.private_key)?
        .psk(3, &key)?
        .build_initiator()?;
    let (noise, hash) = wire::handshake(&mut stream, hs, true).context("the code does not fit")?;
    let mut channel = Channel::new(stream, noise);
    if let Some(expected) = expected_key
        && channel.remote_key().as_deref() != Some(expected)
    {
        bail!("the other device is not the one from the QR code");
    }
    finish(&mut channel, me, &hash, confirm)
}

/// Existing device: answers a pairing attempt on an accepted connection.
pub(crate) fn answer(
    stream: TcpStream,
    mode: u8,
    me: &Identity,
    offer: &Offer,
    confirm: Confirm,
) -> Result<Option<PeerInfo>> {
    let mut stream = stream;
    if offer.used_up() {
        bail!("pairing code used up");
    }
    let key = offer.psk(mode);
    let hs = wire::builder(PAIR)?
        .local_private_key(&me.private_key)?
        .psk(3, &key)?
        .build_responder()?;
    let (noise, hash) = match wire::handshake(&mut stream, hs, false) {
        Ok(done) => done,
        Err(e) => {
            offer.tries.fetch_add(1, Ordering::SeqCst);
            return Err(e.context("wrong pairing code"));
        }
    };
    let mut channel = Channel::new(stream, noise);
    let result = finish(&mut channel, me, &hash, confirm);
    // One pairing per offer.
    offer.tries.store(MAX_TRIES, Ordering::SeqCst);
    result
}

/// Both sides: introduce, ask the person, exchange the decisions.
fn finish(
    channel: &mut Channel,
    me: &Identity,
    hash: &[u8],
    confirm: Confirm,
) -> Result<Option<PeerInfo>> {
    channel.send(&Message::Hello {
        protocol: PROTOCOL,
        peer: me.info(),
        known: None,
    })?;
    let Message::Hello { protocol, peer, .. } = channel.receive()? else {
        bail!("unexpected message");
    };
    if protocol != PROTOCOL {
        bail!("the other device has another version of Kalender");
    }
    if channel.remote_key().as_deref() != Some(peer.public_key.as_slice()) {
        bail!("the other device gave a wrong key");
    }
    let words = check_words(hash);
    let mine = confirm(&words, &peer);
    channel.send(&Message::Decision(mine))?;
    let Message::Decision(theirs) = channel.receive()? else {
        bail!("unexpected message");
    };
    Ok((mine && theirs).then_some(peer))
}
