// SPDX-License-Identifier: GPL-3.0-or-later

//! A small iCalendar (RFC 5545) writer and reader, enough for invitations
//! (iTIP/iMIP: REQUEST, REPLY, CANCEL), shared entries (PUBLISH) and import.

use anyhow::{Result, bail};
use chrono::{Local, NaiveDate, NaiveDateTime, TimeZone, Utc};

use crate::model::{Entry, Freq, Participant, Partstat, RecurEnd, Recurrence};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    /// A plain calendar file, e.g. an export.
    None,
    Publish,
    Request,
    Reply,
    Cancel,
}

impl Method {
    fn as_str(self) -> Option<&'static str> {
        match self {
            Method::None => None,
            Method::Publish => Some("PUBLISH"),
            Method::Request => Some("REQUEST"),
            Method::Reply => Some("REPLY"),
            Method::Cancel => Some("CANCEL"),
        }
    }

    fn parse(s: &str) -> Method {
        match s.to_ascii_uppercase().as_str() {
            "PUBLISH" => Method::Publish,
            "REQUEST" => Method::Request,
            "REPLY" => Method::Reply,
            "CANCEL" => Method::Cancel,
            _ => Method::None,
        }
    }
}

/// An event read from a file.
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub uid: String,
    pub summary: String,
    pub start: NaiveDateTime,
    pub end: NaiveDateTime,
    pub all_day: bool,
    pub location: String,
    pub description: String,
    pub organizer: Option<String>,
    /// Shown name of the organizer, if the file has one.
    pub organizer_name: Option<String>,
    pub attendees: Vec<Participant>,
    pub recurrence: Option<Recurrence>,
    pub sequence: u32,
    /// Days left out of the series (EXDATE).
    pub exdates: Vec<NaiveDate>,
    /// Set when this event replaces one day of a series with the same UID.
    pub recurrence_id: Option<NaiveDate>,
    /// STATUS:CANCELLED.
    pub cancelled: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Calendar {
    pub method: Method,
    pub events: Vec<Event>,
    /// X-WR-CALNAME, the name a published calendar gives itself.
    pub name: Option<String>,
}

pub struct WriteOptions<'a> {
    pub method: Method,
    /// Own address, written as ORGANIZER (requests) or as the answering ATTENDEE (replies).
    pub own_email: &'a str,
    pub own_name: &'a str,
    /// Location and notes are left out when false.
    pub details: bool,
}

/// Writes one calendar object with the given entries.
pub fn write(entries: &[&Entry], options: &WriteOptions) -> String {
    let mut lines: Vec<String> = vec![
        "BEGIN:VCALENDAR".into(),
        "VERSION:2.0".into(),
        "PRODID:-//Kalender//Kalender//DE".into(),
        "CALSCALE:GREGORIAN".into(),
    ];
    if let Some(m) = options.method.as_str() {
        lines.push(format!("METHOD:{m}"));
    }
    let stamp = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    for entry in entries {
        lines.push("BEGIN:VEVENT".into());
        lines.push(format!("UID:{}", entry.uid));
        lines.push(format!("DTSTAMP:{stamp}"));
        lines.push(format!("SEQUENCE:{}", entry.sequence));
        if entry.all_day {
            lines.push(format!(
                "DTSTART;VALUE=DATE:{}",
                entry.start.format("%Y%m%d")
            ));
            lines.push(format!("DTEND;VALUE=DATE:{}", entry.end.format("%Y%m%d")));
        } else {
            lines.push(format!("DTSTART:{}", utc(entry.start)));
            lines.push(format!("DTEND:{}", utc(entry.end)));
        }
        lines.push(format!("SUMMARY:{}", escape(&entry.title)));
        if options.details {
            if !entry.location.is_empty() {
                lines.push(format!("LOCATION:{}", escape(&entry.location)));
            }
            if !entry.notes.is_empty() {
                lines.push(format!("DESCRIPTION:{}", escape(&entry.notes)));
            }
        }
        if let Some(rule) = &entry.recurrence {
            lines.push(format!("RRULE:{}", rrule(rule)));
            for ex in &entry.exceptions {
                if entry.all_day {
                    lines.push(format!("EXDATE;VALUE=DATE:{}", ex.format("%Y%m%d")));
                } else {
                    lines.push(format!("EXDATE:{}", utc(ex.and_time(entry.start.time()))));
                }
            }
        }
        if options.method == Method::Cancel {
            lines.push("STATUS:CANCELLED".into());
        }
        match options.method {
            Method::Reply => {
                if let Some(org) = &entry.organizer {
                    lines.push(format!("ORGANIZER:mailto:{org}"));
                }
                let status = entry
                    .participants
                    .iter()
                    .find(|p| p.email.eq_ignore_ascii_case(options.own_email))
                    .map(|p| p.status)
                    .unwrap_or(Partstat::Accepted);
                lines.push(format!(
                    "ATTENDEE;CN={};PARTSTAT={}:mailto:{}",
                    param(options.own_name),
                    partstat(status),
                    options.own_email
                ));
            }
            Method::Request | Method::Cancel if !entry.participants.is_empty() => {
                lines.push(format!(
                    "ORGANIZER;CN={}:mailto:{}",
                    param(options.own_name),
                    options.own_email
                ));
                for p in &entry.participants {
                    lines.push(format!(
                        "ATTENDEE;CN={};ROLE=REQ-PARTICIPANT;PARTSTAT={};RSVP=TRUE:mailto:{}",
                        param(&p.name),
                        partstat(p.status),
                        p.email
                    ));
                }
            }
            _ => {}
        }
        lines.push("END:VEVENT".into());
    }
    lines.push("END:VCALENDAR".into());
    lines
        .iter()
        .map(|l| fold(l))
        .collect::<Vec<_>>()
        .join("\r\n")
        + "\r\n"
}

