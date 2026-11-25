// SPDX-License-Identifier: GPL-3.0-or-later

//! Settings → Devices: paired devices, pairing with a QR code or a number,
//! comparing check words, and merging or taking over on the first sync.

use std::net::SocketAddr;
use std::rc::Rc;
use std::sync::{Arc, mpsc};

use adw::prelude::*;
use gtk::{gio, glib};
use kalender_link::pair::Offer;
use kalender_link::{DeviceKind, PeerInfo};

use crate::i18n::{tr, trf};
use crate::netsync;
use crate::state::now;
use crate::window::UiRc;

fn kind_icon(kind: DeviceKind) -> &'static str {
    match kind {
        DeviceKind::Desktop => "computer-symbolic",
        DeviceKind::Phone => "phone-symbolic",
        DeviceKind::Tablet => "tablet-symbolic",
    }
}

/// The page in the settings.
pub fn page(ui: &UiRc) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::builder()
        .title(tr("Devices"))
        .icon_name("phone-symbolic")
        .name("devices")
        .build();
    let list = adw::PreferencesGroup::builder()
        .title(tr("Paired devices"))
        .description(tr(
            "Your devices sync directly with each other as soon as they are in the same Wi-Fi – without a server and without an account, end-to-end encrypted. On the way, every device simply works on.",
        ))
        .build();
    let rows: Rc<std::cell::RefCell<Vec<gtk::Widget>>> = Rc::default();
    let fill = {
        let (ui, list, rows) = (ui.clone(), list.clone(), rows.clone());
        Rc::new(move || fill_list(&ui, &list, &rows))
    };
    fill();
    if let Some(net) = netsync::get() {
        let f = fill.clone();
        *net.on_change.borrow_mut() = Some(Box::new(move || f()));
    }
    page.add(&list);

    let settings = adw::PreferencesGroup::builder().title(tr("Sync")).build();
    let on = adw::SwitchRow::builder()
        .title(tr("Sync in the home network"))
        .subtitle(tr("Off: no connections; the pairings stay"))
        .active(ui.state.borrow().settings.sync_enabled)
        .build();
    let u = ui.clone();
    on.connect_active_notify(move |r| {
        u.state.borrow_mut().settings.sync_enabled = r.is_active();
        if let Err(e) = u.state.borrow().save_settings() {
            u.error(&e);
        }
        if let Some(net) = netsync::get() {
            net.apply_setting(&u);
        }
        u.update_sync_status();
    });
    settings.add(&on);
    let this = adw::ActionRow::builder()
        .title(tr("This device"))
        .subtitle(glib::markup_escape_text(&match netsync::local_ip() {
            Some(ip) => format!("{} · {ip}", netsync::host_name()),
            None => netsync::host_name(),
        }))
        .build();
    settings.add(&this);
    page.add(&settings);
    page
}

fn fill_list(
    ui: &UiRc,
    list: &adw::PreferencesGroup,
    rows: &Rc<std::cell::RefCell<Vec<gtk::Widget>>>,
) {
    for row in rows.borrow_mut().drain(..) {
        list.remove(&row);
    }
    let peers = ui.state.borrow().store.peers().unwrap_or_default();
    let mut added: Vec<gtk::Widget> = Vec::new();
    for peer in peers {
        let subtitle = match peer.synced {
            Some(t) => trf("Synced {}", &[&crate::fmt::ago(t, now())]),
            None => tr("Not synced yet"),
        };
        let row = adw::ActionRow::builder()
            .title(glib::markup_escape_text(&peer.info.name))
            .subtitle(subtitle)
            .build();
        row.add_prefix(&gtk::Image::from_icon_name(kind_icon(peer.info.kind)));
        let menu = gio::Menu::new();
        let item = |label: &str, action: &str| {
            let i = gio::MenuItem::new(Some(label), None);
            i.set_action_and_target_value(
                Some(action),
                Some(&(peer.info.device as i64).to_variant()),
            );
            menu.append_item(&i);
        };
        item(&tr("Sync now"), "win.device-sync");
        item(&tr("Remove device …"), "win.device-remove");
        row.add_suffix(
            &gtk::MenuButton::builder()
                .icon_name("view-more-symbolic")
                .menu_model(&menu)
                .valign(gtk::Align::Center)
                .css_classes(["flat", "circular"])
                .build(),
        );
        list.add(&row);
        added.push(row.upcast());
    }
    for (label, icon, join) in [
        (tr("Pair a device …"), "list-add-symbolic", false),
        (
            tr("Pair with another computer …"),
            "network-wired-symbolic",
            true,
        ),
    ] {
        let row = adw::ActionRow::builder()
            .title(label)
            .activatable(true)
            .build();
        row.add_prefix(&gtk::Image::from_icon_name(icon));
        row.add_css_class("accent");
        let u = ui.clone();
        row.connect_activated(move |_| {
            if join {
                join_dialog(&u)
            } else {
                pair_dialog(&u)
            }
        });
        list.add(&row);
        added.push(row.upcast());
    }
    *rows.borrow_mut() = added;
}

