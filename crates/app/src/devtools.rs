// SPDX-License-Identifier: GPL-3.0-or-later

//! Helpers for trying the app: example entries (KALENDER_DEMO=1) and a
//! picture of the window (KALENDER_SHOT=file.png), used for screenshots.

use adw::prelude::*;
use chrono::{Datelike, Duration, NaiveDate};
use gtk::{gdk, glib};
use kalender_core::model::{
    Builtin, Entry, Freq, HalfDay, Participant, Partstat, RecurEnd, Recurrence, Routine, at,
    midnight, week_start,
};

use crate::state::{State, View, today};
use crate::window::UiRc;

/// Fills an empty calendar with the example week of the wireframe.
pub fn demo(state: &mut State) {
    if std::env::var_os("KALENDER_DEMO").is_none() || !state.entries.is_empty() {
        return;
    }
    let kind = |b| state.builtin(b).map(|k| k.id).unwrap_or(1);
    let (event, meeting, task, vacation, birthday) = (
        kind(Builtin::Event),
        kind(Builtin::Meeting),
        kind(Builtin::Task),
        kind(Builtin::Vacation),
        kind(Builtin::Birthday),
    );
    let mon = week_start(today());
    let day = |n: i64| mon + Duration::days(n);
    let mut entries = Vec::new();
    let mut add = |kind, title: &str, d: NaiveDate, a: (u32, u32), b: (u32, u32)| {
        let e = Entry::new(kind, title, at(d, a.0, a.1), at(d, b.0, b.1));
        entries.push(e);
        entries.len() - 1
    };
    let team = add(meeting, "Team-Runde", day(1), (9, 0), (9, 30));
    add(event, "Mittagessen mit Lena", day(1), (12, 30), (13, 30));
    let yoga = add(event, "Yoga", day(1), (18, 0), (19, 30));
    let putzen = add(task, "Putzen", day(1), (15, 0), (17, 0));
    let kunde = add(meeting, "Kundentermin Nordlicht", day(2), (10, 0), (11, 30));
    add(event, "Zahnarzt", day(3), (15, 0), (16, 0));
    let workshop = add(meeting, "Workshop Planung 2027", day(4), (8, 0), (17, 0));
    add(event, "Telefonat Steuerberater", day(4), (12, 0), (13, 0));
    let wohnung = add(event, "Wohnungsübergabe", day(5), (11, 0), (13, 0));
    let steuer = add(task, "Steuer vorbereiten", day(5), (14, 0), (16, 0));
    add(event, "Kaffee bei Oma", day(6), (15, 0), (18, 0));
    let einkauf = add(task, "Einkaufen", day(0), (7, 0), (8, 0));
    add(meeting, "Wochenplanung", day(0), (9, 0), (9, 30));
    add(event, "Chorprobe", day(0), (19, 0), (21, 0));
    let urlaub = add(vacation, "Urlaub an der Ostsee", day(9), (0, 0), (0, 0));
    let halb = add(vacation, "Urlaub", day(3), (0, 0), (0, 0));
    let paul = add(birthday, "Paul", day(3), (0, 0), (0, 0));

    let e = &mut entries;
    e[team].recurrence = Some(Recurrence {
        freq: Freq::Weekly,
        interval: 1,
        weekdays: vec![1],
        end: RecurEnd::Never,
    });
    e[team].location = "Raum 2 / Videoanruf".into();
    e[team].participants = vec![
        p("Lena Koch", Partstat::Accepted),
        p("Paul Lange", Partstat::Accepted),
        p("Jonas Weber", Partstat::NeedsAction),
    ];
    e[yoga].recurrence = Some(Recurrence {
        freq: Freq::Weekly,
        interval: 1,
        weekdays: vec![1],
        end: RecurEnd::Never,
    });
    for s in [
        "Staubsaugen",
        "Bad putzen",
        "Küche wischen",
        "Müll rausbringen",
        "Fenster im Wohnzimmer",
    ] {
        e[putzen].add_subtask(s);
    }
    e[putzen].set_done(day(1), 0, true);
    e[putzen].set_done(day(1), 1, true);
    e[putzen].recurrence = Some(Recurrence {
        freq: Freq::Weekly,
        interval: 1,
        weekdays: vec![1],
        end: RecurEnd::Never,
    });
    e[kunde].location = "Hafenstraße 12, Hamburg".into();
    e[kunde].notes = "Unterlagen vom Oktober mitbringen.".into();
    e[kunde].travel = 30;
    e[kunde].buffer = 15;
    e[kunde].participants = vec![
        p("Sophie Wagner", Partstat::Accepted),
        p("Tom Berg", Partstat::NeedsAction),
        p("Mia Roth", Partstat::Declined),
    ];
    e[workshop].participants = vec![
        p("Lena Koch", Partstat::Accepted),
        p("Jonas Weber", Partstat::Accepted),
    ];
    e[wohnung].location = "Lindenweg 4".into();
    e[wohnung].travel = 30;
    for s in [
        "Belege sortieren",
        "Kontoauszüge herunterladen",
        "Fragen für den Steuerberater notieren",
    ] {
        e[steuer].add_subtask(s);
    }
    for s in ["Milch", "Brot", "Waschmittel"] {
        e[einkauf].add_subtask(s);
    }
    for i in 0..3 {
        e[einkauf].set_done(day(0), i, true);
    }
    e[urlaub].all_day = true;
    e[urlaub].start = midnight(day(9));
    e[urlaub].end = midnight(day(12));
    e[halb].all_day = true;
    e[halb].half = Some(HalfDay::Morning);
    e[halb].start = midnight(day(3));
    e[halb].end = midnight(day(4));
    e[paul].all_day = true;
    e[paul].birth_year = Some(1996);
    let born = day(3).with_year(1996).expect("valid date");
    e[paul].start = midnight(born);
    e[paul].end = midnight(born + Duration::days(1));
    e[paul].recurrence = Some(Recurrence {
        freq: Freq::Yearly,
        interval: 1,
        weekdays: vec![],
        end: RecurEnd::Never,
    });
    for entry in entries {
        let _ = state.save_entry(entry);
    }
    for (name, minutes, subtasks) in [
        (
            "Putzen",
            120,
            vec![
                "Staubsaugen",
                "Bad putzen",
                "Küche wischen",
                "Müll rausbringen",
                "Fenster im Wohnzimmer",
            ],
        ),
        (
            "Wocheneinkauf",
            60,
            vec!["Milch", "Brot", "Obst und Gemüse", "Waschmittel"],
        ),
    ] {
        let _ = state.save_routine(Routine {
            uid: String::new(),
            id: 0,
            name: name.into(),
            minutes,
            subtasks: subtasks.into_iter().map(String::from).collect(),
        });
    }
    state.settings.own_name = "Max Berger".into();
    state.settings.own_email = "max.berger@example.org".into();
}