/// One content line: name, parameters, value.
type Prop = (String, Vec<(String, String)>, String);

/// Reads all events of a calendar file.
pub fn parse(text: &str) -> Result<Calendar> {
    let lines = unfold(text);
    if !lines
        .iter()
        .any(|l| l.eq_ignore_ascii_case("BEGIN:VCALENDAR"))
    {
        bail!("not an iCalendar file");
    }
    let mut method = Method::None;
    let mut cal_name = None;
    let mut events = Vec::new();
    let mut current: Option<Vec<Prop>> = None;
    let mut depth_other = 0usize;

    for line in &lines {
        let Some((name, params, value)) = split_line(line) else {
            continue;
        };
        match (name.as_str(), value.to_ascii_uppercase().as_str()) {
            ("BEGIN", "VEVENT") if current.is_none() => current = Some(Vec::new()),
            ("END", "VEVENT") => {
                if let Some(props) = current.take()
                    && let Some(e) = event_from(&props)
                {
                    events.push(e);
                }
            }
            ("BEGIN", _) if current.is_some() => depth_other += 1,
            ("END", _) if current.is_some() && depth_other > 0 => depth_other -= 1,
            ("METHOD", _) if current.is_none() => method = Method::parse(&value),
            ("X-WR-CALNAME", _) if current.is_none() => {
                cal_name = Some(unescape(&value)).filter(|n| !n.trim().is_empty())
            }
            _ => {
                if let Some(props) = current.as_mut()
                    && depth_other == 0
                {
                    props.push((name, params, value));
                }
            }
        }
    }
    Ok(Calendar {
        method,
        events,
        name: cal_name,
    })
}

fn event_from(props: &[Prop]) -> Option<Event> {
    let get = |n: &str| props.iter().find(|(name, _, _)| name == n);
    let (_, start_params, start_value) = get("DTSTART")?;
    let (start, all_day) = parse_time(start_params, start_value)?;
    let end = match get("DTEND") {
        Some((_, p, v)) => parse_time(p, v)?.0,
        None => match get("DURATION") {
            Some((_, _, v)) => start + parse_duration(v)?,
            None if all_day => start + chrono::Duration::days(1),
            None => start,
        },
    };
    let text = |n: &str| get(n).map(|(_, _, v)| unescape(v)).unwrap_or_default();
    let attendees = props
        .iter()
        .filter(|(n, _, _)| n == "ATTENDEE")
        .map(|(_, params, value)| {
            let email = strip_mailto(value);
            let name = param_value(params, "CN").unwrap_or_else(|| email.clone());
            let status = match param_value(params, "PARTSTAT").as_deref() {
                Some("ACCEPTED") => Partstat::Accepted,
                Some("DECLINED") => Partstat::Declined,
                Some("TENTATIVE") => Partstat::Tentative,
                _ => Partstat::NeedsAction,
            };
            Participant {
                name,
                email,
                status,
            }
        })
        .collect();
    Some(Event {
        uid: text("UID"),
        summary: text("SUMMARY"),
        start,
        end,
        all_day,
        location: text("LOCATION"),
        description: text("DESCRIPTION"),
        organizer: get("ORGANIZER").map(|(_, _, v)| strip_mailto(v)),
        organizer_name: get("ORGANIZER").and_then(|(_, p, _)| param_value(p, "CN")),
        attendees,
        recurrence: get("RRULE").and_then(|(_, _, v)| parse_rrule(v)),
        sequence: get("SEQUENCE")
            .and_then(|(_, _, v)| v.trim().parse().ok())
            .unwrap_or(0),
        exdates: props
            .iter()
            .filter(|(n, _, _)| n == "EXDATE")
            .flat_map(|(_, p, v)| {
                v.split(',')
                    .filter_map(|one| parse_time(p, one).map(|(t, _)| t.date()))
                    .collect::<Vec<_>>()
            })
            .collect(),
        recurrence_id: get("RECURRENCE-ID")
            .and_then(|(_, p, v)| parse_time(p, v))
            .map(|(t, _)| t.date()),
        cancelled: get("STATUS").is_some_and(|(_, _, v)| v.eq_ignore_ascii_case("CANCELLED")),
    })
}

