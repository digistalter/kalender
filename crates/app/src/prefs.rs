// SPDX-License-Identifier: GPL-3.0-or-later

//! Settings: general, kinds, routines; plus the keyboard shortcuts.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use kalender_core::model::{Id, Kind, Routine};

use crate::draw::{IconView, KIND_ICONS, PALETTE};
use crate::i18n::{tr, trf, trn};
use crate::state::{View, kind_label};
use crate::window::UiRc;
use crate::{fmt, share};

const DURATIONS: [i64; 5] = [30, 45, 60, 90, 120];
const REMINDERS: [i64; 7] = [-1, 0, 5, 10, 15, 30, 60];
const BUFFERS: [i64; 5] = [0, 5, 10, 15, 30];

fn combo(title: &str, items: &[String], selected: usize) -> adw::ComboRow {
    let strs: Vec<&str> = items.iter().map(String::as_str).collect();
    adw::ComboRow::builder()
        .title(title)
        .model(&gtk::StringList::new(&strs))
        .selected(selected as u32)
        .build()
}

fn save(ui: &UiRc) {
    if let Err(e) = ui.state.borrow().save_settings() {
        ui.error(&e);
    }
    ui.refresh();
}

pub fn show_routines(ui: &UiRc) {
    show_page(ui, "routines");
}

pub fn show(ui: &UiRc) {
    show_page(ui, "general");
}

pub fn new_kind(ui: &UiRc) {
    kind_dialog(ui, None, None);
}

/// Name, colour and symbol of an existing kind (also of a subscribed calendar).
pub fn edit_kind(ui: &UiRc, id: Id) {
    let kind = ui.state.borrow().kind(id).cloned();
    if let Some(kind) = kind {
        kind_dialog(ui, Some(kind), None);
    }
}

pub fn show_page(ui: &UiRc, page: &str) {
    let dialog = adw::PreferencesDialog::builder()
        .search_enabled(false)
        .build();
    dialog.add(&general_page(ui));
    let kinds = adw::PreferencesPage::builder()
        .title(tr("Kinds"))
        .icon_name("view-list-symbolic")
        .name("kinds")
        .build();
    let routines = adw::PreferencesPage::builder()
        .title(tr("Routines"))
        .icon_name("view-list-bullet-symbolic")
        .name("routines")
        .build();
    dialog.add(&kinds);
    dialog.add(&routines);
    fill_kinds(ui, &kinds, &dialog);
    fill_routines(ui, &routines, &dialog);
    dialog.set_visible_page_name(page);
    crate::devtools::remember_dialog(dialog.upcast_ref());
    dialog.present(Some(&ui.window));
}

