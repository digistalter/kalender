// SPDX-License-Identifier: GPL-3.0-or-later

//! Changing entries: move, copy, duplicate, delete – with series questions and undo.

use std::rc::Rc;

use adw::prelude::*;
use chrono::{Duration, NaiveDate, NaiveDateTime};
use gtk::{gdk, gio, glib};
use kalender_core::model::{Entry, Id, Partstat, RecurEnd, Routine, new_uid};

use crate::i18n::{tr, trf};
use crate::window::UiRc;
use crate::{detail, editor, fmt};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    This,
    Following,
}

/// Asks whether a change of a series applies to one or all following occurrences.
pub fn ask_scope(ui: &UiRc, heading: &str, body: &str, f: impl Fn(&UiRc, Scope) + 'static) {
    let dialog = adw::AlertDialog::new(Some(heading), Some(body));
    dialog.add_responses(&[
        ("cancel", &tr("Cancel")),
        ("following", &tr("This and all following")),
        ("this", &tr("Only this one")),
    ]);
    dialog.set_response_appearance("this", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("this"));
    dialog.set_close_response("cancel");
    let weak = Rc::downgrade(ui);
    dialog.connect_response(None, move |_, response| {
        let Some(u) = weak.upgrade() else { return };
        match response {
            "this" => f(&u, Scope::This),
            "following" => f(&u, Scope::Following),
            _ => {}
        }
    });
    dialog.present(Some(&ui.window));
}

fn save(ui: &UiRc, entry: Entry) -> Option<Id> {
    let result = ui.state.borrow_mut().save_entry(entry);
    match result {
        Ok(id) => Some(id),
        Err(e) => {
            ui.error(&e);
            None
        }
    }
}

/// Restores entries to earlier versions and removes newly made ones.
fn restorer(before: Vec<Entry>, created: Vec<Id>) -> impl Fn(&UiRc) + 'static {
    move |u: &UiRc| {
        let mut st = u.state.borrow_mut();
        for id in &created {
            let _ = st.delete_entry(*id);
        }
        for e in &before {
            let _ = st.save_entry(e.clone());
        }
    }
}

/// A copy of one occurrence as a stand-alone entry.
pub fn detached(entry: &Entry, date: NaiveDate, start: NaiveDateTime) -> Entry {
    let mut copy = entry.clone();
    copy.id = 0;
    copy.uid = new_uid();
    copy.recurrence = None;
    copy.exceptions.clear();
    copy.sequence = 0;
    copy.start = start;
    copy.end = start + entry.duration();
    copy.done.clear();
    let flags = entry.done_on(date);
    copy.done.insert(start.date(), flags);
    copy
}

fn moved_text(start: NaiveDateTime, all_day: bool) -> String {
    if all_day {
        trf("Moved to {}", &[&fmt::date_short(start.date())])
    } else {
        trf(
            "Moved to {}, {}",
            &[&fmt::date_short(start.date()), &fmt::time_of(start)],
        )
    }
}

