//! The arrangements the person uses (design "Layouts de janela (encaixe)": "O agente sugere o arranjo que você mais usa
//! naquele contexto"). Each whole layout applied in the picker — which layout, which app in each region, when — is kept
//! (only on this computer: the last 200, in $XDG_STATE_HOME/kalango/arranjos.json), and the picker suggests the one used
//! most with the apps open now (at least twice in the last 30 days). Settings › Appearance › Dock and multitasking ›
//! "O agente sugere layouts" turns it off.

use kalango_i18n::{tr, trf};
use serde_json::{json, Value};
use std::path::PathBuf;

/// A layout applied: the picker's layout, (region index, app id) of each window placed, and when (unix time).
#[derive(Clone, Debug, PartialEq)]
pub struct Arrangement {
    pub layout: usize,
    pub slots: Vec<(usize, String)>,
    pub t: i64,
}

/// What the picker offers: the arrangement, how many times it was used and the part of the day it is used in, if one.
#[derive(Clone, Debug, PartialEq)]
pub struct Suggestion {
    pub layout: usize,
    pub slots: Vec<(usize, String)>,
    pub times: usize,
    pub part_of_day: Option<PartOfDay>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PartOfDay {
    Morning,
    Afternoon,
    Night,
}

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

pub fn enabled() -> bool {
    crate::configuracoes::ajustes()["sugerir_layouts"].as_bool().unwrap_or(true)
}

pub fn set_enabled(on: bool) {
    crate::configuracoes::salvar_ajuste("sugerir_layouts", json!(on));
    if debug() {
        eprintln!("debug layouts: sugestões {on}");
    }
}

fn file() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).filter(|p| p.is_absolute())
        .unwrap_or_else(|| gtk::glib::home_dir().join(".local/state")).join("kalango/arranjos.json")
}

pub fn read() -> Vec<Arrangement> {
    let v: Vec<Value> = std::fs::read_to_string(file()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    v.iter().filter_map(|a| {
        let slots = a["slots"].as_array()?.iter().filter_map(|s| Some((s[0].as_u64()? as usize, s[1].as_str()?.to_string()))).collect();
        Some(Arrangement { layout: a["layout"].as_u64()? as usize, slots, t: a["t"].as_i64()? })
    }).collect()
}

/// Keeps a layout applied with two windows or more (one window in a region is a snap, not an arrangement).
pub fn record(layout: usize, mut slots: Vec<(usize, String)>) {
    slots.retain(|(_, app)| !app.is_empty());
    if slots.len() < 2 {
        return;
    }
    slots.sort();
    let mut all = read();
    all.push(Arrangement { layout, slots: slots.clone(), t: gtk::glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0) });
    let start = all.len().saturating_sub(200);
    let list: Vec<Value> = all[start..].iter().map(|a| json!({"layout": a.layout, "slots": a.slots.iter().map(|(r, app)| json!([r, app])).collect::<Vec<_>>(), "t": a.t})).collect();
    let f = file();
    if let Some(d) = f.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(&f, Value::Array(list).to_string());
    if debug() {
        eprintln!("debug layouts: arranjo guardado (layout {layout}: {})", slots.iter().map(|(r, a)| format!("{r}={a}")).collect::<Vec<_>>().join(" "));
    }
}

/// The arrangement used most (at least twice in 30 days) whose apps are all open now; the same count goes to the one
/// used last. `hour_of` gives a time's local hour.
pub fn suggest(records: &[Arrangement], open: &[String], now: i64, hour_of: impl Fn(i64) -> Option<i32>) -> Option<Suggestion> {
    let mut groups: Vec<(usize, Vec<(usize, String)>, Vec<i64>)> = Vec::new();
    for a in records.iter().filter(|a| now - a.t < 30 * 86400 && a.slots.len() >= 2) {
        if !a.slots.iter().all(|(_, app)| open.contains(app)) {
            continue;
        }
        match groups.iter_mut().find(|(l, s, _)| *l == a.layout && *s == a.slots) {
            Some(g) => g.2.push(a.t),
            None => groups.push((a.layout, a.slots.clone(), vec![a.t])),
        }
    }
    let (layout, slots, times) = groups.into_iter().filter(|g| g.2.len() >= 2)
        .max_by_key(|g| (g.2.len(), g.2.iter().copied().max().unwrap_or(0)))?;
    let parts: Vec<PartOfDay> = times.iter().filter_map(|t| hour_of(*t)).map(|h| match h {
        5..=11 => PartOfDay::Morning,
        12..=17 => PartOfDay::Afternoon,
        _ => PartOfDay::Night,
    }).collect();
    // a part of the day only when every use of it was then
    let part_of_day = parts.first().copied().filter(|p| parts.len() == times.len() && parts.iter().all(|x| x == p));
    Some(Suggestion { layout, slots, times: times.len(), part_of_day })
}

/// [`suggest`] now, for the apps open in the focused space (`None` when turned off).
pub fn suggestion_now(open: &[String]) -> Option<Suggestion> {
    if !enabled() {
        return None;
    }
    let now = gtk::glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0);
    suggest(&read(), open, now, |t| gtk::glib::DateTime::from_unix_local(t).ok().map(|d| d.hour()))
}