fn general_page(ui: &UiRc) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::builder()
        .title(tr("General"))
        .icon_name("emblem-system-symbolic")
        .name("general")
        .build();
    let s = ui.state.borrow().settings.clone();

    let view = adw::PreferencesGroup::builder()
        .title(tr("Display"))
        .build();
    let week_numbers = adw::SwitchRow::builder()
        .title(tr("Show week numbers"))
        .subtitle(tr("In the month and the small calendar"))
        .active(s.week_numbers)
        .build();
    let dim = adw::SwitchRow::builder()
        .title(tr("Fade the past and finished tasks"))
        .subtitle(tr("Past entries and finished tasks step back"))
        .active(s.dim_past)
        .build();
    let start = combo(
        &tr("View at start"),
        &[tr("Week"), tr("Month")],
        if s.start_view == View::Month { 1 } else { 0 },
    );
    view.add(&week_numbers);
    view.add(&dim);
    view.add(&start);
    page.add(&view);

    let new = adw::PreferencesGroup::builder()
        .title(tr("New entries"))
        .build();
    let duration_labels: Vec<String> = DURATIONS.iter().map(|m| fmt::minutes(*m)).collect();
    let duration = combo(
        &tr("Default length"),
        &duration_labels,
        DURATIONS
            .iter()
            .position(|m| *m == s.default_minutes)
            .unwrap_or(2),
    );
    let reminder_labels: Vec<String> = REMINDERS
        .iter()
        .map(|m| match m {
            -1 => tr("No reminder"),
            0 => tr("At the start"),
            m => trf("{} before", &[&fmt::minutes(*m)]),
        })
        .collect();
    let reminder = combo(
        &tr("Default reminder"),
        &reminder_labels,
        REMINDERS
            .iter()
            .position(|m| *m == s.default_reminder.unwrap_or(-1))
            .unwrap_or(4),
    );
    let buffer_labels: Vec<String> = BUFFERS
        .iter()
        .map(|m| {
            if *m == 0 {
                tr("None")
            } else {
                fmt::minutes(*m)
            }
        })
        .collect();
    let buffer = combo(
        &tr("Buffer after entries"),
        &buffer_labels,
        BUFFERS
            .iter()
            .position(|m| *m == s.default_buffer)
            .unwrap_or(0),
    );
    new.add(&duration);
    new.add(&reminder);
    new.add(&buffer);
    page.add(&new);

    let half = adw::PreferencesGroup::builder()
        .title(tr("Vacation"))
        .build();
    let per_year = adw::SpinRow::with_range(0.0, 100.0, 1.0);
    per_year.set_title(&tr("Vacation days per year"));
    per_year.set_subtitle(&tr("Shows how many days are left; 0 = don't count"));
    per_year.set_value(s.vacation_per_year as f64);
    half.add(&per_year);
    let u = ui.clone();
    per_year.connect_value_notify(move |r| {
        u.state.borrow_mut().settings.vacation_per_year = r.value() as u32;
        save(&u);
    });
    page.add(&half);

    let holidays = adw::PreferencesGroup::builder()
        .title(tr("Holidays"))
        .build();
    let mut names = vec![tr("No holidays"), tr("Germany, nationwide only")];
    names.extend(
        kalender_core::holidays::STATES
            .iter()
            .map(|(_, n)| (*n).to_owned()),
    );
    let current = match s.holiday_state.as_str() {
        "-" => 0,
        "" => 1,
        code => kalender_core::holidays::STATES
            .iter()
            .position(|(c, _)| *c == code)
            .map(|p| p + 2)
            .unwrap_or(1),
    };
    let state_row = combo(&tr("Federal state"), &names, current);
    holidays.add(&state_row);
    page.add(&holidays);

    let invites = adw::PreferencesGroup::builder()
        .title(tr("Invitations"))
        .description(tr(
            "Sender of invitations; answers to this address are recognised.",
        ))
        .build();
    let name = adw::EntryRow::builder()
        .title(tr("Your name"))
        .text(&s.own_name)
        .build();
    let email = adw::EntryRow::builder()
        .title(tr("Your e-mail address"))
        .text(&s.own_email)
        .build();
    invites.add(&name);
    invites.add(&email);
    page.add(&invites);

    let system = adw::PreferencesGroup::builder().title(tr("System")).build();
    let background = adw::SwitchRow::builder()
        .title(tr("Run in the background"))
        .subtitle(tr(
            "Reminders also arrive when the window is closed or not yet opened",
        ))
        .active(s.run_in_background)
        .build();
    let language = combo(
        &tr("Language"),
        &[tr("As in the system"), "Deutsch".into(), "English".into()],
        match s.language.as_str() {
            "de" => 1,
            "en" => 2,
            _ => 0,
        },
    );
    language.set_subtitle(&tr("Takes effect after a restart"));
    system.add(&background);
    system.add(&language);
    page.add(&system);

    let u = ui.clone();
    week_numbers.connect_active_notify(move |r| {
        u.state.borrow_mut().settings.week_numbers = r.is_active();
        save(&u);
    });
    let u = ui.clone();
    dim.connect_active_notify(move |r| {
        u.state.borrow_mut().settings.dim_past = r.is_active();
        save(&u);
    });
    let u = ui.clone();
    start.connect_selected_notify(move |r| {
        u.state.borrow_mut().settings.start_view = if r.selected() == 1 {
            View::Month
        } else {
            View::Week
        };
        save(&u);
    });
    let u = ui.clone();
    duration.connect_selected_notify(move |r| {
        u.state.borrow_mut().settings.default_minutes = DURATIONS[r.selected() as usize];
        save(&u);
    });
    let u = ui.clone();
    reminder.connect_selected_notify(move |r| {
        let m = REMINDERS[r.selected() as usize];
        u.state.borrow_mut().settings.default_reminder = (m >= 0).then_some(m);
        save(&u);
    });
    let u = ui.clone();
    buffer.connect_selected_notify(move |r| {
        u.state.borrow_mut().settings.default_buffer = BUFFERS[r.selected() as usize];
        save(&u);
    });
    let u = ui.clone();
    state_row.connect_selected_notify(move |r| {
        let code = match r.selected() {
            0 => "-".to_owned(),
            1 => String::new(),
            n => kalender_core::holidays::STATES[n as usize - 2].0.to_owned(),
        };
        u.state.borrow_mut().settings.holiday_state = code;
        save(&u);
    });
    let u = ui.clone();
    name.connect_changed(move |e| {
        u.state.borrow_mut().settings.own_name = e.text().trim().to_owned();
        let _ = u.state.borrow().save_settings();
    });
    let u = ui.clone();
    email.connect_changed(move |e| {
        u.state.borrow_mut().settings.own_email = e.text().trim().to_owned();
        let _ = u.state.borrow().save_settings();
    });
    let u = ui.clone();
    background.connect_active_notify(move |r| {
        u.state.borrow_mut().settings.run_in_background = r.is_active();
        save(&u);
        crate::background::sync(r.is_active());
    });
    let u = ui.clone();
    language.connect_selected_notify(move |r| {
        u.state.borrow_mut().settings.language = ["", "de", "en"][r.selected() as usize].to_owned();
        save(&u);
    });
    page.add(&backup_group(ui));
    page
}

