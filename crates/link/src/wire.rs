// SPDX-License-Identifier: GPL-3.0-or-later

//! Frames on a TCP stream, the Noise handshakes and encrypted messages.
//!
//! Every frame is a 2-byte length plus the bytes. Encrypted messages are cut
//! into frames of at most 60 000 bytes; the first byte of each says whether
//! more follow.

use std::io::{Read, Write};
use std::net::{IpAddr, TcpStream};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::{Serialize, de::DeserializeOwned};
use snow::{Builder, HandshakeState, TransportState};

pub(crate) const PAIR: &str = "Noise_XXpsk3_25519_ChaChaPoly_BLAKE2s";
pub(crate) const SYNC: &str = "Noise_KK_25519_ChaChaPoly_BLAKE2s";
/// Waiting longer than this for the other device ends the connection.
pub(crate) const TIMEOUT: Duration = Duration::from_secs(20);
const MAX_FRAME: usize = 65_535;
const CHUNK: usize = 60_000;

pub(crate) fn builder(params: &str) -> Result<Builder<'_>> {
    Ok(Builder::new(params.parse()?))
}

/// Only home networks, VPN to home, and this computer itself.
pub fn is_private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_private() || v4.is_loopback() || v4.is_link_local(),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_private(IpAddr::V4(v4));
            }
            let first = v6.segments()[0];
            v6.is_loopback() || (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80
        }
    }
}

pub(crate) fn prepare(stream: &TcpStream) -> Result<()> {
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    stream.set_nodelay(true)?;
    Ok(())
}

pub(crate) fn write_frame(stream: &mut TcpStream, bytes: &[u8]) -> Result<()> {
    if bytes.len() > MAX_FRAME {
        bail!("frame too large");
    }
    stream.write_all(&(bytes.len() as u16).to_be_bytes())?;
    stream.write_all(bytes)?;
    Ok(())
}

pub(crate) fn read_frame(stream: &mut TcpStream) -> Result<Vec<u8>> {
    let mut len = [0u8; 2];
    stream.read_exact(&mut len).context("connection closed")?;
    let mut buf = vec![0u8; u16::from_be_bytes(len) as usize];
    stream.read_exact(&mut buf).context("connection closed")?;
    Ok(buf)
}

/// Runs a handshake to its end. `initiator` writes the first message.
pub(crate) fn handshake(
    stream: &mut TcpStream,
    mut hs: HandshakeState,
    initiator: bool,
) -> Result<(TransportState, Vec<u8>)> {
    let mut buf = vec![0u8; MAX_FRAME];
    let mut my_turn = initiator;
    while !hs.is_handshake_finished() {
        if my_turn {
            let n = hs.write_message(&[], &mut buf)?;
            write_frame(stream, &buf[..n])?;
        } else {
            let msg = read_frame(stream)?;
            hs.read_message(&msg, &mut buf)
                .context("handshake failed")?;
        }
        my_turn = !my_turn;
    }
    let hash = hs.get_handshake_hash().to_vec();
    Ok((hs.into_transport_mode()?, hash))
}

/// An encrypted connection, after the handshake.
pub(crate) struct Channel {
    pub stream: TcpStream,
    noise: TransportState,
}

impl Channel {
    pub fn new(stream: TcpStream, noise: TransportState) -> Self {
        Self { stream, noise }
    }

    pub fn remote_key(&self) -> Option<Vec<u8>> {
        self.noise.get_remote_static().map(<[u8]>::to_vec)
    }

    pub fn send<T: Serialize>(&mut self, message: &T) -> Result<()> {
        let bytes = serde_json::to_vec(message)?;
        let mut chunks = bytes.chunks(CHUNK).peekable();
        let mut out = vec![0u8; MAX_FRAME];
        if chunks.peek().is_none() {
            let n = self.noise.write_message(&[0], &mut out)?;
            return write_frame(&mut self.stream, &out[..n]);
        }
        while let Some(chunk) = chunks.next() {
            let mut plain = Vec::with_capacity(chunk.len() + 1);
            plain.push(u8::from(chunks.peek().is_some()));
            plain.extend_from_slice(chunk);
            let n = self.noise.write_message(&plain, &mut out)?;
            write_frame(&mut self.stream, &out[..n])?;
        }
        Ok(())
    }

    pub fn receive<T: DeserializeOwned>(&mut self) -> Result<T> {
        let mut bytes = Vec::new();
        let mut plain = vec![0u8; MAX_FRAME];
        loop {
            let frame = read_frame(&mut self.stream)?;
            let n = self
                .noise
                .read_message(&frame, &mut plain)
                .context("damaged message")?;
            if n == 0 {
                bail!("empty message");
            }
            bytes.extend_from_slice(&plain[1..n]);
            if plain[0] == 0 {
                break;
            }
            if bytes.len() > 64 * 1024 * 1024 {
                bail!("message too large");
            }
        }
        Ok(serde_json::from_slice(&bytes)?)
    }
}
