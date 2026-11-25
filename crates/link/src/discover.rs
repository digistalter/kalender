// SPDX-License-Identifier: GPL-3.0-or-later

//! Finding paired devices in the home network with mDNS/DNS-SD, the way
//! printers are found. Each device announces `_kalender._tcp` with its id;
//! nothing else is announced, and nothing leaves the local network.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use anyhow::Result;
use kalender_sync::DeviceId;
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};

use crate::wire::is_private;

pub const SERVICE: &str = "_kalender._tcp.local.";

/// Keeps this device visible while it lives.
pub struct Announcement {
    daemon: ServiceDaemon,
    fullname: String,
}

/// Announces this device on `port`.
pub fn announce(device: DeviceId, port: u16) -> Result<Announcement> {
    let daemon = ServiceDaemon::new()?;
    let name = format!("{device:016x}");
    let info = ServiceInfo::new(
        SERVICE,
        &name,
        &format!("kalender-{name}.local."),
        "",
        port,
        &[
            ("id", device.to_string().as_str()),
            ("v", crate::PROTOCOL.to_string().as_str()),
        ][..],
    )?
    .enable_addr_auto();
    let fullname = info.get_fullname().to_owned();
    daemon.register(info)?;
    Ok(Announcement { daemon, fullname })
}

impl Drop for Announcement {
    fn drop(&mut self) {
        let _ = self.daemon.unregister(&self.fullname);
        let _ = self.daemon.shutdown();
    }
}

/// A device seen in the network.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub device: DeviceId,
    pub addresses: Vec<SocketAddr>,
}

/// Looks for Kalender devices for `wait`, returns what answered.
pub fn browse(wait: Duration) -> Result<Vec<Found>> {
    let daemon = ServiceDaemon::new()?;
    let events = daemon.browse(SERVICE)?;
    let end = Instant::now() + wait;
    let mut found: Vec<Found> = Vec::new();
    while let Some(left) = end.checked_duration_since(Instant::now()) {
        let Ok(event) = events.recv_timeout(left) else {
            break;
        };
        if let ServiceEvent::ServiceResolved(service) = event {
            let Some(device) = service
                .get_property_val_str("id")
                .and_then(|v| v.parse().ok())
            else {
                continue;
            };
            let addresses: Vec<SocketAddr> = service
                .addresses
                .iter()
                .map(|a| a.to_ip_addr())
                .filter(|ip| is_private(*ip))
                .map(|ip| SocketAddr::new(ip, service.port))
                .collect();
            match found.iter_mut().find(|f| f.device == device) {
                Some(f) => f.addresses.extend(addresses),
                None => found.push(Found { device, addresses }),
            }
        }
    }
    let _ = daemon.shutdown();
    Ok(found)
}
