// SPDX-License-Identifier: GPL-3.0-or-later

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use kalender_core::store::Store;

use crate::config;
use crate::i18n::tr;
use crate::state::{Shared, State};
use crate::window::{self, UiRc};

pub fn database_path() -> std::path::PathBuf {
    glib::user_data_dir()
        .join("kalender")
        .join("kalender.sqlite")
}

pub fn run() -> glib::ExitCode {
    let app = adw::Application::builder()
        .application_id(config::APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build();
    let ui: Rc<RefCell<Option<UiRc>>> = Rc::default();
    let hold: Rc<RefCell<Option<gio::ApplicationHoldGuard>>> = Rc::default();

    app.connect_startup(|_| {
        let provider = gtk::CssProvider::new();
        provider.load_from_string(include_str!("style.css"));
        gtk::style_context_add_provider_for_display(
            &gdk::Display::default().expect("display"),
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    });

    // Started at login with --background: load everything for reminders, but show no window.
    let mut args: Vec<String> = std::env::args().collect();
    let hidden = std::rc::Rc::new(std::cell::Cell::new(false));
    if let Some(i) = args.iter().position(|a| a == crate::background::FLAG) {
        args.remove(i);
        hidden.set(true);
    }

    let (u, h) = (ui.clone(), hold.clone());
    app.connect_activate(move |app| {
        if hidden.replace(false) {
            if u.borrow().is_none() {
                main_window(app, &u, &h);
                *h.borrow_mut() = Some(app.hold());
            }
            return;
        }
        main_window(app, &u, &h).window.present();
    });
    let (u, h) = (ui.clone(), hold.clone());
    app.connect_open(move |app, files, _| {
        let ui = main_window(app, &u, &h);
        ui.window.present();
        for file in files {
            crate::share::open_file(&ui, file);
        }
    });
    app.run_with_args(&args)
}

/// The one main window, made on first use.
fn main_window(
    app: &adw::Application,
    slot: &Rc<RefCell<Option<UiRc>>>,
    hold: &Rc<RefCell<Option<gio::ApplicationHoldGuard>>>,
) -> UiRc {
    if let Some(ui) = slot.borrow().as_ref() {
        hold.borrow_mut().take();
        return ui.clone();
    }
    let store = match Store::open(&database_path()) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("cannot open the calendar database: {e:#}");
            Store::in_memory().expect("in-memory database")
        }
    };
    let mut loaded = State::load(store).expect("calendar data");
    crate::devtools::demo(&mut loaded);
    let state: Shared = Rc::new(RefCell::new(loaded));
    let ui = window::build(app, state.clone());
    add_app_actions(app, &ui);
    crate::notify::start(app, state.clone());
    crate::subscriptions::start(&ui);
    crate::backup::start(&ui);
    crate::netsync::start(&ui);
    crate::background::sync(state.borrow().settings.run_in_background);

    // With "Run in the background", closing the window keeps reminders coming.
    let h = hold.clone();
    let app_weak = app.downgrade();
    let st = state.clone();
    ui.window.connect_close_request(move |w| {
        if st.borrow().settings.run_in_background
            && let Some(app) = app_weak.upgrade()
        {
            *h.borrow_mut() = Some(app.hold());
            w.set_visible(false);
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    *slot.borrow_mut() = Some(ui.clone());
    crate::devtools::shot(&ui);
    crate::devtools::sync_test(&ui);
    ui
}

fn add_app_actions(app: &adw::Application, ui: &UiRc) {
    let add = |name: &str, f: Box<dyn Fn(&UiRc)>| {
        let a = gio::SimpleAction::new(name, None);
        let weak = Rc::downgrade(ui);
        a.connect_activate(move |_, _| {
            if let Some(u) = weak.upgrade() {
                f(&u);
            }
        });
        app.add_action(&a);
    };
    add("preferences", Box::new(crate::prefs::show));
    add("shortcuts", Box::new(crate::prefs::shortcuts));
    add(
        "about",
        Box::new(|u| {
            let about = adw::AboutDialog::builder()
                .application_name(config::APP_NAME)
                .application_icon(config::APP_ID)
                .version(config::VERSION)
                .website("https://github.com/digistalter/kalender")
                .issue_url("https://github.com/digistalter/kalender/issues")
                .license_type(gtk::License::Gpl30)
                .developers(vec![tr("The developers of Kalender")])
                .build();
            about.present(Some(&u.window));
        }),
    );
    let quit = gio::SimpleAction::new("quit", None);
    let app_weak = app.downgrade();
    quit.connect_activate(move |_, _| {
        if let Some(app) = app_weak.upgrade() {
            app.quit();
        }
    });
    app.add_action(&quit);
    let activate = gio::SimpleAction::new("activate", None);
    let weak = Rc::downgrade(ui);
    activate.connect_activate(move |_, _| {
        if let Some(u) = weak.upgrade() {
            u.window.present();
        }
    });
    app.add_action(&activate);
    app.set_accels_for_action("app.preferences", &["<Control>comma"]);
    app.set_accels_for_action("app.shortcuts", &["<Control>question"]);
    app.set_accels_for_action("app.quit", &["<Control>q"]);
}
