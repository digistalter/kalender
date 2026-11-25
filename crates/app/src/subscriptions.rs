// SPDX-License-Identifier: GPL-3.0-or-later

//! Subscribed calendars (waste collection, school holidays, a work calendar …):
//! loaded from a link, refreshed every 30 minutes, read-only.

use std::rc::Rc;

use adw::prelude::*;
use anyhow::{Context, Result, bail};
use gtk::glib;
use kalender_core::ics::{self, Calendar};
use kalender_core::model::{Id, Kind, Source};
use kalender_core::subscription::{entries, normalize_url};
use soup::prelude::*;

use crate::draw::PALETTE;
use crate::i18n::{tr, trf};
use crate::state::now;
use crate::window::UiRc;

const REFRESH_MINUTES: u32 = 30;

/// Loads and reads a calendar file from the web.
async fn fetch(url: &str) -> Result<Calendar> {
    let session = soup::Session::new();
    session.set_timeout(30);
    session.set_user_agent(&format!("Kalender/{}", crate::config::VERSION));
    let message = soup::Message::new("GET", url).context(tr("This is not a valid link"))?;
    let bytes = session
        .send_and_read_future(&message, glib::Priority::DEFAULT)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e.message()))?;
    let code = message.status_code();
    if !(200..300).contains(&code) {
        bail!(trf(
            "The server answered with error {}",
            &[&code.to_string()]
        ));
    }
    let text = String::from_utf8_lossy(&bytes);
    ics::parse(&text).context(tr("The link does not lead to a calendar file"))
}

/// Starts regular updates: shortly after the start, then every 30 minutes.
pub fn start(ui: &UiRc) {
    let weak = Rc::downgrade(ui);
    glib::timeout_add_seconds_local_once(3, move || {
        if let Some(u) = weak.upgrade() {
            refresh_all(&u);
        }
    });
    let weak = Rc::downgrade(ui);
    glib::timeout_add_seconds_local(REFRESH_MINUTES * 60, move || {
        let Some(u) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        refresh_all(&u);
        glib::ControlFlow::Continue
    });
}

pub fn refresh_all(ui: &UiRc) {
    let ids: Vec<Id> = ui
        .state
        .borrow()
        .kinds
        .iter()
        .filter(|k| k.source.is_some())
        .map(|k| k.id)
        .collect();
    for id in ids {
        refresh(ui, id, false);
    }
}

/// Loads one subscribed calendar again. `report` shows the result as a toast.
pub fn refresh(ui: &UiRc, kind: Id, report: bool) {
    let Some(url) = ui
        .state
        .borrow()
        .kind(kind)
        .and_then(|k| k.source.as_ref())
        .map(|s| s.url.clone())
    else {
        return;
    };
    let weak = Rc::downgrade(ui);
    glib::MainContext::default().spawn_local(async move {
        let result = fetch(&url).await;
        let Some(u) = weak.upgrade() else { return };
        let Some(mut k) = u.state.borrow().kind(kind).cloned() else {
            return; // removed meanwhile
        };
        let source = k.source.as_mut().expect("subscription");
        let outcome = match result {
            Ok(calendar) => {
                let list = entries(&calendar, kind);
                let saved = u.state.borrow_mut().replace_subscribed(kind, list);
                match saved {
                    Ok(()) => {
                        source.updated = Some(now());
                        source.error = None;
                        Ok(())
                    }
                    Err(e) => Err(e.to_string()),
                }
            }
            Err(e) => {
                tracing::warn!("updating subscription {url}: {e:#}");
                Err(format!("{e:#}"))
            }
        };
        if let Err(e) = &outcome {
            source.error = Some(e.clone());
        }
        let name = crate::state::kind_label(&k);
        if let Err(e) = u.state.borrow_mut().save_kind(k) {
            u.error(&e);
        }
        u.rebuild_kinds();
        u.refresh();
        if report {
            match outcome {
                Ok(()) => u.toast(&trf("“{}” updated", &[&name])),
                Err(e) => u.toast(&trf("“{}” could not be updated: {}", &[&name, &e])),
            }
        }
    });
}