/// Window actions for the menu of each paired device.
pub fn add_actions(ui: &UiRc) {
    let action = |name: &str, f: fn(&UiRc, kalender_sync::DeviceId)| {
        let a = gio::SimpleAction::new(name, Some(glib::VariantTy::INT64));
        let weak = Rc::downgrade(ui);
        a.connect_activate(move |_, v| {
            if let (Some(u), Some(id)) = (weak.upgrade(), v.and_then(|v| v.get::<i64>())) {
                f(&u, id as u64);
            }
        });
        ui.window.add_action(&a);
    };
    action("device-sync", |u, _| {
        if let Some(net) = netsync::get() {
            net.sync_now(u, true);
        }
    });
    action("device-remove", remove_dialog);
    let a = gio::SimpleAction::new("sync-now", None);
    let weak = Rc::downgrade(ui);
    a.connect_activate(move |_, _| {
        if let (Some(u), Some(net)) = (weak.upgrade(), netsync::get()) {
            net.sync_now(&u, true);
        }
    });
    ui.window.add_action(&a);
}

fn remove_dialog(ui: &UiRc, device: kalender_sync::DeviceId) {
    let Some(name) = ui
        .state
        .borrow()
        .store
        .peers()
        .unwrap_or_default()
        .into_iter()
        .find(|p| p.info.device == device)
        .map(|p| p.info.name)
    else {
        return;
    };
    let dialog = adw::AlertDialog::new(
        Some(&tr("Remove device?")),
        Some(&trf(
            "“{}” can no longer sync with this computer. Its entries stay on all devices.",
            &[&name],
        )),
    );
    dialog.add_responses(&[("cancel", &tr("Cancel")), ("remove", &tr("Remove"))]);
    dialog.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
    dialog.set_close_response("cancel");
    let weak = Rc::downgrade(ui);
    dialog.connect_response(Some("remove"), move |_, _| {
        let Some(u) = weak.upgrade() else { return };
        let result = u.state.borrow().store.remove_peer(device);
        match result {
            Ok(()) => {
                u.toast(&trf("“{}” removed", &[&name]));
                u.update_sync_status();
                if let Some(net) = netsync::get() {
                    net.devices_changed();
                }
            }
            Err(e) => u.error(&e),
        }
    });
    dialog.present(Some(&ui.window));
}

/// For the sync test: opens the window and returns „ADDRESS NUMBER“.
pub fn open_pair_dialog_for_test(ui: &UiRc) -> String {
    pair_dialog(ui);
    let net = netsync::get().expect("sync");
    let offer = net.offer.lock().expect("offer").clone();
    let port = (*net.port.lock().expect("port")).unwrap_or_default();
    match offer {
        Some(o) => format!("127.0.0.1:{port} {}", o.number),
        None => String::new(),
    }
}