fn utc(local: NaiveDateTime) -> String {
    let utc = Local
        .from_local_datetime(&local)
        .earliest()
        .map(|t| t.with_timezone(&Utc).naive_utc())
        .unwrap_or(local);
    utc.format("%Y%m%dT%H%M%SZ").to_string()
}

fn parse_time(params: &[(String, String)], value: &str) -> Option<(NaiveDateTime, bool)> {
    let value = value.trim();
    let is_date = param_value(params, "VALUE").is_some_and(|v| v.eq_ignore_ascii_case("DATE"))
        || value.len() == 8;
    if is_date {
        let date = NaiveDate::parse_from_str(value, "%Y%m%d").ok()?;
        return Some((crate::model::midnight(date), true));
    }
    if let Some(stripped) = value.strip_suffix('Z') {
        let naive = NaiveDateTime::parse_from_str(stripped, "%Y%m%dT%H%M%S").ok()?;
        return Some((
            Utc.from_utc_datetime(&naive)
                .with_timezone(&Local)
                .naive_local(),
            false,
        ));
    }
    // TZID or floating time: taken as local time. Good enough within one time zone.
    let naive = NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S").ok()?;
    Some((naive, false))
}

fn parse_duration(v: &str) -> Option<chrono::Duration> {
    // e.g. PT1H30M, P1D, -PT15M
    let v = v.trim();
    let (neg, v) = v
        .strip_prefix('-')
        .map(|r| (true, r))
        .unwrap_or((false, v.trim_start_matches('+')));
    let v = v.strip_prefix('P')?;
    let mut total = chrono::Duration::zero();
    let mut num = String::new();
    for c in v.chars() {
        match c {
            '0'..='9' => num.push(c),
            'T' => {}
            'W' | 'D' | 'H' | 'M' | 'S' => {
                let n: i64 = num.parse().ok()?;
                num.clear();
                total += match c {
                    'W' => chrono::Duration::weeks(n),
                    'D' => chrono::Duration::days(n),
                    'H' => chrono::Duration::hours(n),
                    'M' => chrono::Duration::minutes(n),
                    _ => chrono::Duration::seconds(n),
                };
            }
            _ => return None,
        }
    }
    Some(if neg { -total } else { total })
}

const DAYS: [&str; 7] = ["MO", "TU", "WE", "TH", "FR", "SA", "SU"];

fn rrule(rule: &Recurrence) -> String {
    let freq = match rule.freq {
        Freq::Daily => "DAILY",
        Freq::Weekly => "WEEKLY",
        Freq::Monthly => "MONTHLY",
        Freq::Yearly => "YEARLY",
    };
    let mut out = format!("FREQ={freq}");
    if rule.interval > 1 {
        out.push_str(&format!(";INTERVAL={}", rule.interval));
    }
    if rule.freq == Freq::Weekly && !rule.weekdays.is_empty() {
        let days: Vec<&str> = rule
            .weekdays
            .iter()
            .filter_map(|d| DAYS.get(*d as usize).copied())
            .collect();
        out.push_str(&format!(";BYDAY={}", days.join(",")));
    }
    match rule.end {
        RecurEnd::Never => {}
        RecurEnd::Until(d) => out.push_str(&format!(";UNTIL={}", d.format("%Y%m%d"))),
        RecurEnd::Count(n) => out.push_str(&format!(";COUNT={n}")),
    }
    out
}

