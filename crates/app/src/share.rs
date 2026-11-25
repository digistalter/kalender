// SPDX-License-Identifier: GPL-3.0-or-later

//! Invitations go out as e-mail with a calendar file attached; the mail app
//! sends them. Answers and invitations from others come back as files the
//! calendar opens. Also: import and export of calendar files.

use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};
use kalender_core::ics::{self, Method, WriteOptions};
use kalender_core::model::{Builtin, Entry, Id, Partstat};

use crate::fmt;
use crate::i18n::{tr, trf};
use crate::window::UiRc;

/// Makes sure the own name and address are known; asks once otherwise.
fn with_identity(ui: &UiRc, then: impl Fn(&UiRc) + 'static) {
    let known = !ui.state.borrow().settings.own_email.trim().is_empty();
    if known {
        then(ui);
        return;
    }
    let dialog = adw::AlertDialog::new(
        Some(&tr("Your e-mail address")),
        Some(&tr(
            "Invitations are sent from this address, and answers to it are recognised. You can change it later in the settings.",
        )),
    );
    let list = gtk::ListBox::builder()
        .css_classes(["boxed-list"])
        .selection_mode(gtk::SelectionMode::None)
        .build();
    let name = adw::EntryRow::builder().title(tr("Your name")).build();
    let email = adw::EntryRow::builder()
        .title(tr("Your e-mail address"))
        .build();
    list.append(&name);
    list.append(&email);
    dialog.set_extra_child(Some(&list));
    dialog.add_responses(&[("cancel", &tr("Cancel")), ("ok", &tr("Continue"))]);
    dialog.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
    dialog.set_close_response("cancel");
    let weak = Rc::downgrade(ui);
    dialog.connect_response(Some("ok"), move |_, _| {
        let Some(u) = weak.upgrade() else { return };
        let address = email.text().trim().to_owned();
        if !address.contains('@') {
            u.toast(&tr("Please enter a valid e-mail address"));
            return;
        }
        {
            let mut st = u.state.borrow_mut();
            st.settings.own_email = address;
            st.settings.own_name = name.text().trim().to_owned();
        }
        if let Err(e) = u.state.borrow().save_settings() {
            u.error(&e);
        }
        then(&u);
    });
    dialog.present(Some(&ui.window));
}

fn cache_file(name: &str, content: &str) -> anyhow::Result<PathBuf> {
    let dir = glib::user_cache_dir().join("kalender");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(name);
    std::fs::write(&path, content)?;
    Ok(path)
}

fn when(entry: &Entry) -> String {
    if entry.all_day {
        fmt::date_short(entry.start.date())
    } else {
        format!(
            "{}, {} – {}",
            fmt::date_short(entry.start.date()),
            fmt::time_of(entry.start),
            fmt::time_of(entry.end)
        )
    }
}

/// Opens the mail app with recipients, subject, text and the calendar file.
fn compose(
    ui: &UiRc,
    to: Vec<String>,
    subject: String,
    body: String,
    file: PathBuf,
    done_text: String,
) {
    let weak = Rc::downgrade(ui);
    glib::MainContext::default().spawn_local(async move {
        let result = async {
            let fd: std::os::fd::OwnedFd = std::fs::File::open(&file)?.into();
            ashpd::desktop::email::EmailRequest::default()
                .addresses(to.clone())
                .subject(subject.as_str())
                .body(body.as_str())
                .attach(fd)
                .send()
                .await?;
            anyhow::Ok(())
        }
        .await;
        let Some(u) = weak.upgrade() else { return };
        match result {
            Ok(()) => u.toast(&done_text),
            Err(e) => {
                tracing::warn!("mail portal unavailable: {e:#}");
                // Without the portal: xdg-email, or at least keep the file.
                let ok = std::process::Command::new("xdg-email")
                    .arg("--subject")
                    .arg(&subject)
                    .arg("--body")
                    .arg(&body)
                    .arg("--attach")
                    .arg(&file)
                    .args(&to)
                    .spawn()
                    .is_ok();
                if ok {
                    u.toast(&done_text);
                } else {
                    u.toast(&trf(
                        "No mail app found. The file is here: {}",
                        &[&file.display().to_string()],
                    ));
                }
            }
        }
    });
}