/// Settings → General → Backup.
fn backup_group(ui: &UiRc) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder()
        .title(tr("Backup"))
        .description(tr(
            "Your calendar lives only on this computer. A folder in your Nextcloud or Dropbox, or on a USB drive, keeps the copies safe even if the computer breaks.",
        ))
        .build();
    let auto = adw::SwitchRow::builder()
        .title(tr("Back up automatically"))
        .subtitle(tr("Once a day; the last ten copies are kept"))
        .active(ui.state.borrow().settings.backup)
        .build();
    let folder_text = crate::backup::folder(&ui.state.borrow())
        .display()
        .to_string();
    let folder = adw::ActionRow::builder()
        .title(tr("Folder"))
        .subtitle(glib::markup_escape_text(&folder_text))
        .build();
    let change = gtk::Button::builder()
        .label(tr("Change …"))
        .valign(gtk::Align::Center)
        .build();
    folder.add_suffix(&change);
    let now = button_row(&tr("Back up now"), "document-save-symbolic");
    let restore = button_row(&tr("Restore from backup …"), "document-revert-symbolic");
    group.add(&auto);
    group.add(&folder);
    group.add(&now);
    group.add(&restore);

    let u = ui.clone();
    auto.connect_active_notify(move |r| {
        u.state.borrow_mut().settings.backup = r.is_active();
        save(&u);
    });
    let u = ui.clone();
    let row = folder.clone();
    change.connect_clicked(move |_| {
        let dialog = gtk::FileDialog::builder()
            .title(tr("Folder for backups"))
            .build();
        let u2 = u.clone();
        let row = row.clone();
        dialog.select_folder(
            Some(&u.window),
            gtk::gio::Cancellable::NONE,
            move |result| {
                if let Some(path) = result.ok().and_then(|f| f.path()) {
                    let text = path.display().to_string();
                    u2.state.borrow_mut().settings.backup_dir = text.clone();
                    save(&u2);
                    row.set_subtitle(&glib::markup_escape_text(&text));
                }
            },
        );
    });
    let u = ui.clone();
    now.connect_activated(move |_| {
        let result = crate::backup::run(&u.state.borrow(), true);
        match result {
            Ok(Some(file)) => u.toast(&trf(
                "Backed up as {}",
                &[&file
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default()],
            )),
            Ok(None) => {}
            Err(e) => u.error(&e),
        }
    });
    let u = ui.clone();
    restore.connect_activated(move |_| restore_dialog(&u));
    group
}