fn p(name: &str, status: Partstat) -> Participant {
    let email = format!(
        "{}@example.org",
        name.split(' ').next().unwrap_or("x").to_lowercase()
    );
    Participant {
        name: name.into(),
        email,
        status,
    }
}

/// KALENDER_SHOT=/path.png [KALENDER_SHOT_VIEW=week|month] [KALENDER_SHOT_OPEN=title]:
/// saves a picture of the window and quits.
pub fn shot(ui: &UiRc) {
    let Some(path) = std::env::var_os("KALENDER_SHOT") else {
        return;
    };
    // The screen may be locked: without frames, animations never finish.
    if let Some(settings) = gtk::Settings::default() {
        settings.set_gtk_enable_animations(false);
    }
    ui.stack.set_transition_type(gtk::StackTransitionType::None);
    if let Ok(view) = std::env::var("KALENDER_SHOT_VIEW") {
        ui.set_view(if view == "month" {
            View::Month
        } else {
            View::Week
        });
    }
    if let Some((w, h)) = std::env::var("KALENDER_SHOT_SIZE").ok().and_then(|s| {
        let (w, h) = s.split_once('x')?;
        Some((w.parse().ok()?, h.parse().ok()?))
    }) {
        ui.window.set_default_size(w, h);
    }
    if let Ok(dark) = std::env::var("KALENDER_SHOT_DARK") {
        adw::StyleManager::default().set_color_scheme(if dark == "1" {
            adw::ColorScheme::ForceDark
        } else {
            adw::ColorScheme::ForceLight
        });
    }
    let ui2 = ui.clone();
    glib::timeout_add_local_once(std::time::Duration::from_millis(1200), move || {
        if let Ok(action) = std::env::var("KALENDER_SHOT_ACTION") {
            run_action(&ui2, &action);
        }
        let ui3 = ui2.clone();
        let tries = std::cell::Cell::new(0);
        glib::timeout_add_local(std::time::Duration::from_millis(2500), move || {
            tries.set(tries.get() + 1);
            // Without a frame from the compositor (locked screen) the first try may draw nothing.
            if !save_png(&ui3, std::path::Path::new(&path)) && tries.get() < 6 {
                ui3.window.queue_draw();
                return glib::ControlFlow::Continue;
            }
            if let Some(app) = ui3.window.application() {
                app.quit();
            }
            glib::ControlFlow::Break
        });
    });
}

