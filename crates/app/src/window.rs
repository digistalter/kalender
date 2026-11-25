// SPDX-License-Identifier: GPL-3.0-or-later

//! The main window: sidebar with month calendar and kinds, header with
//! navigation, and the week, month or list in the middle.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use chrono::{Datelike, Duration, NaiveDate};
use gtk::{gdk, gio, glib};
use kalender_core::model::{Id, Kind, midnight, week_start};

use crate::draw::IconView;
use crate::hooks::Hooks;
use crate::i18n::{tr, trf, trn};
use crate::month::MonthView;
use crate::state::{Item, Shared, View, kind_label, now, today};
use crate::week::WeekView;
use crate::{detail, editor, fmt, ops, prefs};

pub struct Ui {
    pub window: adw::ApplicationWindow,
    pub state: Shared,
    pub toasts: adw::ToastOverlay,
    pub split: adw::OverlaySplitView,
    pub stack: gtk::Stack,
    pub title: adw::WindowTitle,
    pub calendar: gtk::Calendar,
    pub kinds_list: gtk::ListBox,
    pub vacation_label: gtk::Label,
    pub routines_button: gtk::MenuButton,
    pub week_button: gtk::ToggleButton,
    pub month_button: gtk::ToggleButton,
    pub agenda: gtk::ListBox,
    pub search_bar: gtk::SearchBar,
    pub search_entry: gtk::SearchEntry,
    pub search_list: gtk::ListBox,
    pub week: RefCell<Option<WeekView>>,
    pub month: RefCell<Option<MonthView>>,
    pub narrow: Cell<bool>,
    /// Target of the context menu: entry and occurrence date.
    pub ctx: Cell<Option<(Id, NaiveDate)>>,
    /// What Ctrl+Z undoes.
    pub last_undo: RefCell<Option<Undo>>,
    syncing: Cell<bool>,
}

pub type UiRc = Rc<Ui>;
pub type Undo = Rc<dyn Fn(&UiRc)>;