fn parse_rrule(v: &str) -> Option<Recurrence> {
    let mut rule = Recurrence {
        freq: Freq::Daily,
        interval: 1,
        weekdays: Vec::new(),
        end: RecurEnd::Never,
    };
    let mut has_freq = false;
    for part in v.split(';') {
        let (k, val) = part.split_once('=')?;
        match k.to_ascii_uppercase().as_str() {
            "FREQ" => {
                has_freq = true;
                rule.freq = match val.to_ascii_uppercase().as_str() {
                    "DAILY" => Freq::Daily,
                    "WEEKLY" => Freq::Weekly,
                    "MONTHLY" => Freq::Monthly,
                    "YEARLY" => Freq::Yearly,
                    _ => return None,
                }
            }
            "INTERVAL" => rule.interval = val.parse().ok()?,
            "COUNT" => rule.end = RecurEnd::Count(val.parse().ok()?),
            "UNTIL" => {
                rule.end = RecurEnd::Until(NaiveDate::parse_from_str(val.get(..8)?, "%Y%m%d").ok()?)
            }
            "BYDAY" => {
                rule.weekdays = val
                    .split(',')
                    .filter_map(|d| DAYS.iter().position(|x| d.ends_with(x)).map(|i| i as u8))
                    .collect()
            }
            _ => {}
        }
    }
    if !rule.weekdays.is_empty() {
        match rule.freq {
            // "Every weekday" is written as a daily rule with BYDAY.
            Freq::Daily if rule.interval == 1 => rule.freq = Freq::Weekly,
            Freq::Weekly => {}
            // "2nd Monday of the month" can't be expressed yet: keep the day of the month.
            _ => rule.weekdays.clear(),
        }
    }
    has_freq.then_some(rule)
}

fn partstat(p: Partstat) -> &'static str {
    match p {
        Partstat::NeedsAction => "NEEDS-ACTION",
        Partstat::Accepted => "ACCEPTED",
        Partstat::Declined => "DECLINED",
        Partstat::Tentative => "TENTATIVE",
    }
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace(';', "\\;")
        .replace(',', "\\,")
        .replace("\r\n", "\\n")
        .replace('\n', "\\n")
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') | Some('N') => out.push('\n'),
                Some(other) => out.push(other),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn param(s: &str) -> String {
    if s.contains([':', ';', ',']) {
        format!("\"{}\"", s.replace('"', "'"))
    } else {
        s.to_owned()
    }
}

/// Lines longer than 75 bytes are folded with CRLF + space.
fn fold(line: &str) -> String {
    let mut out = String::new();
    let mut len = 0;
    for c in line.chars() {
        let w = c.len_utf8();
        if len + w > 75 {
            out.push_str("\r\n ");
            len = 1;
        }
        out.push(c);
        len += w;
    }
    out
}

fn unfold(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        if let Some(rest) = raw.strip_prefix([' ', '\t']) {
            if let Some(last) = lines.last_mut() {
                last.push_str(rest);
            }
        } else if !raw.is_empty() {
            lines.push(raw.to_owned());
        }
    }
    lines
}

/// NAME;PARAM=VALUE;...:VALUE, respecting quoted parameter values.
fn split_line(line: &str) -> Option<Prop> {
    let mut in_quotes = false;
    let mut colon = None;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_quotes = !in_quotes,
            ':' if !in_quotes => {
                colon = Some(i);
                break;
            }
            _ => {}
        }
    }
    let colon = colon?;
    let (head, value) = (&line[..colon], &line[colon + 1..]);
    let mut parts = split_params(head).into_iter();
    let name = parts.next()?.to_ascii_uppercase();
    let params = parts
        .filter_map(|p| {
            let (k, v) = p.split_once('=')?;
            Some((k.to_ascii_uppercase(), v.trim_matches('"').to_owned()))
        })
        .collect();
    Some((name, params, value.to_owned()))
}

fn split_params(head: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    for c in head.chars() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                cur.push(c);
            }
            ';' if !in_quotes => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out
}

fn param_value(params: &[(String, String)], key: &str) -> Option<String> {
    params
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.clone())
}