/// The side a region is on (by its middle), for the suggestion's sentence.
fn side(r: (u16, u16, u16, u16)) -> u8 {
    let mid = u32::from(r.0) + u32::from(r.2) / 2;
    if mid < 450 { 0 } else if mid > 550 { 2 } else { 1 }
}

/// "Editor on the left · Terminal and Calculator on the right": the apps by side, in the regions' order.
pub fn sentence(regions: &[(u16, u16, u16, u16)], slots: &[(usize, String)], name_of: impl Fn(&str) -> String) -> String {
    let mut parts = Vec::new();
    for s in 0..3u8 {
        let names: Vec<String> = slots.iter().filter(|(r, _)| regions.get(*r).is_some_and(|reg| side(*reg) == s)).map(|(_, a)| name_of(a)).collect();
        if names.is_empty() {
            continue;
        }
        let apps = match names.as_slice() {
            [one] => one.clone(),
            [rest @ .., last] => format!("{} {} {last}", rest.join(", "), tr("and")),
            [] => continue,
        };
        parts.push(match s {
            0 => trf("{apps} on the left", &[("apps", apps)]),
            2 => trf("{apps} on the right", &[("apps", apps)]),
            _ => trf("{apps} in the middle", &[("apps", apps)]),
        });
    }
    parts.join(" · ")
}

/// Why it is offered: "(you use this arrangement every night)" or how many times it was used.
pub fn reason(s: &Suggestion) -> String {
    match s.part_of_day {
        Some(PartOfDay::Morning) => tr("you use this arrangement every morning").to_string(),
        Some(PartOfDay::Afternoon) => tr("you use this arrangement every afternoon").to_string(),
        Some(PartOfDay::Night) => tr("you use this arrangement every night").to_string(),
        None => trf("you have used this arrangement {n} times", &[("n", s.times.to_string())]),
    }
}

/// The suggestion's title: what the apps are for ("Layout for coding"), or the layout's own name.
pub fn title(categories: &[&str], layout_name: &str) -> String {
    let mut count: Vec<(&str, usize)> = Vec::new();
    for c in categories.iter().filter(|c| !matches!(**c, "sistema" | "outros")) {
        match count.iter_mut().find(|(x, _)| x == c) {
            Some(e) => e.1 += 1,
            None => count.push((c, 1)),
        }
    }
    let top = count.iter().max_by_key(|(_, n)| *n).map(|(c, _)| *c);
    match top {
        Some("desenvolvimento") => tr("Layout for coding").to_string(),
        Some("criacao") => tr("Layout for creating").to_string(),
        Some("internet") => tr("Layout for browsing").to_string(),
        Some("ia") => tr("Layout for working with AI").to_string(),
        _ => layout_name.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(layout: usize, slots: &[(usize, &str)], t: i64) -> Arrangement {
        Arrangement { layout, slots: slots.iter().map(|(r, s)| (*r, s.to_string())).collect(), t }
    }

    #[test]
    fn the_arrangement_used_most_with_the_open_apps() {
        let day = 86400;
        let now = 40 * day;
        let hour_of = |t: i64| Some(((t % day) / 3600) as i32);
        let at = |d: i64, h: i64| (40 - d) * day + h * 3600;
        let recs = vec![
            a(2, &[(0, "editor"), (1, "terminal"), (2, "docs")], at(3, 21)),
            a(2, &[(0, "editor"), (1, "terminal"), (2, "docs")], at(2, 22)),
            a(0, &[(0, "editor"), (1, "calc")], at(1, 10)),
            a(0, &[(0, "editor"), (1, "calc")], at(1, 15)),
            a(0, &[(0, "editor"), (1, "calc")], at(40, 15)),
        ];
        let open: Vec<String> = ["editor", "terminal", "docs", "calc"].iter().map(|s| s.to_string()).collect();
        // two layouts used twice in 30 days (the third use of "calc" is too old): the one used last wins
        let s = suggest(&recs, &open, now, hour_of).unwrap();
        assert_eq!((s.layout, s.times, s.part_of_day), (0, 2, None));
        // without the calculator open, the night one
        let s = suggest(&recs, &open[..3], now, hour_of).unwrap();
        assert_eq!((s.layout, s.times, s.part_of_day), (2, 2, Some(PartOfDay::Night)));
        // used once: nothing
        assert!(suggest(&recs[..1], &open, now, hour_of).is_none());
    }

    #[test]
    fn sentence_and_title() {
        let regions = [(0, 0, 667, 1000), (667, 0, 333, 500), (667, 500, 333, 500)];
        let slots = vec![(0, "Editor".to_string()), (1, "Terminal".to_string()), (2, "Docs".to_string())];
        let esperado = format!("{} · {}", trf("{apps} on the left", &[("apps", "Editor".into())]),
            trf("{apps} on the right", &[("apps", format!("Terminal {} Docs", tr("and")))]));
        assert_eq!(sentence(&regions, &slots, |a| a.to_string()), esperado);
        assert_eq!(title(&["desenvolvimento", "sistema", "desenvolvimento"], "Focus + stack"), tr("Layout for coding"));
        assert_eq!(title(&["sistema", "outros"], "Halves"), "Halves");
    }
}