pub fn build(app: &adw::Application, state: Shared) -> UiRc {
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title(crate::config::APP_NAME)
        .default_width(1180)
        .default_height(780)
        .width_request(360)
        .height_request(420)
        .icon_name(crate::config::APP_ID)
        .build();

    // Sidebar
    let calendar = gtk::Calendar::new();
    calendar.add_css_class("mini-calendar");
    calendar.set_margin_start(6);
    calendar.set_margin_end(6);
    let kinds_list = gtk::ListBox::new();
    kinds_list.set_selection_mode(gtk::SelectionMode::None);
    kinds_list.add_css_class("navigation-sidebar");
    let kinds_head = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    kinds_head.set_margin_start(18);
    kinds_head.set_margin_end(10);
    kinds_head.set_margin_top(12);
    let kinds_label = gtk::Label::builder()
        .label(tr("Kinds"))
        .xalign(0.0)
        .hexpand(true)
        .css_classes(["heading", "dim-label"])
        .build();
    let add_menu = gio::Menu::new();
    add_menu.append(Some(&tr("New kind …")), Some("win.new-kind"));
    add_menu.append(Some(&tr("Subscribe to calendar …")), Some("win.subscribe"));
    let add_kind = gtk::MenuButton::builder()
        .icon_name("list-add-symbolic")
        .tooltip_text(tr("New kind or subscribed calendar"))
        .menu_model(&add_menu)
        .css_classes(["flat", "circular"])
        .build();
    kinds_head.append(&kinds_label);
    kinds_head.append(&add_kind);
    let side_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    side_box.append(&calendar);
    side_box.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    side_box.append(&kinds_head);
    side_box.append(&kinds_list);
    let vacation_label = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .margin_start(18)
        .margin_end(12)
        .margin_top(10)
        .css_classes(["dim-label", "caption"])
        .visible(false)
        .build();
    side_box.append(&vacation_label);
    let side_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&side_box)
        .build();
    let side_header = adw::HeaderBar::builder().show_title(false).build();
    let prefs_button = gtk::Button::builder()
        .icon_name("emblem-system-symbolic")
        .tooltip_text(tr("Settings (Ctrl+,)"))
        .action_name("app.preferences")
        .build();
    side_header.pack_end(&prefs_button);
    let sidebar = adw::ToolbarView::new();
    sidebar.add_top_bar(&side_header);
    sidebar.set_content(Some(&side_scroll));

    // Content
    let title = adw::WindowTitle::new("", "");
    let header = adw::HeaderBar::builder().title_widget(&title).build();
    let sidebar_button = gtk::ToggleButton::builder()
        .icon_name("sidebar-show-symbolic")
        .tooltip_text(tr("Sidebar (F9)"))
        .visible(false)
        .build();
    let new_button = gtk::Button::builder()
        .icon_name("appointment-new-symbolic")
        .tooltip_text(tr("New entry (Ctrl+N)"))
        .action_name("win.new")
        .build();
    let today_button = gtk::Button::builder()
        .label(tr("Today"))
        .action_name("win.today")
        .tooltip_text(tr("Today (T)"))
        .build();
    let prev = gtk::Button::builder()
        .icon_name("go-previous-symbolic")
        .action_name("win.previous")
        .tooltip_text(tr("Back (Page Up)"))
        .build();
    let next = gtk::Button::builder()
        .icon_name("go-next-symbolic")
        .action_name("win.next")
        .tooltip_text(tr("Forward (Page Down)"))
        .build();
    header.pack_start(&sidebar_button);
    header.pack_start(&new_button);
    let routines_button = gtk::MenuButton::builder()
        .child(&IconView::new("routine", None, 16))
        .has_frame(false)
        .tooltip_text(tr("Routines (R)"))
        .build();
    header.pack_start(&routines_button);
    header.pack_start(&today_button);
    header.pack_start(&prev);
    header.pack_start(&next);
    // Week · Month as linked buttons (works with libadwaita 1.5 on Ubuntu 24.04).
    let week_button = gtk::ToggleButton::builder()
        .label(tr("Week"))
        .tooltip_text(tr("Week (1 or W)"))
        .build();
    let month_button = gtk::ToggleButton::builder()
        .label(tr("Month"))
        .tooltip_text(tr("Month (2 or M)"))
        .group(&week_button)
        .build();
    let toggle = gtk::Box::builder().css_classes(["linked"]).build();
    toggle.append(&week_button);
    toggle.append(&month_button);
    let search_button = gtk::ToggleButton::builder()
        .icon_name("system-search-symbolic")
        .tooltip_text(tr("Search (Ctrl+F)"))
        .build();
    header.pack_end(&search_button);
    header.pack_end(&toggle);

    let search_entry = gtk::SearchEntry::builder()
        .placeholder_text(tr("Search titles, places and notes"))
        .hexpand(true)
        .build();
    let search_bar = gtk::SearchBar::builder()
        .child(
            &adw::Clamp::builder()
                .maximum_size(520)
                .child(&search_entry)
                .build(),
        )
        .build();
    search_bar.connect_entry(&search_entry);
    search_button
        .bind_property("active", &search_bar, "search-mode-enabled")
        .bidirectional()
        .build();

    let stack = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .hhomogeneous(false)
        .vhomogeneous(false)
        .vexpand(true)
        .build();
    let agenda = gtk::ListBox::builder()
        .css_classes(["agenda"])
        .selection_mode(gtk::SelectionMode::None)
        .build();
    let agenda_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(
            &adw::Clamp::builder()
                .maximum_size(640)
                .child(&agenda)
                .build(),
        )
        .build();
    let search_list = gtk::ListBox::builder()
        .css_classes(["boxed-list"])
        .selection_mode(gtk::SelectionMode::None)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    let search_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(
            &adw::Clamp::builder()
                .maximum_size(640)
                .child(&search_list)
                .build(),
        )
        .build();
    stack.add_named(&agenda_scroll, Some("agenda"));
    stack.add_named(&search_scroll, Some("search"));

    let content = adw::ToolbarView::new();
    content.add_top_bar(&header);
    content.add_top_bar(&search_bar);
    content.set_content(Some(&stack));

    let split = adw::OverlaySplitView::builder()
        .sidebar(&sidebar)
        .content(&content)
        .sidebar_width_fraction(0.22)
        .min_sidebar_width(250.0)
        .max_sidebar_width(300.0)
        .build();
    sidebar_button
        .bind_property("active", &split, "show-sidebar")
        .bidirectional()
        .build();
    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&split));
    window.set_content(Some(&toasts));

    let ui = Rc::new(Ui {
        window: window.clone(),
        state: state.clone(),
        toasts,
        split: split.clone(),
        stack: stack.clone(),
        title,
        calendar: calendar.clone(),
        kinds_list,
        vacation_label,
        routines_button: routines_button.clone(),
        week_button: week_button.clone(),
        month_button: month_button.clone(),
        agenda,
        search_bar: search_bar.clone(),
        search_entry: search_entry.clone(),
        search_list,
        week: RefCell::new(None),
        month: RefCell::new(None),
        narrow: Cell::new(false),
        ctx: Cell::new(None),
        last_undo: RefCell::new(None),
        syncing: Cell::new(false),
    });

    let hooks = Rc::new(make_hooks(&ui));
    let week = WeekView::new(state.clone(), hooks.clone());
    let month = MonthView::new(state.clone(), hooks);
    stack.add_named(&week.root, Some("week"));
    stack.add_named(&month, Some("month"));
    *ui.week.borrow_mut() = Some(week);
    *ui.month.borrow_mut() = Some(month);

    // Narrow windows: sidebar as overlay, a list of the next days instead of the grid.
    let breakpoint = adw::Breakpoint::new(
        adw::BreakpointCondition::parse("max-width: 720sp").expect("valid condition"),
    );
    breakpoint.add_setter(&split, "collapsed", Some(&true.to_value()));
    breakpoint.add_setter(&sidebar_button, "visible", Some(&true.to_value()));
    breakpoint.add_setter(&toggle, "visible", Some(&false.to_value()));
    breakpoint.add_setter(&prev, "visible", Some(&false.to_value()));
    breakpoint.add_setter(&next, "visible", Some(&false.to_value()));
    let u = ui.clone();
    breakpoint.connect_apply(move |_| {
        u.narrow.set(true);
        u.refresh();
    });
    let u = ui.clone();
    breakpoint.connect_unapply(move |_| {
        u.narrow.set(false);
        u.refresh();
    });
    window.add_breakpoint(breakpoint);

    let u = ui.clone();
    week_button.connect_toggled(move |b| {
        if !u.syncing.get() && b.is_active() {
            u.set_view(View::Week);
        }
    });
    let u = ui.clone();
    month_button.connect_toggled(move |b| {
        if !u.syncing.get() && b.is_active() {
            u.set_view(View::Month);
        }
    });
    let u = ui.clone();
    calendar.connect_day_selected(move |c| {
        if u.syncing.get() {
            return;
        }
        let d = c.date();
        if let Some(day) =
            NaiveDate::from_ymd_opt(d.year(), d.month() as u32, d.day_of_month() as u32)
        {
            u.state.borrow_mut().selected = day;
            if u.state.borrow().view == View::Month && day.month() == u.shown_month() {
                u.refresh();
            } else {
                u.set_view(View::Week);
            }
            if u.split.is_collapsed() {
                u.split.set_show_sidebar(false);
            }
        }
    });
    let weak = Rc::downgrade(&ui);
    routines_button.set_create_popup_func(move |button| {
        if let Some(u) = weak.upgrade() {
            button.set_popover(Some(&routines_popover(&u)));
        }
    });
    let u = ui.clone();
    search_entry.connect_search_changed(move |_| u.update_search());
    let u = ui.clone();
    search_bar.connect_search_mode_enabled_notify(move |_| u.refresh());

    add_actions(&ui);
    add_keys(&ui);
    ui.rebuild_kinds();
    ui.refresh();

    // Keep "now" and the faded past up to date.
    let weak = Rc::downgrade(&ui);
    glib::timeout_add_seconds_local(60, move || match weak.upgrade() {
        Some(u) => {
            u.redraw();
            glib::ControlFlow::Continue
        }
        None => glib::ControlFlow::Break,
    });
    ui
}

