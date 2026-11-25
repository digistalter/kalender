// SPDX-License-Identifier: GPL-3.0-or-later

//! Drawing helpers for the calendar views: colours, rounded boxes, text and
//! the app's own line icons (the same set in every icon theme).

use adw::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib, graphene, gsk, pango};

pub const ACCENT_LIGHT: &str = "#e95420";
pub const ACCENT_DARK: &str = "#f27a4c";

/// Line icons in a 16×16 box, drawn with a 1.6 px stroke.
pub const ICONS: &[(&str, &str)] = &[
    (
        "calendar",
        "M3.5 3h9A1.5 1.5 0 0 1 14 4.5v8a1.5 1.5 0 0 1-1.5 1.5h-9A1.5 1.5 0 0 1 2 12.5v-8A1.5 1.5 0 0 1 3.5 3zM2 6.5h12M5.5 1.5v3M10.5 1.5v3",
    ),
    (
        "people",
        "M8.3 5.5a2.3 2.3 0 1 1-4.6 0a2.3 2.3 0 1 1 4.6 0zM1.8 13.5a4.2 4.2 0 0 1 8.4 0M13.3 6a1.8 1.8 0 1 1-3.6 0a1.8 1.8 0 1 1 3.6 0zM11 9.4a3.4 3.4 0 0 1 3.6 3.6",
    ),
    (
        "task",
        "M4.5 2h7A2.5 2.5 0 0 1 14 4.5v7a2.5 2.5 0 0 1-2.5 2.5h-7A2.5 2.5 0 0 1 2 11.5v-7A2.5 2.5 0 0 1 4.5 2zM5 8l2 2 4-4",
    ),
    (
        "sun",
        "M10.8 8a2.8 2.8 0 1 1-5.6 0a2.8 2.8 0 1 1 5.6 0zM8 1.5v1.6M8 12.9v1.6M1.5 8h1.6M12.9 8h1.6M3.4 3.4l1.1 1.1M11.5 11.5l1.1 1.1M3.4 12.6l1.1-1.1M11.5 4.5l1.1-1.1",
    ),
    (
        "cake",
        "M2.5 14h11V9h-11zM2.5 11.2c1.4.9 2.4-.9 3.7 0c1.2.9 2.4.9 3.6 0c1.3-.9 2.3-.9 3.7 0M8 9V6.8M8 5.6c-.9 0-1.1-1.1 0-2.6c1.1 1.5.9 2.6 0 2.6z",
    ),
    ("flag", "M3.5 14.5V2M3.5 2.5h8.5l-2 3 2 3H3.5"),
    (
        "globe",
        "M8 14.5a6.5 6.5 0 1 0 0-13 6.5 6.5 0 0 0 0 13zM1.5 8h13M8 1.5c-1.8 1.8-2.7 4-2.7 6.5s.9 4.7 2.7 6.5M8 1.5c1.8 1.8 2.7 4 2.7 6.5s-.9 4.7-2.7 6.5",
    ),
    (
        "star",
        "M8 2l1.8 3.8 4.2.5-3.1 2.9.8 4.1L8 11.3l-3.7 2 .8-4.1L2 6.3l4.2-.5z",
    ),
    (
        "pin",
        "M8 14.5c0 0 4.5-4.2 4.5-7.8a4.5 4.5 0 0 0-9 0c0 3.6 4.5 7.8 4.5 7.8zM9.6 6.7a1.6 1.6 0 1 1-3.2 0a1.6 1.6 0 1 1 3.2 0z",
    ),
    (
        "car",
        "M2.5 11V8.2L4 5h8l1.5 3.2V11zM2.5 8.2h11M6.1 11.2a1.1 1.1 0 1 1-2.2 0a1.1 1.1 0 1 1 2.2 0zM12.1 11.2a1.1 1.1 0 1 1-2.2 0a1.1 1.1 0 1 1 2.2 0z",
    ),
    ("note", "M3 3.5h10M3 6.5h10M3 9.5h7M3 12.5h5"),
    ("bell", "M4 11V7a4 4 0 0 1 8 0v4l1 1.5H3zM6.5 14h3"),
    (
        "heart",
        "M8 13.5C8 13.5 2 10 2 6a3 3 0 0 1 6-1a3 3 0 0 1 6 1c0 4-6 7.5-6 7.5z",
    ),
    (
        "repeat",
        "M3 7V6a2 2 0 0 1 2-2h7.5M10.5 2l2 2-2 2M13 9v1a2 2 0 0 1-2 2H3.5M5.5 14l-2-2 2-2",
    ),
    (
        "routine",
        "M3 7V6a2 2 0 0 1 2-2h7.5M10.5 2l2 2-2 2M5.5 10l1.5 1.5 3-3M13 9v1a2 2 0 0 1-2 2",
    ),
    (
        "clock",
        "M14 8a6 6 0 1 1-12 0a6 6 0 1 1 12 0zM8 4.5V8l2.5 1.5",
    ),
    ("check", "M3 8.5l3 3 7-7"),
];