/// Picks a backup file and, after asking, replaces all data with it.
fn restore_dialog(ui: &UiRc) {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some(&tr("Calendar backups")));
    filter.add_suffix("sqlite");
    let filters = gtk::gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    let initial = gtk::gio::File::for_path(crate::backup::folder(&ui.state.borrow()));
    let dialog = gtk::FileDialog::builder()
        .title(tr("Restore from backup"))
        .filters(&filters)
        .initial_folder(&initial)
        .build();
    let u = ui.clone();
    dialog.open(Some(&ui.window), gtk::gio::Cancellable::NONE, move |result| {
        let Some(path) = result.ok().and_then(|f| f.path()) else {
            return;
        };
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let ask = adw::AlertDialog::new(
            Some(&tr("Restore backup?")),
            Some(&trf(
                "All current entries, kinds, routines and settings are replaced by the content of “{}”. Before that, the current state is saved as a backup too.",
                &[&name],
            )),
        );
        ask.add_responses(&[("cancel", &tr("Cancel")), ("restore", &tr("Restore"))]);
        ask.set_response_appearance("restore", adw::ResponseAppearance::Destructive);
        ask.set_close_response("cancel");
        let u2 = u.clone();
        ask.connect_response(Some("restore"), move |_, _| {
            // Keep the current state, just in case.
            let safety = crate::backup::run(&u2.state.borrow(), true);
            if let Err(e) = safety {
                u2.error(&e);
                return;
            }
            let result = u2.state.borrow_mut().restore_from(&path);
            match result {
                Ok(()) => {
                    u2.rebuild_kinds();
                    u2.refresh();
                    u2.toast(&tr("Backup restored"));
                }
                Err(e) => u2.error(&e),
            }
        });
        ask.present(Some(&u.window));
    });
}

/// A row that works like a button (libadwaita 1.5 has no ButtonRow yet).
fn button_row(title: &str, icon: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(title)
        .activatable(true)
        .build();
    row.add_prefix(&gtk::Image::from_icon_name(icon));
    row.add_css_class("accent");
    row
}

