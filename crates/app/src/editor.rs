// SPDX-License-Identifier: GPL-3.0-or-later

//! The window for new and changed entries. The kind is chosen at the top;
//! the fields below follow the kind (meeting: participants, task: subtasks,
//! vacation: whole or half days).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, Timelike};
use gtk::glib;
use kalender_core::model::{
    Builtin, Entry, Freq, HalfDay, Id, Participant, Partstat, RecurEnd, Recurrence, Routine, at,
    midnight,
};

use crate::draw::IconView;
use crate::i18n::{tr, trf, trn};
use crate::ops::{self, Scope};
use crate::state::kind_label;
use crate::window::UiRc;
use crate::{detail, fmt, share};

#[derive(Clone)]
struct Draft {
    existing: Option<(Id, NaiveDate)>,
    kind: Id,
    title: String,
    start: NaiveDateTime,
    end: NaiveDateTime,
    all_day: bool,
    half: Option<HalfDay>,
    location: String,
    notes: String,
    reminder: Option<i64>,
    travel: i64,
    buffer: i64,
    recurrence: Option<Recurrence>,
    subtasks: Vec<String>,
    participants: Vec<Participant>,
    routine: Option<Id>,
    save_routine: bool,
    /// Vacation over whole days: number of vacation days and whether weekends count.
    count_sat: bool,
    count_sun: bool,
    /// Birthdays: day, month and (optional) year of birth.
    birth_day: u32,
    birth_month: u32,
    birth_year: Option<i32>,
}

const REMINDERS: [Option<i64>; 9] = [
    None,
    Some(0),
    Some(5),
    Some(10),
    Some(15),
    Some(30),
    Some(60),
    Some(120),
    Some(1440),
];
const SPANS: [i64; 7] = [0, 15, 30, 45, 60, 90, 120];

pub fn open_new(
    ui: &UiRc,
    start: NaiveDateTime,
    end: NaiveDateTime,
    all_day: bool,
    kind: Option<Id>,
) {
    let st = ui.state.borrow();
    let kind = kind
        .or_else(|| st.builtin(Builtin::Event).map(|k| k.id))
        .unwrap_or(1);
    let draft = Draft {
        existing: None,
        kind,
        title: String::new(),
        start,
        end,
        all_day,
        half: None,
        location: String::new(),
        notes: String::new(),
        reminder: st.settings.default_reminder,
        travel: 0,
        buffer: st.settings.default_buffer,
        recurrence: None,
        subtasks: Vec::new(),
        participants: Vec::new(),
        routine: None,
        save_routine: false,
        count_sat: false,
        count_sun: false,
        birth_day: start.day(),
        birth_month: start.month(),
        birth_year: None,
    };
    let builtin = st.kind(kind).and_then(|k| k.builtin);
    drop(st);
    let mut draft = draft;
    switch_kind(&mut draft, None, builtin);
    present(ui, draft);
}

/// A sensible start for a new entry: the next full hour today, otherwise 9:00.
pub fn next_start(ui: &UiRc) -> NaiveDateTime {
    let day = ui.state.borrow().selected;
    if day == crate::state::today() {
        let n = crate::state::now();
        midnight(day) + Duration::hours(n.hour() as i64 + 1)
    } else {
        at(day, 9, 0)
    }
}

/// A new task filled from a routine.
pub fn open_routine(ui: &UiRc, routine: Id) {
    let Some(r) = ui
        .state
        .borrow()
        .routines
        .iter()
        .find(|r| r.id == routine)
        .cloned()
    else {
        return;
    };
    let task = ui.state.borrow().builtin(Builtin::Task).map(|k| k.id);
    let start = next_start(ui);
    let minutes = if r.minutes > 0 { r.minutes } else { 60 };
    open_new_with(
        ui,
        start,
        start + Duration::minutes(minutes),
        task,
        move |d| {
            d.routine = Some(r.id);
            d.title = r.name.clone();
            d.subtasks = r.subtasks.clone();
        },
    );
}

fn open_new_with(
    ui: &UiRc,
    start: NaiveDateTime,
    end: NaiveDateTime,
    kind: Option<Id>,
    fill: impl FnOnce(&mut Draft) + 'static,
) {
    DRAFT_FILL.with(|f| *f.borrow_mut() = Some(Box::new(fill)));
    open_new(ui, start, end, false, kind);
}

/// Fills a new draft before the editor is shown (e.g. from a routine).
type DraftFill = Box<dyn FnOnce(&mut Draft)>;

thread_local! {
    /// Extra values for the next new draft (used by routines).
    static DRAFT_FILL: RefCell<Option<DraftFill>> = const { RefCell::new(None) };
}

/// Adjusts the draft when the kind changes, so every kind gets fields that make sense.
fn switch_kind(d: &mut Draft, from: Option<Builtin>, to: Option<Builtin>) {
    let date = d.start.date();
    let timed = |d: &mut Draft| {
        if d.all_day {
            d.all_day = false;
            d.start = at(date, 10, 0);
            d.end = d.start + Duration::hours(1);
        }
        d.half = None;
    };
    if matches!(from, Some(Builtin::Birthday) | Some(Builtin::Vacation)) && from != to {
        d.recurrence = None;
    }
    match to {
        // Meetings and tasks always have a time.
        Some(Builtin::Meeting) | Some(Builtin::Task) => timed(d),
        Some(Builtin::Vacation) => {
            if from != to {
                d.all_day = true;
                d.half = None;
                d.start = midnight(date);
                d.end = midnight(date + Duration::days(1));
            }
            d.recurrence = None;
        }
        Some(Builtin::Birthday) => {
            d.all_day = true;
            d.half = None;
            d.birth_day = date.day();
            d.birth_month = date.month();
            d.reminder = Some(0);
            d.travel = 0;
            d.buffer = 0;
        }
        _ => {
            if matches!(from, Some(Builtin::Birthday) | Some(Builtin::Vacation)) {
                timed(d);
            }
            d.half = None;
        }
    }
}