fn make_hooks(ui: &UiRc) -> Hooks {
    let weak = Rc::downgrade(ui);
    let open = {
        let weak = weak.clone();
        Box::new(
            move |w: &gtk::Widget, id: Id, date: NaiveDate, rect: gdk::Rectangle| {
                if let Some(u) = weak.upgrade() {
                    detail::show(&u, w, id, date, rect);
                }
            },
        )
    };
    let context = {
        let weak = weak.clone();
        Box::new(
            move |w: &gtk::Widget, id: Id, date: NaiveDate, x: f64, y: f64| {
                if let Some(u) = weak.upgrade() {
                    ops::context_menu(&u, w, id, date, x, y);
                }
            },
        )
    };
    let create = {
        let weak = weak.clone();
        Box::new(move |a, b, all_day| {
            if let Some(u) = weak.upgrade() {
                editor::open_new(&u, a, b, all_day, None);
            }
        })
    };
    let moved = {
        let weak = weak.clone();
        Box::new(move |id, date, start, copy| {
            if let Some(u) = weak.upgrade() {
                ops::move_or_copy(&u, id, date, start, copy);
            }
        })
    };
    let select_day = Box::new(move |day: NaiveDate, show_week: bool| {
        if let Some(u) = weak.upgrade() {
            u.state.borrow_mut().selected = day;
            if show_week {
                u.set_view(View::Week);
            } else {
                u.refresh();
            }
        }
    });
    Hooks {
        open,
        context,
        create,
        moved,
        select_day,
    }
}

