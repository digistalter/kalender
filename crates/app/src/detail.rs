// SPDX-License-Identifier: GPL-3.0-or-later

//! The short view of an entry, shown as a popover next to it.

use std::rc::Rc;

use adw::prelude::*;
use chrono::NaiveDate;
use gtk::{gdk, glib};
use kalender_core::model::{Freq, Id, Partstat, RecurEnd};
use kalender_core::recur;

use crate::draw::IconView;
use crate::i18n::{tr, trf, trn};
use crate::state::{Item, kind_label};
use crate::week::WeekView;
use crate::window::UiRc;
use crate::{editor, fmt, ops, share};

pub fn show(ui: &UiRc, widget: &gtk::Widget, id: Id, date: NaiveDate, rect: gdk::Rectangle) {
    let Some(content) = build(ui, id, date) else {
        return;
    };
    let popover = gtk::Popover::builder()
        .child(&content)
        .position(gtk::PositionType::Right)
        .build();
    popover.set_parent(widget);
    popover.set_pointing_to(Some(&rect));
    popover.add_css_class("entry-details");
    // Del, Ctrl+D and E work while the details are open.
    let keys = gtk::EventControllerKey::new();
    let weak = Rc::downgrade(ui);
    let p = popover.clone();
    keys.connect_key_pressed(move |_, key, _, mods| {
        let Some(u) = weak.upgrade() else {
            return glib::Propagation::Proceed;
        };
        let ctrl = mods.contains(gdk::ModifierType::CONTROL_MASK);
        let action: Option<fn(&UiRc, Id, NaiveDate)> = match key {
            gdk::Key::Delete | gdk::Key::KP_Delete => Some(ops::delete),
            gdk::Key::d | gdk::Key::D if ctrl => Some(ops::duplicate),
            gdk::Key::e | gdk::Key::E if !ctrl => Some(editor::open_existing),
            _ => None,
        };
        match action {
            Some(f) => {
                p.popdown();
                f(&u, id, date);
                glib::Propagation::Stop
            }
            None => glib::Propagation::Proceed,
        }
    });
    popover.add_controller(keys);
    WeekView::set_selected(id);
    ui.redraw();
    let weak = Rc::downgrade(ui);
    popover.connect_closed(move |p| {
        WeekView::set_selected(0);
        if let Some(u) = weak.upgrade() {
            u.redraw();
        }
        let p = p.clone();
        glib::idle_add_local_once(move || p.unparent());
    });
    LAST.with(|l| *l.borrow_mut() = Some(popover.clone()));
    popover.popup();
}

thread_local! {
    /// The open popover, for screenshots of the app.
    pub static LAST: std::cell::RefCell<Option<gtk::Popover>> = const { std::cell::RefCell::new(None) };
}

/// For the context menu "Open": the popover points at the middle of the window.
pub fn show_centered(ui: &UiRc, id: Id, date: NaiveDate, _rect: gdk::Rectangle) {
    let Some(child) = ui.stack.visible_child() else {
        return;
    };
    let rect = gdk::Rectangle::new(child.width() / 2, child.height() / 3, 1, 1);
    show(ui, &child, id, date, rect);
}

fn close(widget: &impl IsA<gtk::Widget>) {
    if let Some(p) = widget.ancestor(gtk::Popover::static_type()) {
        p.downcast::<gtk::Popover>().expect("popover").popdown();
    }
}

fn row(icon: &str, text: &str) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    b.append(&IconView::new(icon, None, 16));
    let label = gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .wrap(true)
        .max_width_chars(36)
        .selectable(false)
        .build();
    b.append(&label);
    b.add_css_class("detail-row");
    b
}

pub fn repeat_text(rule: &kalender_core::model::Recurrence) -> String {
    let base = match (rule.freq, rule.interval) {
        (Freq::Daily, 1) => tr("Every day"),
        (Freq::Daily, n) => trf("Every {} days", &[&n.to_string()]),
        (Freq::Weekly, 1) => tr("Every week"),
        (Freq::Weekly, 2) => tr("Every 2 weeks"),
        (Freq::Weekly, n) => trf("Every {} weeks", &[&n.to_string()]),
        (Freq::Monthly, 1) => tr("Every month"),
        (Freq::Monthly, n) => trf("Every {} months", &[&n.to_string()]),
        (Freq::Yearly, _) => tr("Every year"),
    };
    let mut out = base;
    if rule.freq == Freq::Weekly && !rule.weekdays.is_empty() {
        let days: Vec<String> = rule
            .weekdays
            .iter()
            .map(|d| fmt::weekday_short(*d as u32))
            .collect();
        out = trf("{} on {}", &[&out, &days.join(", ")]);
    }
    match rule.end {
        RecurEnd::Never => out,
        RecurEnd::Until(d) => trf("{}, until {}", &[&out, &fmt::date_short(d)]),
        RecurEnd::Count(n) => trf("{}, {} times", &[&out, &n.to_string()]),
    }
}

