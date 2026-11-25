// SPDX-License-Identifier: GPL-3.0-or-later

//! The week: a header with the days and the all-day row, and a scrollable
//! time grid. Overlapping entries share the width; entries can be dragged.

use std::cell::{Cell, OnceCell, RefCell};
use std::rc::Rc;

use chrono::{Duration, NaiveDate, NaiveDateTime, Timelike};
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};
use kalender_core::layout;
use kalender_core::model::{Id, midnight, week_start};

use crate::draw::{self, Palette, Text};
use crate::fmt;
use crate::hooks::{Hit, Hooks, hit_at};
use crate::i18n::{tr, trf};
use crate::state::{Item, Shared, now};

pub const GUTTER: f64 = 56.0;
pub const HOUR: f64 = 46.0;
const HEAD: f64 = 46.0;
const BAR: f64 = 22.0;

fn monday(state: &Shared) -> NaiveDate {
    week_start(state.borrow().selected)
}

fn col_width(width: f64) -> f64 {
    ((width - GUTTER) / 7.0).max(10.0)
}

fn day_at(state: &Shared, width: f64, x: f64) -> Option<NaiveDate> {
    if x < GUTTER {
        return None;
    }
    let col = ((x - GUTTER) / col_width(width)).floor() as i64;
    (0..7)
        .contains(&col)
        .then(|| monday(state) + Duration::days(col))
}

/// All-day items of the week, each with its first and last column and a row.
fn all_day_rows(items: &[Item], monday: NaiveDate) -> (Vec<(Item, i64, i64, usize)>, usize) {
    let mut rows: Vec<i64> = Vec::new();
    let mut out = Vec::new();
    for item in items.iter().filter(|i| i.entry.all_day) {
        let first = (item.occ.start.date() - monday).num_days().clamp(0, 6);
        let last = ((item.occ.end.date() - monday).num_days() - 1).clamp(first, 6);
        let row = match rows.iter().position(|end| *end < first) {
            Some(r) => r,
            None => {
                rows.push(-1);
                rows.len() - 1
            }
        };
        rows[row] = last;
        out.push((item.clone(), first, last, row));
    }
    let count = rows.len();
    (out, count)
}

mod head_imp {
    use super::*;

