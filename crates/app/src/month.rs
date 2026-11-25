// SPDX-License-Identifier: GPL-3.0-or-later

//! The month: one cell per day with its entries; entries can be dragged to another day.

use std::cell::{Cell, OnceCell, RefCell};
use std::rc::Rc;

use chrono::{Datelike, Duration, NaiveDate};
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};
use kalender_core::model::{Id, midnight, week_start};

use crate::draw::{self, Palette, Text};
use crate::fmt;
use crate::hooks::{Hit, Hooks, hit_at};
use crate::i18n::{tr, trn};
use crate::state::{Shared, now};
use crate::week::draw_bar;

const HEAD: f64 = 26.0;
const WEEK_NO: f64 = 30.0;
const LINE: f64 = 19.0;

struct Geometry {
    first: NaiveDate,
    rows: i64,
    left: f64,
    cw: f64,
    rh: f64,
}

fn geometry(state: &Shared, width: f64, height: f64) -> Geometry {
    let st = state.borrow();
    let sel = st.selected;
    let month_first = NaiveDate::from_ymd_opt(sel.year(), sel.month(), 1).expect("valid date");
    let first = week_start(month_first);
    let next_month = month_first
        .checked_add_months(chrono::Months::new(1))
        .expect("valid date");
    let rows = ((next_month - first).num_days() + 6) / 7;
    let left = if st.settings.week_numbers {
        WEEK_NO
    } else {
        0.0
    };
    Geometry {
        first,
        rows,
        left,
        cw: (width - left) / 7.0,
        rh: (height - HEAD) / rows as f64,
    }
}

