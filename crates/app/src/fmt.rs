// SPDX-License-Identifier: GPL-3.0-or-later

//! Dates and times as text, independent of the system's C locale.

use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime, Timelike};

use crate::i18n::{tr, trcf, trf};

pub fn weekday_short(index: u32) -> String {
    match index % 7 {
        0 => tr("Mon"),
        1 => tr("Tue"),
        2 => tr("Wed"),
        3 => tr("Thu"),
        4 => tr("Fri"),
        5 => tr("Sat"),
        _ => tr("Sun"),
    }
}

pub fn weekday_long(index: u32) -> String {
    match index % 7 {
        0 => tr("Monday"),
        1 => tr("Tuesday"),
        2 => tr("Wednesday"),
        3 => tr("Thursday"),
        4 => tr("Friday"),
        5 => tr("Saturday"),
        _ => tr("Sunday"),
    }
}

pub fn month_name(month: u32) -> String {
    match month {
        1 => tr("January"),
        2 => tr("February"),
        3 => tr("March"),
        4 => tr("April"),
        5 => tr("May"),
        6 => tr("June"),
        7 => tr("July"),
        8 => tr("August"),
        9 => tr("September"),
        10 => tr("October"),
        11 => tr("November"),
        _ => tr("December"),
    }
}

pub fn weekday_index(date: NaiveDate) -> u32 {
    date.weekday().num_days_from_monday()
}

pub fn time(t: NaiveTime) -> String {
    format!("{:02}:{:02}", t.hour(), t.minute())
}

pub fn time_of(t: NaiveDateTime) -> String {
    time(t.time())
}

/// "Wed, 25 Nov" / „Mi., 25.11.“
pub fn date_short(d: NaiveDate) -> String {
    trcf(
        "short date: weekday, day, month",
        "{}, {} {}",
        &[
            &weekday_short(weekday_index(d)),
            &d.day().to_string(),
            &month_short(d.month()),
        ],
    )
}

/// "Wednesday, 25 November" without the year.
pub fn date_medium(d: NaiveDate) -> String {
    trcf(
        "date: weekday, day, month",
        "{}, {} {}",
        &[
            &weekday_long(weekday_index(d)),
            &d.day().to_string(),
            &month_name(d.month()),
        ],
    )
}

pub fn month_short(month: u32) -> String {
    match month {
        1 => gettextrs::pgettext("month abbreviation", "Jan"),
        2 => gettextrs::pgettext("month abbreviation", "Feb"),
        3 => gettextrs::pgettext("month abbreviation", "Mar"),
        4 => gettextrs::pgettext("month abbreviation", "Apr"),
        5 => gettextrs::pgettext("month abbreviation", "May"),
        6 => gettextrs::pgettext("month abbreviation", "Jun"),
        7 => gettextrs::pgettext("month abbreviation", "Jul"),
        8 => gettextrs::pgettext("month abbreviation", "Aug"),
        9 => gettextrs::pgettext("month abbreviation", "Sep"),
        10 => gettextrs::pgettext("month abbreviation", "Oct"),
        11 => gettextrs::pgettext("month abbreviation", "Nov"),
        _ => gettextrs::pgettext("month abbreviation", "Dec"),
    }
}

pub fn month_year(d: NaiveDate) -> String {
    format!("{} {}", month_name(d.month()), d.year())
}

/// "23 – 29 November 2026" or "30 November – 6 December 2026"
pub fn week_range(monday: NaiveDate) -> String {
    let sunday = monday + chrono::Duration::days(6);
    if monday.month() == sunday.month() {
        trf(
            "{} – {} {} {}",
            &[
                &monday.day().to_string(),
                &sunday.day().to_string(),
                &month_name(sunday.month()),
                &sunday.year().to_string(),
            ],
        )
    } else {
        trf(
            "{} {} – {} {} {}",
            &[
                &monday.day().to_string(),
                &month_name(monday.month()),
                &sunday.day().to_string(),
                &month_name(sunday.month()),
                &sunday.year().to_string(),
            ],
        )
    }
}

pub fn minutes(m: i64) -> String {
    if m >= 60 && m % 60 == 0 {
        crate::i18n::trn("{} hour", "{} hours", (m / 60) as usize)
    } else if m > 60 {
        trf(
            "{} h {} min",
            &[&(m / 60).to_string(), &(m % 60).to_string()],
        )
    } else {
        trf("{} min", &[&m.to_string()])
    }
}

pub fn iso_week(d: NaiveDate) -> u32 {
    d.iso_week().week()
}

/// 12, 12½ or 12.4 days.
pub fn days(value: f64) -> String {
    let whole = value.trunc();
    let rest = value - whole;
    if rest.abs() < 0.01 {
        format!("{}", whole as i64)
    } else if (rest - 0.5).abs() < 0.01 {
        if whole == 0.0 {
            "½".to_owned()
        } else {
            format!("{}½", whole as i64)
        }
    } else {
        format!("{value:.1}")
    }
}

/// „just now“, „3 min ago“, „2 h ago“, or the date.
pub fn ago(then: chrono::NaiveDateTime, now: chrono::NaiveDateTime) -> String {
    let minutes = (now - then).num_minutes().max(0);
    match minutes {
        0 => crate::i18n::tr("just now"),
        1..=59 => crate::i18n::trf("{} min ago", &[&minutes.to_string()]),
        60..=1439 => crate::i18n::trf("{} h ago", &[&(minutes / 60).to_string()]),
        _ => date_short(then.date()),
    }
}