    #[derive(Default)]
    pub struct WeekHead {
        pub state: OnceCell<Shared>,
        pub hooks: OnceCell<Rc<Hooks>>,
        pub hits: RefCell<Vec<Hit>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for WeekHead {
        const NAME: &'static str = "KalenderWeekHead";
        type Type = super::WeekHead;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for WeekHead {}

    impl WidgetImpl for WeekHead {
        fn measure(&self, orientation: gtk::Orientation, _for: i32) -> (i32, i32, i32, i32) {
            if orientation == gtk::Orientation::Horizontal {
                return (300, 600, -1, -1);
            }
            let rows = self.state.get().map(|state| {
                let monday = monday(state);
                let items = state
                    .borrow()
                    .items(midnight(monday), midnight(monday + Duration::days(7)));
                all_day_rows(&items, monday).1
            });
            let h = HEAD + rows.unwrap_or(0).max(1) as f64 * BAR + 8.0;
            (h as i32, h as i32, -1, -1)
        }

        fn snapshot(&self, s: &gtk::Snapshot) {
            let Some(state) = self.state.get() else {
                return;
            };
            let widget = self.obj();
            let p = Palette::of(&*widget);
            let width = widget.width() as f64;
            let height = widget.height() as f64;
            let cw = col_width(width);
            let monday = monday(state);
            let st = state.borrow();
            let today = crate::state::today();
            let now = now();
            let mut hits = Vec::new();

            for i in 0..7 {
                let day = monday + Duration::days(i);
                let x = GUTTER + i as f64 * cw;
                if i >= 5 {
                    draw::line(s, x, 0.0, cw, height, &p.fill);
                }
                draw::line(s, x, HEAD, 1.0, height - HEAD, &p.line);
                let label = fmt::weekday_short(i as u32);
                let num = day.format("%-d").to_string();
                let center = x + cw / 2.0;
                let is_today = day == today;
                let selected = day == st.selected && !is_today;
                draw::text(
                    &*widget,
                    s,
                    &Text::new(&label, 12.0, p.muted),
                    center - 34.0,
                    15.0,
                    30.0,
                    20.0,
                );
                let circle = 28.0;
                let cx = center + 2.0;
                if is_today {
                    draw::fill_round(s, cx, 9.0, circle, circle, circle / 2.0, &p.accent);
                } else if selected {
                    draw::stroke_round(s, cx, 9.0, circle, circle, circle / 2.0, 2.0, &p.ink);
                }
                let color = if is_today {
                    draw::rgba("#ffffff")
                } else if day < today && st.settings.dim_past {
                    p.muted
                } else {
                    p.ink
                };
                let mut t = Text::new(&num, 15.0, color);
                if is_today || selected {
                    t = t.bold();
                }
                let tw = if num.len() > 1 { 18.0 } else { 9.0 };
                draw::text(&*widget, s, &t, cx + (circle - tw) / 2.0, 13.0, 24.0, 24.0);
                hits.push(Hit {
                    x,
                    y: 0.0,
                    w: cw,
                    h: HEAD,
                    entry: 0,
                    date: day,
                });
            }
            draw::text(
                &*widget,
                s,
                &Text::new(&tr("all-day"), 9.5, p.muted),
                6.0,
                HEAD + 4.0,
                GUTTER - 10.0,
                16.0,
            );
            draw::line(s, 0.0, height - 1.0, width, 1.0, &p.line);
            draw::line(s, 0.0, HEAD, width, 1.0, &p.line);

            let items = st.items(midnight(monday), midnight(monday + Duration::days(7)));
            let (bars, _) = all_day_rows(&items, monday);
            for (item, first, last, row) in bars {
                let x = GUTTER + first as f64 * cw + 3.0;
                let w = (last - first + 1) as f64 * cw - 6.0;
                let y = HEAD + 4.0 + row as f64 * BAR;
                let faded = st.faded(&item, now);
                if faded {
                    s.push_opacity(0.45);
                }
                draw_bar(&*widget, s, &p, &item, x, y, w, BAR - 3.0);
                if faded {
                    s.pop();
                }
                hits.push(Hit {
                    x,
                    y,
                    w,
                    h: BAR - 3.0,
                    entry: item.entry.id,
                    date: item.occ.date,
                });
            }
            *self.hits.borrow_mut() = hits;
        }
    }
}

glib::wrapper! {
    pub struct WeekHead(ObjectSubclass<head_imp::WeekHead>) @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

/// One all-day bar (also used by the month).
#[allow(clippy::too_many_arguments)]
pub fn draw_bar(
    widget: &impl IsA<gtk::Widget>,
    s: &gtk::Snapshot,
    p: &Palette,
    item: &Item,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
) {
    let color = draw::rgba(&item.kind.color);
    let white = draw::rgba("#ffffff");
    let (fg, label) = match item.entry.half {
        Some(_) => {
            draw::fill_round(s, x, y, w, h, 5.0, &p.tint(&color));
            draw::fill_round(
                s,
                x,
                y,
                w / 2.0,
                h,
                5.0,
                &draw::mix(&color, &p.paper, if p.dark { 0.5 } else { 0.32 }),
            );
            draw::line(s, x, y, 3.0, h, &color);
            (p.ink, format!("½ {}", item.title()))
        }
        None => {
            draw::fill_round(s, x, y, w, h, 5.0, &color);
            (white, item.title())
        }
    };
    let icon_color = if item.entry.half.is_some() {
        color
    } else {
        white
    };
    draw::icon(
        s,
        &item.kind.icon,
        x + 5.0,
        y + (h - 12.0) / 2.0,
        12.0,
        &icon_color,
    );
    draw::text(
        widget,
        s,
        &Text::new(&label, 12.0, fg),
        x + 21.0,
        y + (h - 15.0) / 2.0,
        w - 25.0,
        h,
    );
}

fn create_all_day(head: &WeekHead, x: f64) {
    let state = head.imp().state.get().expect("state");
    if let Some(day) = day_at(state, head.width() as f64, x) {
        let hooks = head.imp().hooks.get().expect("hooks");
        (hooks.create)(midnight(day), midnight(day + Duration::days(1)), true);
    }
}

impl WeekHead {
    fn setup(&self, state: Shared, hooks: Rc<Hooks>) {
        let _ = self.imp().state.set(state.clone());
        let _ = self.imp().hooks.set(hooks.clone());
        let click = gtk::GestureClick::new();
        click.set_button(0);
        let this = self.downgrade();
        click.connect_pressed(move |g, n, x, y| {
            let Some(this) = this.upgrade() else { return };
            let hit = hit_at(&this.imp().hits.borrow(), x, y);
            let hooks = this.imp().hooks.get().expect("hooks");
            match (g.current_button(), hit) {
                (3, Some(h)) if h.entry != 0 => {
                    (hooks.context)(this.upcast_ref(), h.entry, h.date, x, y)
                }
                (1, Some(h)) if h.entry != 0 && n == 1 => {
                    (hooks.open)(this.upcast_ref(), h.entry, h.date, h.rectangle())
                }
                (1, Some(h)) if h.entry == 0 => (hooks.select_day)(h.date, false),
                // Double click or right click on a free spot: new all-day entry there.
                (1, None) if n == 2 => create_all_day(&this, x),
                (3, None) if y > HEAD => create_all_day(&this, x),
                _ => {}
            }
        });
        self.add_controller(click);
        self.set_tooltip_text(None);
    }
}

#[derive(Clone, Copy)]
pub enum Drag {
    Move {
        entry: Id,
        date: NaiveDate,
        grab: f64,
        duration: f64,
    },
    Create {
        start: f64,
    },
}

mod grid_imp {
    use super::*;

    #[derive(Default)]
    pub struct WeekGrid {
        pub state: OnceCell<Shared>,
        pub hooks: OnceCell<Rc<Hooks>>,
        pub hits: RefCell<Vec<Hit>>,
        pub drag: Cell<Option<Drag>>,
        pub origin: Cell<(f64, f64)>,
        /// (day, start hour, end hour, label) of the dashed outline while dragging.
        pub ghost: RefCell<Option<(NaiveDate, f64, f64, String)>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for WeekGrid {
        const NAME: &'static str = "KalenderWeekGrid";
        type Type = super::WeekGrid;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for WeekGrid {}

    impl WidgetImpl for WeekGrid {
        fn measure(&self, orientation: gtk::Orientation, _for: i32) -> (i32, i32, i32, i32) {
            if orientation == gtk::Orientation::Horizontal {
                (300, 600, -1, -1)
            } else {
                let h = (24.0 * HOUR) as i32;
                (h, h, -1, -1)
            }
        }

        fn snapshot(&self, s: &gtk::Snapshot) {
            let Some(state) = self.state.get() else {
                return;
            };
            let widget = self.obj();
            let p = Palette::of(&*widget);
            let width = widget.width() as f64;
            let cw = col_width(width);
            let monday = monday(state);
            let st = state.borrow();
            let today = crate::state::today();
            let now = now();
            let mut hits = Vec::new();

            for i in 0..7 {
                let x = GUTTER + i as f64 * cw;
                if i >= 5 {
                    draw::line(s, x, 0.0, cw, 24.0 * HOUR, &p.fill);
                }
                draw::line(s, x, 0.0, 1.0, 24.0 * HOUR, &p.line);
            }
            for h in 0..24 {
                let y = h as f64 * HOUR;
                draw::line(s, GUTTER - 6.0, y, width - GUTTER + 6.0, 1.0, &p.line);
                if h > 0 {
                    let label = format!("{h:02}:00");
                    draw::text(
                        &*widget,
                        s,
                        &Text::new(&label, 11.0, p.muted),
                        8.0,
                        y - 7.0,
                        GUTTER - 14.0,
                        16.0,
                    );
                }
            }

            for i in 0..7 {
                let day = monday + Duration::days(i);
                let x0 = GUTTER + i as f64 * cw;
                let day_start = midnight(day);
                let items: Vec<Item> = st
                    .items(day_start, day_start + Duration::days(1))
                    .into_iter()
                    .filter(|it| !it.entry.all_day)
                    .collect();
                let hours = |t: NaiveDateTime| {
                    ((t - day_start).num_minutes() as f64 / 60.0).clamp(0.0, 24.0)
                };
                let spans: Vec<(f64, f64)> = items
                    .iter()
                    .map(|it| {
                        let a = hours(it.occ.start);
                        let b = hours(it.occ.end).max(a + 0.25);
                        (
                            a - it.entry.travel as f64 / 60.0,
                            b + it.entry.buffer as f64 / 60.0,
                        )
                    })
                    .collect();
                let lanes = layout::lanes(&spans);
                for (item, lane) in items.iter().zip(lanes) {
                    let lw = (cw - 6.0) / lane.lanes as f64;
                    let x = x0 + 3.0 + lane.lane as f64 * lw;
                    let w = lw - if lane.lanes > 1 { 2.0 } else { 0.0 };
                    let a = hours(item.occ.start);
                    let b = hours(item.occ.end).max(a + 0.25);
                    let faded = st.faded(item, now);
                    if faded {
                        s.push_opacity(0.45);
                    }
                    if item.entry.travel > 0 {
                        let t = item.entry.travel as f64 / 60.0;
                        let y = (a - t).max(0.0) * HOUR;
                        let h = a * HOUR - y - 2.0;
                        draw::hatch(s, x, y + 1.0, w, h, 6.0, &p.line);
                        draw::stroke_round(s, x, y + 1.0, w, h, 6.0, 1.0, &p.line);
                        if h > 14.0 {
                            draw::icon(s, "car", x + 5.0, y + 3.0, 11.0, &p.muted);
                            draw::text(
                                &*widget,
                                s,
                                &Text::new(&fmt::minutes(item.entry.travel), 10.5, p.muted),
                                x + 19.0,
                                y + 2.0,
                                w - 22.0,
                                14.0,
                            );
                        }
                    }
                    if item.entry.buffer > 0 {
                        let y = b * HOUR + 1.0;
                        let h = item.entry.buffer as f64 / 60.0 * HOUR - 2.0;
                        draw::hatch(s, x, y, w, h, 6.0, &p.line);
                        draw::stroke_round(s, x, y, w, h, 6.0, 1.0, &p.line);
                    }
                    let y = a * HOUR + 1.0;
                    let h = (b - a) * HOUR - 2.0;
                    draw_block(&*widget, s, &p, item, x, y, w, h);
                    if faded {
                        s.pop();
                    }
                    if SELECTED.with(|c| c.get()) == item.entry.id {
                        draw::stroke_round(s, x - 1.0, y - 1.0, w + 2.0, h + 2.0, 7.0, 2.0, &p.ink);
                    }
                    if !item.is_virtual() {
                        hits.push(Hit {
                            x,
                            y,
                            w,
                            h,
                            entry: item.entry.id,
                            date: item.occ.date,
                        });
                    }
                }
                if day == today {
                    let y = now.time().num_seconds_from_midnight() as f64 / 3600.0 * HOUR;
                    draw::line(s, x0, y - 1.0, cw, 2.0, &p.accent);
                    draw::fill_round(s, x0 - 5.0, y - 5.0, 10.0, 10.0, 5.0, &p.accent);
                }
            }

            if let Some((day, a, b, label)) = self.ghost.borrow().clone() {
                let i = (day - monday).num_days() as f64;
                let x = GUTTER + i * cw + 3.0;
                let y = a * HOUR + 1.0;
                let h = (b - a) * HOUR - 2.0;
                draw::fill_round(s, x, y, cw - 6.0, h, 6.0, &draw::alpha(&p.paper, 0.7));
                draw::stroke_round(s, x, y, cw - 6.0, h, 6.0, 2.0, &p.ink);
                draw::text(
                    &*widget,
                    s,
                    &Text::new(&label, 11.0, p.ink).wrap(),
                    x + 6.0,
                    y + 4.0,
                    cw - 18.0,
                    h - 6.0,
                );
            }
            *self.hits.borrow_mut() = hits;
        }
    }
}

thread_local! {
    /// The entry whose details are open, outlined in the grid.
    pub static SELECTED: Cell<Id> = const { Cell::new(0) };
}

/// One timed block in the grid.
#[allow(clippy::too_many_arguments)]
fn draw_block(
    widget: &impl IsA<gtk::Widget>,
    s: &gtk::Snapshot,
    p: &Palette,
    item: &Item,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
) {
    let color = draw::rgba(&item.kind.color);
    draw::fill_round(s, x, y, w, h, 6.0, &p.tint(&color));
    draw::fill_round(s, x, y, 3.5, h, 2.0, &color);
    let deep = p.deep(&color);
    let progress = item.progress();
    let short = h < 30.0;
    let icon_y = y + if short { (h - 12.0) / 2.0 } else { 5.0 };
    draw::icon(s, &item.kind.icon, x + 8.0, icon_y, 12.0, &color);
    let label = item.title();
    let title = Text::new(&label, 12.0, deep).bold().strike(item.done());
    let title = if short { title } else { title.wrap() };
    let text_x = x + 24.0;
    let text_w = w - 28.0;
    let used = draw::text(
        widget,
        s,
        &title,
        text_x,
        y + if short { (h - 16.0) / 2.0 } else { 3.0 },
        text_w,
        if short {
            16.0
        } else {
            (h - 20.0).clamp(16.0, 34.0)
        },
    );
    if short {
        return;
    }
    let mut line = format!(
        "{} – {}",
        fmt::time_of(item.occ.start),
        fmt::time_of(item.occ.end)
    );
    if let Some((done, total)) = progress {
        line.push_str(&format!(" · {done}/{total}"));
    }
    if !item.entry.location.is_empty() && w > 160.0 {
        line.push_str(&format!(" · {}", item.entry.location));
    }
    let ty = y + 4.0 + used;
    if ty + 12.0 < y + h {
        draw::text(
            widget,
            s,
            &Text::new(&line, 10.5, p.muted),
            text_x,
            ty,
            text_w,
            14.0,
        );
    }
    if let Some((done, total)) = progress
        && h > 52.0
    {
        let by = ty + 17.0;
        if by + 4.0 < y + h {
            let bw = w - 16.0;
            draw::fill_round(s, x + 8.0, by, bw, 3.0, 1.5, &p.line);
            draw::fill_round(
                s,
                x + 8.0,
                by,
                bw * done as f64 / total.max(1) as f64,
                3.0,
                1.5,
                &color,
            );
        }
    }
}

glib::wrapper! {
    pub struct WeekGrid(ObjectSubclass<grid_imp::WeekGrid>) @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

fn snap(hours: f64, step: f64) -> f64 {
    (hours / step).round() * step
}

impl WeekGrid {
    fn setup(&self, state: Shared, hooks: Rc<Hooks>) {
        let imp = self.imp();
        let _ = imp.state.set(state);
        let _ = imp.hooks.set(hooks);

        let click = gtk::GestureClick::new();
        click.set_button(0);
        let this = self.downgrade();
        click.connect_released(move |g, n, x, y| {
            let Some(this) = this.upgrade() else { return };
            let hit = hit_at(&this.imp().hits.borrow(), x, y);
            let hooks = this.imp().hooks.get().expect("hooks");
            match (g.current_button(), hit) {
                (1, Some(h)) if n == 1 => {
                    (hooks.open)(this.upcast_ref(), h.entry, h.date, h.rectangle())
                }
                (1, None) if n == 2 => {
                    let state = this.imp().state.get().expect("state");
                    if let Some(day) = day_at(state, this.width() as f64, x) {
                        let start = snap(y / HOUR, 0.5).min(23.0);
                        let minutes = state.borrow().settings.default_minutes;
                        let a = midnight(day) + Duration::minutes((start * 60.0) as i64);
                        (hooks.create)(a, a + Duration::minutes(minutes), false);
                    }
                }
                _ => {}
            }
        });
        let this = self.downgrade();
        click.connect_pressed(move |g, _n, x, y| {
            let Some(this) = this.upgrade() else { return };
            if g.current_button() != 3 {
                return;
            }
            let hooks = this.imp().hooks.get().expect("hooks");
            match hit_at(&this.imp().hits.borrow(), x, y) {
                Some(h) => (hooks.context)(this.upcast_ref(), h.entry, h.date, x, y),
                // Right click on a free spot: new entry at that time.
                None => {
                    let state = this.imp().state.get().expect("state");
                    if let Some(day) = day_at(state, this.width() as f64, x) {
                        let start = (((y / HOUR) * 2.0).floor() / 2.0).min(23.0);
                        let minutes = state.borrow().settings.default_minutes;
                        let a = midnight(day) + Duration::minutes((start * 60.0) as i64);
                        (hooks.create)(a, a + Duration::minutes(minutes), false);
                    }
                }
            }
        });
        self.add_controller(click);

        let drag = gtk::GestureDrag::new();
        drag.set_button(1);
        let this = self.downgrade();
        drag.connect_drag_begin(move |_, x, y| {
            let Some(this) = this.upgrade() else { return };
            let imp = this.imp();
            imp.origin.set((x, y));
            let state = imp.state.get().expect("state");
            let width = this.width() as f64;
            let d = match hit_at(&imp.hits.borrow(), x, y) {
                // Entries of subscribed calendars stay where they are.
                Some(h) if state.borrow().is_read_only(h.entry) => None,
                Some(h) => state.borrow().entry(h.entry).map(|e| Drag::Move {
                    entry: h.entry,
                    date: h.date,
                    grab: y / HOUR - (h.y - 1.0) / HOUR,
                    duration: e.duration().num_minutes() as f64 / 60.0,
                }),
                None => day_at(state, width, x).map(|_| Drag::Create {
                    start: snap(y / HOUR, 0.5),
                }),
            };
            imp.drag.set(d);
        });
        let this = self.downgrade();
        drag.connect_drag_update(move |g, dx, dy| {
            let Some(this) = this.upgrade() else { return };
            let imp = this.imp();
            if dx.abs() < 5.0 && dy.abs() < 5.0 && imp.ghost.borrow().is_none() {
                return;
            }
            g.set_state(gtk::EventSequenceState::Claimed);
            let (ox, oy) = imp.origin.get();
            let (x, y) = (ox + dx, oy + dy);
            let state = imp.state.get().expect("state");
            let Some(day) = day_at(
                state,
                this.width() as f64,
                x.clamp(GUTTER + 1.0, this.width() as f64 - 1.0),
            ) else {
                return;
            };
            let copy = g
                .current_event_state()
                .contains(gdk::ModifierType::CONTROL_MASK);
            let ghost = match imp.drag.get() {
                Some(Drag::Move {
                    entry,
                    grab,
                    duration,
                    ..
                }) => {
                    let start = snap(y / HOUR - grab, 0.25).clamp(0.0, 24.0 - duration);
                    let title = state
                        .borrow()
                        .entry(entry)
                        .map(|e| e.title.clone())
                        .unwrap_or_default();
                    let mut label = format!(
                        "{title}\n{} {} – {}",
                        fmt::date_short(day),
                        hm(start),
                        hm(start + duration)
                    );
                    if copy {
                        label.push_str(&format!(" · {}", tr("copy")));
                    }
                    Some((day, start, start + duration, label))
                }
                Some(Drag::Create { start, .. }) => {
                    let end = snap(y / HOUR, 0.25).clamp(start + 0.25, 24.0);
                    Some((day, start, end, format!("{} – {}", hm(start), hm(end))))
                }
                None => None,
            };
            *imp.ghost.borrow_mut() = ghost;
            this.queue_draw();
        });
        let this = self.downgrade();
        drag.connect_drag_end(move |g, _, _| {
            let Some(this) = this.upgrade() else { return };
            let imp = this.imp();
            let ghost = imp.ghost.borrow_mut().take();
            let d = imp.drag.take();
            this.queue_draw();
            let (Some((day, a, b, _)), Some(d)) = (ghost, d) else {
                return;
            };
            let hooks = imp.hooks.get().expect("hooks");
            let at = |h: f64| midnight(day) + Duration::minutes((h * 60.0).round() as i64);
            match d {
                Drag::Move { entry, date, .. } => {
                    let copy = g
                        .current_event_state()
                        .contains(gdk::ModifierType::CONTROL_MASK);
                    (hooks.moved)(entry, date, at(a), copy);
                }
                Drag::Create { .. } => (hooks.create)(at(a), at(b), false),
            }
        });
        self.add_controller(drag);
    }
}

fn hm(hours: f64) -> String {
    let m = (hours * 60.0).round() as i64;
    format!("{:02}:{:02}", m / 60, m % 60)
}

/// The week: header on top, scrollable time grid below.
pub struct WeekView {
    pub root: gtk::Box,
    pub head: WeekHead,
    pub grid: WeekGrid,
    pub scroller: gtk::ScrolledWindow,
}

impl WeekView {
    pub fn new(state: Shared, hooks: Rc<Hooks>) -> Self {
        let head: WeekHead = glib::Object::new();
        head.setup(state.clone(), hooks.clone());
        let grid: WeekGrid = glib::Object::new();
        grid.setup(state, hooks);
        grid.set_hexpand(true);
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&grid)
            .build();
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.append(&head);
        root.append(&scroller);
        head.update_property(&[gtk::accessible::Property::Label(&tr("Days of the week"))]);
        grid.update_property(&[gtk::accessible::Property::Label(&trf(
            "{} – time grid",
            &[&tr("Week")],
        ))]);
        let view = Self {
            root,
            head,
            grid,
            scroller,
        };
        view.scroll_to_morning();
        view
    }

    pub fn refresh(&self) {
        self.head.queue_resize();
        self.head.queue_draw();
        self.grid.queue_draw();
    }

    /// Shows the working day (from 7:00) first.
    pub fn scroll_to_morning(&self) {
        let adj = self.scroller.vadjustment();
        glib::idle_add_local_once(move || adj.set_value(7.0 * HOUR - 6.0));
    }

    pub fn set_selected(id: Id) {
        SELECTED.with(|c| c.set(id));
    }
}