pub fn move_or_copy(ui: &UiRc, id: Id, date: NaiveDate, start: NaiveDateTime, copy: bool) {
    let Some(entry) = ui.state.borrow().entry(id).cloned() else {
        return;
    };
    let subscribed = ui.state.borrow().is_read_only(id);
    if !copy && refuse_read_only(ui, id) {
        return;
    }
    if copy {
        let mut new = detached(&entry, date, start);
        if subscribed {
            // A copy from a subscribed calendar becomes an own appointment.
            if let Some(k) = ui
                .state
                .borrow()
                .builtin(kalender_core::model::Builtin::Event)
            {
                new.kind = k.id;
            }
        }
        new.done.clear();
        for p in &mut new.participants {
            p.status = Partstat::NeedsAction;
        }
        if let Some(new_id) = save(ui, new) {
            ui.refresh();
            let text = trf("Copy on {}", &[&fmt::date_short(start.date())]);
            ui.toast_undo(&text, restorer(vec![], vec![new_id]));
        }
        return;
    }
    if is_birthday(ui, &entry) {
        // A birthday is one endless series: moving it moves the birthday itself.
        let occ_start = kalender_core::recur::occurrence_on(&entry, date)
            .map(|o| o.start)
            .unwrap_or(entry.start);
        let shift = start - occ_start;
        let mut e = entry.clone();
        e.start += shift;
        e.end += shift;
        e.sequence += 1;
        save(ui, e);
        ui.refresh();
        ui.toast_undo(&moved_text(start, true), restorer(vec![entry], vec![]));
        return;
    }
    if entry.recurrence.is_some() {
        let heading = tr("Move recurring entry?");
        let body = trf("“{}” is part of a series.", &[&entry.title]);
        ui.window.present();
        ask_scope(ui, &heading, &body, move |u, scope| {
            apply_move(u, &entry, date, start, scope)
        });
    } else {
        apply_move(ui, &entry, date, start, Scope::This);
    }
}

fn apply_move(ui: &UiRc, entry: &Entry, date: NaiveDate, start: NaiveDateTime, scope: Scope) {
    let before = vec![entry.clone()];
    let mut created = Vec::new();
    if entry.recurrence.is_none() {
        let mut e = entry.clone();
        let shift = start - e.start;
        e.start = start;
        e.end += shift;
        e.sequence += 1;
        if let Some(flags) = e.done.remove(&date) {
            e.done.insert(start.date(), flags);
        }
        save(ui, e);
    } else {
        let mut series = entry.clone();
        let mut part = detached(entry, date, start);
        match scope {
            Scope::This => series.exceptions.push(date),
            Scope::Following => {
                series.recurrence.as_mut().expect("series").end =
                    RecurEnd::Until(date - Duration::days(1));
                part.recurrence = entry.recurrence.clone();
                part.done.clear();
            }
        }
        let first_of_series = date == entry.start.date() && scope == Scope::Following;
        if first_of_series {
            let _ = ui.state.borrow_mut().delete_entry(series.id);
        } else {
            save(ui, series);
        }
        if let Some(id) = save(ui, part) {
            created.push(id);
        }
    }
    ui.refresh();
    let all_day = entry.all_day;
    let text = moved_text(start, all_day);
    let undo = restorer(before, created);
    if !entry.participants.is_empty() {
        let id = entry.id;
        ui.toast_action(
            &text,
            &tr("Inform"),
            move |u| crate::share::send_update(u, id),
            undo,
        );
    } else {
        ui.toast_undo(&text, undo);
    }
}

pub fn duplicate(ui: &UiRc, id: Id, date: NaiveDate) {
    let Some(entry) = ui.state.borrow().entry(id).cloned() else {
        return;
    };
    let Some(occ) = kalender_core::recur::occurrence_on(&entry, date) else {
        return;
    };
    // From a subscribed calendar: copy to the same time into the own calendar.
    let shift = if ui.state.borrow().is_read_only(id) {
        0
    } else {
        1
    };
    move_or_copy(ui, id, date, occ.start + Duration::days(shift), true);
}

/// Shows a hint and returns true for entries of subscribed calendars.
pub fn refuse_read_only(ui: &UiRc, id: Id) -> bool {
    let read_only = ui.state.borrow().is_read_only(id);
    if read_only {
        ui.toast(&tr("Subscribed calendars can only be read"));
    }
    read_only
}

pub fn move_by_days(ui: &UiRc, id: Id, date: NaiveDate, days: i64) {
    let Some(entry) = ui.state.borrow().entry(id).cloned() else {
        return;
    };
    let start = kalender_core::recur::occurrence_on(&entry, date)
        .map(|o| o.start)
        .unwrap_or(entry.start);
    move_or_copy(ui, id, date, start + Duration::days(days), false);
}