fn add_actions(ui: &UiRc) {
    let action = |name: &str, f: Box<dyn Fn(&UiRc)>| {
        let a = gio::SimpleAction::new(name, None);
        let weak = Rc::downgrade(ui);
        a.connect_activate(move |_, _| {
            if let Some(u) = weak.upgrade() {
                f(&u);
            }
        });
        ui.window.add_action(&a);
    };
    action(
        "new",
        Box::new(|u| {
            let base = editor::next_start(u);
            let minutes = u.state.borrow().settings.default_minutes;
            editor::open_new(u, base, base + Duration::minutes(minutes), false, None);
        }),
    );
    action(
        "today",
        Box::new(|u| {
            u.state.borrow_mut().selected = today();
            u.refresh();
            if let Some(w) = u.week.borrow().as_ref() {
                w.scroll_to_morning();
            }
        }),
    );
    action("previous", Box::new(|u| u.step(-1)));
    action("next", Box::new(|u| u.step(1)));
    action("week", Box::new(|u| u.set_view(View::Week)));
    action("month", Box::new(|u| u.set_view(View::Month)));
    action("search", Box::new(|u| u.search_bar.set_search_mode(true)));
    action(
        "sidebar",
        Box::new(|u| u.split.set_show_sidebar(!u.split.shows_sidebar())),
    );
    action("import", Box::new(crate::share::import_file));
    action("new-kind", Box::new(prefs::new_kind));
    action("subscribe", Box::new(crate::subscriptions::add_dialog));
    // Actions on one subscribed calendar, with its kind id.
    let with_id = |name: &str, f: fn(&UiRc, Id)| {
        let a = gio::SimpleAction::new(name, Some(glib::VariantTy::INT64));
        let weak = Rc::downgrade(ui);
        a.connect_activate(move |_, v| {
            if let (Some(u), Some(id)) = (weak.upgrade(), v.and_then(|v| v.get::<i64>())) {
                f(&u, id);
            }
        });
        ui.window.add_action(&a);
    };
    with_id("sub-refresh", |u, id| {
        crate::subscriptions::refresh(u, id, true)
    });
    with_id("sub-edit", prefs::edit_kind);
    with_id("sub-remove", crate::subscriptions::remove);
    action("undo", Box::new(|u| u.undo_last()));
    ops::add_context_actions(ui);

    if let Some(app) = ui.window.application() {
        app.set_accels_for_action("win.new", &["<Control>n"]);
        app.set_accels_for_action("win.search", &["<Control>f"]);
        app.set_accels_for_action("win.sidebar", &["F9"]);
        app.set_accels_for_action("win.undo", &["<Control>z"]);
    }
}

/// Single keys work while no text field has the focus.
fn add_keys(ui: &UiRc) {
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Bubble);
    let weak = Rc::downgrade(ui);
    keys.connect_key_pressed(move |_, key, _, mods| {
        let Some(u) = weak.upgrade() else {
            return glib::Propagation::Proceed;
        };
        if mods.intersects(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::ALT_MASK) {
            return glib::Propagation::Proceed;
        }
        let name = match key {
            gdk::Key::t | gdk::Key::T => "today",
            gdk::Key::w | gdk::Key::W | gdk::Key::_1 => "week",
            gdk::Key::m | gdk::Key::M | gdk::Key::_2 => "month",
            gdk::Key::Page_Up => "previous",
            gdk::Key::Page_Down => "next",
            gdk::Key::slash => "search",
            gdk::Key::r | gdk::Key::R => {
                u.routines_button.popup();
                return glib::Propagation::Stop;
            }
            gdk::Key::question | gdk::Key::F1 => {
                prefs::shortcuts(&u);
                return glib::Propagation::Stop;
            }
            _ => return glib::Propagation::Proceed,
        };
        gio::prelude::ActionGroupExt::activate_action(&u.window, name, None);
        glib::Propagation::Stop
    });
    ui.window.add_controller(keys);
}