/// "Subscribe to calendar": asks for the link, loads it and adds it as a read-only kind.
pub fn add_dialog(ui: &UiRc) {
    let dialog = adw::AlertDialog::new(
        Some(&tr("Subscribe to calendar")),
        Some(&tr(
            "Paste the link to a calendar, e.g. waste collection, school holidays, your club or a work calendar. Subscribed calendars can only be read and are updated every 30 minutes.",
        )),
    );
    let list = gtk::ListBox::builder()
        .css_classes(["boxed-list"])
        .selection_mode(gtk::SelectionMode::None)
        .build();
    let link = adw::EntryRow::builder()
        .title(tr("Link (ends with .ics, or webcal://)"))
        .input_purpose(gtk::InputPurpose::Url)
        .build();
    let name = adw::EntryRow::builder()
        .title(tr("Name (optional)"))
        .build();
    list.append(&link);
    list.append(&name);
    dialog.set_extra_child(Some(&list));
    dialog.add_responses(&[("cancel", &tr("Cancel")), ("add", &tr("Subscribe"))]);
    dialog.set_response_appearance("add", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("add"));
    dialog.set_close_response("cancel");
    dialog.set_response_enabled("add", false);
    let d = dialog.clone();
    link.connect_changed(move |e| d.set_response_enabled("add", !e.text().trim().is_empty()));
    let weak = Rc::downgrade(ui);
    dialog.connect_response(Some("add"), move |_, _| {
        if let Some(u) = weak.upgrade() {
            subscribe(&u, &link.text(), name.text().trim());
        }
    });
    dialog.present(Some(&ui.window));
}

/// Loads the calendar behind `link` and adds it, or tells what went wrong.
pub fn subscribe(ui: &UiRc, link: &str, name: &str) {
    let url = normalize_url(link);
    let wanted = name.to_owned();
    ui.toast(&tr("Loading calendar …"));
    let weak = Rc::downgrade(ui);
    glib::MainContext::default().spawn_local(async move {
        let result = fetch(&url).await;
        let Some(u) = weak.upgrade() else { return };
        match result {
            Ok(calendar) => add(&u, url, wanted, calendar),
            Err(e) => u.toast(&trf("Could not subscribe: {}", &[&format!("{e:#}")])),
        }
    });
}

fn add(ui: &UiRc, url: String, wanted: String, calendar: Calendar) {
    let name = if !wanted.is_empty() {
        wanted
    } else if let Some(n) = calendar.name.clone() {
        n
    } else {
        glib::Uri::parse(&url, glib::UriFlags::NONE)
            .ok()
            .and_then(|u| u.host())
            .map(|h| h.to_string())
            .unwrap_or_else(|| tr("Subscribed calendar"))
    };
    // A colour not used yet, if there is one.
    let color = {
        let st = ui.state.borrow();
        PALETTE
            .iter()
            .find(|c| !st.kinds.iter().any(|k| k.color.eq_ignore_ascii_case(c)))
            .unwrap_or(&PALETTE[4])
            .to_string()
    };
    let kind = Kind {
        id: 0,
        name,
        color,
        icon: "globe".into(),
        uid: String::new(),
        builtin: None,
        visible: true,
        source: Some(Source {
            url,
            updated: Some(now()),
            error: None,
        }),
    };
    let title = kind.name.clone();
    let result = (|| -> Result<usize> {
        let mut st = ui.state.borrow_mut();
        let id = st.save_kind(kind)?;
        let list = entries(&calendar, id);
        let n = list.len();
        st.replace_subscribed(id, list)?;
        Ok(n)
    })();
    match result {
        Ok(n) => {
            ui.rebuild_kinds();
            ui.refresh();
            let count = crate::i18n::trn("{} entry", "{} entries", n);
            ui.toast(&trf("“{}” subscribed · {}", &[&title, &count]));
        }
        Err(e) => ui.error(&e),
    }
}

/// Asks before removing a subscription and its entries.
pub fn remove(ui: &UiRc, kind: Id) {
    let Some(name) = ui.state.borrow().kind(kind).map(crate::state::kind_label) else {
        return;
    };
    let dialog = adw::AlertDialog::new(
        Some(&tr("Remove subscription?")),
        Some(&trf(
            "“{}” and its entries disappear from your calendar. You can subscribe again at any time.",
            &[&name],
        )),
    );
    dialog.add_responses(&[("cancel", &tr("Cancel")), ("remove", &tr("Remove"))]);
    dialog.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
    dialog.set_close_response("cancel");
    let weak = Rc::downgrade(ui);
    dialog.connect_response(Some("remove"), move |_, _| {
        let Some(u) = weak.upgrade() else { return };
        let result = u.state.borrow_mut().delete_kind(kind);
        match result {
            Ok(()) => {
                u.rebuild_kinds();
                u.refresh();
                u.toast(&trf("“{}” removed", &[&name]));
            }
            Err(e) => u.error(&e),
        }
    });
    dialog.present(Some(&ui.window));
}