fn options<'a>(
    ui_name: &'a str,
    ui_email: &'a str,
    method: Method,
    details: bool,
) -> WriteOptions<'a> {
    WriteOptions {
        method,
        own_email: ui_email,
        own_name: ui_name,
        details,
    }
}

fn greeting_name(ui: &UiRc) -> String {
    let st = ui.state.borrow();
    st.settings
        .own_name
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_owned()
}

/// Sends (or sends again) the invitation of a meeting to all participants.
pub fn invite(ui: &UiRc, id: Id) {
    with_identity(ui, move |u| {
        let Some(entry) = u.state.borrow().entry(id).cloned() else {
            return;
        };
        if entry.participants.is_empty() {
            return;
        }
        let text = {
            let st = u.state.borrow();
            ics::write(
                &[&entry],
                &options(
                    &st.settings.own_name,
                    &st.settings.own_email,
                    Method::Request,
                    true,
                ),
            )
        };
        let file = match cache_file("einladung.ics", &text) {
            Ok(f) => f,
            Err(e) => return u.error(&e),
        };
        let to: Vec<String> = entry
            .participants
            .iter()
            .map(|p| format!("{} <{}>", p.name, p.email))
            .collect();
        let subject = trf("Invitation: {} · {}", &[&entry.title, &when(&entry)]);
        let mut body = trf(
            "Hello,\n\nI would like to invite you to “{}”:\n\n{}",
            &[&entry.title, &when(&entry)],
        );
        if !entry.location.is_empty() {
            body.push_str(&format!("\n{}", entry.location));
        }
        body.push_str(&format!(
            "\n\n{}\n\n{}\n{}",
            tr("You can accept or decline directly in your calendar or with the attachment."),
            tr("Best regards"),
            greeting_name(u)
        ));
        compose(
            u,
            to,
            subject,
            body,
            file,
            tr("Invitation prepared in your mail app"),
        );
    });
}

/// After a change: send the invitation again, so the participants get the update.
pub fn send_update(ui: &UiRc, id: Id) {
    let Some(entry) = ui.state.borrow().entry(id).cloned() else {
        return;
    };
    if !entry.participants.is_empty() {
        invite(ui, id);
    }
}

/// Context menu → "Save as file …": one entry as a calendar file.
pub fn save_file(ui: &UiRc, id: Id) {
    let Some(entry) = ui.state.borrow().entry(id).cloned() else {
        return;
    };
    let text = {
        let st = ui.state.borrow();
        ics::write(
            &[&entry],
            &options(
                &st.settings.own_name,
                &st.settings.own_email,
                Method::Publish,
                true,
            ),
        )
    };
    let name: String = entry
        .title
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    let dialog = gtk::FileDialog::builder()
        .title(tr("Save entry as file"))
        .initial_name(format!("{}.ics", name.trim_matches('-').to_lowercase()))
        .build();
    let weak = Rc::downgrade(ui);
    dialog.save(Some(&ui.window), gio::Cancellable::NONE, move |result| {
        let Some(u) = weak.upgrade() else { return };
        if let Ok(file) = result
            && let Some(path) = file.path()
        {
            match std::fs::write(&path, &text) {
                Ok(()) => u.toast(&trf(
                    "Saved as {}",
                    &[&path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default()],
                )),
                Err(e) => u.error(&e.into()),
            }
        }
    });
}

