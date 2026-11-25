// SPDX-License-Identifier: GPL-3.0-or-later

//! Syncing with paired devices in the home network (planung/KONZEPT-ABGLEICH.md).
//!
//! The network runs in its own threads; the calendar stays in the main
//! thread. Network threads reach it through [`Bridge`], which sends a
//! request to the main loop and waits for the answer. The main loop also
//! asks the person to compare check words and shows the results.
//!
//! This device listens all the time (while syncing is on) and announces
//! itself with mDNS. It also syncs by itself: shortly after own changes and
//! every five minutes, with every paired device it can reach.

use std::cell::{Cell, RefCell};
use std::net::{SocketAddr, TcpListener, UdpSocket};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use adw::prelude::*;
use anyhow::{Result, anyhow};
use gtk::glib;
use kalender_core::devices::Peer;
use kalender_link::pair::Offer;
use kalender_link::sync::{Accepted, accept, sync};
use kalender_link::{Calendar, DeviceKind, Identity, PeerInfo, Peers, discover};
use kalender_sync::{Change, DeviceId, Known};

use crate::i18n::{tr, trf};
use crate::state::now;
use crate::window::UiRc;

/// Preferred port; another one is taken if it is busy.
const PORT: u16 = 47011;
const EVERY_MINUTES: u32 = 5;
const AFTER_CHANGE_SECONDS: u32 = 8;
const WAIT: Duration = Duration::from_secs(120);

