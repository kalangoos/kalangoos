//! What happens to each notification (Settings › Notifications): the person's rules per app (off, banners and sound,
//! banners only, only in the agent's digest), Do not disturb (by hand, or by itself from 22:00 to 07:00 and while the
//! focus timer runs) and what still breaks through it (alarms, high-risk approvals, important people), where the
//! banners show, the island, the lock screen and the notifications' sound. Kept in `configuracoes.json` under
//! "notifications"; every notification is logged (`notificacoes.jsonl`, a week) for the page's counts and apps.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    BannersSound,
    Banners,
    DigestOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppRule {
    pub on: bool,
    pub mode: Mode,
}

impl Default for AppRule {
    fn default() -> Self {
        AppRule { on: true, mode: Mode::BannersSound }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Position {
    /// centred under the island (the design's banners)
    #[default]
    Island,
    TopRight,
    TopLeft,
    BottomRight,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LockScreen {
    All,
    /// only the apps' names, never what a notification says
    #[default]
    NoContent,
    Nothing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Breaks {
    pub alarms: bool,
    pub approvals: bool,
    pub people: bool,
}

impl Default for Breaks {
    fn default() -> Self {
        Breaks { alarms: true, approvals: true, people: true }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Policy {
    /// Do not disturb by itself from 22:00 to 07:00 and while the focus timer runs
    pub dnd_auto: bool,
    /// non-urgent notifications of "digest only" apps wait for the agent's digest at 12:00 and 18:00
    pub agent_digest: bool,
    pub apps: BTreeMap<String, AppRule>,
    pub position: Position,
    /// long activities (a copy, the agent's task) become a card in the island
    pub in_island: bool,
    pub lock_screen: LockScreen,
    /// the notifications' sound, 0 – 1
    pub volume: f64,
    pub people: Vec<String>,
    pub breaks: Breaks,
}

impl Default for Policy {
    fn default() -> Self {
        Policy { dnd_auto: true, agent_digest: true, apps: BTreeMap::new(), position: Position::Island, in_island: true, lock_screen: LockScreen::NoContent,
            volume: 0.35, people: vec![], breaks: Breaks::default() }
    }
}

pub fn read() -> Policy {
    serde_json::from_value(crate::configuracoes::ajustes()["notifications"].clone()).unwrap_or_default()
}

pub fn save(p: &Policy) {
    crate::configuracoes::salvar_ajuste("notifications", serde_json::to_value(p).unwrap_or(Value::Null));
}

/// Changes the policy and keeps it.
pub fn update(f: impl FnOnce(&mut Policy)) -> Policy {
    let mut p = read();
    f(&mut p);
    save(&p);
    p
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    Banner { sound: bool },
    /// into the notifications list, no banner (Do not disturb)
    ListOnly,
    /// waits for the agent's digest (in the list, marked)
    Digest,
    /// the app is off
    Drop,
}

/// What a notification means to the person.
#[derive(Clone, Copy, Debug, Default)]
pub struct Notice<'a> {
    pub app: &'a str,
    pub title: &'a str,
    pub body: &'a str,
    /// urgency "critical" (alarms, a battery about to end)
    pub critical: bool,
    /// an approval the agent waits for
    pub approval: bool,
}

/// Mentions someone the person marked as important.
pub fn important(p: &Policy, n: &Notice) -> bool {
    p.people.iter().map(|x| x.trim()).filter(|x| !x.is_empty()).any(|x| {
        let x = x.to_lowercase();
        n.title.to_lowercase().contains(&x) || n.body.to_lowercase().contains(&x)
    })
}

pub fn route(p: &Policy, n: &Notice, dnd: bool) -> Route {
    let rule = p.apps.get(n.app).copied().unwrap_or_default();
    let important = important(p, n);
    if !rule.on && !n.approval {
        return Route::Drop;
    }
    let breaks = (n.critical && p.breaks.alarms) || (n.approval && p.breaks.approvals) || (important && p.breaks.people);
    if dnd && !breaks {
        return Route::ListOnly;
    }
    if rule.mode == Mode::DigestOnly && p.agent_digest && !important && !n.critical && !n.approval {
        return Route::Digest;
    }
    // with its sound, or when it matters more: an alarm, someone important
    Route::Banner { sound: rule.mode == Mode::BannersSound || n.critical || important }
}

/// Do not disturb by itself: from 22:00 to 07:00, or while the focus timer runs.
pub fn auto_dnd(p: &Policy, hour: u32, focus_running: bool) -> bool {
    p.dnd_auto && (!(7..22).contains(&hour) || focus_running)
}

/// The agent's digest goes out at these hours.
pub const DIGEST_HOURS: [u32; 2] = [12, 18];

// ------------------------------------------------------------ the log (a week)

fn log_path() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).filter(|p| p.is_absolute())
        .unwrap_or_else(|| gtk::glib::home_dir().join(".local/state")).join("kalango/notificacoes.jsonl")
}

pub fn route_name(r: Route) -> &'static str {
    match r {
        Route::Banner { .. } => "banner",
        Route::ListOnly => "lista",
        Route::Digest => "resumo",
        Route::Drop => "descartada",
    }
}

/// Keeps one notification's app (and icon) and route.
pub fn log(app: &str, icon: &str, r: Route) {
    let now = gtk::glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0);
    let p = log_path();
    let keep: String = std::fs::read_to_string(&p).unwrap_or_default().lines()
        .filter(|l| serde_json::from_str::<Value>(l).ok().and_then(|v| v["t"].as_i64()).is_some_and(|t| t > now - 7 * 86_400))
        .map(|l| format!("{l}\n")).collect();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(&p, keep + &json!({"t": now, "app": app, "icone": icon, "rota": route_name(r)}).to_string() + "\n");
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Counts {
    pub today: usize,
    pub digested: usize,
    /// the apps that sent notifications this week, most first: (app, icon, how many)
    pub apps: Vec<(String, String, usize)>,
}

pub fn counts_from(text: &str, day_start: i64) -> Counts {
    let mut c = Counts::default();
    let mut apps: BTreeMap<String, (String, usize)> = BTreeMap::new();
    for v in text.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()) {
        let t = v["t"].as_i64().unwrap_or(0);
        let app = v["app"].as_str().unwrap_or("").to_string();
        if t >= day_start {
            c.today += 1;
            if v["rota"] == "resumo" {
                c.digested += 1;
            }
        }
        if !app.is_empty() {
            let e = apps.entry(app).or_default();
            if let Some(i) = v["icone"].as_str().filter(|i| !i.is_empty()) {
                e.0 = i.to_string();
            }
            e.1 += 1;
        }
    }
    let mut v: Vec<(String, String, usize)> = apps.into_iter().map(|(a, (i, n))| (a, i, n)).collect();
    v.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(&b.0)));
    c.apps = v;
    c
}