/// Settings → "Export as file": all own entries.
pub fn export_all(ui: &UiRc) {
    let text = {
        let st = ui.state.borrow();
        let entries: Vec<&Entry> = st.entries.iter().collect();
        ics::write(
            &entries,
            &options(
                &st.settings.own_name,
                &st.settings.own_email,
                Method::None,
                true,
            ),
        )
    };
    let dialog = gtk::FileDialog::builder()
        .title(tr("Export calendar"))
        .initial_name("kalender.ics")
        .build();
    let weak = Rc::downgrade(ui);
    dialog.save(Some(&ui.window), gio::Cancellable::NONE, move |result| {
        let Some(u) = weak.upgrade() else { return };
        if let Ok(file) = result
            && let Some(path) = file.path()
        {
            match std::fs::write(&path, &text) {
                Ok(()) => u.toast(&tr("Calendar exported")),
                Err(e) => u.error(&e.into()),
            }
        }
    });
}

pub fn import_file(ui: &UiRc) {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some(&tr("Calendar files")));
    filter.add_mime_type("text/calendar");
    filter.add_suffix("ics");
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    let dialog = gtk::FileDialog::builder()
        .title(tr("Import calendar file"))
        .filters(&filters)
        .build();
    let weak = Rc::downgrade(ui);
    dialog.open(Some(&ui.window), gio::Cancellable::NONE, move |result| {
        if let (Some(u), Ok(file)) = (weak.upgrade(), result) {
            open_file(&u, &file);
        }
    });
}

/// A calendar file opened from the mail app, the file manager or "Import".
pub fn open_file(ui: &UiRc, file: &gio::File) {
    let text = match file.load_contents(gio::Cancellable::NONE) {
        Ok((bytes, _)) => String::from_utf8_lossy(&bytes).to_string(),
        Err(e) => return ui.error(&e.into()),
    };
    let calendar = match ics::parse(&text) {
        Ok(c) => c,
        Err(e) => return ui.error(&e),
    };
    match calendar.method {
        Method::Reply => {
            for ev in calendar.events {
                apply_reply(ui, ev);
            }
        }
        Method::Cancel => {
            for ev in calendar.events {
                apply_cancel(ui, ev);
            }
        }
        Method::Request | Method::Publish if calendar.events.len() == 1 => offer_event(
            ui,
            calendar.events.into_iter().next().expect("one event"),
            calendar.method,
        ),
        _ => import_events(ui, calendar.events),
    }
}

fn kind_for(ui: &UiRc, b: Builtin) -> Id {
    let st = ui.state.borrow();
    st.builtin(b)
        .or_else(|| st.builtin(Builtin::Event))
        .map(|k| k.id)
        .unwrap_or(1)
}

fn entry_from(ui: &UiRc, ev: &ics::Event) -> Entry {
    let kind = kind_for(
        ui,
        if ev.attendees.is_empty() {
            Builtin::Event
        } else {
            Builtin::Meeting
        },
    );
    let mut e = Entry::new(kind, &ev.summary, ev.start, ev.end);
    if !ev.uid.is_empty() {
        e.uid = ev.uid.clone();
    }
    e.all_day = ev.all_day;
    e.location = ev.location.clone();
    e.notes = ev.description.clone();
    e.recurrence = ev.recurrence.clone();
    e.sequence = ev.sequence;
    e.organizer = ev.organizer.clone();
    let own = ui.state.borrow().settings.own_email.to_lowercase();
    e.participants = ev
        .attendees
        .iter()
        .filter(|p| p.email.to_lowercase() != own)
        .cloned()
        .collect();
    e
}

fn import_events(ui: &UiRc, events: Vec<ics::Event>) {
    let mut added = 0;
    for ev in &events {
        let exists = ui
            .state
            .borrow()
            .entries
            .iter()
            .any(|e| !ev.uid.is_empty() && e.uid == ev.uid);
        if exists {
            continue;
        }
        let entry = entry_from(ui, ev);
        if ui.state.borrow_mut().save_entry(entry).is_ok() {
            added += 1;
        }
    }
    ui.refresh();
    ui.toast(&crate::i18n::trn(
        "{} entry imported",
        "{} entries imported",
        added,
    ));
}