fn run_action(ui: &UiRc, action: &str) {
    let (name, arg) = action.split_once(':').unwrap_or((action, ""));
    match name {
        "open" => {
            let found = ui
                .state
                .borrow()
                .entries
                .iter()
                .find(|e| e.title == arg)
                .map(|e| (e.id, e.start.date()));
            if let Some((id, date)) = found {
                let date = kalender_core::recur::occurrences(
                    ui.state.borrow().entry(id).expect("entry"),
                    midnight(week_start(today())),
                    midnight(week_start(today()) + Duration::days(7)),
                )
                .first()
                .map(|o| o.date)
                .unwrap_or(date);
                crate::detail::show_centered(ui, id, date, gdk::Rectangle::new(0, 0, 1, 1));
            }
        }
        "new" => {
            let kind = ui
                .state
                .borrow()
                .kinds
                .iter()
                .find(|k| k.name == arg)
                .map(|k| k.id);
            let start = at(today(), 10, 0);
            crate::editor::open_new(ui, start, start + Duration::hours(1), false, kind);
        }
        "prefs" if arg.is_empty() => crate::prefs::show(ui),
        "prefs" => crate::prefs::show_page(ui, arg),
        "ctx" | "edit" | "dup" | "delete" => {
            let found = ui
                .state
                .borrow()
                .entries
                .iter()
                .find(|e| e.title == arg)
                .map(|e| (e.id, e.start.date()));
            if let Some((id, date)) = found {
                match name {
                    "ctx" => {
                        let child = ui.stack.visible_child().expect("page");
                        crate::ops::context_menu(ui, &child, id, date, 400.0, 300.0);
                    }
                    "edit" => crate::editor::open_existing(ui, id, date),
                    "dup" => crate::ops::duplicate(ui, id, date),
                    _ => crate::ops::delete(ui, id, date),
                }
            }
        }
        "search" => {
            ui.search_bar.set_search_mode(true);
            ui.search_entry.set_text(arg);
        }
        "kinds" => crate::prefs::new_kind(ui),
        "shortcuts" => crate::prefs::shortcuts(ui),
        "routines" => ui.routines_button.popup(),
        "backup" => {
            let _ = crate::backup::run(&ui.state.borrow(), true);
        }
        "subscribe" => crate::subscriptions::subscribe(ui, arg, ""),
        "notify" => {
            let st = ui.state.borrow();
            let from = midnight(today());
            if let Some(item) = st
                .items(from, from + Duration::days(7))
                .into_iter()
                .find(|i| i.entry.title == arg)
            {
                let app = ui.window.application().expect("app");
                crate::notify::send(app.downcast_ref().expect("adw app"), &item);
            }
        }
        _ => {}
    }
}

fn save_png(ui: &UiRc, path: &std::path::Path) -> bool {
    let window: gtk::Widget = ui.window.clone().upcast();
    if !render(&window, path) {
        return false;
    }
    // Popovers have their own surface; saved next to the window picture.
    if let Some(p) = crate::detail::LAST
        .with(|l| l.borrow().clone())
        .filter(|p| p.is_visible())
    {
        render(p.upcast_ref(), &path.with_extension("popover.png"));
    }
    if let Some(child) = DIALOG.with(|l| l.borrow().clone()).and_then(|d| d.child()) {
        render(&child, &path.with_extension("dialog.png"));
    }
    true
}

thread_local! {
    static DIALOG: std::cell::RefCell<Option<adw::Dialog>> = const { std::cell::RefCell::new(None) };
}

/// The last opened dialog, for screenshots of the app.
pub fn remember_dialog(dialog: &adw::Dialog) {
    if std::env::var_os("KALENDER_SHOT").is_some() {
        DIALOG.with(|l| *l.borrow_mut() = Some(dialog.clone()));
    }
}

fn render(window: &gtk::Widget, path: &std::path::Path) -> bool {
    let paintable = gtk::WidgetPaintable::new(Some(window));
    let (w, h) = (window.width(), window.height());
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, w as f64, h as f64);
    let node = snapshot.to_node().or_else(|| {
        let again = gtk::Snapshot::new();
        paintable
            .current_image()
            .snapshot(&again, w as f64, h as f64);
        again.to_node()
    });
    let Some(node) = node else {
        tracing::warn!("screenshot: nothing drawn");
        return false;
    };
    let Some(renderer) = window.native().and_then(|n| n.renderer()) else {
        tracing::warn!("screenshot: no renderer");
        return false;
    };
    let texture = renderer.render_texture(
        &node,
        Some(&gtk::graphene::Rect::new(0.0, 0.0, w as f32, h as f32)),
    );
    if let Err(e) = texture.save_to_png(path) {
        tracing::warn!("screenshot failed: {e}");
        return false;
    }
    true
}