/// Icons a user can give their own kinds.
pub const KIND_ICONS: [&str; 12] = [
    "star", "calendar", "people", "task", "sun", "pin", "car", "note", "bell", "flag", "cake",
    "heart",
];

/// Colours a user can give their own kinds; chosen to stay readable in light and dark mode.
pub const PALETTE: [&str; 12] = [
    "#3b82c4", "#2e9d76", "#7a6ff0", "#e8a33d", "#2a9db0", "#b3952b", "#c2457a", "#d9483b",
    "#5f8f3a", "#8a6a4f", "#e06fb0", "#8a8a85",
];

pub fn rgba(hex: &str) -> gdk::RGBA {
    gdk::RGBA::parse(hex).unwrap_or(gdk::RGBA::new(0.5, 0.5, 0.5, 1.0))
}

pub fn alpha(c: &gdk::RGBA, a: f32) -> gdk::RGBA {
    gdk::RGBA::new(c.red(), c.green(), c.blue(), c.alpha() * a)
}

/// `a` mixed into `b` by `t` (0 = b, 1 = a).
pub fn mix(a: &gdk::RGBA, b: &gdk::RGBA, t: f32) -> gdk::RGBA {
    gdk::RGBA::new(
        a.red() * t + b.red() * (1.0 - t),
        a.green() * t + b.green() * (1.0 - t),
        a.blue() * t + b.blue() * (1.0 - t),
        1.0,
    )
}

pub struct Palette {
    pub ink: gdk::RGBA,
    pub muted: gdk::RGBA,
    pub line: gdk::RGBA,
    pub fill: gdk::RGBA,
    pub paper: gdk::RGBA,
    pub accent: gdk::RGBA,
    pub dark: bool,
}

impl Palette {
    pub fn of(widget: &impl IsA<gtk::Widget>) -> Self {
        let ink = widget.as_ref().color();
        let dark = adw::StyleManager::default().is_dark();
        let paper = if dark {
            rgba("#242424")
        } else {
            rgba("#ffffff")
        };
        Self {
            muted: alpha(&ink, 0.6),
            line: alpha(&ink, 0.11),
            fill: alpha(&ink, 0.045),
            accent: rgba(if dark { ACCENT_DARK } else { ACCENT_LIGHT }),
            paper,
            ink,
            dark,
        }
    }

    /// Soft background of an entry in the colour of its kind.
    pub fn tint(&self, color: &gdk::RGBA) -> gdk::RGBA {
        mix(color, &self.paper, if self.dark { 0.32 } else { 0.18 })
    }

    /// Title colour on a tinted background.
    pub fn deep(&self, color: &gdk::RGBA) -> gdk::RGBA {
        if self.dark {
            mix(color, &self.ink, 0.35)
        } else {
            mix(color, &self.ink, 0.45)
        }
    }
}

pub fn rect(x: f64, y: f64, w: f64, h: f64) -> graphene::Rect {
    graphene::Rect::new(x as f32, y as f32, w.max(0.0) as f32, h.max(0.0) as f32)
}

pub fn fill_round(s: &gtk::Snapshot, x: f64, y: f64, w: f64, h: f64, r: f64, color: &gdk::RGBA) {
    let bounds = rect(x, y, w, h);
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0) as f32;
    s.push_rounded_clip(&gsk::RoundedRect::from_rect(bounds, r));
    s.append_color(color, &bounds);
    s.pop();
}

#[allow(clippy::too_many_arguments)]
pub fn stroke_round(
    s: &gtk::Snapshot,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    r: f64,
    width: f32,
    color: &gdk::RGBA,
) {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0) as f32;
    let rr = gsk::RoundedRect::from_rect(rect(x, y, w, h), r);
    s.append_border(&rr, &[width; 4], &[*color, *color, *color, *color]);
}

pub fn line(s: &gtk::Snapshot, x: f64, y: f64, w: f64, h: f64, color: &gdk::RGBA) {
    s.append_color(color, &rect(x, y, w, h));
}

/// Diagonal stripes, used for travel time and buffers.
pub fn hatch(s: &gtk::Snapshot, x: f64, y: f64, w: f64, h: f64, r: f64, color: &gdk::RGBA) {
    let bounds = rect(x, y, w, h);
    s.push_rounded_clip(&gsk::RoundedRect::from_rect(bounds, r as f32));
    let clear = alpha(color, 0.0);
    s.append_repeating_linear_gradient(
        &bounds,
        &graphene::Point::new(x as f32, y as f32),
        &graphene::Point::new(x as f32 + 6.0, y as f32 + 6.0),
        &[
            gsk::ColorStop::new(0.0, *color),
            gsk::ColorStop::new(0.45, *color),
            gsk::ColorStop::new(0.5, clear),
            gsk::ColorStop::new(1.0, clear),
        ],
    );
    s.pop();
}

pub struct Text<'a> {
    pub text: &'a str,
    pub size: f64,
    pub bold: bool,
    pub color: gdk::RGBA,
    /// Wraps into several lines up to `max_height` instead of one ellipsized line.
    pub wrap: bool,
    pub strike: bool,
}