fn offer_event(ui: &UiRc, ev: ics::Event, method: Method) {
    let existing = ui
        .state
        .borrow()
        .entries
        .iter()
        .find(|e| e.uid == ev.uid)
        .cloned();
    let from = ev
        .organizer_name
        .clone()
        .or_else(|| ev.organizer.clone())
        .unwrap_or_default();
    let heading = match (method, existing.is_some()) {
        (Method::Request, false) => trf("Invitation from {}", &[&from]),
        (Method::Request, true) => tr("Updated invitation"),
        (_, true) => tr("Updated entry"),
        _ => tr("Add entry?"),
    };
    let time = if ev.all_day {
        fmt::date_medium(ev.start.date())
    } else {
        format!(
            "{}, {} – {}",
            fmt::date_medium(ev.start.date()),
            fmt::time_of(ev.start),
            fmt::time_of(ev.end)
        )
    };
    let mut body = format!("{}\n{time}", trf("“{}”", &[&ev.summary]));
    if !ev.location.is_empty() {
        body.push_str(&format!("\n{}", ev.location));
    }
    let dialog = adw::AlertDialog::new(Some(&heading), Some(&body));
    if method == Method::Request {
        dialog.add_responses(&[
            ("later", &tr("Later")),
            ("decline", &tr("Decline")),
            ("accept", &tr("Accept")),
        ]);
        dialog.set_response_appearance("decline", adw::ResponseAppearance::Destructive);
        dialog.set_response_appearance("accept", adw::ResponseAppearance::Suggested);
        dialog.set_close_response("later");
    } else {
        dialog.add_responses(&[("cancel", &tr("Cancel")), ("add", &tr("Add to calendar"))]);
        dialog.set_response_appearance("add", adw::ResponseAppearance::Suggested);
        dialog.set_close_response("cancel");
    }
    let weak = Rc::downgrade(ui);
    dialog.connect_response(None, move |_, response| {
        let Some(u) = weak.upgrade() else { return };
        let mut entry = entry_from(&u, &ev);
        if let Some(old) = &existing {
            entry.id = old.id;
            entry.kind = old.kind;
            entry.reminder = old.reminder;
            entry.subtasks = old.subtasks.clone();
            entry.done = old.done.clone();
        }
        match response {
            "add" => {
                entry.organizer = None;
                save_and_show(&u, entry, &tr("Added to the calendar"));
            }
            "accept" | "decline" => {
                let status = if response == "accept" {
                    Partstat::Accepted
                } else {
                    Partstat::Declined
                };
                let date = entry.start.date();
                if status == Partstat::Accepted {
                    save_and_show(&u, entry.clone(), &tr("Accepted and added"));
                } else if let Some(old) = &existing {
                    let _ = u.state.borrow_mut().delete_entry(old.id);
                    u.refresh();
                }
                reply(&u, &entry, status, &ev);
                let _ = date;
            }
            _ => {}
        }
    });
    dialog.present(Some(&ui.window));
}

fn save_and_show(ui: &UiRc, entry: Entry, text: &str) {
    let date = entry.start.date();
    let result = ui.state.borrow_mut().save_entry(entry);
    match result {
        Ok(_) => {
            ui.reveal(date);
            ui.toast(text);
        }
        Err(e) => ui.error(&e),
    }
}

