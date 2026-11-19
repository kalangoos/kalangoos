//! The person's events, from their local calendars: KalangoOS's own ("Pessoal", "Trabalho": ICS files in
//! `~/.local/share/kalango/calendario/`, which the Calendar app writes) and the ones the desktop's calendar service
//! keeps (`~/.local/share/evolution/calendar/*/calendar.ics`, read only). For the "Hoje" bar and the Calendar app.
//! Covers what calendars usually hold: timed events (with a time zone, in UTC or "floating"), all-day events, an end
//! (DTEND or DURATION), and daily or weekly repetition (with UNTIL, COUNT and BYDAY).

use gtk::glib;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Event {
    /// minutes after midnight (local time); None for all day
    pub start: Option<u32>,
    pub title: String,
    /// minutes after midnight it ends (an hour after its start when the file does not say)
    pub end: Option<u32>,
    pub uid: String,
    /// it repeats (moving it moves every time it happens)
    pub repeats: bool,
}

/// KalangoOS's own calendars: (file name, the name shown).
pub const OWN: &[(&str, &str)] = &[("pessoal", "Personal"), ("trabalho", "Work")];

pub fn own_dir() -> PathBuf {
    glib::user_data_dir().join("kalango/calendario")
}

pub fn own_file(id: &str) -> PathBuf {
    own_dir().join(format!("{id}.ics"))
}

/// The local calendar files: KalangoOS's own first, then the desktop's.
pub fn sources() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = OWN.iter().map(|(id, _)| own_file(id)).filter(|f| f.is_file()).collect();
    let base = glib::user_data_dir().join("evolution/calendar");
    for e in std::fs::read_dir(&base).into_iter().flatten().flatten() {
        let f = e.path().join("calendar.ics");
        if f.is_file() {
            out.push(f);
        }
    }
    out
}

/// Today's events from every local calendar, sorted (all-day first, then by time).
pub fn today() -> Vec<Event> {
    let now = glib::DateTime::now_local().ok();
    let Some(now) = now else { return vec![] };
    day(now.year(), now.month(), now.day_of_month())
}

/// One day's events of one calendar file.
pub fn day_of(file: &Path, y: i32, m: i32, d: i32) -> Vec<Event> {
    events_on(file, y, m, d)
}

/// A calendar file's own name (X-WR-CALNAME), if it has one.
pub fn name_of(file: &Path) -> Option<String> {
    std::fs::read_to_string(file).ok().and_then(|t| unfold(&t).into_iter().find_map(|l| l.strip_prefix("X-WR-CALNAME:").map(unescape)))
}

fn stamp(y: i32, m: i32, d: i32, minutes: u32) -> String {
    format!("{y:04}{m:02}{d:02}T{:02}{:02}00", minutes / 60, minutes % 60)
}

fn escape(v: &str) -> String {
    v.replace('\\', "\\\\").replace(';', "\\;").replace(',', "\\,").replace('\n', "\\n")
}

