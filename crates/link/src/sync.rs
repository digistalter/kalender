// SPDX-License-Identifier: GPL-3.0-or-later

//! Syncing two paired devices, and the listening side for all connections.
//!
//! Order on the wire, after the Noise `KK` handshake:
//! 1. the connecting device says hello with what it knows,
//! 2. the other answers the same,
//! 3. the connecting device sends what the other is missing, in batches,
//! 4. the other does the same.
//!
//! Every batch is taken over as a whole. If the connection breaks, what has
//! arrived stays; the next meeting continues from there.

use std::net::{SocketAddr, TcpStream};

use anyhow::{Context, Result, bail};
use kalender_sync::{Change, DeviceId, Known};

use crate::pair::{self, Confirm, MODE_NUMBER, MODE_QR, Offer};
use crate::wire::{self, Channel, SYNC};
use crate::{Calendar, Identity, Message, PROTOCOL, PeerInfo, Peers};

const MODE_SYNC: u8 = b'S';
/// Changes per batch.
const BATCH: usize = 200;

/// What a meeting brought.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub sent: usize,
    pub received: usize,
    /// Whether the calendar here changed.
    pub changed: bool,
}

/// Connects to a paired device and syncs both ways.
pub fn sync(
    address: SocketAddr,
    me: &Identity,
    peer: &PeerInfo,
    calendar: &dyn Calendar,
) -> Result<Report> {
    sync_limited(address, me, peer, calendar, None)
}

/// Like [`sync`]; with `stop_after` the connection is dropped after that many
/// batches (to test broken connections).
#[doc(hidden)]
pub fn sync_limited(
    address: SocketAddr,
    me: &Identity,
    peer: &PeerInfo,
    calendar: &dyn Calendar,
    stop_after: Option<usize>,
) -> Result<Report> {
    let mut stream = TcpStream::connect_timeout(&address, wire::TIMEOUT)?;
    wire::prepare(&stream)?;
    let mut opening = vec![MODE_SYNC];
    opening.extend_from_slice(&me.device.to_be_bytes());
    wire::write_frame(&mut stream, &opening)?;
    let hs = wire::builder(SYNC)?
        .local_private_key(&me.private_key)?
        .remote_public_key(&peer.public_key)?
        .build_initiator()?;
    let (noise, _) = wire::handshake(&mut stream, hs, true)
        .context("the other device does not know this one")?;
    let mut channel = Channel::new(stream, noise);
    channel.send(&hello(me, calendar))?;
    let theirs = receive_hello(&mut channel, peer.device)?;
    let sent = send_missing(&mut channel, calendar, &theirs, stop_after)?;
    if stop_after.is_some() {
        return Ok(Report {
            sent,
            ..Report::default()
        });
    }
    let (received, changed) = receive_missing(&mut channel, calendar)?;
    Ok(Report {
        sent,
        received,
        changed,
    })
}

/// The result of an accepted connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Accepted {
    Synced {
        peer: PeerInfo,
        report: Report,
    },
    Paired(PeerInfo),
    /// Pairing was declined by one of the two people.
    NotPaired,
}

/// Answers one incoming connection: a sync from a paired device, or a pairing
/// attempt if an `offer` is open. Connections from outside the home network
/// are refused.
pub fn accept(
    stream: TcpStream,
    me: &Identity,
    calendar: &dyn Calendar,
    peers: &dyn Peers,
    offer: Option<&Offer>,
    confirm: Confirm,
) -> Result<Accepted> {
    let mut stream = stream;
    let from = stream.peer_addr()?;
    if !wire::is_private(from.ip()) {
        bail!("refused connection from outside the home network: {from}");
    }
    wire::prepare(&stream)?;
    let opening = wire::read_frame(&mut stream)?;
    match opening.first().copied() {
        Some(mode @ (MODE_QR | MODE_NUMBER)) => {
            let offer = offer.context("no pairing open")?;
            Ok(match pair::answer(stream, mode, me, offer, confirm)? {
                Some(peer) => Accepted::Paired(peer),
                None => Accepted::NotPaired,
            })
        }
        Some(MODE_SYNC) if opening.len() == 9 => {
            let device = DeviceId::from_be_bytes(opening[1..9].try_into()?);
            let peer = peers.peer(device).context("unknown device")?;
            let hs = wire::builder(SYNC)?
                .local_private_key(&me.private_key)?
                .remote_public_key(&peer.public_key)?
                .build_responder()?;
            let (noise, _) =
                wire::handshake(&mut stream, hs, false).context("not a paired device")?;
            let mut channel = Channel::new(stream, noise);
            let theirs = receive_hello(&mut channel, peer.device)?;
            channel.send(&hello(me, calendar))?;
            let (received, changed) = receive_missing(&mut channel, calendar)?;
            let sent = send_missing(&mut channel, calendar, &theirs, None)?;
            Ok(Accepted::Synced {
                peer,
                report: Report {
                    sent,
                    received,
                    changed,
                },
            })
        }
        _ => bail!("unknown request"),
    }
}

fn hello(me: &Identity, calendar: &dyn Calendar) -> Message {
    Message::Hello {
        protocol: PROTOCOL,
        peer: me.info(),
        known: Some(calendar.known()),
    }
}

fn receive_hello(channel: &mut Channel, expected: DeviceId) -> Result<Known> {
    let Message::Hello {
        protocol,
        peer,
        known,
    } = channel.receive()?
    else {
        bail!("unexpected message");
    };
    if protocol != PROTOCOL {
        bail!("the other device has another version of Kalender");
    }
    if peer.device != expected {
        bail!("the other device is not who it says");
    }
    Ok(known.unwrap_or_default())
}

fn send_missing(
    channel: &mut Channel,
    calendar: &dyn Calendar,
    theirs: &Known,
    stop_after: Option<usize>,
) -> Result<usize> {
    let missing: Vec<Change> = calendar.changes_for(theirs);
    for (i, batch) in missing.chunks(BATCH).enumerate() {
        if stop_after == Some(i) {
            return Ok(i * BATCH);
        }
        channel.send(&Message::Changes(batch.to_vec()))?;
    }
    channel.send(&Message::Done)?;
    Ok(missing.len())
}

fn receive_missing(channel: &mut Channel, calendar: &dyn Calendar) -> Result<(usize, bool)> {
    let mut received = 0;
    let mut changed = false;
    loop {
        match channel.receive()? {
            Message::Changes(batch) => {
                received += batch.len();
                changed |= calendar.apply(&batch)?;
            }
            Message::Done => return Ok((received, changed)),
            _ => bail!("unexpected message"),
        }
    }
}