impl Ui {
    pub fn shown_month(&self) -> u32 {
        self.state.borrow().selected.month()
    }

    pub fn set_view(self: &Rc<Self>, view: View) {
        self.state.borrow_mut().view = view;
        self.refresh();
    }

    pub fn step(self: &Rc<Self>, dir: i64) {
        {
            let mut st = self.state.borrow_mut();
            st.selected = match st.view {
                View::Week => st.selected + Duration::weeks(dir),
                View::Month => {
                    let first = NaiveDate::from_ymd_opt(st.selected.year(), st.selected.month(), 1)
                        .expect("valid date");
                    if dir > 0 {
                        first.checked_add_months(chrono::Months::new(1))
                    } else {
                        first.checked_sub_months(chrono::Months::new(1))
                    }
                    .unwrap_or(first)
                }
            };
        }
        self.refresh();
    }

    /// Only redraws; nothing in the layout changed.
    pub fn redraw(&self) {
        if let Some(w) = self.week.borrow().as_ref() {
            w.refresh();
        }
        if let Some(m) = self.month.borrow().as_ref() {
            m.refresh();
        }
    }

    /// Brings every part of the window up to date with the state.
    pub fn refresh(self: &Rc<Self>) {
        self.syncing.set(true);
        let (view, selected, week_numbers) = {
            let st = self.state.borrow();
            (st.view, st.selected, st.settings.week_numbers)
        };
        self.week_button.set_active(view == View::Week);
        self.month_button.set_active(view == View::Month);
        let d = glib::DateTime::from_local(
            selected.year(),
            selected.month() as i32,
            selected.day() as i32,
            0,
            0,
            0.0,
        );
        if let Ok(d) = d {
            self.calendar.select_day(&d);
        }
        self.calendar.set_show_week_numbers(week_numbers);
        self.update_vacation();
        self.syncing.set(false);

        let page = if self.search_bar.is_search_mode() && !self.search_entry.text().is_empty() {
            "search"
        } else if self.narrow.get() {
            "agenda"
        } else if view == View::Month {
            "month"
        } else {
            "week"
        };
        self.stack.set_visible_child_name(page);
        match (page, view) {
            ("agenda", _) => {
                self.title.set_title(&fmt::month_year(selected));
                self.title.set_subtitle(&tr("Next days"));
                self.fill_agenda();
            }
            (_, View::Month) => {
                self.title.set_title(&fmt::month_year(selected));
                self.title.set_subtitle("");
            }
            _ => {
                let monday = week_start(selected);
                self.title.set_title(&fmt::week_range(monday));
                let mut sub = trf("Week {}", &[&fmt::iso_week(monday).to_string()]);
                if monday == week_start(today()) {
                    sub = trf("{} · this week", &[&sub]);
                }
                self.title.set_subtitle(&sub);
            }
        }
        self.redraw();
    }

    pub fn toast(&self, text: &str) {
        let toast = adw::Toast::new(text);
        self.toasts.add_toast(toast);
    }

    /// A toast with "Undo" that runs `undo`.
    pub fn toast_undo(self: &Rc<Self>, text: &str, undo: impl Fn(&UiRc) + 'static) {
        let undo: Rc<dyn Fn(&UiRc)> = Rc::new(undo);
        *self.last_undo.borrow_mut() = Some(undo.clone());
        let toast = adw::Toast::builder()
            .title(text)
            .button_label(tr("Undo"))
            .build();
        let weak = Rc::downgrade(self);
        toast.connect_button_clicked(move |_| {
            if let Some(u) = weak.upgrade() {
                u.undo_with(&undo);
            }
        });
        self.toasts.add_toast(toast);
    }

    /// Like `toast_undo`, but the toast offers another action; Ctrl+Z still undoes.
    pub fn toast_action(
        self: &Rc<Self>,
        text: &str,
        button: &str,
        action: impl Fn(&UiRc) + 'static,
        undo: impl Fn(&UiRc) + 'static,
    ) {
        *self.last_undo.borrow_mut() = Some(Rc::new(undo));
        let toast = adw::Toast::builder()
            .title(text)
            .button_label(button)
            .timeout(8)
            .build();
        let weak = Rc::downgrade(self);
        toast.connect_button_clicked(move |_| {
            if let Some(u) = weak.upgrade() {
                action(&u);
            }
        });
        self.toasts.add_toast(toast);
    }