fn strip_mailto(v: &str) -> String {
    let v = v.trim();
    v.strip_prefix("mailto:")
        .or_else(|| v.strip_prefix("MAILTO:"))
        .unwrap_or(v)
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Participant, at, midnight};

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn meeting() -> Entry {
        let mut e = Entry::new(
            2,
            "Kundentermin; Nordlicht, Hamburg",
            at(d(2026, 11, 25), 10, 0),
            at(d(2026, 11, 25), 11, 30),
        );
        e.location = "Hafenstraße 12, Hamburg".into();
        e.notes = "Unterlagen\nmitbringen".into();
        e.participants = vec![Participant {
            name: "Sophie Wagner".into(),
            email: "sophie@example.org".into(),
            status: Partstat::NeedsAction,
        }];
        e
    }

    #[test]
    fn request_roundtrip() {
        let e = meeting();
        let text = write(
            &[&e],
            &WriteOptions {
                method: Method::Request,
                own_email: "max@example.org",
                own_name: "Max Berger",
                details: true,
            },
        );
        assert!(text.contains("METHOD:REQUEST"));
        assert!(text.lines().all(|l| l.trim_end_matches('\r').len() <= 75));
        let cal = parse(&text).unwrap();
        assert_eq!(cal.method, Method::Request);
        let ev = &cal.events[0];
        assert_eq!(ev.uid, e.uid);
        assert_eq!(ev.summary, e.title);
        assert_eq!(ev.start, e.start);
        assert_eq!(ev.end, e.end);
        assert_eq!(ev.location, e.location);
        assert_eq!(ev.description, e.notes);
        assert_eq!(ev.organizer.as_deref(), Some("max@example.org"));
        assert_eq!(ev.attendees[0].email, "sophie@example.org");
        assert_eq!(ev.attendees[0].name, "Sophie Wagner");
    }

    #[test]
    fn publish_without_details() {
        let e = meeting();
        let text = write(
            &[&e],
            &WriteOptions {
                method: Method::Publish,
                own_email: "max@example.org",
                own_name: "Max",
                details: false,
            },
        );
        assert!(!text.contains("LOCATION"));
        assert!(!text.contains("DESCRIPTION"));
        assert!(!text.contains("ATTENDEE"));
    }

    #[test]
    fn all_day_and_rule() {
        let mut e = Entry::new(
            4,
            "Urlaub",
            midnight(d(2026, 12, 2)),
            midnight(d(2026, 12, 5)),
        );
        e.all_day = true;
        e.recurrence = Some(Recurrence {
            freq: Freq::Weekly,
            interval: 2,
            weekdays: vec![1, 3],
            end: RecurEnd::Count(10),
        });
        let text = write(
            &[&e],
            &WriteOptions {
                method: Method::None,
                own_email: "",
                own_name: "",
                details: true,
            },
        );
        assert!(text.contains("DTSTART;VALUE=DATE:20261202"));
        assert!(text.contains("RRULE:FREQ=WEEKLY;INTERVAL=2;BYDAY=TU,TH;COUNT=10"));
        let ev = &parse(&text).unwrap().events[0];
        assert!(ev.all_day);
        assert_eq!(ev.end, midnight(d(2026, 12, 5)));
        assert_eq!(ev.recurrence, e.recurrence);
    }

    #[test]
    fn reads_reply_from_other_programs() {
        let text = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nMETHOD:REPLY\r\nBEGIN:VEVENT\r\nUID:abc@kalender.local\r\nDTSTART;TZID=Europe/Berlin:20261125T100000\r\nDURATION:PT1H30M\r\nATTENDEE;PARTSTAT=ACCEPTED;CN=\"Berg, Tom\":mailto:tom@example.org\r\nSUMMARY:Kunden\r\n termin\r\nBEGIN:VALARM\r\nTRIGGER:-PT15M\r\nACTION:DISPLAY\r\nEND:VALARM\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let cal = parse(text).unwrap();
        assert_eq!(cal.method, Method::Reply);
        let ev = &cal.events[0];
        assert_eq!(ev.summary, "Kundentermin");
        assert_eq!(ev.end - ev.start, chrono::Duration::minutes(90));
        assert_eq!(ev.attendees[0].name, "Berg, Tom");
        assert_eq!(ev.attendees[0].status, Partstat::Accepted);
    }

    #[test]
    fn rejects_other_files() {
        assert!(parse("hello").is_err());
    }
}