impl<'a> Text<'a> {
    pub fn new(text: &'a str, size: f64, color: gdk::RGBA) -> Self {
        Self {
            text,
            size,
            bold: false,
            color,
            wrap: false,
            strike: false,
        }
    }

    pub fn bold(mut self) -> Self {
        self.bold = true;
        self
    }

    pub fn wrap(mut self) -> Self {
        self.wrap = true;
        self
    }

    pub fn strike(mut self, on: bool) -> Self {
        self.strike = on;
        self
    }
}

/// Draws text at (x, y) within `width`; returns the height used.
pub fn text(
    widget: &impl IsA<gtk::Widget>,
    s: &gtk::Snapshot,
    t: &Text,
    x: f64,
    y: f64,
    width: f64,
    max_height: f64,
) -> f64 {
    if width < 4.0 || max_height < 4.0 {
        return 0.0;
    }
    let layout = widget.as_ref().create_pango_layout(Some(t.text));
    let mut font = widget
        .as_ref()
        .pango_context()
        .font_description()
        .unwrap_or_default();
    font.set_absolute_size(t.size * pango::SCALE as f64);
    if t.bold {
        font.set_weight(pango::Weight::Semibold);
    }
    layout.set_font_description(Some(&font));
    layout.set_width((width * pango::SCALE as f64) as i32);
    if t.strike {
        let attrs = pango::AttrList::new();
        attrs.insert(pango::AttrInt::new_strikethrough(true));
        layout.set_attributes(Some(&attrs));
    }
    layout.set_ellipsize(pango::EllipsizeMode::End);
    if t.wrap {
        layout.set_wrap(pango::WrapMode::WordChar);
        layout.set_height((max_height * pango::SCALE as f64) as i32);
    } else {
        layout.set_height(0);
    }
    let (_, logical) = layout.pixel_extents();
    let height = (logical.height() as f64).min(max_height.max(logical.height() as f64));
    s.save();
    s.translate(&graphene::Point::new(x as f32, y as f32));
    s.append_layout(&layout, &t.color);
    s.restore();
    height
}

fn icon_path(name: &str) -> Option<gsk::Path> {
    thread_local! {
        static CACHE: std::cell::RefCell<std::collections::HashMap<String, Option<gsk::Path>>> = Default::default();
    }
    CACHE.with(|c| {
        c.borrow_mut()
            .entry(name.to_owned())
            .or_insert_with(|| {
                let data = ICONS.iter().find(|(n, _)| *n == name).map(|(_, d)| *d)?;
                gsk::Path::parse(data).ok()
            })
            .clone()
    })
}

/// Draws one of the app's line icons in a `size`×`size` square.
pub fn icon(s: &gtk::Snapshot, name: &str, x: f64, y: f64, size: f64, color: &gdk::RGBA) {
    let Some(path) = icon_path(name).or_else(|| icon_path("star")) else {
        return;
    };
    let scale = (size / 16.0) as f32;
    s.save();
    s.translate(&graphene::Point::new(x as f32, y as f32));
    s.scale(scale, scale);
    let stroke = gsk::Stroke::new(1.6);
    stroke.set_line_cap(gsk::LineCap::Round);
    stroke.set_line_join(gsk::LineJoin::Round);
    s.append_stroke(&path, &stroke, color);
    s.restore();
}

// A small widget showing one line icon in a fixed colour (or the text colour).
mod imp {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    pub struct IconView {
        pub name: RefCell<String>,
        pub color: RefCell<Option<String>>,
        pub size: std::cell::Cell<i32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for IconView {
        const NAME: &'static str = "KalenderIconView";
        type Type = super::IconView;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for IconView {}

    impl WidgetImpl for IconView {
        fn measure(&self, _o: gtk::Orientation, _for: i32) -> (i32, i32, i32, i32) {
            let size = self.size.get().max(16);
            (size, size, -1, -1)
        }

        fn snapshot(&self, s: &gtk::Snapshot) {
            let widget = self.obj();
            let w = widget.width() as f64;
            let h = widget.height() as f64;
            let size = self.size.get().max(16) as f64;
            let color = match self.color.borrow().as_deref() {
                Some(c) => rgba(c),
                None => widget.color(),
            };
            icon(
                s,
                &self.name.borrow(),
                (w - size) / 2.0,
                (h - size) / 2.0,
                size,
                &color,
            );
        }
    }
}

glib::wrapper! {
    pub struct IconView(ObjectSubclass<imp::IconView>) @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl IconView {
    pub fn new(name: &str, color: Option<&str>, size: i32) -> Self {
        let view: Self = glib::Object::new();
        view.set(name, color);
        view.imp().size.set(size);
        view.set_valign(gtk::Align::Center);
        view.set_halign(gtk::Align::Center);
        view.set_accessible_role(gtk::AccessibleRole::Img);
        view
    }

    pub fn set(&self, name: &str, color: Option<&str>) {
        *self.imp().name.borrow_mut() = name.to_owned();
        *self.imp().color.borrow_mut() = color.map(str::to_owned);
        self.queue_draw();
    }
}