/// A new event in one of KalangoOS's own calendars (the file made when needed); its UID.
pub fn add(calendar: &str, title: &str, (y, m, d): (i32, i32, i32), start: u32, end: u32) -> std::io::Result<String> {
    let f = own_file(calendar);
    std::fs::create_dir_all(own_dir())?;
    let name = OWN.iter().find(|(id, _)| *id == calendar).map(|(_, n)| kalango_i18n::tr(n)).unwrap_or(calendar);
    let mut text = std::fs::read_to_string(&f).unwrap_or_else(|_| format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//KalangoOS//Calendario//PT\r\nX-WR-CALNAME:{}\r\nEND:VCALENDAR\r\n", escape(name)));
    let uid = format!("{}-{}@kalangoos", glib::monotonic_time(), std::process::id());
    let now = glib::DateTime::now_utc().ok().and_then(|t| t.format("%Y%m%dT%H%M%SZ").ok()).map(|s| s.to_string()).unwrap_or_default();
    let ev = format!("BEGIN:VEVENT\r\nUID:{uid}\r\nDTSTAMP:{now}\r\nDTSTART:{}\r\nDTEND:{}\r\nSUMMARY:{}\r\nEND:VEVENT\r\n",
        stamp(y, m, d, start), stamp(y, m, d, end.max(start + 15)), escape(title));
    match text.rfind("END:VCALENDAR") {
        Some(i) => text.insert_str(i, &ev),
        None => text.push_str(&ev),
    }
    std::fs::write(&f, text)?;
    Ok(uid)
}

/// The VEVENT of `uid` in a file, changed by `f` (None: taken out). Only KalangoOS's own calendars are written.
fn rewrite(file: &Path, uid: &str, f: impl Fn(Vec<String>) -> Option<Vec<String>>) -> std::io::Result<bool> {
    if !file.starts_with(own_dir()) {
        return Ok(false);
    }
    let lines = unfold(&std::fs::read_to_string(file)?);
    let mut out: Vec<String> = Vec::new();
    let mut ev: Option<Vec<String>> = None;
    let mut done = false;
    for l in lines {
        if l == "BEGIN:VEVENT" {
            ev = Some(vec![l]);
        } else if let Some(mut e) = ev.take() {
            e.push(l.clone());
            if l == "END:VEVENT" {
                if e.iter().any(|x| x.strip_prefix("UID:") == Some(uid)) {
                    done = true;
                    if let Some(n) = f(e) {
                        out.extend(n);
                    }
                } else {
                    out.extend(e);
                }
            } else {
                ev = Some(e);
            }
        } else {
            out.push(l);
        }
    }
    std::fs::write(file, out.join("\r\n") + "\r\n")?;
    Ok(done)
}

/// Moves an event to another day and time (a repeating one keeps repeating from there).
pub fn move_to(file: &Path, uid: &str, (y, m, d): (i32, i32, i32), start: u32, end: u32) -> std::io::Result<bool> {
    rewrite(file, uid, |e| Some(e.into_iter().map(|l| {
        if l.starts_with("DTSTART") {
            format!("DTSTART:{}", stamp(y, m, d, start))
        } else if l.starts_with("DTEND") {
            format!("DTEND:{}", stamp(y, m, d, end))
        } else if l.starts_with("DURATION") {
            format!("DTEND:{}", stamp(y, m, d, end))
        } else {
            l
        }
    }).collect()))
}

pub fn rename(file: &Path, uid: &str, title: &str) -> std::io::Result<bool> {
    rewrite(file, uid, |e| Some(e.into_iter().map(|l| if l.starts_with("SUMMARY") { format!("SUMMARY:{}", escape(title)) } else { l }).collect()))
}

pub fn remove(file: &Path, uid: &str) -> std::io::Result<bool> {
    rewrite(file, uid, |_| None)
}

/// One day's events from every local calendar, sorted (all-day first, then by time).
pub fn day(y: i32, m: i32, d: i32) -> Vec<Event> {
    let mut out: Vec<Event> = sources().iter().flat_map(|f| events_on(f, y, m, d)).collect();
    out.sort_by_key(|e| e.start.map(|m| m + 1).unwrap_or(0));
    out.dedup();
    out
}

/// The days of a month that have events (the calendar widget's dots), reading each file once.
pub fn days_with_events(y: i32, m: i32) -> Vec<i32> {
    let textos: Vec<String> = sources().iter().filter_map(|f| std::fs::read_to_string(f).ok()).collect();
    let dias = glib::DateTime::from_local(y, m, 1, 0, 0, 0.0).ok().and_then(|d| d.add_months(1).ok()).and_then(|d| d.add_days(-1).ok())
        .map(|d| d.day_of_month()).unwrap_or(30);
    (1..=dias).filter(|d| textos.iter().any(|t| !parse_day(t, y, m, *d).is_empty())).collect()
}

fn events_on(file: &Path, y: i32, m: i32, d: i32) -> Vec<Event> {
    std::fs::read_to_string(file).map(|t| parse_day(&t, y, m, d)).unwrap_or_default()
}

/// ICS lines, unfolded (a line starting with a space or tab continues the previous one).
fn unfold(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for raw in text.lines() {
        let raw = raw.trim_end_matches('\r');
        if (raw.starts_with(' ') || raw.starts_with('\t')) && !lines.is_empty() {
            lines.last_mut().unwrap().push_str(&raw[1..]);
        } else {
            lines.push(raw.to_string());
        }
    }
    lines
}

fn unescape(v: &str) -> String {
    v.replace("\\n", " ").replace("\\N", " ").replace("\\,", ",").replace("\\;", ";").replace("\\\\", "\\")
}

/// A DTSTART as (local date, minutes after midnight or None for all day).
fn start_of(params: &str, value: &str) -> Option<((i32, i32, i32), Option<u32>)> {
    let num = |s: &str| s.parse::<i32>().ok();
    let (date, time) = value.split_once('T').map(|(a, b)| (a, Some(b))).unwrap_or((value, None));
    if date.len() != 8 {
        return None;
    }
    let (y, mo, d) = (num(&date[0..4])?, num(&date[4..6])?, num(&date[6..8])?);
    let Some(t) = time else { return Some(((y, mo, d), None)) };
    if t.len() < 4 {
        return None;
    }
    let (h, mi) = (num(&t[0..2])?, num(&t[2..4])?);
    let s = t.get(4..6).and_then(num).unwrap_or(0);
    let tz = if t.ends_with('Z') {
        Some(glib::TimeZone::utc())
    } else {
        params.split(';').find_map(|p| p.strip_prefix("TZID=")).map(|id| glib::TimeZone::new(Some(id.trim_matches('"'))))
    };
    let dt = match tz {
        Some(tz) => glib::DateTime::new(&tz, y, mo, d, h, mi, s as f64).ok()?.to_local().ok()?,
        None => glib::DateTime::from_local(y, mo, d, h, mi, s as f64).ok()?,
    };
    Some(((dt.year(), dt.month(), dt.day_of_month()), Some((dt.hour() * 60 + dt.minute()) as u32)))
}

/// A DURATION's minutes ("PT1H30M", "PT45M", "P1D").
fn duration_of(v: &str) -> Option<u32> {
    let mut total = 0u32;
    let mut n = String::new();
    for c in v.trim_start_matches(['P', '+']).chars() {
        match c {
            '0'..='9' => n.push(c),
            'W' => { total += n.parse::<u32>().ok()? * 7 * 1440; n.clear(); }
            'D' => { total += n.parse::<u32>().ok()? * 1440; n.clear(); }
            'H' => { total += n.parse::<u32>().ok()? * 60; n.clear(); }
            'M' => { total += n.parse::<u32>().ok()?; n.clear(); }
            'S' | 'T' => n.clear(),
            _ => return None,
        }
    }
    Some(total)
}

fn day_number(y: i32, m: i32, d: i32) -> Option<i64> {
    glib::DateTime::from_utc(y, m, d, 12, 0, 0.0).ok().map(|t| t.to_unix() / 86400)
}

/// Does the event (first on `first`, repeating by `rrule`) happen on `day`?
fn occurs(first: (i32, i32, i32), rrule: Option<&str>, day: (i32, i32, i32)) -> bool {
    if first == day {
        return true;
    }
    let Some(rule) = rrule else { return false };
    let (Some(a), Some(b)) = (day_number(first.0, first.1, first.2), day_number(day.0, day.1, day.2)) else { return false };
    if b < a {
        return false;
    }
    let get = |k: &str| rule.split(';').find_map(|p| p.strip_prefix(k).map(str::to_string));
    if let Some(until) = get("UNTIL=") {
        if let Some((u, _)) = start_of("", &until) {
            if let Some(un) = day_number(u.0, u.1, u.2) {
                if b > un {
                    return false;
                }
            }
        }
    }
    let interval = get("INTERVAL=").and_then(|v| v.parse::<i64>().ok()).unwrap_or(1).max(1);
    let weekday = |n: i64| ["TH", "FR", "SA", "SU", "MO", "TU", "WE"][(n.rem_euclid(7)) as usize];
    let ok = match get("FREQ=").as_deref() {
        Some("DAILY") => (b - a) % interval == 0,
        Some("WEEKLY") => {
            let days: Vec<String> = get("BYDAY=").map(|v| v.split(',').map(str::to_string).collect()).unwrap_or_else(|| vec![weekday(a).to_string()]);
            days.iter().any(|d| d == weekday(b)) && ((b - a) / 7) % interval == 0
        }
        _ => false,
    };
    if !ok {
        return false;
    }
    if let Some(count) = get("COUNT=").and_then(|v| v.parse::<i64>().ok()) {
        // occurrences before (and including) this day
        let n = match get("FREQ=").as_deref() {
            Some("DAILY") => (b - a) / interval + 1,
            _ => (a..=b).filter(|x| occurs(first, Some(&rule.replace(&format!("COUNT={count}"), "")), {
                let t = glib::DateTime::from_unix_utc(x * 86400).unwrap();
                (t.year(), t.month(), t.day_of_month())
            })).count() as i64,
        };
        return n <= count;
    }
    true
}

pub(crate) fn parse_day(text: &str, y: i32, m: i32, d: i32) -> Vec<Event> {
    let mut out = Vec::new();
    let mut in_event = false;
    let (mut title, mut start, mut rrule, mut cancelled) = (String::new(), None::<((i32, i32, i32), Option<u32>)>, None::<String>, false);
    let (mut end, mut duration, mut uid) = (None::<((i32, i32, i32), Option<u32>)>, None::<u32>, String::new());
    for line in unfold(text) {
        match line.as_str() {
            "BEGIN:VEVENT" => {
                in_event = true;
                (title, start, rrule, cancelled) = (String::new(), None, None, false);
                (end, duration, uid) = (None, None, String::new());
            }
            "END:VEVENT" => {
                in_event = false;
                if let Some((first, minutes)) = start {
                    if !cancelled && occurs(first, rrule.as_deref(), (y, m, d)) {
                        // (its end the same day; past midnight it ends with the day)
                        let end_min = minutes.map(|s| match (end, duration) {
                            (Some((ed, Some(em))), _) if ed == first => em.max(s),
                            (Some(_), _) => 24 * 60,
                            (None, Some(dm)) => (s + dm).min(24 * 60),
                            _ => (s + 60).min(24 * 60),
                        });
                        out.push(Event { start: minutes, title: if title.is_empty() { "—".into() } else { title.clone() }, end: end_min, uid: uid.clone(), repeats: rrule.is_some() });
                    }
                }
            }
            l if in_event => {
                let Some((key, value)) = l.split_once(':') else { continue };
                let (name, params) = key.split_once(';').unwrap_or((key, ""));
                match name {
                    "SUMMARY" => title = unescape(value),
                    "DTSTART" => start = start_of(params, value),
                    "DTEND" => end = start_of(params, value),
                    "DURATION" => duration = duration_of(value),
                    "UID" => uid = value.to_string(),
                    "RRULE" => rrule = Some(value.to_string()),
                    "STATUS" => cancelled = value == "CANCELLED",
                    _ => {}
                }
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const ICS: &str = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nSUMMARY:Reunião de produto\r\nDTSTART;TZID=America/Sao_Paulo:20261115T150000\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nSUMMARY:Academia\r\nDTSTART:20261102T183000\r\nRRULE:FREQ=WEEKLY;BYDAY=SU,MO\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nSUMMARY:Revisar PR do\r\n  lunar-shell\r\nDTSTART:20261115T210000\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nSUMMARY:Feriado\r\nDTSTART;VALUE=DATE:20261115\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nSUMMARY:Cancelado\r\nSTATUS:CANCELLED\r\nDTSTART:20261115T100000\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nSUMMARY:Outro dia\r\nDTSTART:20261116T100000\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    #[test]
    fn the_events_of_a_day() {
        let e = parse_day(ICS, 2026, 11, 15);
        let titles: Vec<&str> = e.iter().map(|e| e.title.as_str()).collect();
        assert!(titles.contains(&"Reunião de produto"));
        assert!(titles.contains(&"Academia"), "weekly on Sundays");
        assert!(titles.contains(&"Revisar PR do lunar-shell"), "folded line");
        assert!(titles.contains(&"Feriado"));
        assert!(!titles.contains(&"Cancelado") && !titles.contains(&"Outro dia"));
        assert_eq!(e.iter().find(|e| e.title == "Feriado").unwrap().start, None);
        assert_eq!(e.iter().find(|e| e.title == "Revisar PR do lunar-shell").unwrap().start, Some(21 * 60));
    }

    #[test]
    fn ends_and_writing() {
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:a\r\nSUMMARY:Dentista\r\nDTSTART:20261119T140000\r\nDTEND:20261119T150000\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:b\r\nSUMMARY:Foco\r\nDTSTART:20261117T130000\r\nDURATION:PT1H30M\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let e = parse_day(ics, 2026, 11, 19);
        assert_eq!((e[0].start, e[0].end, e[0].uid.as_str()), (Some(840), Some(900), "a"));
        assert_eq!(parse_day(ics, 2026, 11, 17)[0].end, Some(13 * 60 + 90));
        assert_eq!(duration_of("PT45M"), Some(45));
        assert_eq!(duration_of("P1D"), Some(1440));
    }

    #[test]
    fn repetition_rules() {
        assert!(occurs((2026, 11, 1), Some("FREQ=DAILY;INTERVAL=2"), (2026, 11, 15)));
        assert!(!occurs((2026, 11, 1), Some("FREQ=DAILY;INTERVAL=2"), (2026, 11, 16)));
        assert!(!occurs((2026, 11, 1), Some("FREQ=DAILY;UNTIL=20261110"), (2026, 11, 15)));
        assert!(!occurs((2026, 11, 1), Some("FREQ=DAILY;COUNT=5"), (2026, 11, 15)));
        assert!(occurs((2026, 11, 1), Some("FREQ=WEEKLY"), (2026, 11, 15)), "same weekday");
        assert!(!occurs((2026, 11, 20), Some("FREQ=DAILY"), (2026, 11, 15)), "not before it starts");
    }
}