/// What network threads ask the main thread.
enum Request {
    Known(mpsc::Sender<Known>),
    ChangesFor(Known, mpsc::Sender<Vec<Change>>),
    Apply(Vec<Change>, mpsc::Sender<std::result::Result<bool, String>>),
    Peer(DeviceId, mpsc::Sender<Option<PeerInfo>>),
    Confirm([&'static str; 4], PeerInfo, mpsc::Sender<bool>),
    Event(Event),
}

/// What happened, for the main thread to show and remember.
#[derive(Debug)]
pub enum Event {
    Paired(PeerInfo, Option<SocketAddr>),
    PairFailed(String),
    Synced {
        peer: DeviceId,
        address: Option<SocketAddr>,
    },
    /// A sync run started by this device is over; `reached` = devices reached.
    RunDone {
        reached: Vec<String>,
        missed: Vec<String>,
        report: bool,
    },
}

/// The calendar as seen from a network thread.
#[derive(Clone)]
pub struct Bridge {
    tx: async_channel::Sender<Request>,
}

impl Bridge {
    fn ask<T>(&self, make: impl FnOnce(mpsc::Sender<T>) -> Request) -> Result<T> {
        let (tx, rx) = mpsc::channel();
        self.tx
            .send_blocking(make(tx))
            .map_err(|_| anyhow!("app closed"))?;
        rx.recv_timeout(WAIT)
            .map_err(|_| anyhow!("no answer from the app"))
    }

    pub fn event(&self, event: Event) {
        let _ = self.tx.send_blocking(Request::Event(event));
    }

    pub fn confirm(&self, words: &[&'static str; 4], peer: &PeerInfo) -> bool {
        let (tx, rx) = mpsc::channel();
        if self
            .tx
            .send_blocking(Request::Confirm(*words, peer.clone(), tx))
            .is_err()
        {
            return false;
        }
        rx.recv_timeout(Duration::from_secs(300)).unwrap_or(false)
    }
}

impl Calendar for Bridge {
    fn known(&self) -> Known {
        self.ask(Request::Known).unwrap_or_default()
    }
    fn changes_for(&self, theirs: &Known) -> Vec<Change> {
        self.ask(|tx| Request::ChangesFor(theirs.clone(), tx))
            .unwrap_or_default()
    }
    fn apply(&self, changes: &[Change]) -> Result<bool> {
        self.ask(|tx| Request::Apply(changes.to_vec(), tx))?
            .map_err(|e| anyhow!(e))
    }
}

impl Peers for Bridge {
    fn peer(&self, device: DeviceId) -> Option<PeerInfo> {
        self.ask(|tx| Request::Peer(device, tx)).ok().flatten()
    }
}

/// Main-thread side, kept for the life of the window.
pub struct NetSync {
    pub identity: Identity,
    bridge: Bridge,
    /// The pairing invitation while the „Gerät koppeln“ window is open.
    pub offer: Arc<Mutex<Option<Arc<Offer>>>>,
    /// Port this device listens on, once the listener runs.
    pub port: Arc<Mutex<Option<u16>>>,
    stop: Arc<AtomicBool>,
    running: Cell<bool>,
    busy: Arc<AtomicBool>,
    /// The own changes last sent, to sync only when something is new.
    pushed: RefCell<Option<kalender_sync::Stamp>>,
    after_change: RefCell<Option<glib::SourceId>>,
    /// Refreshes the device list in the settings, while they are open.
    pub on_change: RefCell<Option<Box<dyn Fn()>>>,
    /// The open „Gerät koppeln“ window, closed once a device is paired.
    pub pair_window: RefCell<Option<glib::WeakRef<adw::AlertDialog>>>,
}

thread_local! {
    static NET: RefCell<Option<Rc<NetSync>>> = const { RefCell::new(None) };
}

/// The running sync part, if started.
pub fn get() -> Option<Rc<NetSync>> {
    NET.with(|n| n.borrow().clone())
}

/// This computer's name, as other devices see it.
pub fn host_name() -> String {
    glib::host_name().to_string()
}

/// The address other devices in the home network reach this computer at
/// (no packet is sent to find it out).
pub fn local_ip() -> Option<std::net::IpAddr> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:9").ok()?;
    socket
        .local_addr()
        .ok()
        .map(|a| a.ip())
        .filter(|ip| kalender_link::is_private(*ip))
}

pub fn start(ui: &UiRc) {
    if std::env::var_os("KALENDER_DEMO").is_some() {
        return; // demo data never leaves this computer
    }
    let identity = match ui
        .state
        .borrow()
        .store
        .identity(&host_name(), DeviceKind::Desktop)
    {
        Ok(id) => id,
        Err(e) => {
            tracing::warn!("sync: no identity: {e:#}");
            return;
        }
    };
    let (tx, rx) = async_channel::unbounded();
    let net = Rc::new(NetSync {
        identity,
        bridge: Bridge { tx },
        offer: Arc::default(),
        port: Arc::default(),
        stop: Arc::default(),
        running: Cell::new(false),
        busy: Arc::default(),
        pushed: RefCell::default(),
        after_change: RefCell::default(),
        on_change: RefCell::default(),
        pair_window: RefCell::default(),
    });
    NET.with(|n| *n.borrow_mut() = Some(net.clone()));

    // Answers network threads on the main loop.
    let weak = Rc::downgrade(ui);
    glib::MainContext::default().spawn_local(async move {
        while let Ok(request) = rx.recv().await {
            let Some(u) = weak.upgrade() else { break };
            handle(&u, request);
        }
    });

    net.apply_setting(ui);
    let weak = Rc::downgrade(ui);
    glib::timeout_add_seconds_local(EVERY_MINUTES * 60, move || {
        let Some(u) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        if let Some(net) = get() {
            net.sync_now(&u, false);
        }
        glib::ControlFlow::Continue
    });
    let weak = Rc::downgrade(ui);
    glib::timeout_add_seconds_local_once(4, move || {
        if let (Some(u), Some(net)) = (weak.upgrade(), get()) {
            net.sync_now(&u, false);
        }
    });
}

impl NetSync {
    /// Starts or pauses listening, following the setting.
    pub fn apply_setting(&self, ui: &UiRc) {
        let enabled = ui.state.borrow().settings.sync_enabled;
        if enabled && !self.running.get() {
            self.stop.store(false, Ordering::SeqCst);
            self.listen();
            self.running.set(true);
        } else if !enabled && self.running.get() {
            self.stop.store(true, Ordering::SeqCst);
            // Wake the listener so it sees the stop.
            if let Some(port) = *self.port.lock().expect("port") {
                let _ = std::net::TcpStream::connect(("127.0.0.1", port));
            }
            self.running.set(false);
        }
    }

    fn listen(&self) {
        let (bridge, me, offer, stop, port_slot) = (
            self.bridge.clone(),
            self.identity.clone(),
            self.offer.clone(),
            self.stop.clone(),
            self.port.clone(),
        );
        std::thread::spawn(move || {
            let listener =
                TcpListener::bind(("0.0.0.0", PORT)).or_else(|_| TcpListener::bind(("0.0.0.0", 0)));
            let listener = match listener {
                Ok(l) => l,
                Err(e) => return tracing::warn!("sync: cannot listen: {e}"),
            };
            let port = listener.local_addr().map(|a| a.port()).unwrap_or(PORT);
            *port_slot.lock().expect("port") = Some(port);
            let _announce = discover::announce(me.device, port)
                .map_err(|e| tracing::warn!("sync: not announced: {e:#}"))
                .ok();
            for stream in listener.incoming() {
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(stream) = stream else { continue };
                let from = stream.peer_addr().ok();
                let current = offer.lock().expect("offer").clone();
                let open = current.as_deref().filter(|o| !o.used_up());
                let b = bridge.clone();
                let mut confirm = |w: &[&'static str; 4], p: &PeerInfo| b.confirm(w, p);
                match accept(stream, &me, &bridge, &bridge, open, &mut confirm) {
                    Ok(Accepted::Paired(info)) => bridge.event(Event::Paired(info, None)),
                    Ok(Accepted::Synced { peer, .. }) => bridge.event(Event::Synced {
                        peer: peer.device,
                        address: None,
                    }),
                    Ok(Accepted::NotPaired) => {
                        bridge.event(Event::PairFailed(tr("Pairing was cancelled")))
                    }
                    Err(e) => {
                        tracing::info!("sync: connection from {from:?} refused: {e:#}");
                        if open.is_some() && current.as_deref().is_some_and(Offer::used_up) {
                            bridge.event(Event::PairFailed(tr("The code did not fit")));
                        }
                    }
                }
            }
            *port_slot.lock().expect("port") = None;
        });
    }

    pub fn devices_changed(&self) {
        if let Some(f) = self.on_change.borrow().as_ref() {
            f();
        }
    }

    /// After own changes: sync a little later, once things are quiet.
    pub fn changed_here(&self, ui: &UiRc) {
        if let Some(id) = self.after_change.borrow_mut().take() {
            id.remove();
        }
        let weak = Rc::downgrade(ui);
        let id = glib::timeout_add_seconds_local_once(AFTER_CHANGE_SECONDS, move || {
            if let (Some(u), Some(net)) = (weak.upgrade(), get()) {
                net.after_change.borrow_mut().take();
                net.sync_now(&u, false);
            }
        });
        *self.after_change.borrow_mut() = Some(id);
    }

    /// Whether this device changed anything since the last sync run.
    fn has_news(&self, ui: &UiRc) -> bool {
        let own = ui
            .state
            .borrow()
            .store
            .sync_known()
            .get(&self.identity.device)
            .copied();
        own != *self.pushed.borrow()
    }

    /// Syncs with every paired device that can be reached. `report` shows the
    /// result as a toast (for „Jetzt abgleichen“).
    pub fn sync_now(&self, ui: &UiRc, report: bool) {
        let (enabled, peers) = {
            let st = ui.state.borrow();
            (
                st.settings.sync_enabled,
                st.store.peers().unwrap_or_default(),
            )
        };
        if !enabled || peers.is_empty() || self.busy.load(Ordering::SeqCst) {
            return;
        }
        if !report
            && !self.has_news(ui)
            && peers.iter().all(|p| {
                p.synced.is_some_and(|t| {
                    now() - t < chrono::Duration::minutes(EVERY_MINUTES as i64 - 1)
                })
            })
        {
            return;
        }
        *self.pushed.borrow_mut() = ui
            .state
            .borrow()
            .store
            .sync_known()
            .get(&self.identity.device)
            .copied();
        self.busy.store(true, Ordering::SeqCst);
        let (bridge, me, busy) = (
            self.bridge.clone(),
            self.identity.clone(),
            self.busy.clone(),
        );
        std::thread::spawn(move || {
            let found = discover::browse(Duration::from_secs(2)).unwrap_or_default();
            let (mut reached, mut missed) = (Vec::new(), Vec::new());
            for peer in peers {
                let mut addresses: Vec<SocketAddr> =
                    peer.address.iter().filter_map(|a| a.parse().ok()).collect();
                if let Some(f) = found.iter().find(|f| f.device == peer.info.device) {
                    for a in &f.addresses {
                        if !addresses.contains(a) {
                            addresses.push(*a);
                        }
                    }
                }
                let done = addresses
                    .iter()
                    .find_map(|a| sync(*a, &me, &peer.info, &bridge).ok().map(|r| (*a, r)));
                match done {
                    Some((address, r)) => {
                        reached.push(peer.info.name.clone());
                        let _ = r;
                        bridge.event(Event::Synced {
                            peer: peer.info.device,
                            address: Some(address),
                        });
                    }
                    None => missed.push(peer.info.name.clone()),
                }
            }
            busy.store(false, Ordering::SeqCst);
            bridge.event(Event::RunDone {
                reached,
                missed,
                report,
            });
        });
    }

    /// This device joins another one (which shows a number): pairs, then syncs.
    pub fn pair_with(&self, address: SocketAddr, code: String) {
        let (bridge, me) = (self.bridge.clone(), self.identity.clone());
        std::thread::spawn(move || {
            let b = bridge.clone();
            let mut confirm = |w: &[&'static str; 4], p: &PeerInfo| b.confirm(w, p);
            match kalender_link::pair::pair(address, &me, &code, None, &mut confirm) {
                Ok(Some(info)) => bridge.event(Event::Paired(info, Some(address))),
                Ok(None) => bridge.event(Event::PairFailed(tr("Pairing was cancelled"))),
                Err(e) => bridge.event(Event::PairFailed(format!("{e:#}"))),
            }
        });
    }
}

/// Answers one request from a network thread, on the main thread.
fn handle(ui: &UiRc, request: Request) {
    match request {
        Request::Known(tx) => {
            let _ = tx.send(ui.state.borrow().store.sync_known());
        }
        Request::ChangesFor(theirs, tx) => {
            let _ = tx.send(ui.state.borrow().store.sync_changes_for(&theirs));
        }
        Request::Apply(changes, tx) => {
            let result = ui
                .state
                .borrow()
                .store
                .sync_apply(&changes)
                .map_err(|e| format!("{e:#}"));
            if result == Ok(true) {
                reload_soon(ui);
            }
            let _ = tx.send(result);
        }
        Request::Peer(device, tx) => {
            let _ = tx.send(ui.state.borrow().store.peer(device));
        }
        Request::Confirm(words, peer, tx) => crate::devices::confirm_words(ui, words, peer, tx),
        Request::Event(event) => on_event(ui, event),
    }
}

/// Shows what other devices changed, once a batch is through.
fn reload_soon(ui: &UiRc) {
    thread_local! {
        static PENDING: Cell<bool> = const { Cell::new(false) };
    }
    if PENDING.with(|p| p.replace(true)) {
        return;
    }
    let weak = Rc::downgrade(ui);
    glib::idle_add_local_once(move || {
        PENDING.with(|p| p.set(false));
        let Some(u) = weak.upgrade() else { return };
        let result = u.state.borrow_mut().reload();
        if let Err(e) = result {
            u.error(&e);
        }
        u.rebuild_kinds();
        u.refresh_without_sync();
    });
}

fn on_event(ui: &UiRc, event: Event) {
    match event {
        Event::Paired(info, address) => {
            let peer = Peer {
                info: info.clone(),
                address: address.map(|a| a.to_string()),
                synced: None,
            };
            if let Err(e) = ui.state.borrow().store.save_peer(&peer) {
                return ui.error(&e);
            }
            if let Some(net) = get() {
                *net.offer.lock().expect("offer") = None;
                net.devices_changed();
                if let Some(d) = net
                    .pair_window
                    .borrow_mut()
                    .take()
                    .and_then(|w| w.upgrade())
                {
                    d.close();
                }
            }
            ui.toast(&trf("“{}” paired", &[&info.name]));
            if address.is_some() {
                // This device joined: merge or take over, then sync.
                crate::devices::after_joining(ui, &info);
            }
        }
        Event::PairFailed(message) => ui.toast(&trf("Not paired: {}", &[&message])),
        Event::Synced { peer, address } => {
            let store = &ui.state.borrow().store;
            if let Some(mut p) = store
                .peers()
                .unwrap_or_default()
                .into_iter()
                .find(|p| p.info.device == peer)
            {
                p.synced = Some(now());
                if let Some(a) = address {
                    p.address = Some(a.to_string());
                }
                let _ = store.save_peer(&p);
            }
            ui.update_sync_status();
            if let Some(net) = get() {
                net.devices_changed();
            }
        }
        Event::RunDone {
            reached,
            missed,
            report,
        } => {
            ui.update_sync_status();
            if report {
                if !missed.is_empty() {
                    ui.toast(&trf(
                        "{} not reachable right now. Changes are synced as soon as you are in the same Wi-Fi.",
                        &[&missed.join(", ")],
                    ));
                } else if !reached.is_empty() {
                    ui.toast(&trf("Synced with {}", &[&reached.join(", ")]));
                }
            }
        }
    }
}
