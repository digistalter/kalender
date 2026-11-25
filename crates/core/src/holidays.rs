// SPDX-License-Identifier: GPL-3.0-or-later

//! Public holidays in Germany, nationwide and per federal state.

use chrono::{Datelike, Duration, NaiveDate};

/// German federal states by their usual abbreviation.
pub const STATES: [(&str, &str); 16] = [
    ("BW", "Baden-Württemberg"),
    ("BY", "Bayern"),
    ("BE", "Berlin"),
    ("BB", "Brandenburg"),
    ("HB", "Bremen"),
    ("HH", "Hamburg"),
    ("HE", "Hessen"),
    ("MV", "Mecklenburg-Vorpommern"),
    ("NI", "Niedersachsen"),
    ("NW", "Nordrhein-Westfalen"),
    ("RP", "Rheinland-Pfalz"),
    ("SL", "Saarland"),
    ("SN", "Sachsen"),
    ("ST", "Sachsen-Anhalt"),
    ("SH", "Schleswig-Holstein"),
    ("TH", "Thüringen"),
];

/// Easter Sunday (Gregorian calendar, anonymous algorithm).
pub fn easter(year: i32) -> NaiveDate {
    let a = year % 19;
    let b = year / 100;
    let c = year % 100;
    let d = b / 4;
    let e = b % 4;
    let f = (b + 8) / 25;
    let g = (b - f + 1) / 3;
    let h = (19 * a + b - d - g + 15) % 30;
    let i = c / 4;
    let k = c % 4;
    let l = (32 + 2 * e + 2 * i - h - k) % 7;
    let m = (a + 11 * h + 22 * l) / 451;
    let month = (h + l - 7 * m + 114) / 31;
    let day = (h + l - 7 * m + 114) % 31 + 1;
    NaiveDate::from_ymd_opt(year, month as u32, day as u32).expect("valid Easter date")
}

/// Holidays of `year` for `state` (empty string: nationwide only), sorted by date.
pub fn holidays(year: i32, state: &str) -> Vec<(NaiveDate, &'static str)> {
    let date = |m: u32, d: u32| NaiveDate::from_ymd_opt(year, m, d).expect("valid date");
    let easter = easter(year);
    let in_state = |states: &[&str]| states.contains(&state);
    let mut out = vec![
        (date(1, 1), "Neujahr"),
        (easter - Duration::days(2), "Karfreitag"),
        (easter + Duration::days(1), "Ostermontag"),
        (date(5, 1), "Tag der Arbeit"),
        (easter + Duration::days(39), "Christi Himmelfahrt"),
        (easter + Duration::days(50), "Pfingstmontag"),
        (date(10, 3), "Tag der Deutschen Einheit"),
        (date(12, 25), "1. Weihnachtstag"),
        (date(12, 26), "2. Weihnachtstag"),
    ];
    if in_state(&["BW", "BY", "ST"]) {
        out.push((date(1, 6), "Heilige Drei Könige"));
    }
    if in_state(&["BE", "MV"]) {
        out.push((date(3, 8), "Internationaler Frauentag"));
    }
    if in_state(&["BB"]) {
        out.push((easter, "Ostersonntag"));
        out.push((easter + Duration::days(49), "Pfingstsonntag"));
    }
    if in_state(&["BW", "BY", "HE", "NW", "RP", "SL"]) {
        out.push((easter + Duration::days(60), "Fronleichnam"));
    }
    if in_state(&["SL"]) {
        out.push((date(8, 15), "Mariä Himmelfahrt"));
    }
    if in_state(&["TH"]) {
        out.push((date(9, 20), "Weltkindertag"));
    }
    if in_state(&["BB", "HB", "HH", "MV", "NI", "SN", "ST", "SH", "TH"]) {
        out.push((date(10, 31), "Reformationstag"));
    }
    if in_state(&["BW", "BY", "NW", "RP", "SL"]) {
        out.push((date(11, 1), "Allerheiligen"));
    }
    if in_state(&["SN"]) {
        // Wednesday before 23 November.
        let mut day = date(11, 22);
        while day.weekday() != chrono::Weekday::Wed {
            day -= Duration::days(1);
        }
        out.push((day, "Buß- und Bettag"));
    }
    out.sort();
    out
}

/// Holidays between `from` (inclusive) and `to` (exclusive).
pub fn holidays_between(
    from: NaiveDate,
    to: NaiveDate,
    state: &str,
) -> Vec<(NaiveDate, &'static str)> {
    (from.year()..=to.year())
        .flat_map(|year| holidays(year, state))
        .filter(|(day, _)| *day >= from && *day < to)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn easter_dates() {
        assert_eq!(easter(2024), d(2024, 3, 31));
        assert_eq!(easter(2026), d(2026, 4, 5));
        assert_eq!(easter(2027), d(2027, 3, 28));
    }

    #[test]
    fn nrw_has_fronleichnam_and_allerheiligen() {
        let names: Vec<_> = holidays(2026, "NW").iter().map(|(_, n)| *n).collect();
        assert!(names.contains(&"Fronleichnam"));
        assert!(names.contains(&"Allerheiligen"));
        assert!(!names.contains(&"Reformationstag"));
        assert_eq!(names.len(), 11);
    }

    #[test]
    fn saxony_buss_und_bettag() {
        let days = holidays(2026, "SN");
        assert!(days.contains(&(d(2026, 11, 18), "Buß- und Bettag")));
    }

    #[test]
    fn range_crosses_new_year() {
        let days = holidays_between(d(2026, 12, 20), d(2027, 1, 7), "BY");
        let names: Vec<_> = days.iter().map(|(_, n)| *n).collect();
        assert_eq!(
            names,
            vec![
                "1. Weihnachtstag",
                "2. Weihnachtstag",
                "Neujahr",
                "Heilige Drei Könige"
            ]
        );
    }
}