fn tile(icon: &str, color: &str) -> gtk::Widget {
    let frame = gtk::Box::builder()
        .width_request(40)
        .height_request(32)
        .hexpand(false)
        .halign(gtk::Align::Start)
        .valign(gtk::Align::Center)
        .css_classes(["kind-tile"])
        .build();
    let provider = gtk::CssProvider::new();
    let class = format!("tile-{}", color.trim_start_matches('#'));
    provider.load_from_string(&format!(
        ".kind-tile.{class} {{ background-color: {color}; border-radius: 8px; }}"
    ));
    gtk::style_context_add_provider_for_display(
        &gtk::gdk::Display::default().expect("display"),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    frame.add_css_class(&class);
    let icon = IconView::new(icon, Some("#ffffff"), 18);
    icon.set_halign(gtk::Align::Center);
    icon.set_valign(gtk::Align::Center);
    icon.set_hexpand(true);
    frame.append(&icon);
    frame.upcast()
}

fn fill_kinds(ui: &UiRc, page: &adw::PreferencesPage, dialog: &adw::PreferencesDialog) {
    let group = adw::PreferencesGroup::builder()
        .title(tr("Kinds"))
        .description(tr(
            "Every entry has one kind. Its colour and symbol show what kind of entry something is.",
        ))
        .build();
    let kinds = ui.state.borrow().kinds.clone();
    for kind in kinds {
        let count = ui
            .state
            .borrow()
            .entries
            .iter()
            .filter(|e| e.kind == kind.id)
            .count();
        let origin = match kind.builtin {
            _ if kind.source.is_some() => tr("Subscribed"),
            Some(b) if b.is_automatic() => tr("Automatic"),
            Some(_) => tr("Default"),
            None => tr("Own kind"),
        };
        let row = adw::ActionRow::builder()
            .title(glib::markup_escape_text(&kind_label(&kind)))
            .subtitle(format!(
                "{origin} · {}",
                trn("{} entry", "{} entries", count)
            ))
            .activatable(true)
            .build();
        row.add_prefix(&tile(&kind.icon, &kind.color));
        row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
        let u = ui.clone();
        let d = dialog.clone();
        let k = kind.clone();
        row.connect_activated(move |_| kind_dialog(&u, Some(k.clone()), Some(d.clone())));
        group.add(&row);
    }
    let add = button_row(&tr("Add kind …"), "list-add-symbolic");
    let u = ui.clone();
    let d = dialog.clone();
    add.connect_activated(move |_| kind_dialog(&u, None, Some(d.clone())));
    group.add(&add);
    page.add(&group);

    let transfer = adw::PreferencesGroup::builder()
        .title(tr("Transfer entries"))
        .build();
    let import = button_row(&tr("Import from file …"), "document-open-symbolic");
    let export = button_row(&tr("Export as file …"), "document-save-symbolic");
    let u = ui.clone();
    import.connect_activated(move |_| share::import_file(&u));
    let u = ui.clone();
    export.connect_activated(move |_| share::export_all(&u));
    transfer.add(&import);
    transfer.add(&export);
    page.add(&transfer);
}

/// New or changed kind: name, colour and symbol.
fn kind_dialog(ui: &UiRc, kind: Option<Kind>, prefs: Option<adw::PreferencesDialog>) {
    let editing = kind.is_some();
    let kind = kind.unwrap_or(Kind {
        id: 0,
        name: String::new(),
        color: PALETTE[8].into(),
        icon: "star".into(),
        uid: String::new(),
        builtin: None,
        visible: true,
        source: None,
    });
    let current = Rc::new(RefCell::new(kind.clone()));
    let dialog = adw::AlertDialog::new(
        Some(&if editing {
            tr("Change kind")
        } else {
            tr("New kind")
        }),
        None,
    );
    let b = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let list = gtk::ListBox::builder()
        .css_classes(["boxed-list"])
        .selection_mode(gtk::SelectionMode::None)
        .build();
    let name = adw::EntryRow::builder()
        .title(tr("Name"))
        .text(kind_label(&kind))
        .build();
    list.append(&name);
    b.append(&list);

    let preview = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let preview_icon = IconView::new(&kind.icon, Some(&kind.color), 20);
    let preview_label = gtk::Label::builder()
        .label(if kind.name.is_empty() {
            tr("Sport")
        } else {
            kind_label(&kind)
        })
        .css_classes(["heading"])
        .build();
    preview.append(&preview_icon);
    preview.append(&preview_label);
    preview.set_halign(gtk::Align::Center);

    b.append(
        &gtk::Label::builder()
            .label(tr("Colour"))
            .xalign(0.0)
            .css_classes(["dim-label", "caption-heading"])
            .build(),
    );
    let colors = gtk::FlowBox::builder()
        .max_children_per_line(6)
        .min_children_per_line(6)
        .selection_mode(gtk::SelectionMode::None)
        .column_spacing(6)
        .row_spacing(6)
        .homogeneous(true)
        .css_classes(["chips"])
        .build();
    for color in PALETTE {
        let button = gtk::Button::builder()
            .child(&tile("check", color))
            .css_classes(["flat"])
            .tooltip_text(color)
            .build();
        let c = current.clone();
        let (pi, pl) = (preview_icon.clone(), preview_label.clone());
        button.connect_clicked(move |_| {
            c.borrow_mut().color = color.to_owned();
            let k = c.borrow();
            pi.set(&k.icon, Some(&k.color));
            let _ = &pl;
        });
        colors.insert(&button, -1);
    }
    b.append(&colors);
    b.append(
        &gtk::Label::builder()
            .label(tr("Symbol"))
            .xalign(0.0)
            .css_classes(["dim-label", "caption-heading"])
            .build(),
    );
    let icons = gtk::FlowBox::builder()
        .max_children_per_line(6)
        .min_children_per_line(6)
        .selection_mode(gtk::SelectionMode::None)
        .column_spacing(6)
        .row_spacing(6)
        .homogeneous(true)
        .css_classes(["chips"])
        .build();
    for icon in KIND_ICONS {
        let button = gtk::Button::builder()
            .child(&IconView::new(icon, None, 18))
            .tooltip_text(icon)
            .build();
        let c = current.clone();
        let pi = preview_icon.clone();
        button.connect_clicked(move |_| {
            c.borrow_mut().icon = icon.to_owned();
            let k = c.borrow();
            pi.set(&k.icon, Some(&k.color));
        });
        icons.insert(&button, -1);
    }
    b.append(&icons);
    b.append(
        &gtk::Label::builder()
            .label(tr("Preview"))
            .xalign(0.0)
            .css_classes(["dim-label", "caption-heading"])
            .build(),
    );
    b.append(&preview);
    let pl = preview_label.clone();
    name.connect_changed(move |e| pl.set_label(&e.text()));
    dialog.set_extra_child(Some(&b));

    // Subscriptions are removed from their menu in the sidebar.
    let can_delete = kind.builtin.is_none() && kind.source.is_none() && editing;
    if can_delete {
        dialog.add_response("delete", &tr("Delete"));
        dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    }
    dialog.add_responses(&[
        ("cancel", &tr("Cancel")),
        ("save", &if editing { tr("Save") } else { tr("Add") }),
    ]);
    dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("save"));
    dialog.set_close_response("cancel");
    let weak = Rc::downgrade(ui);
    let parent: gtk::Widget = match &prefs {
        Some(p) => p.clone().upcast(),
        None => ui.window.clone().upcast(),
    };
    dialog.connect_response(None, move |_, response| {
        let Some(u) = weak.upgrade() else { return };
        let mut k = current.borrow().clone();
        match response {
            "save" => {
                let text = name.text().trim().to_owned();
                if text.is_empty() {
                    u.toast(&tr("Please enter a name"));
                    return;
                }
                if text != kind_label(&k) {
                    k.name = text;
                }
                let result = u.state.borrow_mut().save_kind(k);
                if let Err(e) = result {
                    u.error(&e);
                }
            }
            "delete" => {
                let result = u.state.borrow_mut().delete_kind(k.id);
                match result {
                    Ok(()) => u.toast(&trf(
                        "Kind “{}” deleted; its entries are appointments now",
                        &[&k.name],
                    )),
                    Err(e) => u.error(&e),
                }
            }
            _ => return,
        }
        u.rebuild_kinds();
        u.refresh();
        if let Some(p) = &prefs {
            p.close();
            show_page(&u, "kinds");
        }
    });
    dialog.present(Some(&parent));
}