pub fn open_existing(ui: &UiRc, id: Id, date: NaiveDate) {
    if ops::refuse_read_only(ui, id) {
        return;
    }
    let Some(entry) = ui.state.borrow().entry(id).cloned() else {
        return;
    };
    let occ = kalender_core::recur::occurrence_on(&entry, date);
    let (start, end) = occ
        .map(|o| (o.start, o.end))
        .unwrap_or((entry.start, entry.end));
    let draft = Draft {
        existing: Some((id, date)),
        kind: entry.kind,
        title: entry.title.clone(),
        start,
        end,
        all_day: entry.all_day,
        half: entry.half,
        location: entry.location.clone(),
        notes: entry.notes.clone(),
        reminder: entry.reminder,
        travel: entry.travel,
        buffer: entry.buffer,
        recurrence: entry.recurrence.clone(),
        subtasks: entry.subtask_texts(),
        participants: entry.participants.clone(),
        routine: None,
        save_routine: false,
        count_sat: entry.count_saturday,
        count_sun: entry.count_sunday,
        birth_day: entry.start.day(),
        birth_month: entry.start.month(),
        birth_year: entry.birth_year,
    };
    present(ui, draft);
}

struct Editor {
    ui: UiRc,
    draft: RefCell<Draft>,
    dialog: adw::Dialog,
    page: adw::ToolbarView,
    save: gtk::Button,
    title_bar: adw::WindowTitle,
    focus_title: Cell<bool>,
}

fn present(ui: &UiRc, mut draft: Draft) {
    if let Some(fill) = DRAFT_FILL.with(|f| f.borrow_mut().take()) {
        fill(&mut draft);
    }
    let dialog = adw::Dialog::builder()
        .content_width(560)
        .content_height(720)
        .build();
    let page = adw::ToolbarView::new();
    let header = adw::HeaderBar::builder()
        .show_end_title_buttons(false)
        .show_start_title_buttons(false)
        .build();
    let cancel = gtk::Button::with_label(&tr("Cancel"));
    let save = gtk::Button::builder()
        .css_classes(["suggested-action"])
        .build();
    header.pack_start(&cancel);
    header.pack_end(&save);
    let title_bar = adw::WindowTitle::new("", "");
    header.set_title_widget(Some(&title_bar));
    page.add_top_bar(&header);
    dialog.set_child(Some(&page));
    let editor = Rc::new(Editor {
        ui: ui.clone(),
        draft: RefCell::new(draft),
        dialog: dialog.clone(),
        page,
        save: save.clone(),
        title_bar,
        focus_title: Cell::new(true),
    });
    let d = dialog.clone();
    cancel.connect_clicked(move |_| {
        d.close();
    });
    let e = editor.clone();
    save.connect_clicked(move |_| e.commit());
    editor.rebuild();
    crate::devtools::remember_dialog(&dialog);
    dialog.present(Some(&ui.window));
}

fn group(title: Option<&str>) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    if let Some(t) = title {
        g.set_title(t);
    }
    g
}

/// A button showing a date; clicking opens a calendar.
fn date_button(date: NaiveDate, on_change: impl Fn(NaiveDate) + 'static) -> gtk::MenuButton {
    let button = gtk::MenuButton::builder()
        .label(fmt::date_short(date))
        .valign(gtk::Align::Center)
        .build();
    let calendar = gtk::Calendar::new();
    if let Ok(d) = glib::DateTime::from_local(
        date.year(),
        date.month() as i32,
        date.day() as i32,
        0,
        0,
        0.0,
    ) {
        calendar.select_day(&d);
    }
    let popover = gtk::Popover::builder().child(&calendar).build();
    button.set_popover(Some(&popover));
    let b = button.clone();
    calendar.connect_day_selected(move |c| {
        let d = c.date();
        if let Some(day) =
            NaiveDate::from_ymd_opt(d.year(), d.month() as u32, d.day_of_month() as u32)
        {
            b.set_label(&fmt::date_short(day));
            on_change(day);
        }
        if let Some(p) = b.popover() {
            p.popdown();
        }
    });
    button
}

/// A compact choice of times in steps of 15 minutes (plus the current time, if it is in between).
fn time_fields(t: NaiveDateTime, on_change: impl Fn(u32, u32) + 'static) -> gtk::DropDown {
    let mut times: Vec<(u32, u32)> = (0..96).map(|i| (i / 4, (i % 4) * 15)).collect();
    let current = (t.hour(), t.minute());
    if !times.contains(&current) {
        times.push(current);
        times.sort_unstable();
    }
    let labels: Vec<String> = times
        .iter()
        .map(|(h, m)| format!("{h:02}:{m:02}"))
        .collect();
    let strs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let drop = gtk::DropDown::from_strings(&strs);
    drop.set_valign(gtk::Align::Center);
    drop.set_enable_search(true);
    drop.set_selected(times.iter().position(|x| *x == current).unwrap_or(0) as u32);
    drop.update_property(&[gtk::accessible::Property::Label(&tr("Time"))]);
    drop.connect_selected_notify(move |d| {
        if let Some((h, m)) = times.get(d.selected() as usize) {
            on_change(*h, *m);
        }
    });
    drop
}

fn combo(
    title: &str,
    items: &[String],
    selected: usize,
    on_change: impl Fn(usize) + 'static,
) -> adw::ComboRow {
    let strs: Vec<&str> = items.iter().map(String::as_str).collect();
    let row = adw::ComboRow::builder()
        .title(title)
        .model(&gtk::StringList::new(&strs))
        .selected(selected as u32)
        .build();
    row.connect_selected_notify(move |r| on_change(r.selected() as usize));
    row
}

fn span_label(m: i64) -> String {
    if m == 0 { tr("None") } else { fmt::minutes(m) }
}

impl Editor {
    fn kind_builtin(&self) -> Option<Builtin> {
        let kind = self.draft.borrow().kind;
        self.ui.state.borrow().kind(kind).and_then(|k| k.builtin)
    }