/// Shows a QR code and a number; the other device scans or types it.
fn pair_dialog(ui: &UiRc) {
    let Some(net) = netsync::get() else { return };
    let port = *net.port.lock().expect("port");
    let (Some(port), Some(ip)) = (port, netsync::local_ip()) else {
        ui.toast(&tr(
            "Turn on “Sync in the home network” and connect to your Wi-Fi first",
        ));
        return;
    };
    let offer = match Offer::new() {
        Ok(o) => Arc::new(o),
        Err(e) => return ui.error(&e),
    };
    let address = SocketAddr::new(ip, port);
    let qr_text = offer.qr(&net.identity, &[address]);
    *net.offer.lock().expect("offer") = Some(offer.clone());

    let body = gtk::Box::new(gtk::Orientation::Vertical, 10);
    body.append(&qr_view(&qr_text));
    let number: String = offer
        .number
        .chars()
        .enumerate()
        .flat_map(|(i, c)| if i == 3 { vec![' ', c] } else { vec![c] })
        .collect();
    body.append(
        &gtk::Label::builder()
            .label(trf(
                "Without a camera: address {} · number {}",
                &[&address.to_string(), &number],
            ))
            .wrap(true)
            .justify(gtk::Justification::Center)
            .css_classes(["dim-label"])
            .build(),
    );
    let dialog = adw::AlertDialog::new(
        Some(&tr("Pair a device")),
        Some(&tr(
            "Open Kalender on the phone or tablet → “Pair with computer” and scan the code.",
        )),
    );
    dialog.set_extra_child(Some(&body));
    dialog.add_responses(&[("close", &tr("Close"))]);
    dialog.set_close_response("close");
    let n = net.clone();
    dialog.connect_closed(move |_| {
        *n.offer.lock().expect("offer") = None;
    });
    // Closes by itself once paired.
    *net.pair_window.borrow_mut() = Some(dialog.downgrade());
    dialog.present(Some(&ui.window));
}

/// Draws a QR code.
fn qr_view(text: &str) -> gtk::Widget {
    let area = gtk::DrawingArea::builder()
        .content_width(220)
        .content_height(220)
        .halign(gtk::Align::Center)
        .build();
    let Ok(code) = qrcode::QrCode::new(text.as_bytes()) else {
        return area.upcast();
    };
    let width = code.width();
    let dark: Vec<bool> = code
        .to_colors()
        .into_iter()
        .map(|c| c == qrcode::Color::Dark)
        .collect();
    area.set_draw_func(move |_, cr, w, h| {
        let quiet = 4.0;
        let size = f64::from(w.min(h));
        let cell = size / (width as f64 + 2.0 * quiet);
        cr.set_source_rgb(1.0, 1.0, 1.0);
        cr.rectangle(0.0, 0.0, size, size);
        let _ = cr.fill();
        cr.set_source_rgb(0.0, 0.0, 0.0);
        for (i, on) in dark.iter().enumerate() {
            if *on {
                let (x, y) = ((i % width) as f64, (i / width) as f64);
                cr.rectangle(
                    (x + quiet) * cell,
                    (y + quiet) * cell,
                    cell + 0.3,
                    cell + 0.3,
                );
            }
        }
        let _ = cr.fill();
    });
    area.upcast()
}

