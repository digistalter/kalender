// SPDX-License-Identifier: GPL-3.0-or-later

//! Reminders as system notifications.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use chrono::{Duration, NaiveDateTime};
use gtk::{gio, glib};

use crate::fmt;
use crate::i18n::trf;
use crate::state::{Shared, now};

/// Checks twice a minute which reminders are due since the last check.
pub fn start(app: &adw::Application, state: Shared) {
    let last = Rc::new(Cell::new(now() - Duration::minutes(1)));
    let app = app.downgrade();
    glib::timeout_add_seconds_local(30, move || {
        let Some(app) = app.upgrade() else {
            return glib::ControlFlow::Break;
        };
        let current = now();
        check(&app, &state, last.get(), current);
        last.set(current);
        glib::ControlFlow::Continue
    });
}

fn check(app: &adw::Application, state: &Shared, from: NaiveDateTime, to: NaiveDateTime) {
    let st = state.borrow();
    for item in st.items(from - Duration::days(1), to + Duration::days(2)) {
        let Some(minutes) = item.entry.reminder else {
            continue;
        };
        if item.is_virtual() {
            continue;
        }
        let leave = item.occ.start - Duration::minutes(item.entry.travel);
        // All-day entries remind at 9:00 of the day (or the chosen time before that).
        let due = if item.entry.all_day {
            item.occ.start + Duration::hours(9) - Duration::minutes(minutes)
        } else {
            leave - Duration::minutes(minutes)
        };
        if due <= from || due > to {
            continue;
        }
        send(app, &item);
    }
}

/// Shows the reminder for one occurrence as a system notification.
pub fn send(app: &adw::Application, item: &crate::state::Item) {
    let leave = item.occ.start - Duration::minutes(item.entry.travel);
    let mut body = if item.entry.all_day {
        fmt::date_medium(item.occ.date)
    } else if item.entry.travel > 0 {
        trf(
            "Time to leave: {} · starts {}",
            &[&fmt::time_of(leave), &fmt::time_of(item.occ.start)],
        )
    } else {
        format!(
            "{} – {}",
            fmt::time_of(item.occ.start),
            fmt::time_of(item.occ.end)
        )
    };
    if !item.entry.location.is_empty() {
        body.push_str(&format!(" · {}", item.entry.location));
    }
    let n = gio::Notification::new(&item.title());
    n.set_body(Some(&body));
    n.set_default_action("app.activate");
    n.set_priority(gio::NotificationPriority::High);
    app.send_notification(
        Some(&format!("entry-{}-{}", item.entry.id, item.occ.date)),
        &n,
    );
}