    /// Builds all fields from the draft; called again when the structure changes.
    fn rebuild(self: &Rc<Self>) {
        let draft = self.draft.borrow().clone();
        let builtin = self.kind_builtin();
        let (task, meeting, vacation, birthday) = (
            builtin == Some(Builtin::Task),
            builtin == Some(Builtin::Meeting),
            builtin == Some(Builtin::Vacation),
            builtin == Some(Builtin::Birthday),
        );
        // Appointments and meetings can have people invited.
        let invites = meeting || builtin == Some(Builtin::Event);
        let editing = draft.existing.is_some();
        let kind_name = self
            .ui
            .state
            .borrow()
            .kind(draft.kind)
            .map(kind_label)
            .unwrap_or_default();

        let heading = if editing {
            trf("Edit {}", &[&kind_name])
        } else {
            match builtin {
                Some(Builtin::Meeting) => tr("New meeting"),
                Some(Builtin::Task) => tr("New task"),
                Some(Builtin::Vacation) => tr("New vacation"),
                Some(Builtin::Birthday) => tr("New birthday"),
                _ if builtin.is_none() => trf("New: {}", &[&kind_name]),
                _ => tr("New appointment"),
            }
        };
        self.title_bar.set_title(&heading);
        self.dialog.set_title(&heading);
        let save_label = if editing {
            tr("Save")
        } else if invites && !draft.participants.is_empty() {
            tr("Add and invite")
        } else {
            tr("Add")
        };
        self.save.set_label(&save_label);

        let content = gtk::Box::new(gtk::Orientation::Vertical, 18);
        content.set_margin_top(12);
        content.set_margin_bottom(24);
        content.set_margin_start(12);
        content.set_margin_end(12);

        // Kind chips
        if !editing {
            // Rows of three equally wide chips, centred, so the block stays symmetric.
            let wrap = gtk::Box::new(gtk::Orientation::Vertical, 6);
            wrap.set_halign(gtk::Align::Center);
            let same_width = gtk::SizeGroup::new(gtk::SizeGroupMode::Horizontal);
            let kinds = self.ui.state.borrow().pickable_kinds();
            let mut line: Option<gtk::Box> = None;
            for (i, k) in kinds.into_iter().enumerate() {
                if i % 3 == 0 {
                    let l = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                    l.set_halign(gtk::Align::Center);
                    wrap.append(&l);
                    line = Some(l);
                }
                let b = gtk::ToggleButton::builder()
                    .active(k.id == draft.kind)
                    .css_classes(["kind-chip"])
                    .build();
                let inner = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                inner.set_halign(gtk::Align::Center);
                inner.append(&IconView::new(
                    &k.icon,
                    Some(if k.id == draft.kind {
                        "#ffffff"
                    } else {
                        &k.color
                    }),
                    16,
                ));
                inner.append(&gtk::Label::new(Some(&kind_label(&k))));
                b.set_child(Some(&inner));
                if k.id == draft.kind {
                    set_chip_color(&b, &k.color);
                }
                let this = self.clone();
                let id = k.id;
                let builtin = k.builtin;
                b.connect_clicked(move |_| {
                    {
                        let from = this.kind_builtin();
                        let mut d = this.draft.borrow_mut();
                        d.kind = id;
                        switch_kind(&mut d, from, builtin);
                    }
                    this.rebuild();
                });
                same_width.add_widget(&b);
                line.as_ref().expect("line").append(&b);
            }
            content.append(&wrap);
        }

        // Routines for new tasks
        if task && !editing {
            let routines = self.ui.state.borrow().routines.clone();
            if !routines.is_empty() {
                let wrap = gtk::FlowBox::builder()
                    .column_spacing(6)
                    .row_spacing(6)
                    .max_children_per_line(8)
                    .selection_mode(gtk::SelectionMode::None)
                    .css_classes(["chips"])
                    .build();
                let label = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                label.append(&IconView::new("routine", None, 16));
                label.append(
                    &gtk::Label::builder()
                        .label(tr("Use routine:"))
                        .css_classes(["dim-label"])
                        .build(),
                );
                wrap.append(&label);
                for r in routines {
                    let b = gtk::ToggleButton::builder()
                        .label(&r.name)
                        .active(draft.routine == Some(r.id))
                        .css_classes(["pill", "small-pill"])
                        .build();
                    let this = self.clone();
                    b.connect_clicked(move |_| {
                        {
                            let mut d = this.draft.borrow_mut();
                            if d.routine == Some(r.id) {
                                d.routine = None;
                            } else {
                                d.routine = Some(r.id);
                                d.title = r.name.clone();
                                d.subtasks = r.subtasks.clone();
                                if !d.all_day && r.minutes > 0 {
                                    d.end = d.start + Duration::minutes(r.minutes);
                                }
                            }
                        }
                        this.rebuild();
                    });
                    wrap.append(&b);
                }
                content.append(&wrap);
            }
        }

        // Title
        let g = group(None);
        let title = adw::EntryRow::builder()
            .title(if birthday { tr("Name") } else { tr("Title") })
            .text(&draft.title)
            .build();
        let this = self.clone();
        title.connect_changed(move |e| this.draft.borrow_mut().title = e.text().to_string());
        let this = self.clone();
        title.connect_entry_activated(move |_| this.commit());
        g.add(&title);
        content.append(&g);
        if self.focus_title.replace(false) {
            let t = title.clone();
            glib::idle_add_local_once(move || {
                t.grab_focus();
            });
        }

        // Time
        let g = group(None);
        if vacation {
            self.vacation_rows(&g, &draft);
        } else if birthday {
            self.birthday_rows(&g, &draft);
        } else {
            // Meetings and tasks always have a time; other kinds may be all-day.
            if !task && !meeting {
                let all_day = adw::SwitchRow::builder()
                    .title(tr("All-day"))
                    .active(draft.all_day)
                    .build();
                let this = self.clone();
                all_day.connect_active_notify(move |s| {
                    {
                        let mut d = this.draft.borrow_mut();
                        d.all_day = s.is_active();
                        if d.all_day {
                            d.start = midnight(d.start.date());
                            d.end = midnight(d.end.date().max(d.start.date()) + Duration::days(1));
                        } else {
                            d.start = at(d.start.date(), 10, 0);
                            d.end = d.start + Duration::hours(1);
                        }
                    }
                    this.rebuild();
                });
                g.add(&all_day);
            }
            if draft.all_day {
                let first = draft.start.date();
                let last = (draft.end.date() - Duration::days(1)).max(first);
                g.add(&self.date_row(&tr("First day"), first, true));
                g.add(&self.date_row(&tr("Last day"), last, false));
            } else {
                g.add(&self.datetime_row(&tr("Start"), true));
                g.add(&self.datetime_row(&tr("End"), false));
            }
            self.add_repeat_rows(&g, &draft);
        }
        content.append(&g);

        // Subtasks
        if task {
            let g = group(Some(&tr("Subtasks")));
            g.set_description(Some(&tr(
                "Can stay empty – subtasks can also be added later in the calendar.",
            )));
            for (i, s) in draft.subtasks.iter().enumerate() {
                let row = adw::ActionRow::builder()
                    .title(glib::markup_escape_text(s))
                    .build();
                row.add_prefix(&gtk::CheckButton::builder().sensitive(false).build());
                let remove = gtk::Button::builder()
                    .icon_name("window-close-symbolic")
                    .css_classes(["flat", "circular"])
                    .valign(gtk::Align::Center)
                    .tooltip_text(tr("Remove subtask"))
                    .build();
                let this = self.clone();
                remove.connect_clicked(move |_| {
                    this.draft.borrow_mut().subtasks.remove(i);
                    this.rebuild();
                });
                row.add_suffix(&remove);
                g.add(&row);
            }
            let add = adw::EntryRow::builder()
                .title(tr("Add subtask (Enter)"))
                .build();
            let this = self.clone();
            add.connect_entry_activated(move |e| {
                let text = e.text().trim().to_owned();
                if !text.is_empty() {
                    this.draft.borrow_mut().subtasks.push(text);
                    this.rebuild_keep_focus("add-subtask");
                }
            });
            add.set_widget_name("add-subtask");
            g.add(&add);
            let label = match draft.routine.and_then(|id| {
                self.ui
                    .state
                    .borrow()
                    .routines
                    .iter()
                    .find(|r| r.id == id)
                    .map(|r| r.name.clone())
            }) {
                Some(name) => trf("Update routine “{}”", &[&name]),
                None => tr("Save as routine"),
            };
            let save_routine = adw::SwitchRow::builder()
                .title(label)
                .subtitle(tr("Remember title, duration and subtasks as a template"))
                .active(draft.save_routine)
                .build();
            let this = self.clone();
            save_routine.connect_active_notify(move |s| {
                this.draft.borrow_mut().save_routine = s.is_active()
            });
            let g2 = group(None);
            g2.add(&save_routine);
            content.append(&g);
            content.append(&g2);
        }

        // Participants
        if invites {
            let g = group(Some(&tr("Participants")));
            g.set_description(Some(&tr(
                "They get an invitation by e-mail and can accept or decline in their own calendar.",
            )));
            for (i, p) in draft.participants.iter().enumerate() {
                let row = adw::ActionRow::builder()
                    .title(glib::markup_escape_text(&p.name))
                    .subtitle(glib::markup_escape_text(&p.email))
                    .build();
                let remove = gtk::Button::builder()
                    .icon_name("window-close-symbolic")
                    .css_classes(["flat", "circular"])
                    .valign(gtk::Align::Center)
                    .tooltip_text(tr("Remove"))
                    .build();
                let this = self.clone();
                remove.connect_clicked(move |_| {
                    this.draft.borrow_mut().participants.remove(i);
                    this.rebuild();
                });
                row.add_suffix(&remove);
                g.add(&row);
            }
            let add = adw::EntryRow::builder()
                .title(tr("Add participant: name and e-mail address"))
                .build();
            add.set_widget_name("add-participant");
            let this = self.clone();
            add.connect_entry_activated(move |e| {
                if let Some(p) = parse_participant(&e.text()) {
                    this.draft.borrow_mut().participants.push(p);
                    this.rebuild_keep_focus("add-participant");
                } else {
                    e.add_css_class("error");
                }
            });
            g.add(&add);
            content.append(&g);
        }

        // Reminder, travel, buffer
        let g = group(None);
        let reminder_labels: Vec<String> = REMINDERS
            .iter()
            .map(|r| match r {
                None => tr("No reminder"),
                Some(0) => tr("At the start"),
                Some(m) => trf("{} before", &[&fmt::minutes(*m)]),
            })
            .collect();
        let selected = REMINDERS
            .iter()
            .position(|r| *r == draft.reminder)
            .unwrap_or(4);
        let this = self.clone();
        let reminder = combo(&tr("Reminder"), &reminder_labels, selected, move |i| {
            this.draft.borrow_mut().reminder = REMINDERS[i]
        });
        if !vacation {
            g.add(&reminder);
        }
        if !vacation && !birthday && !draft.all_day {
            let labels: Vec<String> = SPANS.iter().map(|m| span_label(*m)).collect();
            let this = self.clone();
            let travel = combo(
                &tr("Travel time"),
                &labels,
                SPANS.iter().position(|m| *m == draft.travel).unwrap_or(0),
                move |i| this.draft.borrow_mut().travel = SPANS[i],
            );
            travel.set_subtitle(&tr("Kept free before the entry"));
            let this = self.clone();
            let buffer = combo(
                &tr("Buffer after"),
                &labels,
                SPANS.iter().position(|m| *m == draft.buffer).unwrap_or(0),
                move |i| this.draft.borrow_mut().buffer = SPANS[i],
            );
            buffer.set_subtitle(&tr("Time to breathe before the next entry"));
            if !task {
                g.add(&travel);
            }
            g.add(&buffer);
        }
        if g.first_child().is_some() && !vacation {
            content.append(&g);
        }

        // Place and notes
        if !birthday && !task && !vacation {
            let g = group(None);
            let place_title = if meeting {
                tr("Place or video link")
            } else {
                tr("Place")
            };
            let place = adw::EntryRow::builder()
                .title(place_title)
                .text(&draft.location)
                .build();
            let this = self.clone();
            place.connect_changed(move |e| this.draft.borrow_mut().location = e.text().to_string());
            g.add(&place);
            content.append(&g);
        }
        let notes_group = group(Some(&tr("Notes")));
        let notes = gtk::TextView::builder()
            .wrap_mode(gtk::WrapMode::WordChar)
            .top_margin(10)
            .bottom_margin(10)
            .left_margin(12)
            .right_margin(12)
            .height_request(90)
            .build();
        notes.buffer().set_text(&draft.notes);
        let this = self.clone();
        notes.buffer().connect_changed(move |b| {
            this.draft.borrow_mut().notes =
                b.text(&b.start_iter(), &b.end_iter(), false).to_string();
        });
        let frame = gtk::Frame::builder()
            .child(&notes)
            .css_classes(["view", "card"])
            .build();
        notes_group.add(&frame);
        content.append(&notes_group);

        if editing {
            let delete = gtk::Button::builder()
                .label(trf("Delete {}", &[&kind_name]))
                .css_classes(["destructive-action", "pill"])
                .halign(gtk::Align::Center)
                .build();
            let this = self.clone();
            delete.connect_clicked(move |_| {
                if let Some((id, date)) = this.draft.borrow().existing {
                    this.dialog.close();
                    ops::delete(&this.ui, id, date);
                }
            });
            content.append(&delete);
        }

        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(
                &adw::Clamp::builder()
                    .maximum_size(520)
                    .child(&content)
                    .build(),
            )
            .build();
        let old_scroll = self
            .page
            .content()
            .and_then(|c| c.downcast::<gtk::ScrolledWindow>().ok())
            .map(|s| s.vadjustment().value());
        self.page.set_content(Some(&scroll));
        if let Some(v) = old_scroll {
            let adj = scroll.vadjustment();
            glib::idle_add_local_once(move || adj.set_value(v));
        }
    }