/// This computer joins another one that shows a number.
fn join_dialog(ui: &UiRc) {
    let dialog = adw::AlertDialog::new(
        Some(&tr("Pair with another computer")),
        Some(&tr(
            "On the other computer, open Settings → Devices → “Pair a device …” and type its address and number here.",
        )),
    );
    let list = gtk::ListBox::builder()
        .css_classes(["boxed-list"])
        .selection_mode(gtk::SelectionMode::None)
        .build();
    let address = adw::EntryRow::builder()
        .title(tr("Address (e.g. 192.168.178.20)"))
        .build();
    let number = adw::EntryRow::builder()
        .title(tr("Number"))
        .input_purpose(gtk::InputPurpose::Digits)
        .build();
    list.append(&address);
    list.append(&number);
    dialog.set_extra_child(Some(&list));
    dialog.add_responses(&[("cancel", &tr("Cancel")), ("pair", &tr("Pair"))]);
    dialog.set_response_appearance("pair", adw::ResponseAppearance::Suggested);
    dialog.set_close_response("cancel");
    let weak = Rc::downgrade(ui);
    dialog.connect_response(Some("pair"), move |_, _| {
        let Some(u) = weak.upgrade() else { return };
        let text = address.text().trim().to_owned();
        let parsed: Option<SocketAddr> = text.parse().ok().or_else(|| {
            text.parse::<std::net::IpAddr>()
                .ok()
                .map(|ip| SocketAddr::new(ip, 47011))
        });
        let code: String = number.text().chars().filter(char::is_ascii_digit).collect();
        match (parsed, code.len()) {
            (Some(a), 6) => {
                u.toast(&tr("Connecting …"));
                if let Some(net) = netsync::get() {
                    net.pair_with(a, code);
                }
            }
            (None, _) => u.toast(&tr("Please check the address")),
            _ => u.toast(&tr("The number has six digits")),
        }
    });
    dialog.present(Some(&ui.window));
}

/// Asks the person whether the check words match those on the other device.
pub fn confirm_words(
    ui: &UiRc,
    words: [&'static str; 4],
    peer: PeerInfo,
    answer: mpsc::Sender<bool>,
) {
    if crate::devtools::auto_confirm() {
        tracing::info!("check words {}", words.join(" · "));
        let _ = answer.send(true);
        return;
    }
    ui.window.present();
    let dialog = adw::AlertDialog::new(
        Some(&trf("Pair “{}”?", &[&peer.name])),
        Some(&tr("Do these words match the ones on the other device?")),
    );
    dialog.set_extra_child(Some(
        &gtk::Label::builder()
            .label(words.join(" · "))
            .css_classes(["title-2"])
            .margin_top(6)
            .build(),
    ));
    dialog.add_responses(&[("no", &tr("No")), ("yes", &tr("Pair"))]);
    dialog.set_response_appearance("yes", adw::ResponseAppearance::Suggested);
    dialog.set_close_response("no");
    dialog.connect_response(None, move |_, response| {
        let _ = answer.send(response == "yes");
    });
    dialog.present(Some(&ui.window));
}

/// After this computer joined another device: merge or take over, then sync.
pub fn after_joining(ui: &UiRc, other: &PeerInfo) {
    let has_own = ui.state.borrow().store.has_own_data().unwrap_or(false);
    let sync = |u: &UiRc| {
        if let Some(net) = netsync::get() {
            net.sync_now(u, true);
        }
    };
    if !has_own || crate::devtools::auto_confirm() {
        return sync(ui);
    }
    let dialog = adw::AlertDialog::new(
        Some(&tr("This calendar already has entries")),
        Some(&tr(
            "Merge both calendars into one, or replace this one by the calendar of the other device? Before replacing, a backup is made.",
        )),
    );
    dialog.add_responses(&[
        ("take", &trf("Take over from “{}”", &[&other.name])),
        ("merge", &tr("Merge both")),
    ]);
    dialog.set_response_appearance("merge", adw::ResponseAppearance::Suggested);
    dialog.set_response_appearance("take", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("merge"));
    dialog.set_close_response("merge");
    let weak = Rc::downgrade(ui);
    dialog.connect_response(None, move |_, response| {
        let Some(u) = weak.upgrade() else { return };
        if response == "take" {
            let result = (|| -> anyhow::Result<()> {
                crate::backup::run(&u.state.borrow(), true)?;
                u.state.borrow().store.reset_for_takeover()?;
                u.state.borrow_mut().reload()?;
                Ok(())
            })();
            if let Err(e) = result {
                return u.error(&e);
            }
            u.rebuild_kinds();
            u.refresh_without_sync();
        }
        sync(&u);
    });
    dialog.present(Some(&ui.window));
}