    fn undo_with(self: &Rc<Self>, undo: &Rc<dyn Fn(&UiRc)>) {
        let is_last = self
            .last_undo
            .borrow()
            .as_ref()
            .is_some_and(|l| Rc::ptr_eq(l, undo));
        if is_last {
            self.last_undo.borrow_mut().take();
        }
        undo(self);
        self.refresh();
    }

    pub fn undo_last(self: &Rc<Self>) {
        let last = self.last_undo.borrow_mut().take();
        match last {
            Some(undo) => {
                undo(self);
                self.refresh();
                self.toast(&tr("Undone"));
            }
            None => self.toast(&tr("Nothing to undo")),
        }
    }

    pub fn error(&self, error: &anyhow::Error) {
        tracing::warn!("{error:#}");
        self.toast(&trf("Something went wrong: {}", &[&error.to_string()]));
    }

    /// "Vacation 2026: 12½ of 30 days left" below the kinds, when counted.
    pub fn update_vacation(&self) {
        let st = self.state.borrow();
        let total = st.settings.vacation_per_year;
        self.vacation_label.set_visible(total > 0);
        if total > 0 {
            let year = st.selected.year();
            let left = total as f64 - st.vacation_used(year);
            self.vacation_label.set_label(&trf(
                "Vacation {}: {} of {} days left",
                &[&year.to_string(), &fmt::days(left), &total.to_string()],
            ));
        }
    }

    pub fn rebuild_kinds(self: &Rc<Self>) {
        while let Some(row) = self.kinds_list.first_child() {
            self.kinds_list.remove(&row);
        }
        let (own, subscribed): (Vec<Kind>, Vec<Kind>) = self
            .state
            .borrow()
            .kinds
            .iter()
            .cloned()
            .partition(|k| k.source.is_none());
        let mut heading_done = false;
        for kind in own.into_iter().chain(subscribed) {
            if kind.source.is_some() && !heading_done {
                heading_done = true;
                // Heading above the subscribed calendars.
                let head = gtk::Label::builder()
                    .label(tr("Subscribed"))
                    .xalign(0.0)
                    .margin_start(12)
                    .margin_top(12)
                    .margin_bottom(2)
                    .css_classes(["heading", "dim-label"])
                    .build();
                let head_row = gtk::ListBoxRow::builder()
                    .child(&head)
                    .activatable(false)
                    .selectable(false)
                    .build();
                self.kinds_list.append(&head_row);
            }
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            let check = gtk::CheckButton::builder()
                .active(kind.visible)
                .tooltip_text(tr("Show or hide"))
                .build();
            set_check_color(&check, &kind.color);
            let label = gtk::Label::builder()
                .label(kind_label(&kind))
                .tooltip_text(kind_label(&kind))
                .xalign(0.0)
                .hexpand(true)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .width_chars(1)
                .build();
            row.append(&check);
            row.append(&label);
            if let Some(source) = &kind.source {
                row.append(&subscription_suffix(kind.id, source));
            }
            row.set_margin_start(6);
            row.set_margin_end(6);
            let weak = Rc::downgrade(self);
            let id = kind.id;
            check.connect_toggled(move |c| {
                let Some(u) = weak.upgrade() else { return };
                let kind = u.state.borrow().kind(id).cloned();
                if let Some(mut kind) = kind {
                    kind.visible = c.is_active();
                    let result = u.state.borrow_mut().save_kind(kind);
                    if let Err(e) = result {
                        u.error(&e);
                    }
                    u.refresh();
                }
            });
            let list_row = gtk::ListBoxRow::builder()
                .child(&row)
                .activatable(true)
                .build();
            let c = check.clone();
            let gesture = gtk::GestureClick::new();
            gesture.connect_released(move |_, _, _, _| c.set_active(!c.is_active()));
            label.add_controller(gesture);
            self.kinds_list.append(&list_row);
        }
    }