    /// "3 days of vacation · 18 of 30 days left afterwards".
    fn vacation_info(&self, d: &Draft) -> String {
        let st = self.ui.state.borrow();
        let year = d.start.year();
        let from = NaiveDate::from_ymd_opt(year, 1, 1).expect("valid date");
        let to = NaiveDate::from_ymd_opt(year + 1, 1, 1).expect("valid date");
        let holidays = st.holiday_dates(from, to);
        let mut probe = Entry::new(d.kind, "", d.start, d.end);
        probe.all_day = true;
        probe.half = d.half;
        probe.count_saturday = d.count_sat;
        probe.count_sunday = d.count_sun;
        let this_entry = kalender_core::vacation::days_used(&probe, from, to, &holidays);
        let mut parts = vec![if this_entry.fract() == 0.0 {
            trn(
                "{} day of vacation",
                "{} days of vacation",
                this_entry as usize,
            )
        } else {
            trf("Vacation days: {}", &[&fmt::days(this_entry)])
        }];
        let total = st.settings.vacation_per_year;
        if total > 0 {
            let mut used = st.vacation_used(year) + this_entry;
            if let Some((id, _)) = d.existing
                && let Some(old) = st.entry(id)
            {
                used -= kalender_core::vacation::days_used(old, from, to, &holidays);
            }
            parts.push(trf(
                "{} of {} days left afterwards",
                &[&fmt::days(total as f64 - used), &total.to_string()],
            ));
        }
        parts.join(" · ")
    }