fn fill_routines(ui: &UiRc, page: &adw::PreferencesPage, dialog: &adw::PreferencesDialog) {
    let group = adw::PreferencesGroup::builder()
        .title(tr("Routines"))
        .description(tr("A routine is a template for tasks. It can also be made in the calendar: right click on a task → “Save as routine”. Changes only apply to new tasks."))
        .build();
    let routines = ui.state.borrow().routines.clone();
    if routines.is_empty() {
        group.add(
            &adw::ActionRow::builder()
                .title(tr("No routines yet"))
                .css_classes(["dim-label"])
                .build(),
        );
    }
    for r in routines {
        let preview: Vec<&str> = r.subtasks.iter().take(3).map(String::as_str).collect();
        let row = adw::ActionRow::builder()
            .title(glib::markup_escape_text(&r.name))
            .subtitle(glib::markup_escape_text(&format!(
                "{} · {} · {}",
                trn("{} subtask", "{} subtasks", r.subtasks.len()),
                fmt::minutes(r.minutes),
                preview.join(", ")
            )))
            .activatable(true)
            .build();
        row.add_prefix(&tile("routine", "#2e9d76"));
        row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
        let u = ui.clone();
        let d = dialog.clone();
        let routine = r.clone();
        row.connect_activated(move |_| routine_dialog(&u, Some(routine.clone()), d.clone()));
        group.add(&row);
    }
    let add = button_row(&tr("Add routine …"), "list-add-symbolic");
    let u = ui.clone();
    let d = dialog.clone();
    add.connect_activated(move |_| routine_dialog(&u, None, d.clone()));
    group.add(&add);
    page.add(&group);
}