    fn fill_agenda(self: &Rc<Self>) {
        while let Some(row) = self.agenda.first_child() {
            self.agenda.remove(&row);
        }
        let st = self.state.borrow();
        let start = st.selected.min(today());
        let mut any = false;
        for i in 0..21 {
            let day = start + Duration::days(i);
            let items = st.items_on(day);
            if items.is_empty() {
                continue;
            }
            any = true;
            let mut head = fmt::date_medium(day);
            if day == today() {
                head = trf("Today · {}", &[&head]);
            } else if day == today() + Duration::days(1) {
                head = trf("Tomorrow · {}", &[&head]);
            }
            let label = gtk::Label::builder()
                .label(head)
                .xalign(0.0)
                .css_classes([
                    "heading",
                    if day == today() {
                        "accent"
                    } else {
                        "dim-label"
                    },
                ])
                .margin_top(14)
                .margin_start(12)
                .build();
            self.agenda.append(
                &gtk::ListBoxRow::builder()
                    .child(&label)
                    .activatable(false)
                    .selectable(false)
                    .build(),
            );
            for item in items {
                self.agenda.append(&item_row(self, &item, false));
            }
        }
        if !any {
            let page = adw::StatusPage::builder()
                .title(tr("No entries"))
                .description(tr("Add your first entry with “New entry”."))
                .icon_name("x-office-calendar-symbolic")
                .build();
            self.agenda.append(&page);
        }
    }

    fn update_search(self: &Rc<Self>) {
        while let Some(row) = self.search_list.first_child() {
            self.search_list.remove(&row);
        }
        let query = self.search_entry.text().to_lowercase();
        if query.trim().is_empty() {
            self.refresh();
            return;
        }
        let st = self.state.borrow();
        let mut found: Vec<Item> = Vec::new();
        for entry in &st.entries {
            let hay = format!("{} {} {}", entry.title, entry.location, entry.notes).to_lowercase();
            if !hay.contains(query.trim()) {
                continue;
            }
            let Some(kind) = st.kind(entry.kind).filter(|k| k.visible).cloned() else {
                continue;
            };
            let date = next_date(entry);
            let occ = kalender_core::recur::occurrence_on(entry, date).unwrap_or(
                kalender_core::model::Occurrence {
                    entry: entry.id,
                    date: entry.start.date(),
                    start: entry.start,
                    end: entry.end,
                },
            );
            found.push(Item {
                occ,
                entry: entry.clone(),
                kind,
            });
        }
        found.sort_by_key(|i| i.occ.start);
        drop(st);
        let count = gtk::Label::builder()
            .label(trn("{} result", "{} results", found.len()))
            .xalign(0.0)
            .css_classes(["dim-label"])
            .margin_start(12)
            .margin_top(8)
            .margin_bottom(8)
            .build();
        self.search_list.append(
            &gtk::ListBoxRow::builder()
                .child(&count)
                .activatable(false)
                .build(),
        );
        for item in &found {
            self.search_list.append(&item_row(self, item, true));
        }
        self.stack.set_visible_child_name("search");
    }
}

impl Ui {
    /// Jump to an occurrence and show its week.
    pub fn reveal(self: &Rc<Self>, date: NaiveDate) {
        self.search_bar.set_search_mode(false);
        self.state.borrow_mut().selected = date;
        self.set_view(View::Week);
    }
}

/// The saved routines: one click makes a new task from it.
fn routines_popover(ui: &UiRc) -> gtk::Popover {
    let list = gtk::Box::new(gtk::Orientation::Vertical, 2);
    list.set_margin_top(6);
    list.set_margin_bottom(6);
    list.set_margin_start(6);
    list.set_margin_end(6);
    list.append(
        &gtk::Label::builder()
            .label(tr("New task from routine"))
            .xalign(0.0)
            .css_classes(["heading"])
            .margin_start(8)
            .margin_bottom(4)
            .build(),
    );
    let routines = ui.state.borrow().routines.clone();
    if routines.is_empty() {
        list.append(
            &gtk::Label::builder()
                .label(tr(
                    "No routines yet. Save a task as routine, or add one in the settings.",
                ))
                .wrap(true)
                .max_width_chars(30)
                .xalign(0.0)
                .css_classes(["dim-label"])
                .margin_start(8)
                .margin_end(8)
                .build(),
        );
    }
    let popover = gtk::Popover::builder().child(&list).build();
    for r in routines {
        let inner = gtk::Box::new(gtk::Orientation::Vertical, 0);
        inner.append(&gtk::Label::builder().label(&r.name).xalign(0.0).build());
        inner.append(
            &gtk::Label::builder()
                .label(format!(
                    "{} · {}",
                    trn("{} subtask", "{} subtasks", r.subtasks.len()),
                    fmt::minutes(r.minutes)
                ))
                .xalign(0.0)
                .css_classes(["dim-label", "caption"])
                .build(),
        );
        let b = gtk::Button::builder()
            .child(&inner)
            .css_classes(["flat"])
            .build();
        let weak = Rc::downgrade(ui);
        let p = popover.clone();
        let id = r.id;
        b.connect_clicked(move |_| {
            p.popdown();
            if let Some(u) = weak.upgrade() {
                editor::open_routine(&u, id);
            }
        });
        list.append(&b);
    }
    list.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    let manage = gtk::Button::builder()
        .label(tr("Manage routines …"))
        .css_classes(["flat"])
        .build();
    let weak = Rc::downgrade(ui);
    let p = popover.clone();
    manage.connect_clicked(move |_| {
        p.popdown();
        if let Some(u) = weak.upgrade() {
            prefs::show_routines(&u);
        }
    });
    list.append(&manage);
    popover
}