    fn vacation_rows(self: &Rc<Self>, g: &adw::PreferencesGroup, draft: &Draft) {
        let half = draft.half.is_some();
        let this = self.clone();
        g.add(&combo(
            &tr("Scope"),
            &[tr("Whole days"), tr("Half day")],
            usize::from(half),
            move |i| {
                {
                    let mut d = this.draft.borrow_mut();
                    let date = d.start.date();
                    d.all_day = true;
                    d.half = (i == 1).then_some(HalfDay::Morning);
                    d.start = midnight(date);
                    if i == 1 {
                        d.end = midnight(date + Duration::days(1));
                    }
                }
                this.rebuild();
            },
        ));
        if half {
            g.add(&self.date_row(&tr("Day"), draft.start.date(), true));
        } else {
            let first = draft.start.date();
            let last = (draft.end.date() - Duration::days(1)).max(first);
            g.add(&self.date_row(&tr("First day of vacation"), first, true));
            g.add(&self.date_row(&tr("Last day of vacation"), last, false));
            // Only ask about weekends when the vacation actually spans one.
            let spans = |wd: chrono::Weekday| {
                last > first
                    && first
                        .iter_days()
                        .take_while(|d| *d <= last)
                        .any(|d| d.weekday() == wd)
            };
            for (label, saturday, active) in [
                (tr("Saturday counts as vacation day"), true, draft.count_sat),
                (tr("Sunday counts as vacation day"), false, draft.count_sun),
            ] {
                if !spans(if saturday {
                    chrono::Weekday::Sat
                } else {
                    chrono::Weekday::Sun
                }) {
                    continue;
                }
                let sw = adw::SwitchRow::builder()
                    .title(label)
                    .active(active)
                    .build();
                let this = self.clone();
                sw.connect_active_notify(move |s| {
                    {
                        let mut d = this.draft.borrow_mut();
                        if saturday {
                            d.count_sat = s.is_active()
                        } else {
                            d.count_sun = s.is_active()
                        }
                    }
                    this.rebuild();
                });
                g.add(&sw);
            }
        }
        let info = adw::ActionRow::builder()
            .title(glib::markup_escape_text(&self.vacation_info(draft)))
            .css_classes(["dim-label"])
            .build();
        g.add(&info);
    }

    fn birthday_rows(self: &Rc<Self>, g: &adw::PreferencesGroup, draft: &Draft) {
        let info = adw::ActionRow::builder().css_classes(["dim-label"]).build();
        let update = {
            let this = self.clone();
            let info = info.clone();
            Rc::new(move || {
                let d = this.draft.borrow().clone();
                let text = match (birthday_date(&d), d.birth_year) {
                    (None, _) => tr("This date does not exist"),
                    (Some(_), None) => tr("Without the year of birth, the age is not shown"),
                    (Some(date), Some(_)) => {
                        let next = next_birthday(date, crate::state::today());
                        trf(
                            "Turns {} on {}",
                            &[
                                &kalender_core::vacation::age(date.year(), next).to_string(),
                                &fmt::date_medium(next),
                            ],
                        )
                    }
                };
                info.set_title(&glib::markup_escape_text(&text));
            })
        };
        let days: Vec<String> = (1..=31).map(|d| d.to_string()).collect();
        let this = self.clone();
        let u = update.clone();
        g.add(&combo(
            &tr("Day"),
            &days,
            draft.birth_day.saturating_sub(1) as usize,
            move |i| {
                this.draft.borrow_mut().birth_day = i as u32 + 1;
                u();
            },
        ));
        let months: Vec<String> = (1..=12).map(fmt::month_name).collect();
        let this = self.clone();
        let u = update.clone();
        g.add(&combo(
            &tr("Month"),
            &months,
            draft.birth_month.saturating_sub(1) as usize,
            move |i| {
                this.draft.borrow_mut().birth_month = i as u32 + 1;
                u();
            },
        ));
        let year = adw::EntryRow::builder()
            .title(tr("Year of birth (optional)"))
            .text(draft.birth_year.map(|y| y.to_string()).unwrap_or_default())
            .input_purpose(gtk::InputPurpose::Digits)
            .build();
        let this = self.clone();
        let u = update.clone();
        year.connect_changed(move |e| {
            let text = e.text();
            let parsed = text
                .trim()
                .parse::<i32>()
                .ok()
                .filter(|y| (1900..=crate::state::today().year()).contains(y));
            if text.trim().is_empty() || parsed.is_some() {
                e.remove_css_class("error")
            } else {
                e.add_css_class("error")
            }
            this.draft.borrow_mut().birth_year = parsed;
            u();
        });
        g.add(&year);
        g.add(&info);
        update();
    }

