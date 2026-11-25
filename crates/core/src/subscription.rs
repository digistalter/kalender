// SPDX-License-Identifier: GPL-3.0-or-later

//! Subscribed calendars: a link to a calendar file (.ics) somewhere on the web,
//! e.g. waste collection, school holidays or a work calendar. Its events become
//! read-only entries of one kind.

use std::collections::HashMap;

use crate::ics::Calendar;
use crate::model::{Entry, Id};

/// Turns what people copy into a link calendar apps can read:
/// `webcal://` → `https://`, Dropbox preview links → direct download,
/// Nextcloud/ownCloud share links → `/download`.
pub fn normalize_url(input: &str) -> String {
    let mut url = input.trim().to_owned();
    for scheme in ["webcals://", "webcal://"] {
        if let Some(rest) = url.strip_prefix(scheme) {
            url = format!("https://{rest}");
        }
    }
    if !url.contains("://") && !url.is_empty() {
        url = format!("https://{url}");
    }
    if url.contains("dropbox.com/") {
        if url.contains("dl=0") {
            url = url.replace("dl=0", "dl=1");
        } else if !url.contains("dl=1") && !url.contains("raw=1") {
            url.push(if url.contains('?') { '&' } else { '?' });
            url.push_str("dl=1");
        }
    } else if let Some(pos) = url.find("/s/") {
        // Nextcloud/ownCloud public share: https://host/s/TOKEN or /index.php/s/TOKEN
        let token = &url[pos + 3..];
        if !token.is_empty() && !token.contains('/') && !token.contains('?') {
            url.push_str("/download");
        }
    }
    url
}

/// The entries of a subscribed calendar, all of kind `kind`.
/// Changed days of a series replace that day; cancelled events are left out.
pub fn entries(calendar: &Calendar, kind: Id) -> Vec<Entry> {
    // Days replaced or cancelled per series (by UID).
    let mut replaced: HashMap<&str, Vec<chrono::NaiveDate>> = HashMap::new();
    for ev in &calendar.events {
        if let Some(day) = ev.recurrence_id {
            replaced.entry(ev.uid.as_str()).or_default().push(day);
        }
    }
    let mut out = Vec::new();
    for (i, ev) in calendar.events.iter().enumerate() {
        if ev.cancelled {
            continue;
        }
        let mut e = Entry::new(kind, &ev.summary, ev.start, ev.end.max(ev.start));
        e.uid = if ev.uid.is_empty() {
            format!("subscribed-{kind}-{i}")
        } else if ev.recurrence_id.is_some() {
            format!("{}#{}", ev.uid, ev.start.format("%Y%m%d"))
        } else {
            ev.uid.clone()
        };
        e.all_day = ev.all_day;
        e.location = ev.location.clone();
        e.notes = ev.description.clone();
        e.reminder = None;
        if ev.recurrence_id.is_none() {
            e.recurrence = ev.recurrence.clone();
            e.exceptions = ev.exdates.clone();
            if let Some(days) = replaced.get(ev.uid.as_str()) {
                e.exceptions.extend(days);
            }
        }
        out.push(e);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ics::parse;
    use chrono::NaiveDate;

    #[test]
    fn links_become_direct() {
        assert_eq!(
            normalize_url("webcal://example.org/muell.ics"),
            "https://example.org/muell.ics"
        );
        assert_eq!(
            normalize_url("https://www.dropbox.com/scl/fi/abc/k.ics?rlkey=x&dl=0"),
            "https://www.dropbox.com/scl/fi/abc/k.ics?rlkey=x&dl=1"
        );
        assert_eq!(
            normalize_url("https://cloud.example.org/s/AbC123"),
            "https://cloud.example.org/s/AbC123/download"
        );
        assert_eq!(
            normalize_url(" calendar.google.com/calendar/ical/x/basic.ics "),
            "https://calendar.google.com/calendar/ical/x/basic.ics"
        );
    }

    #[test]
    fn series_with_moved_and_cancelled_days() {
        let text = "BEGIN:VCALENDAR\r\nX-WR-CALNAME:Verein\r\n\
BEGIN:VEVENT\r\nUID:a\r\nDTSTART;TZID=Europe/Berlin:20261102T180000\r\nDTEND;TZID=Europe/Berlin:20261102T193000\r\nRRULE:FREQ=WEEKLY;BYDAY=MO\r\nEXDATE;TZID=Europe/Berlin:20261109T180000\r\nSUMMARY:Training\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:a\r\nRECURRENCE-ID;TZID=Europe/Berlin:20261116T180000\r\nDTSTART;TZID=Europe/Berlin:20261117T180000\r\nDTEND;TZID=Europe/Berlin:20261117T193000\r\nSUMMARY:Training (Di)\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:b\r\nDTSTART;VALUE=DATE:20261120\r\nSUMMARY:Abgesagt\r\nSTATUS:CANCELLED\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:c\r\nDTSTART;VALUE=DATE:20261123\r\nRRULE:FREQ=DAILY;BYDAY=MO,TU,WE,TH,FR\r\nSUMMARY:Werktags\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let cal = parse(text).unwrap();
        assert_eq!(cal.name.as_deref(), Some("Verein"));
        let list = entries(&cal, 7);
        assert_eq!(list.len(), 3);
        let d = |m, day| NaiveDate::from_ymd_opt(2026, m, day).unwrap();
        let series = &list[0];
        assert_eq!(series.exceptions, vec![d(11, 9), d(11, 16)]);
        assert_eq!(list[1].title, "Training (Di)");
        assert!(list[1].recurrence.is_none());
        assert_eq!(
            list[2].recurrence.as_ref().unwrap().freq,
            crate::model::Freq::Weekly
        );
        assert!(list.iter().all(|e| e.kind == 7 && e.reminder.is_none()));
    }
}