pub fn delete(ui: &UiRc, id: Id, date: NaiveDate) {
    if refuse_read_only(ui, id) {
        return;
    }
    let Some(entry) = ui.state.borrow().entry(id).cloned() else {
        return;
    };
    if is_birthday(ui, &entry) {
        // Birthdays never end on their own; deleting removes the whole birthday.
        let first = entry.start.date();
        apply_delete(ui, &entry, first, Scope::Following);
    } else if entry.recurrence.is_some() {
        let body = trf("“{}” is part of a series.", &[&entry.title]);
        ask_scope(
            ui,
            &tr("Delete recurring entry?"),
            &body,
            move |u, scope| apply_delete(u, &entry, date, scope),
        );
    } else {
        apply_delete(ui, &entry, date, Scope::Following);
    }
}

fn apply_delete(ui: &UiRc, entry: &Entry, date: NaiveDate, scope: Scope) {
    let whole =
        entry.recurrence.is_none() || (scope == Scope::Following && date <= entry.start.date());
    if whole {
        let result = ui.state.borrow_mut().delete_entry(entry.id);
        if let Err(e) = result {
            ui.error(&e);
            return;
        }
    } else {
        let mut series = entry.clone();
        match scope {
            Scope::This => series.exceptions.push(date),
            Scope::Following => {
                series.recurrence.as_mut().expect("series").end =
                    RecurEnd::Until(date - Duration::days(1))
            }
        }
        save(ui, series);
    }
    ui.refresh();
    let text = trf("“{}” deleted", &[&entry.title]);
    ui.toast_undo(&text, restorer(vec![entry.clone()], vec![]));
}

pub fn save_as_routine(ui: &UiRc, id: Id) {
    let Some(entry) = ui.state.borrow().entry(id).cloned() else {
        return;
    };
    let existing = ui
        .state
        .borrow()
        .routines
        .iter()
        .find(|r| r.name == entry.title)
        .cloned();
    let routine = Routine {
        id: existing.map(|r| r.id).unwrap_or(0),
        name: entry.title.clone(),
        minutes: entry.duration().num_minutes(),
        subtasks: entry.subtasks.clone(),
    };
    let result = ui.state.borrow_mut().save_routine(routine);
    match result {
        Ok(_) => ui.toast(&trf("Saved as routine “{}”", &[&entry.title])),
        Err(e) => ui.error(&e),
    }
}

/// Lets the user pick a day to move an occurrence to.
fn pick_date(ui: &UiRc, id: Id, date: NaiveDate) {
    let calendar = gtk::Calendar::new();
    if let Ok(d) = glib::DateTime::from_local(
        chrono::Datelike::year(&date),
        chrono::Datelike::month(&date) as i32,
        chrono::Datelike::day(&date) as i32,
        0,
        0,
        0.0,
    ) {
        calendar.select_day(&d);
    }
    let dialog = adw::AlertDialog::new(Some(&tr("Move to …")), None);
    dialog.set_extra_child(Some(&calendar));
    dialog.add_responses(&[("cancel", &tr("Cancel")), ("move", &tr("Move"))]);
    dialog.set_response_appearance("move", adw::ResponseAppearance::Suggested);
    dialog.set_close_response("cancel");
    let weak = Rc::downgrade(ui);
    dialog.connect_response(Some("move"), move |_, _| {
        let Some(u) = weak.upgrade() else { return };
        let d = calendar.date();
        if let Some(day) =
            NaiveDate::from_ymd_opt(d.year(), d.month() as u32, d.day_of_month() as u32)
        {
            move_by_days(&u, id, date, (day - date).num_days());
        }
    });
    dialog.present(Some(&ui.window));
}