    fn rebuild_keep_focus(self: &Rc<Self>, name: &str) {
        self.rebuild();
        if let Some(content) = self.page.content() {
            let name = name.to_owned();
            glib::idle_add_local_once(move || {
                if let Some(w) = find_named(&content, &name) {
                    w.grab_focus();
                }
            });
        }
    }

    fn date_row(self: &Rc<Self>, title: &str, date: NaiveDate, first: bool) -> adw::ActionRow {
        let row = adw::ActionRow::builder().title(title).build();
        row.add_prefix(&IconView::new("calendar", None, 16));
        let this = self.clone();
        let button = date_button(date, move |day| {
            {
                let mut d = this.draft.borrow_mut();
                if first {
                    let len = (d.end.date() - d.start.date()).num_days().max(1);
                    d.start = midnight(day);
                    d.end = midnight(day + Duration::days(len));
                } else {
                    d.end = midnight(day.max(d.start.date()) + Duration::days(1));
                }
            }
            this.rebuild();
        });
        row.add_suffix(&button);
        row
    }

    fn datetime_row(self: &Rc<Self>, title: &str, start: bool) -> adw::ActionRow {
        let draft = self.draft.borrow().clone();
        let value = if start { draft.start } else { draft.end };
        let row = adw::ActionRow::builder().title(title).build();
        row.add_prefix(&IconView::new("clock", None, 16));
        let this = self.clone();
        let button = date_button(value.date(), move |day| {
            {
                let mut d = this.draft.borrow_mut();
                if start {
                    let len = d.end - d.start;
                    d.start = day.and_time(d.start.time());
                    d.end = d.start + len;
                } else {
                    d.end = day
                        .and_time(d.end.time())
                        .max(d.start + Duration::minutes(5));
                }
            }
            this.rebuild();
        });
        let this = self.clone();
        let fields = time_fields(value, move |h, m| {
            let mut d = this.draft.borrow_mut();
            if start {
                let len = d.end - d.start;
                d.start = at(d.start.date(), h, m);
                d.end = d.start + len;
            } else {
                let end = at(d.end.date(), h, m);
                d.end = if end > d.start {
                    end
                } else {
                    end + Duration::days(1)
                };
            }
        });
        row.add_suffix(&button);
        row.add_suffix(&fields);
        row
    }

    fn add_repeat_rows(self: &Rc<Self>, g: &adw::PreferencesGroup, draft: &Draft) {
        let choices = [
            tr("Never"),
            tr("Every day"),
            tr("Every week"),
            tr("Every 2 weeks"),
            tr("Every month"),
            tr("Every year"),
        ];
        let current = match &draft.recurrence {
            None => 0,
            Some(r) => match (r.freq, r.interval) {
                (Freq::Daily, _) => 1,
                (Freq::Weekly, 2) => 3,
                (Freq::Weekly, _) => 2,
                (Freq::Monthly, _) => 4,
                (Freq::Yearly, _) => 5,
            },
        };
        let this = self.clone();
        let row = combo(&tr("Repeat"), &choices, current, move |i| {
            {
                let mut d = this.draft.borrow_mut();
                let end = d
                    .recurrence
                    .as_ref()
                    .map(|r| r.end)
                    .unwrap_or(RecurEnd::Never);
                let weekday = kalender_core::model::weekday_index(d.start.date().weekday());
                d.recurrence = match i {
                    0 => None,
                    1 => Some(Recurrence {
                        freq: Freq::Daily,
                        interval: 1,
                        weekdays: vec![],
                        end,
                    }),
                    2 => Some(Recurrence {
                        freq: Freq::Weekly,
                        interval: 1,
                        weekdays: vec![weekday],
                        end,
                    }),
                    3 => Some(Recurrence {
                        freq: Freq::Weekly,
                        interval: 2,
                        weekdays: vec![weekday],
                        end,
                    }),
                    4 => Some(Recurrence {
                        freq: Freq::Monthly,
                        interval: 1,
                        weekdays: vec![],
                        end,
                    }),
                    _ => Some(Recurrence {
                        freq: Freq::Yearly,
                        interval: 1,
                        weekdays: vec![],
                        end,
                    }),
                };
            }
            this.rebuild();
        });
        row.add_prefix(&IconView::new("repeat", None, 16));
        g.add(&row);
        let Some(rule) = &draft.recurrence else {
            return;
        };
        if rule.freq == Freq::Weekly {
            let days_row = adw::ActionRow::builder().title(tr("On")).build();
            let days = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            days.set_valign(gtk::Align::Center);
            for i in 0..7u8 {
                let b = gtk::ToggleButton::builder()
                    .label(fmt::weekday_short(i as u32))
                    .active(rule.weekdays.contains(&i))
                    .css_classes(["circular", "weekday"])
                    .build();
                let this = self.clone();
                b.connect_toggled(move |b| {
                    let mut d = this.draft.borrow_mut();
                    if let Some(r) = d.recurrence.as_mut() {
                        if b.is_active() {
                            if !r.weekdays.contains(&i) {
                                r.weekdays.push(i);
                                r.weekdays.sort_unstable();
                            }
                        } else {
                            r.weekdays.retain(|x| *x != i);
                        }
                    }
                });
                days.append(&b);
            }
            days_row.add_suffix(&days);
            g.add(&days_row);
        }
        let end_choice = match rule.end {
            RecurEnd::Never => 0,
            RecurEnd::Until(_) => 1,
            RecurEnd::Count(_) => 2,
        };
        let this = self.clone();
        let ends = combo(
            &tr("Ends"),
            &[tr("Never"), tr("On a date"), tr("After a number of times")],
            end_choice,
            move |i| {
                {
                    let mut d = this.draft.borrow_mut();
                    let start = d.start.date();
                    if let Some(r) = d.recurrence.as_mut() {
                        r.end = match i {
                            1 => RecurEnd::Until(start + Duration::days(90)),
                            2 => RecurEnd::Count(10),
                            _ => RecurEnd::Never,
                        };
                    }
                }
                this.rebuild();
            },
        );
        g.add(&ends);
        match rule.end {
            RecurEnd::Until(date) => {
                let row = adw::ActionRow::builder().title(tr("Last date")).build();
                let this = self.clone();
                row.add_suffix(&date_button(date, move |day| {
                    if let Some(r) = this.draft.borrow_mut().recurrence.as_mut() {
                        r.end = RecurEnd::Until(day);
                    }
                }));
                g.add(&row);
            }
            RecurEnd::Count(n) => {
                let spin = adw::SpinRow::with_range(1.0, 500.0, 1.0);
                spin.set_title(&tr("Number of times"));
                spin.set_value(n as f64);
                let this = self.clone();
                spin.connect_value_notify(move |s| {
                    if let Some(r) = this.draft.borrow_mut().recurrence.as_mut() {
                        r.end = RecurEnd::Count(s.value() as u32);
                    }
                });
                g.add(&spin);
            }
            RecurEnd::Never => {}
        }
        let summary = adw::ActionRow::builder()
            .title(glib::markup_escape_text(&detail::repeat_text(rule)))
            .css_classes(["dim-label"])
            .build();
        g.add(&summary);
    }