fn day_at(g: &Geometry, x: f64, y: f64) -> Option<NaiveDate> {
    if x < g.left || y < HEAD {
        return None;
    }
    let col = ((x - g.left) / g.cw).floor() as i64;
    let row = ((y - HEAD) / g.rh).floor() as i64;
    ((0..7).contains(&col) && (0..g.rows).contains(&row))
        .then(|| g.first + Duration::days(row * 7 + col))
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct MonthView {
        pub state: OnceCell<Shared>,
        pub hooks: OnceCell<Rc<Hooks>>,
        pub hits: RefCell<Vec<Hit>>,
        /// Clickable "+ n more" areas: (rect, day).
        pub more: RefCell<Vec<(Hit, NaiveDate)>>,
        pub drag: Cell<Option<(Id, NaiveDate)>>,
        pub origin: Cell<(f64, f64)>,
        pub target: Cell<Option<NaiveDate>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MonthView {
        const NAME: &'static str = "KalenderMonthView";
        type Type = super::MonthView;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for MonthView {}

    impl WidgetImpl for MonthView {
        fn measure(&self, orientation: gtk::Orientation, _for: i32) -> (i32, i32, i32, i32) {
            if orientation == gtk::Orientation::Horizontal {
                (300, 700, -1, -1)
            } else {
                (300, 600, -1, -1)
            }
        }

        fn snapshot(&self, s: &gtk::Snapshot) {
            let Some(state) = self.state.get() else {
                return;
            };
            let widget = self.obj();
            let p = Palette::of(&*widget);
            let (width, height) = (widget.width() as f64, widget.height() as f64);
            let g = geometry(state, width, height);
            let st = state.borrow();
            let today = crate::state::today();
            let now = now();
            let month = st.selected.month();
            let mut hits = Vec::new();
            let mut more = Vec::new();

            for i in 0..7 {
                let x = g.left + i as f64 * g.cw;
                draw::text(
                    &*widget,
                    s,
                    &Text::new(&fmt::weekday_short(i), 12.0, p.muted),
                    x + 8.0,
                    5.0,
                    g.cw - 10.0,
                    18.0,
                );
            }
            if g.left > 0.0 {
                draw::text(
                    &*widget,
                    s,
                    &Text::new(&tr("Wk"), 10.5, p.muted),
                    4.0,
                    6.0,
                    g.left - 6.0,
                    16.0,
                );
            }
            draw::line(s, 0.0, HEAD - 1.0, width, 1.0, &p.line);

            let items = st.items(
                midnight(g.first),
                midnight(g.first + Duration::days(g.rows * 7)),
            );
            for row in 0..g.rows {
                let y = HEAD + row as f64 * g.rh;
                if g.left > 0.0 {
                    let week = fmt::iso_week(g.first + Duration::days(row * 7)).to_string();
                    draw::text(
                        &*widget,
                        s,
                        &Text::new(&week, 10.5, p.muted),
                        8.0,
                        y + 6.0,
                        g.left - 8.0,
                        14.0,
                    );
                }
                for col in 0..7 {
                    let day = g.first + Duration::days(row * 7 + col);
                    let x = g.left + col as f64 * g.cw;
                    let other = day.month() != month;
                    if other {
                        draw::line(s, x, y, g.cw, g.rh, &draw::alpha(&p.ink, 0.075));
                    } else if col >= 5 {
                        draw::line(s, x, y, g.cw, g.rh, &p.fill);
                    }
                    if self.target.get() == Some(day) {
                        draw::fill_round(
                            s,
                            x + 2.0,
                            y + 2.0,
                            g.cw - 4.0,
                            g.rh - 4.0,
                            6.0,
                            &draw::alpha(&p.accent, 0.15),
                        );
                        draw::stroke_round(
                            s,
                            x + 2.0,
                            y + 2.0,
                            g.cw - 4.0,
                            g.rh - 4.0,
                            6.0,
                            2.0,
                            &p.accent,
                        );
                    }
                    draw::line(s, x, y, 1.0, g.rh, &p.line);
                    draw::line(s, x, y + g.rh - 1.0, g.cw, 1.0, &p.line);
                    let num = day.day().to_string();
                    let is_today = day == today;
                    if is_today {
                        draw::fill_round(s, x + 4.0, y + 4.0, 24.0, 24.0, 12.0, &p.accent);
                    } else if day == st.selected {
                        draw::stroke_round(s, x + 4.0, y + 4.0, 24.0, 24.0, 12.0, 2.0, &p.ink);
                    }
                    let color = if is_today {
                        draw::rgba("#ffffff")
                    } else if other || (day < today && st.settings.dim_past) {
                        p.muted
                    } else {
                        p.ink
                    };
                    let tw = if num.len() > 1 { 15.0 } else { 8.0 };
                    let mut t = Text::new(&num, 13.0, color);
                    if is_today {
                        t = t.bold();
                    }
                    draw::text(&*widget, s, &t, x + 16.0 - tw / 2.0, y + 7.0, 22.0, 18.0);

                    let day_items: Vec<_> = items
                        .iter()
                        .filter(|i| {
                            i.occ.start.date() <= day
                                && (i.occ.end > midnight(day) || i.occ.start.date() == day)
                        })
                        .filter(|i| i.entry.all_day || i.occ.start.date() == day)
                        .collect();
                    let room = ((g.rh - 32.0) / LINE).floor().max(0.0) as usize;
                    let shown = if day_items.len() > room {
                        room.saturating_sub(1)
                    } else {
                        day_items.len()
                    };
                    for (n, item) in day_items.iter().take(shown).enumerate() {
                        let ly = y + 30.0 + n as f64 * LINE;
                        let lx = x + 4.0;
                        let lw = g.cw - 8.0;
                        let faded = st.faded(item, now);
                        if faded {
                            s.push_opacity(0.45);
                        }
                        if item.entry.all_day {
                            draw_bar(&*widget, s, &p, item, lx, ly, lw, LINE - 3.0);
                        } else {
                            let color = draw::rgba(&item.kind.color);
                            draw::icon(s, &item.kind.icon, lx + 2.0, ly + 2.0, 12.0, &color);
                            let time = fmt::time_of(item.occ.start);
                            draw::text(
                                &*widget,
                                s,
                                &Text::new(&time, 11.0, p.muted),
                                lx + 18.0,
                                ly + 1.0,
                                36.0,
                                16.0,
                            );
                            let mut title = item.title();
                            if let Some((done, total)) = item.progress() {
                                title.push_str(&format!("  {done}/{total}"));
                            }
                            draw::text(
                                &*widget,
                                s,
                                &Text::new(&title, 12.0, p.ink).strike(item.done()),
                                lx + 54.0,
                                ly,
                                lw - 56.0,
                                17.0,
                            );
                        }
                        if faded {
                            s.pop();
                        }
                        if !item.is_virtual() {
                            hits.push(Hit {
                                x: lx,
                                y: ly,
                                w: lw,
                                h: LINE - 2.0,
                                entry: item.entry.id,
                                date: item.occ.date,
                            });
                        }
                    }
                    let rest = day_items.len() - shown;
                    if rest > 0 {
                        let ly = y + 30.0 + shown as f64 * LINE;
                        let label = trn("+ {} more", "+ {} more", rest);
                        draw::text(
                            &*widget,
                            s,
                            &Text::new(&label, 11.0, p.muted),
                            x + 8.0,
                            ly + 1.0,
                            g.cw - 12.0,
                            16.0,
                        );
                        more.push((
                            Hit {
                                x: x + 4.0,
                                y: ly,
                                w: g.cw - 8.0,
                                h: LINE,
                                entry: 0,
                                date: day,
                            },
                            day,
                        ));
                    }
                }
            }
            *self.hits.borrow_mut() = hits;
            *self.more.borrow_mut() = more;
        }
    }
}

glib::wrapper! {
    pub struct MonthView(ObjectSubclass<imp::MonthView>) @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl MonthView {
    pub fn new(state: Shared, hooks: Rc<Hooks>) -> Self {
        let view: Self = glib::Object::new();
        let imp = view.imp();
        let _ = imp.state.set(state);
        let _ = imp.hooks.set(hooks);
        view.set_hexpand(true);
        view.set_vexpand(true);
        view.update_property(&[gtk::accessible::Property::Label(&tr("Month"))]);

        let click = gtk::GestureClick::new();
        click.set_button(0);
        let this = view.downgrade();
        click.connect_pressed(move |g, n, x, y| {
            let Some(this) = this.upgrade() else { return };
            let imp = this.imp();
            let hooks = imp.hooks.get().expect("hooks");
            let state = imp.state.get().expect("state");
            let hit = hit_at(&imp.hits.borrow(), x, y);
            if g.current_button() == 3 {
                match hit {
                    Some(h) => (hooks.context)(this.upcast_ref(), h.entry, h.date, x, y),
                    // Right click on a free spot of a day: new entry on that day.
                    None => {
                        let geo = geometry(state, this.width() as f64, this.height() as f64);
                        if let Some(day) = day_at(&geo, x, y) {
                            let minutes = state.borrow().settings.default_minutes;
                            let start = midnight(day) + Duration::hours(9);
                            (hooks.create)(start, start + Duration::minutes(minutes), false);
                        }
                    }
                }
                return;
            }
            if let Some((_, day)) = imp.more.borrow().iter().find(|(h, _)| h.contains(x, y)) {
                (hooks.select_day)(*day, true);
                return;
            }
            if hit.is_some() {
                return;
            }
            let geo = geometry(state, this.width() as f64, this.height() as f64);
            if let Some(day) = day_at(&geo, x, y) {
                if n == 2 {
                    let minutes = state.borrow().settings.default_minutes;
                    let start = midnight(day) + Duration::hours(9);
                    (hooks.create)(start, start + Duration::minutes(minutes), false);
                } else {
                    (hooks.select_day)(day, false);
                }
            }
        });
        let this = view.downgrade();
        click.connect_released(move |g, n, x, y| {
            let Some(this) = this.upgrade() else { return };
            if g.current_button() != 1 || n != 1 {
                return;
            }
            if let Some(h) = hit_at(&this.imp().hits.borrow(), x, y) {
                let hooks = this.imp().hooks.get().expect("hooks");
                (hooks.open)(this.upcast_ref(), h.entry, h.date, h.rectangle());
            }
        });
        view.add_controller(click);

        let drag = gtk::GestureDrag::new();
        drag.set_button(1);
        let this = view.downgrade();
        drag.connect_drag_begin(move |_, x, y| {
            let Some(this) = this.upgrade() else { return };
            let imp = this.imp();
            imp.origin.set((x, y));
            let state = imp.state.get().expect("state");
            imp.drag.set(
                hit_at(&imp.hits.borrow(), x, y)
                    .filter(|h| !state.borrow().is_read_only(h.entry))
                    .map(|h| (h.entry, h.date)),
            );
        });
        let this = view.downgrade();
        drag.connect_drag_update(move |g, dx, dy| {
            let Some(this) = this.upgrade() else { return };
            let imp = this.imp();
            if imp.drag.get().is_none()
                || (dx.abs() < 6.0 && dy.abs() < 6.0 && imp.target.get().is_none())
            {
                return;
            }
            g.set_state(gtk::EventSequenceState::Claimed);
            let (ox, oy) = imp.origin.get();
            let geo = geometry(
                imp.state.get().expect("state"),
                this.width() as f64,
                this.height() as f64,
            );
            imp.target.set(day_at(&geo, ox + dx, oy + dy));
            this.queue_draw();
        });
        let this = view.downgrade();
        drag.connect_drag_end(move |g, _, _| {
            let Some(this) = this.upgrade() else { return };
            let imp = this.imp();
            let target = imp.target.take();
            let dragged = imp.drag.take();
            this.queue_draw();
            let (Some(day), Some((entry, date))) = (target, dragged) else {
                return;
            };
            if day == date {
                return;
            }
            let state = imp.state.get().expect("state");
            let start = state.borrow().entry(entry).map(|e| e.start.time());
            if let Some(time) = start {
                let copy = g
                    .current_event_state()
                    .contains(gdk::ModifierType::CONTROL_MASK);
                (imp.hooks.get().expect("hooks").moved)(entry, date, day.and_time(time), copy);
            }
        });
        view.add_controller(drag);
        view
    }

    pub fn refresh(&self) {
        self.queue_draw();
    }
}