/// The next occurrence of an entry from today, or its start.
fn next_date(entry: &kalender_core::model::Entry) -> NaiveDate {
    let from = midnight(today());
    kalender_core::recur::occurrences(entry, from, from + Duration::days(400))
        .first()
        .map(|o| o.date)
        .unwrap_or(entry.start.date())
}

/// A row in the list of the next days or the search results.
fn item_row(ui: &UiRc, item: &Item, with_date: bool) -> gtk::ListBoxRow {
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(&crate::state::display_title(
            &item.entry,
            item.occ.date,
        )))
        .activatable(true)
        .build();
    let time = if item.entry.all_day {
        tr("all-day")
    } else {
        format!(
            "{} – {}",
            fmt::time_of(item.occ.start),
            fmt::time_of(item.occ.end)
        )
    };
    let mut sub = if with_date {
        format!("{} · {time}", fmt::date_short(item.occ.date))
    } else {
        time
    };
    sub = format!("{} · {sub}", kind_label(&item.kind));
    if let Some((done, total)) = item.progress() {
        sub.push_str(&format!(" · {done}/{total}"));
    }
    row.set_subtitle(&glib::markup_escape_text(&sub));
    row.add_prefix(&IconView::new(&item.kind.icon, Some(&item.kind.color), 18));
    if ui.state.borrow().faded(item, now()) {
        row.set_opacity(0.5);
    }
    let id = item.entry.id;
    let date = item.occ.date;
    let virtual_item = item.is_virtual();
    let weak = Rc::downgrade(ui);
    row.connect_activated(move |r| {
        if virtual_item {
            return;
        }
        if let Some(u) = weak.upgrade() {
            let rect = gdk::Rectangle::new(0, 0, r.width(), r.height());
            if with_date {
                u.reveal(date);
            } else {
                detail::show(&u, r.upcast_ref(), id, date, rect);
            }
        }
    });
    row.upcast()
}

/// Tints a check button in the colour of its kind.
pub fn set_check_color(check: &gtk::CheckButton, color: &str) {
    let provider = gtk::CssProvider::new();
    let class = format!("kind-{}", color.trim_start_matches('#'));
    provider.load_from_string(&format!(
        "checkbutton.{class} check:checked {{ background-color: {color}; color: white; }}"
    ));
    gtk::style_context_add_provider_for_display(
        &gdk::Display::default().expect("display"),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    check.add_css_class(&class);
}

/// Status and menu next to a subscribed calendar in the sidebar.
fn subscription_suffix(id: Id, source: &kalender_core::model::Source) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    if let Some(error) = &source.error {
        let warn = gtk::Image::builder()
            .icon_name("dialog-warning-symbolic")
            .tooltip_text(trf("Last update failed: {}", &[error]))
            .css_classes(["warning"])
            .build();
        b.append(&warn);
    }
    let menu = gio::Menu::new();
    let item = |label: &str, action: &str| {
        let i = gio::MenuItem::new(Some(label), None);
        i.set_action_and_target_value(Some(action), Some(&id.to_variant()));
        menu.append_item(&i);
    };
    item(&tr("Update now"), "win.sub-refresh");
    item(&tr("Name and colour …"), "win.sub-edit");
    item(&tr("Remove subscription …"), "win.sub-remove");
    let updated = source
        .updated
        .map(|t| {
            trf(
                "Updated {}",
                &[&format!(
                    "{} {}",
                    fmt::date_short(t.date()),
                    fmt::time_of(t)
                )],
            )
        })
        .unwrap_or_else(|| tr("Not updated yet"));
    let button = gtk::MenuButton::builder()
        .icon_name("view-more-symbolic")
        .menu_model(&menu)
        .tooltip_text(updated)
        .valign(gtk::Align::Center)
        .css_classes(["flat", "circular"])
        .build();
    b.append(&button);
    b
}