    /// Turns the draft into an entry and saves it.
    fn commit(self: &Rc<Self>) {
        let mut draft = self.draft.borrow().clone();
        if draft.title.trim().is_empty() {
            self.ui.toast(&tr("Please enter a title"));
            return;
        }
        let builtin = self.kind_builtin();
        match builtin {
            Some(Builtin::Vacation) => {
                let first = draft.start.date();
                let last = if draft.half.is_some() {
                    first
                } else {
                    (draft.end.date() - Duration::days(1)).max(first)
                };
                draft.all_day = true;
                draft.start = midnight(first);
                draft.end = midnight(last + Duration::days(1));
                draft.reminder = None;
            }
            Some(Builtin::Birthday) => {
                let Some(date) = birthday_date(&draft) else {
                    self.ui.toast(&tr("This date does not exist"));
                    return;
                };
                draft.all_day = true;
                draft.start = midnight(date);
                draft.end = midnight(date + Duration::days(1));
                draft.recurrence = Some(Recurrence {
                    freq: Freq::Yearly,
                    interval: 1,
                    weekdays: vec![],
                    end: RecurEnd::Never,
                });
            }
            _ => {}
        }
        let existing = draft
            .existing
            .and_then(|(id, _)| self.ui.state.borrow().entry(id).cloned());
        let mut entry = existing
            .clone()
            .unwrap_or_else(|| Entry::new(draft.kind, &draft.title, draft.start, draft.end));
        entry.kind = draft.kind;
        entry.title = draft.title.trim().to_owned();
        entry.all_day = draft.all_day;
        entry.birth_year = if builtin == Some(Builtin::Birthday) {
            draft.birth_year
        } else {
            None
        };
        entry.count_saturday = builtin == Some(Builtin::Vacation) && draft.count_sat;
        entry.count_sunday = builtin == Some(Builtin::Vacation) && draft.count_sun;
        entry.half = if builtin == Some(Builtin::Vacation) {
            draft.half
        } else {
            None
        };
        entry.location = if matches!(
            builtin,
            Some(Builtin::Task) | Some(Builtin::Vacation) | Some(Builtin::Birthday)
        ) {
            String::new()
        } else {
            draft.location.trim().to_owned()
        };
        entry.notes = draft.notes.trim().to_owned();
        entry.reminder = draft.reminder;
        entry.travel = if entry.all_day || builtin == Some(Builtin::Task) {
            0
        } else {
            draft.travel
        };
        entry.buffer = if entry.all_day { 0 } else { draft.buffer };
        entry.recurrence = if builtin == Some(Builtin::Vacation) {
            None
        } else {
            draft.recurrence.clone()
        };
        if let Some(r) = entry.recurrence.as_mut()
            && r.freq == Freq::Weekly
            && r.weekdays.is_empty()
        {
            r.weekdays.push(kalender_core::model::weekday_index(
                draft.start.date().weekday(),
            ));
        }
        entry.participants = draft.participants.clone();
        // Subtasks that stay keep their id and ticks.
        entry.set_subtask_texts(&draft.subtasks);
        if existing.is_some() {
            entry.sequence += 1;
        }

        let ui = self.ui.clone();
        let invite = matches!(builtin, Some(Builtin::Meeting) | Some(Builtin::Event))
            && !entry.participants.is_empty()
            && existing
                .as_ref()
                .is_none_or(|e| e.participants != entry.participants);
        if draft.save_routine && !entry.subtasks.is_empty() {
            let id = draft
                .routine
                .or_else(|| {
                    ui.state
                        .borrow()
                        .routines
                        .iter()
                        .find(|r| r.name == entry.title)
                        .map(|r| r.id)
                })
                .unwrap_or(0);
            let routine = Routine {
                id,
                uid: String::new(),
                name: entry.title.clone(),
                minutes: (draft.end - draft.start).num_minutes(),
                subtasks: entry.subtask_texts(),
            };
            if let Err(e) = ui.state.borrow_mut().save_routine(routine) {
                ui.error(&e);
            }
        }

        match (existing, draft.existing) {
            (Some(old), Some((_, date)))
                if old.recurrence.is_some()
                    && old.recurrence == entry.recurrence
                    && builtin != Some(Builtin::Birthday) =>
            {
                // A changed occurrence of a series.
                self.dialog.close();
                let body = trf("“{}” is part of a series.", &[&old.title]);
                let changed = entry.clone();
                ops::ask_scope(
                    &ui,
                    &tr("Change recurring entry?"),
                    &body,
                    move |u, scope| {
                        save_series_part(u, &old, &changed, date, draft.start, draft.end, scope);
                    },
                );
            }
            (old, _) => {
                // A series keeps its first day; only the time and length change.
                // A birthday is the whole series: its date is the date of birth.
                let keeps_first_day =
                    |o: &&Entry| o.recurrence.is_some() && builtin != Some(Builtin::Birthday);
                let shift_start = match old.as_ref().filter(keeps_first_day) {
                    Some(o) => o.start.date().and_time(draft.start.time()),
                    None => draft.start,
                };
                let draft_end = shift_start + (draft.end - draft.start);
                if !entry.all_day || entry.half.is_some() {
                    let len = draft.end - draft.start;
                    entry.start = if entry.all_day {
                        midnight(shift_start.date())
                    } else {
                        shift_start
                    };
                    entry.end = if entry.all_day {
                        midnight(shift_start.date() + Duration::days(1))
                    } else {
                        shift_start + len
                    };
                } else {
                    entry.start = midnight(shift_start.date());
                    entry.end =
                        midnight(draft_end.date().max(shift_start.date() + Duration::days(1)));
                }
                let result = ui.state.borrow_mut().save_entry(entry);
                match result {
                    Ok(id) => {
                        self.dialog.close();
                        ui.state.borrow_mut().selected = draft.start.date();
                        ui.refresh();
                        ui.toast(&if draft.existing.is_some() {
                            tr("Saved")
                        } else {
                            tr("Added")
                        });
                        if invite {
                            share::invite(&ui, id);
                        }
                    }
                    Err(e) => ui.error(&e),
                }
            }
        }
    }
}