/// Our answer to someone else's invitation, prepared in the mail app.
fn reply(ui: &UiRc, entry: &Entry, status: Partstat, ev: &ics::Event) {
    let Some(organizer) = ev.organizer.clone() else {
        return;
    };
    let entry = entry.clone();
    with_identity(ui, move |u| {
        let mut answer = entry.clone();
        let (own_name, own_email) = {
            let st = u.state.borrow();
            (st.settings.own_name.clone(), st.settings.own_email.clone())
        };
        answer.participants = vec![kalender_core::model::Participant {
            name: own_name.clone(),
            email: own_email.clone(),
            status,
        }];
        answer.organizer = Some(organizer.clone());
        let text = ics::write(
            &[&answer],
            &options(&own_name, &own_email, Method::Reply, false),
        );
        let file = match cache_file("antwort.ics", &text) {
            Ok(f) => f,
            Err(e) => return u.error(&e),
        };
        let (subject, body) = if status == Partstat::Accepted {
            (
                trf("Accepted: {}", &[&entry.title]),
                trf("I accept: “{}” – {}", &[&entry.title, &when(&entry)]),
            )
        } else {
            (
                trf("Declined: {}", &[&entry.title]),
                trf(
                    "Unfortunately I can't make it: “{}” – {}",
                    &[&entry.title, &when(&entry)],
                ),
            )
        };
        compose(
            u,
            vec![organizer.clone()],
            subject,
            body,
            file,
            tr("Answer prepared in your mail app"),
        );
    });
}

fn apply_reply(ui: &UiRc, ev: ics::Event) {
    let entry = ui
        .state
        .borrow()
        .entries
        .iter()
        .find(|e| e.uid == ev.uid)
        .cloned();
    let Some(entry) = entry else {
        ui.toast(&tr(
            "This answer belongs to an entry that is not in the calendar.",
        ));
        return;
    };
    let Some(answer) = ev.attendees.first().cloned() else {
        return;
    };
    let (heading, icon) = match answer.status {
        Partstat::Accepted => (trf("{} has accepted", &[&answer.name]), "✓"),
        Partstat::Declined => (trf("{} has declined", &[&answer.name]), "✕"),
        Partstat::Tentative => (trf("{} might come", &[&answer.name]), "~"),
        Partstat::NeedsAction => (trf("{} has not decided yet", &[&answer.name]), "?"),
    };
    let body = trf(
        "{} Answer to your invitation to “{}”, {}",
        &[icon, &entry.title, &when(&entry)],
    );
    let dialog = adw::AlertDialog::new(Some(&heading), Some(&body));
    dialog.add_responses(&[("ignore", &tr("Ignore")), ("apply", &tr("Apply"))]);
    dialog.set_response_appearance("apply", adw::ResponseAppearance::Suggested);
    dialog.set_close_response("ignore");
    let weak = Rc::downgrade(ui);
    dialog.connect_response(Some("apply"), move |_, _| {
        let Some(u) = weak.upgrade() else { return };
        let mut entry = entry.clone();
        match entry
            .participants
            .iter_mut()
            .find(|p| p.email.eq_ignore_ascii_case(&answer.email))
        {
            Some(p) => p.status = answer.status,
            None => entry.participants.push(answer.clone()),
        }
        let date = entry.start.date();
        let result = u.state.borrow_mut().save_entry(entry.clone());
        match result {
            Ok(_) => {
                u.reveal(date);
                u.toast(&tr("Answer applied"));
            }
            Err(e) => u.error(&e),
        }
    });
    dialog.present(Some(&ui.window));
}

fn apply_cancel(ui: &UiRc, ev: ics::Event) {
    let entry = ui
        .state
        .borrow()
        .entries
        .iter()
        .find(|e| e.uid == ev.uid)
        .cloned();
    let Some(entry) = entry else { return };
    let dialog = adw::AlertDialog::new(
        Some(&tr("Entry cancelled")),
        Some(&trf(
            "“{}” was cancelled by the organizer.",
            &[&entry.title],
        )),
    );
    dialog.add_responses(&[
        ("keep", &tr("Keep")),
        ("remove", &tr("Remove from calendar")),
    ]);
    dialog.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
    dialog.set_close_response("keep");
    let weak = Rc::downgrade(ui);
    dialog.connect_response(Some("remove"), move |_, _| {
        if let Some(u) = weak.upgrade() {
            let _ = u.state.borrow_mut().delete_entry(entry.id);
            u.refresh();
            u.toast(&tr("Removed"));
        }
    });
    dialog.present(Some(&ui.window));
}