fn build(ui: &UiRc, id: Id, date: NaiveDate) -> Option<gtk::Box> {
    let st = ui.state.borrow();
    let entry = st.entry(id)?.clone();
    let kind = st.kind(entry.kind)?.clone();
    let occ = recur::occurrence_on(&entry, date)?;
    let item = Item {
        occ,
        entry: entry.clone(),
        kind: kind.clone(),
    };
    drop(st);

    let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
    root.set_margin_top(10);
    root.set_margin_bottom(8);
    root.set_margin_start(12);
    root.set_margin_end(12);
    root.set_width_request(320);

    let head = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let icon = IconView::new(&kind.icon, Some(&kind.color), 18);
    icon.set_valign(gtk::Align::Start);
    icon.set_margin_top(3);
    head.append(&icon);
    let titles = gtk::Box::new(gtk::Orientation::Vertical, 2);
    titles.set_hexpand(true);
    let title = gtk::Label::builder()
        .label(crate::state::display_title(&entry, occ.date))
        .xalign(0.0)
        .wrap(true)
        .max_width_chars(28)
        .css_classes(["title-3"])
        .build();
    let when = if entry.all_day {
        match entry.half {
            Some(_) => trf("{} · half day", &[&fmt::date_medium(occ.date)]),
            None if entry.day_count() > 1 => format!(
                "{} – {}",
                fmt::date_short(occ.start.date()),
                fmt::date_short(occ.end.date() - chrono::Duration::days(1))
            ),
            None => trf("{} · all-day", &[&fmt::date_medium(occ.date)]),
        }
    } else {
        format!(
            "{} · {} – {}",
            fmt::date_medium(occ.date),
            fmt::time_of(occ.start),
            fmt::time_of(occ.end)
        )
    };
    let sub = gtk::Label::builder()
        .label(format!("{} · {when}", kind_label(&kind)))
        .xalign(0.0)
        .wrap(true)
        .max_width_chars(34)
        .css_classes(["dim-label", "caption"])
        .build();
    titles.append(&title);
    titles.append(&sub);
    head.append(&titles);
    let close_button = gtk::Button::builder()
        .icon_name("window-close-symbolic")
        .css_classes(["flat", "circular"])
        .valign(gtk::Align::Start)
        .tooltip_text(tr("Close (Esc)"))
        .build();
    close_button.connect_clicked(close);
    head.append(&close_button);
    root.append(&head);

    let rows = gtk::Box::new(gtk::Orientation::Vertical, 6);
    if let Some(rule) = &entry.recurrence {
        rows.append(&row("repeat", &repeat_text(rule)));
    }
    if entry.travel > 0 || entry.buffer > 0 {
        let mut parts = Vec::new();
        if entry.travel > 0 {
            parts.push(trf("{} travel time", &[&fmt::minutes(entry.travel)]));
        }
        if entry.buffer > 0 {
            parts.push(trf("{} buffer after", &[&fmt::minutes(entry.buffer)]));
        }
        rows.append(&row("car", &parts.join(" · ")));
    }
    if !entry.location.is_empty() {
        rows.append(&row("pin", &entry.location));
    }
    if let Some(r) = entry.reminder {
        let text = if r == 0 {
            tr("At the start")
        } else {
            trf("{} before", &[&fmt::minutes(r)])
        };
        rows.append(&row("bell", &text));
    }
    if !entry.notes.is_empty() {
        rows.append(&row("note", &entry.notes));
    }
    if !entry.participants.is_empty() {
        let yes = entry
            .participants
            .iter()
            .filter(|p| p.status == Partstat::Accepted)
            .count();
        let no = entry
            .participants
            .iter()
            .filter(|p| p.status == Partstat::Declined)
            .count();
        let mut text = trn(
            "{} participant",
            "{} participants",
            entry.participants.len(),
        );
        text.push_str(&format!(" · {}", trn("{} accepted", "{} accepted", yes)));
        if no > 0 {
            text.push_str(&format!(" · {}", trn("{} declined", "{} declined", no)));
        }
        rows.append(&row("people", &text));
        let list = gtk::Box::new(gtk::Orientation::Vertical, 2);
        list.set_margin_start(26);
        for p in &entry.participants {
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let (mark, class, label) = match p.status {
                Partstat::Accepted => ("✓", "success", tr("accepted")),
                Partstat::Declined => ("✕", "error", tr("declined")),
                Partstat::Tentative => ("~", "warning", tr("maybe")),
                Partstat::NeedsAction => ("?", "dim-label", tr("no answer yet")),
            };
            line.append(
                &gtk::Label::builder()
                    .label(mark)
                    .css_classes([class, "heading"])
                    .width_chars(2)
                    .build(),
            );
            line.append(
                &gtk::Label::builder()
                    .label(&p.name)
                    .xalign(0.0)
                    .hexpand(true)
                    .tooltip_text(&p.email)
                    .ellipsize(gtk::pango::EllipsizeMode::End)
                    .build(),
            );
            line.append(
                &gtk::Label::builder()
                    .label(label)
                    .css_classes(["dim-label", "caption"])
                    .build(),
            );
            list.append(&line);
        }
        rows.append(&list);
    }
    if kind.source.is_some() {
        rows.append(&row(
            "globe",
            &trf("From “{}” · can only be read", &[&kind_label(&kind)]),
        ));
    }
    if rows.first_child().is_some() {
        root.append(&rows);
    }

    if entry.has_tasks() || kind.builtin == Some(kalender_core::model::Builtin::Task) {
        root.append(&tasks_box(ui, &item));
    }

    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    actions.set_margin_top(6);
    let delete = gtk::Button::builder()
        .icon_name("user-trash-symbolic")
        .tooltip_text(tr("Delete (Del)"))
        .css_classes(["flat"])
        .build();
    let duplicate = gtk::Button::builder()
        .icon_name("edit-copy-symbolic")
        .tooltip_text(tr("Duplicate (Ctrl+D)"))
        .css_classes(["flat"])
        .build();
    let read_only = kind.source.is_some();
    if !read_only {
        actions.append(&delete);
        actions.append(&duplicate);
    }
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    actions.append(&spacer);
    let own = entry.organizer.is_none() && !read_only;
    // Only entries with invited people can be sent again; adding people is done in "Edit".
    if own && !entry.participants.is_empty() {
        let send = gtk::Button::builder()
            .label(tr("Invite"))
            .tooltip_text(tr("Send the invitation again"))
            .build();
        let weak = Rc::downgrade(ui);
        send.connect_clicked(move |b| {
            close(b);
            if let Some(u) = weak.upgrade() {
                share::invite(&u, id)
            }
        });
        actions.append(&send);
    }
    let edit = gtk::Button::builder()
        .label(tr("Edit"))
        .css_classes(["suggested-action"])
        .build();
    if read_only {
        // Subscribed: can only be looked at, or taken over as an own appointment.
        let copy = gtk::Button::builder()
            .label(tr("Copy to my calendar"))
            .build();
        let weak = Rc::downgrade(ui);
        copy.connect_clicked(move |b| {
            close(b);
            if let Some(u) = weak.upgrade() {
                ops::duplicate(&u, id, date);
            }
        });
        actions.append(&copy);
    } else {
        actions.append(&edit);
    }
    root.append(&actions);

    let weak = Rc::downgrade(ui);
    delete.connect_clicked(move |b| {
        close(b);
        if let Some(u) = weak.upgrade() {
            ops::delete(&u, id, date);
        }
    });
    let weak = Rc::downgrade(ui);
    duplicate.connect_clicked(move |b| {
        close(b);
        if let Some(u) = weak.upgrade() {
            ops::duplicate(&u, id, date);
        }
    });
    let weak = Rc::downgrade(ui);
    edit.connect_clicked(move |b| {
        close(b);
        if let Some(u) = weak.upgrade() {
            editor::open_existing(&u, id, date);
        }
    });
    Some(root)
}