fn save_series_part(
    ui: &UiRc,
    old: &Entry,
    changed: &Entry,
    date: NaiveDate,
    start: NaiveDateTime,
    end: NaiveDateTime,
    scope: Scope,
) {
    let mut series = old.clone();
    let mut part = ops::detached(changed, date, start);
    part.end = end;
    match scope {
        Scope::This => series.exceptions.push(date),
        Scope::Following => {
            series.recurrence.as_mut().expect("series").end =
                RecurEnd::Until(date - Duration::days(1));
            part.recurrence = changed.recurrence.clone();
            part.done.clear();
        }
    }
    let result = (|| -> anyhow::Result<()> {
        let mut st = ui.state.borrow_mut();
        if scope == Scope::Following && date <= old.start.date() {
            st.delete_entry(old.id)?;
        } else {
            st.save_entry(series)?;
        }
        st.save_entry(part)?;
        Ok(())
    })();
    match result {
        Ok(()) => {
            ui.refresh();
            ui.toast(&tr("Saved"));
        }
        Err(e) => ui.error(&e),
    }
}

/// The birthday as a date (in the year of birth, or 2000 if unknown – a leap year, so 29 February works).
fn birthday_date(d: &Draft) -> Option<NaiveDate> {
    NaiveDate::from_ymd_opt(d.birth_year.unwrap_or(2000), d.birth_month, d.birth_day)
}

/// The next birthday from `today` on (29 February falls on 28 February in other years).
fn next_birthday(birth: NaiveDate, today: NaiveDate) -> NaiveDate {
    let on = |year: i32| {
        NaiveDate::from_ymd_opt(year, birth.month(), birth.day())
            .or_else(|| NaiveDate::from_ymd_opt(year, birth.month(), birth.day() - 1))
            .expect("valid date")
    };
    let this_year = on(today.year());
    if this_year >= today {
        this_year
    } else {
        on(today.year() + 1)
    }
}

/// "Lena Koch <lena@example.org>", "lena@example.org" or "Lena Koch, lena@example.org".
fn parse_participant(text: &str) -> Option<Participant> {
    let text = text.trim();
    let (name, email) = if let (Some(a), Some(b)) = (text.find('<'), text.rfind('>')) {
        (
            text[..a].trim().trim_matches(['"', ',']).trim().to_owned(),
            text[a + 1..b].trim().to_owned(),
        )
    } else if let Some(pos) = text.rfind([' ', ',']) {
        let (n, e) = text.split_at(pos);
        (
            n.trim().trim_matches(',').trim().to_owned(),
            e.trim_matches([' ', ',']).to_owned(),
        )
    } else {
        (String::new(), text.to_owned())
    };
    if !email.contains('@') || email.contains(' ') {
        return None;
    }
    let name = if name.is_empty() {
        email.split('@').next().unwrap_or(&email).to_owned()
    } else {
        name
    };
    Some(Participant {
        name,
        email,
        status: Partstat::NeedsAction,
    })
}

fn find_named(root: &gtk::Widget, name: &str) -> Option<gtk::Widget> {
    if root.widget_name() == name {
        return Some(root.clone());
    }
    let mut child = root.first_child();
    while let Some(c) = child {
        if let Some(found) = find_named(&c, name) {
            return Some(found);
        }
        child = c.next_sibling();
    }
    None
}

fn set_chip_color(button: &gtk::ToggleButton, color: &str) {
    let provider = gtk::CssProvider::new();
    let class = format!("chip-{}", color.trim_start_matches('#'));
    provider.load_from_string(&format!(
        "button.kind-chip.{class}:checked {{ background-color: {color}; color: white; }}"
    ));
    gtk::style_context_add_provider_for_display(
        &gtk::gdk::Display::default().expect("display"),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    button.add_css_class(&class);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn participant_forms() {
        let p = parse_participant("Lena Koch <lena@example.org>").unwrap();
        assert_eq!(
            (p.name.as_str(), p.email.as_str()),
            ("Lena Koch", "lena@example.org")
        );
        let p = parse_participant("tom@example.org").unwrap();
        assert_eq!(
            (p.name.as_str(), p.email.as_str()),
            ("tom", "tom@example.org")
        );
        let p = parse_participant("Mia Roth, mia@example.org").unwrap();
        assert_eq!(p.name, "Mia Roth");
        assert!(parse_participant("keine adresse").is_none());
    }
}
