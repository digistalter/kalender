// SPDX-License-Identifier: GPL-3.0-or-later

//! What the calendar views ask the window to do.

use chrono::{NaiveDate, NaiveDateTime};
use gtk::gdk;
use kalender_core::model::Id;

pub type OpenHook = Box<dyn Fn(&gtk::Widget, Id, NaiveDate, gdk::Rectangle)>;
pub type ContextHook = Box<dyn Fn(&gtk::Widget, Id, NaiveDate, f64, f64)>;

pub struct Hooks {
    /// Show the details of an occurrence, pointing at `rect` inside `widget`.
    pub open: OpenHook,
    /// Right click on an occurrence at (x, y) inside `widget`.
    pub context: ContextHook,
    /// New entry for the given range; `all_day` when made in the all-day row or the month.
    pub create: Box<dyn Fn(NaiveDateTime, NaiveDateTime, bool)>,
    /// An occurrence was dragged: entry, its date, new start, copy instead of move.
    pub moved: Box<dyn Fn(Id, NaiveDate, NaiveDateTime, bool)>,
    /// A day was clicked; `show_week` when the week of that day should open.
    pub select_day: Box<dyn Fn(NaiveDate, bool)>,
}

/// Hit area of an occurrence drawn in a view.
#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub entry: Id,
    pub date: NaiveDate,
}

impl Hit {
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x <= self.x + self.w && y >= self.y && y <= self.y + self.h
    }

    pub fn rectangle(&self) -> gdk::Rectangle {
        gdk::Rectangle::new(
            self.x as i32,
            self.y as i32,
            self.w.max(1.0) as i32,
            self.h.max(1.0) as i32,
        )
    }
}

pub fn hit_at(hits: &[Hit], x: f64, y: f64) -> Option<Hit> {
    // Later items are drawn on top.
    hits.iter().rev().find(|h| h.contains(x, y)).copied()
}