fn routine_dialog(ui: &UiRc, routine: Option<Routine>, prefs: adw::PreferencesDialog) {
    let editing = routine.is_some();
    let routine = routine.unwrap_or(Routine {
        id: 0,
        uid: String::new(),
        name: String::new(),
        minutes: 60,
        subtasks: Vec::new(),
    });
    let dialog = adw::AlertDialog::new(
        Some(&if editing {
            tr("Change routine")
        } else {
            tr("New routine")
        }),
        None,
    );
    let b = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let list = gtk::ListBox::builder()
        .css_classes(["boxed-list"])
        .selection_mode(gtk::SelectionMode::None)
        .build();
    let name = adw::EntryRow::builder()
        .title(tr("Name"))
        .text(&routine.name)
        .build();
    let minutes = adw::SpinRow::with_range(15.0, 480.0, 15.0);
    minutes.set_title(&tr("Length in minutes"));
    minutes.set_value(routine.minutes as f64);
    list.append(&name);
    list.append(&minutes);
    b.append(&list);
    b.append(
        &gtk::Label::builder()
            .label(tr("Subtasks, one per line"))
            .xalign(0.0)
            .css_classes(["dim-label", "caption-heading"])
            .build(),
    );
    let text = gtk::TextView::builder()
        .wrap_mode(gtk::WrapMode::WordChar)
        .top_margin(8)
        .bottom_margin(8)
        .left_margin(10)
        .right_margin(10)
        .height_request(140)
        .build();
    text.buffer().set_text(&routine.subtasks.join("\n"));
    b.append(
        &gtk::Frame::builder()
            .child(&text)
            .css_classes(["view"])
            .build(),
    );
    dialog.set_extra_child(Some(&b));
    if editing {
        dialog.add_response("delete", &tr("Delete"));
        dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    }
    dialog.add_responses(&[
        ("cancel", &tr("Cancel")),
        ("save", &if editing { tr("Save") } else { tr("Add") }),
    ]);
    dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);
    dialog.set_close_response("cancel");
    let weak = Rc::downgrade(ui);
    let id: Id = routine.id;
    let routine_uid = routine.uid.clone();
    let parent = prefs.clone();
    dialog.connect_response(None, move |_, response| {
        let Some(u) = weak.upgrade() else { return };
        match response {
            "save" => {
                let buffer = text.buffer();
                let subtasks: Vec<String> = buffer
                    .text(&buffer.start_iter(), &buffer.end_iter(), false)
                    .lines()
                    .map(|l| l.trim().to_owned())
                    .filter(|l| !l.is_empty())
                    .collect();
                let n = name.text().trim().to_owned();
                if n.is_empty() {
                    u.toast(&tr("Please enter a name"));
                    return;
                }
                let r = Routine {
                    id,
                    uid: routine_uid.clone(),
                    name: n,
                    minutes: minutes.value() as i64,
                    subtasks,
                };
                if let Err(e) = u.state.borrow_mut().save_routine(r) {
                    u.error(&e);
                }
            }
            "delete" => {
                if let Err(e) = u.state.borrow_mut().delete_routine(id) {
                    u.error(&e);
                }
            }
            _ => return,
        }
        prefs.close();
        show_page(&u, "routines");
    });
    dialog.present(Some(&parent));
}

/// Overview of all keyboard shortcuts.
pub fn shortcuts(ui: &UiRc) {
    let dialog = adw::Dialog::builder()
        .title(tr("Keyboard shortcuts"))
        .content_width(520)
        .content_height(620)
        .build();
    let page = adw::PreferencesPage::new();
    let groups: [(String, Vec<(String, &str)>); 3] = [
        (
            tr("View"),
            vec![
                (tr("Week"), "1"),
                (tr("Month"), "2"),
                (tr("Today"), "t"),
                (tr("Back"), "Page_Up"),
                (tr("Forward"), "Page_Down"),
                (tr("Sidebar"), "F9"),
            ],
        ),
        (
            tr("Entries"),
            vec![
                (tr("New entry"), "<Control>n"),
                (tr("Undo"), "<Control>z"),
                (tr("Copy while dragging"), "Control_L"),
            ],
        ),
        (
            tr("General"),
            vec![
                (tr("Search"), "<Control>f"),
                (tr("Settings"), "<Control>comma"),
                (tr("Keyboard shortcuts"), "question"),
                (tr("Quit"), "<Control>q"),
            ],
        ),
    ];
    for (title, items) in groups {
        let group = adw::PreferencesGroup::builder().title(title).build();
        for (label, accel) in items {
            let row = adw::ActionRow::builder().title(label).build();
            row.add_suffix(
                &gtk::ShortcutLabel::builder()
                    .accelerator(accel)
                    .valign(gtk::Align::Center)
                    .build(),
            );
            group.add(&row);
        }
        page.add(&group);
    }
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&page));
    dialog.set_child(Some(&view));
    crate::devtools::remember_dialog(&dialog);
    dialog.present(Some(&ui.window));
}