pub fn counts() -> Counts {
    let now = gtk::glib::DateTime::now_local();
    let day_start = now.as_ref().ok().and_then(|d| gtk::glib::DateTime::from_local(d.year(), d.month(), d.day_of_month(), 0, 0, 0.0).ok()).map(|d| d.to_unix()).unwrap_or(0);
    counts_from(&std::fs::read_to_string(log_path()).unwrap_or_default(), day_start)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notice<'a>(app: &'a str, title: &'a str) -> Notice<'a> {
        Notice { app, title, body: "", critical: false, approval: false }
    }

    #[test]
    fn each_app_rule_and_do_not_disturb_route_a_notification() {
        let mut p = Policy { people: vec!["Marina".into()], ..Policy::default() };
        p.apps.insert("Música".into(), AppRule { on: false, mode: Mode::BannersSound });
        p.apps.insert("Correio".into(), AppRule { on: true, mode: Mode::DigestOnly });
        p.apps.insert("Discord".into(), AppRule { on: true, mode: Mode::Banners });
        assert_eq!(route(&p, &notice("Música", "Tocando"), false), Route::Drop);
        assert_eq!(route(&p, &notice("Correio", "Promoção"), false), Route::Digest);
        // someone important is never held for the digest, and breaks through Do not disturb
        assert_eq!(route(&p, &notice("Correio", "Marina: almoço?"), false), Route::Banner { sound: true });
        assert_eq!(route(&p, &notice("Correio", "marina escreveu"), true), Route::Banner { sound: true });
        assert_eq!(route(&p, &notice("Discord", "nova mensagem"), false), Route::Banner { sound: false });
        assert_eq!(route(&p, &notice("Discord", "nova mensagem"), true), Route::ListOnly);
        let alarme = Notice { critical: true, ..notice("Relógio", "Alarme") };
        assert_eq!(route(&p, &alarme, true), Route::Banner { sound: true });
        p.breaks.alarms = false;
        assert_eq!(route(&p, &alarme, true), Route::ListOnly);
        // the agent's approvals show even from an app turned off
        p.apps.insert("Agentes".into(), AppRule { on: false, mode: Mode::Banners });
        assert_eq!(route(&p, &Notice { approval: true, ..notice("Agentes", "Aprovar?") }, true), Route::Banner { sound: false });
        // without the agent's digest, "digest only" shows as banners
        p.agent_digest = false;
        assert_eq!(route(&p, &notice("Correio", "Promoção"), false), Route::Banner { sound: false });
    }

    #[test]
    fn do_not_disturb_by_itself_at_night_and_in_focus() {
        let p = Policy::default();
        assert!(auto_dnd(&p, 23, false) && auto_dnd(&p, 6, false) && auto_dnd(&p, 14, true));
        assert!(!auto_dnd(&p, 7, false) && !auto_dnd(&p, 21, false));
        assert!(!auto_dnd(&Policy { dnd_auto: false, ..p }, 23, true));
    }

    #[test]
    fn the_log_counts_today_the_digested_and_the_apps() {
        let t = [(50, "Discord", "banner"), (150, "Correio", "resumo"), (160, "Correio", "resumo"), (170, "Discord", "lista")]
            .iter().map(|(t, a, r)| json!({"t": t, "app": a, "icone": "x", "rota": r}).to_string()).collect::<Vec<_>>().join("\n");
        let c = counts_from(&t, 100);
        assert_eq!((c.today, c.digested), (3, 2));
        assert_eq!(c.apps.iter().map(|(a, _, n)| (a.as_str(), *n)).collect::<Vec<_>>(), vec![("Correio", 2), ("Discord", 2)]);
    }
}