/// Progress and checkable subtasks; changes are saved at once.
fn tasks_box(ui: &UiRc, item: &Item) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let (done, total) = item.entry.progress(item.occ.date);
    let progress_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let label = gtk::Label::builder()
        .label(trf(
            "{} of {} done",
            &[&done.to_string(), &total.to_string()],
        ))
        .css_classes(["caption", "dim-label"])
        .build();
    let bar = gtk::LevelBar::builder()
        .min_value(0.0)
        .max_value(total.max(1) as f64)
        .value(done as f64)
        .hexpand(true)
        .valign(gtk::Align::Center)
        .css_classes(["task-progress"])
        .build();
    bar.remove_offset_value(Some(gtk::LEVEL_BAR_OFFSET_LOW));
    bar.remove_offset_value(Some(gtk::LEVEL_BAR_OFFSET_HIGH));
    bar.remove_offset_value(Some(gtk::LEVEL_BAR_OFFSET_FULL));
    progress_row.append(&label);
    progress_row.append(&bar);
    if total > 0 && done == total {
        progress_row.append(
            &gtk::Label::builder()
                .label(tr("Done"))
                .css_classes(["success", "caption-heading"])
                .build(),
        );
    }
    b.append(&progress_row);

    let id = item.entry.id;
    let date = item.occ.date;
    let flags = item.entry.done_on(date);
    for (index, text) in item.entry.subtask_texts().iter().enumerate() {
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let check = gtk::CheckButton::builder()
            .label(text)
            .active(flags[index])
            .hexpand(true)
            .build();
        if flags[index] {
            check.add_css_class("done-task");
        }
        let weak = Rc::downgrade(ui);
        let (bar, label) = (bar.clone(), label.clone());
        check.connect_toggled(move |c| {
            let Some(u) = weak.upgrade() else { return };
            let entry = u.state.borrow().entry(id).cloned();
            if let Some(mut entry) = entry {
                entry.set_done(date, index, c.is_active());
                let (d, t) = entry.progress(date);
                bar.set_value(d as f64);
                label.set_label(&trf("{} of {} done", &[&d.to_string(), &t.to_string()]));
                if c.is_active() {
                    c.add_css_class("done-task")
                } else {
                    c.remove_css_class("done-task")
                }
                let result = u.state.borrow_mut().save_entry(entry);
                if let Err(e) = result {
                    u.error(&e);
                }
                u.redraw();
            }
        });
        let remove = gtk::Button::builder()
            .icon_name("window-close-symbolic")
            .css_classes(["flat", "circular", "dim-label"])
            .tooltip_text(tr("Remove subtask"))
            .build();
        let weak = Rc::downgrade(ui);
        let line_ref = line.clone();
        remove.connect_clicked(move |_| {
            let Some(u) = weak.upgrade() else { return };
            let entry = u.state.borrow().entry(id).cloned();
            if let Some(mut entry) = entry {
                entry.remove_subtask(index);
                let result = u.state.borrow_mut().save_entry(entry);
                if let Err(e) = result {
                    u.error(&e);
                }
                line_ref.set_visible(false);
                u.redraw();
            }
        });
        line.append(&check);
        line.append(&remove);
        b.append(&line);
    }
    let add = gtk::Entry::builder()
        .placeholder_text(tr("Add subtask"))
        .primary_icon_name("list-add-symbolic")
        .build();
    let weak = Rc::downgrade(ui);
    let list = b.clone();
    add.connect_activate(move |e| {
        let text = e.text().trim().to_owned();
        if text.is_empty() {
            return;
        }
        let Some(u) = weak.upgrade() else { return };
        let entry = u.state.borrow().entry(id).cloned();
        if let Some(mut entry) = entry {
            entry.add_subtask(&text);
            let result = u.state.borrow_mut().save_entry(entry);
            if let Err(err) = result {
                u.error(&err);
                return;
            }
            let check = gtk::CheckButton::builder().label(&text).build();
            let index = u
                .state
                .borrow()
                .entry(id)
                .map(|e| e.subtasks.len() - 1)
                .unwrap_or(0);
            let weak2 = Rc::downgrade(&u);
            check.connect_toggled(move |c| {
                let Some(u) = weak2.upgrade() else { return };
                let entry = u.state.borrow().entry(id).cloned();
                if let Some(mut entry) = entry {
                    entry.set_done(date, index, c.is_active());
                    let _ = u.state.borrow_mut().save_entry(entry);
                    u.redraw();
                }
            });
            list.insert_child_after(
                &check,
                list.last_child().and_then(|l| l.prev_sibling()).as_ref(),
            );
            e.set_text("");
            u.redraw();
        }
    });
    b.append(&add);
    b
}