pub fn add_context_actions(ui: &UiRc) {
    let group = gio::SimpleActionGroup::new();
    type Handler = Box<dyn Fn(&UiRc, Id, NaiveDate)>;
    let add = |name: &str, f: Handler| {
        let a = gio::SimpleAction::new(name, None);
        let weak = Rc::downgrade(ui);
        a.connect_activate(move |_, _| {
            let Some(u) = weak.upgrade() else { return };
            if let Some((id, date)) = u.ctx.get() {
                f(&u, id, date);
            }
        });
        group.add_action(&a);
    };
    add(
        "open",
        Box::new(|u, id, date| {
            let rect = gdk::Rectangle::new(0, 0, 1, 1);
            detail::show_centered(u, id, date, rect);
        }),
    );
    add("edit", Box::new(editor::open_existing));
    add("duplicate", Box::new(duplicate));
    add(
        "later",
        Box::new(|u, id, date| move_by_days(u, id, date, 1)),
    );
    add(
        "earlier",
        Box::new(|u, id, date| move_by_days(u, id, date, -1)),
    );
    add(
        "next-week",
        Box::new(|u, id, date| move_by_days(u, id, date, 7)),
    );
    add("pick", Box::new(pick_date));
    add("routine", Box::new(|u, id, _| save_as_routine(u, id)));
    add("file", Box::new(|u, id, _| crate::share::save_file(u, id)));
    add("delete", Box::new(delete));
    ui.window.insert_action_group("ctx", Some(&group));
}

pub fn context_menu(ui: &UiRc, widget: &gtk::Widget, id: Id, date: NaiveDate, x: f64, y: f64) {
    let Some(entry) = ui.state.borrow().entry(id).cloned() else {
        return;
    };
    ui.ctx.set(Some((id, date)));
    let menu = gio::Menu::new();
    if ui.state.borrow().is_read_only(id) {
        let only = gio::Menu::new();
        only.append(Some(&tr("Open")), Some("ctx.open"));
        only.append(Some(&tr("Copy to my calendar")), Some("ctx.duplicate"));
        only.append(Some(&tr("Save as file …")), Some("ctx.file"));
        menu.append_section(None, &only);
        show_menu(widget, &menu, x, y);
        return;
    }
    let first = gio::Menu::new();
    first.append(Some(&tr("Open")), Some("ctx.open"));
    first.append(Some(&tr("Edit")), Some("ctx.edit"));
    menu.append_section(None, &first);
    let second = gio::Menu::new();
    second.append(Some(&tr("Duplicate")), Some("ctx.duplicate"));
    let move_to = gio::Menu::new();
    move_to.append(Some(&tr("One day later")), Some("ctx.later"));
    move_to.append(Some(&tr("One day earlier")), Some("ctx.earlier"));
    move_to.append(Some(&tr("One week later")), Some("ctx.next-week"));
    move_to.append(Some(&tr("Pick a date …")), Some("ctx.pick"));
    second.append_submenu(Some(&tr("Move to …")), &move_to);
    if entry.has_tasks() {
        second.append(Some(&tr("Save as routine")), Some("ctx.routine"));
    }
    second.append(Some(&tr("Save as file …")), Some("ctx.file"));
    menu.append_section(None, &second);
    let third = gio::Menu::new();
    third.append(Some(&tr("Delete")), Some("ctx.delete"));
    menu.append_section(None, &third);
    show_menu(widget, &menu, x, y);
}

fn show_menu(widget: &gtk::Widget, menu: &gio::Menu, x: f64, y: f64) {
    let popover = gtk::PopoverMenu::from_model(Some(menu));
    popover.set_parent(widget);
    popover.set_has_arrow(false);
    popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
    popover.connect_closed(|p| {
        let p = p.clone();
        glib::idle_add_local_once(move || p.unparent());
    });
    crate::detail::LAST.with(|l| *l.borrow_mut() = Some(popover.clone().upcast()));
    popover.popup();
}

fn is_birthday(ui: &UiRc, entry: &Entry) -> bool {
    ui.state
        .borrow()
        .kind(entry.kind)
        .is_some_and(|k| k.builtin == Some(kalender_core::model::Builtin::Birthday))
}
