//! Configurações › Energia (design "Configurações · Energia"): the power mode (power-profiles-daemon) and letting
//! the agent choose it (performance while the local model works); what the computer drew in the last 24 hours and
//! which apps drew most — sampled every minute by the shell (the processor's energy counter through kalangod, the
//! graphics card's draw from nvidia-smi, each app's share of them by its processor time); and in the panel the
//! battery (where there is one), when the screens go dark and the computer suspends (the compositor's idle
//! notifications), the agents keeping it awake (logind), and the graphics card in use (prime-select, through kalangod
//! with the person's approval).

use crate::configuracoes::{ajustes, depurando, interruptor, salvar_ajuste};
use crate::configuracoes_moldura::Moldura;
use crate::conversa::ui_icon;
use crate::superficies::{caixa, estrela, rotulo};
use gtk::prelude::*;
use gtk::{glib, Align, Orientation};
use kalango_i18n::{n_, tr, trf};
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

/// power-profiles-daemon's command (a stand-in in the session test).
pub fn powerprofilesctl() -> String {
    std::env::var("KALANGO_POWERPROFILESCTL").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "powerprofilesctl".into())
}

/// UPower's command (a stand-in with a laptop's battery in the session test).
fn upower_cmd() -> String {
    std::env::var("KALANGO_UPOWER").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "upower".into())
}

/// Where the batteries are (sysfs; a folder of the session test's own there).
fn power_supply() -> PathBuf {
    std::env::var("KALANGO_POWER_SUPPLY").ok().filter(|s| !s.is_empty()).map(PathBuf::from).unwrap_or_else(|| "/sys/class/power_supply".into())
}

/// The laptop's lid (ACPI; a folder of the session test's own there).
fn has_lid() -> bool {
    let d = std::env::var("KALANGO_ACPI_LID").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/proc/acpi/button/lid".into());
    std::fs::read_dir(d).map(|mut d| d.next().is_some()).unwrap_or(false)
}

/// What closing the lid does now (the file kalangod writes; logind's default otherwise).
fn lid_action_now() -> &'static str {
    let current = std::fs::read_to_string("/etc/systemd/logind.conf.d/50-kalango-tampa.conf").ok()
        .and_then(|t| t.lines().find_map(|l| l.trim().strip_prefix("HandleLidSwitch=").map(str::to_string)));
    match current.as_deref() {
        Some("poweroff") => "desligar",
        Some("ignore") => "nada",
        _ => "suspender",
    }
}

/// The charge limit: Some(on) where the battery has one.
fn charge_limit_now() -> Option<bool> {
    let d = std::fs::read_dir(power_supply()).ok()?;
    d.flatten().find_map(|e| std::fs::read_to_string(e.path().join("charge_control_end_threshold")).ok())
        .and_then(|t| t.trim().parse::<u32>().ok()).map(|v| v < 100)
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Battery {
    pub pct: f64,
    pub charging: bool,
    pub full: bool,
    /// minutes until full (charging) or empty
    pub minutes: Option<u32>,
    /// the battery's health (capacity), %
    pub health: Option<u32>,
}

/// One of UPower's times ("35,0 minutes", "1,5 hours") in minutes.
fn minutes_of(t: &str) -> Option<u32> {
    let mut p = t.split_whitespace();
    let n: f64 = p.next()?.replace(',', ".").parse().ok()?;
    let m = match p.next()? {
        u if u.starts_with("hour") => n * 60.0,
        u if u.starts_with("day") => n * 1440.0,
        u if u.starts_with("second") => n / 60.0,
        _ => n,
    };
    Some(m.round() as u32)
}

pub fn parse_upower(text: &str) -> Option<Battery> {
    let field = |k: &str| text.lines().find_map(|l| l.trim().strip_prefix(k).map(|x| x.trim().to_string()));
    if field("present:").as_deref() != Some("yes") {
        return None;
    }
    let state = field("state:").unwrap_or_default();
    Some(Battery {
        pct: field("percentage:")?.trim_end_matches('%').replace(',', ".").parse().ok()?,
        charging: state == "charging",
        full: state == "fully-charged",
        minutes: field(if state == "charging" { "time to full:" } else { "time to empty:" }).and_then(|t| minutes_of(&t)),
        health: field("capacity:").and_then(|c| c.trim_end_matches('%').replace(',', ".").parse::<f64>().ok()).map(|c| c.round() as u32),
    })
}

pub(crate) fn read_battery() -> Option<Battery> {
    let o = std::process::Command::new(upower_cmd()).arg("-e").env("LC_ALL", "C").output().ok()?;
    let path = String::from_utf8_lossy(&o.stdout).lines().find(|l| l.contains("/battery_"))?.trim().to_string();
    let i = std::process::Command::new(upower_cmd()).args(["-i", &path]).env("LC_ALL", "C").output().ok()?;
    parse_upower(&String::from_utf8_lossy(&i.stdout))
}

/// A minute, in milliseconds (the session test makes it shorter, to see the idle times and the samples happen).
pub fn minute_ms() -> u64 {
    std::env::var("KALANGO_MINUTO_MS").ok().and_then(|s| s.parse().ok()).filter(|m| *m > 0).unwrap_or(60_000)
}

// ------------------------------------------------------------ the power history (a sample a minute, 24 h kept)

fn history_path() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).filter(|p| p.is_absolute())
        .unwrap_or_else(|| glib::home_dir().join(".local/state")).join("kalango/energia.jsonl")
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sample {
    pub t: i64,
    /// the whole draw in watts (what could be measured)
    pub w: f64,
    /// what it drew since the sample before, in watt-hours
    pub wh: f64,
    /// watt-hours of this minute, by app
    pub apps: Vec<(String, f64)>,
}

pub fn parse_history(text: &str, desde: i64) -> Vec<Sample> {
    text.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()).filter_map(|v| Some(Sample {
        t: v["t"].as_i64()?,
        w: v["w"].as_f64().unwrap_or(0.0),
        wh: v["wh"].as_f64().or_else(|| Some(v["w"].as_f64()? / 60.0)).unwrap_or(0.0),
        apps: v["apps"].as_object().map(|o| o.iter().filter_map(|(k, x)| Some((k.clone(), x.as_f64()?))).collect()).unwrap_or_default(),
    })).filter(|a| a.t >= desde).collect()
}

/// kWh, the mean and the peak (and when) of a day's samples.
pub fn summarize(samples: &[Sample]) -> (f64, f64, Option<(f64, i64)>) {
    if samples.is_empty() {
        return (0.0, 0.0, None);
    }
    let kwh = samples.iter().map(|a| a.wh).sum::<f64>() / 1000.0;
    let mean_label = samples.iter().map(|a| a.w).sum::<f64>() / samples.len() as f64;
    let pico = samples.iter().max_by(|a, b| a.w.total_cmp(&b.w)).map(|a| (a.w, a.t));
    (kwh, mean_label, pico)
}

/// The apps that drew most (kWh), the first three.
pub fn top_apps(samples: &[Sample]) -> Vec<(String, f64)> {
    let mut sum: HashMap<String, f64> = HashMap::new();
    for a in samples {
        for (n, wh) in &a.apps {
            *sum.entry(n.clone()).or_default() += wh / 1000.0;
        }
    }
    let mut v: Vec<_> = sum.into_iter().filter(|(_, k)| *k > 0.0).collect();
    v.sort_by(|a, b| b.1.total_cmp(&a.1));
    v.truncate(3);
    v
}

/// Each process's processor time now (ticks), by its app's name.
fn cpu_time_by_app() -> HashMap<String, u64> {
    let mut m = HashMap::new();
    let Ok(d) = std::fs::read_dir("/proc") else { return m };
    for e in d.flatten() {
        let Some(pid) = e.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else { continue };
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else { continue };
        let Some(end) = stat.rfind(')') else { continue };
        let nome = stat[stat.find('(').map(|i| i + 1).unwrap_or(0)..end].to_string();
        let f: Vec<&str> = stat[end + 2..].split_whitespace().collect();
        let t = f.get(11).and_then(|x| x.parse::<u64>().ok()).unwrap_or(0) + f.get(12).and_then(|x| x.parse::<u64>().ok()).unwrap_or(0);
        *m.entry(app_name(&nome)).or_default() += t;
    }
    m
}

/// A process's name as a person knows its app.
fn app_name(comm: &str) -> String {
    match comm {
        "llama-server" | "llama-cli" | "kalango-agente" => tr("Local model").to_string(),
        "chrome" | "chromium" | "google-chrome" => "Chrome".into(),
        "firefox" | "firefox-bin" | "Isolated Web Co" | "Web Content" => "Firefox".into(),
        "cargo" | "rustc" | "cc1" | "ld" => tr("Builds (cargo)").to_string(),
        c => {
            let mut s = c.to_string();
            if let Some(p) = s.get_mut(0..1) {
                p.make_ascii_uppercase();
            }
            s
        }
    }
}

thread_local! {
    /// the last sample's counters: (time, processor µJ, processor ticks by app)
    static LAST_COUNTERS: RefCell<Option<(i64, Option<u64>, HashMap<String, u64>)>> = RefCell::default();
}

/// Every minute: what the computer drew since the last minute, kept with each app's share. (In the background:
/// kalangod, nvidia-smi and /proc are read off the interface thread.)
pub fn sample_power() {
    let before = LAST_COUNTERS.with(|a| a.borrow().clone());
    crate::lateral::em_segundo_plano(move || {
        let now = glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0);
        let cpu_uj = crate::servicos::kalangod_rpc("kalango/energia", json!({})).ok().and_then(|v| v["cpu_uj"].as_u64())
            .or_else(|| std::fs::read_to_string("/sys/class/powercap/intel-rapl:0/energy_uj").ok().and_then(|t| t.trim().parse().ok()));
        let gpu_w = std::process::Command::new("nvidia-smi").args(["--query-gpu=power.draw", "--format=csv,noheader,nounits"]).output().ok()
            .and_then(|o| String::from_utf8_lossy(&o.stdout).lines().next()?.trim().parse::<f64>().ok());
        let times = cpu_time_by_app();
        let mut sample = None;
        if let Some((t0, uj0, times0)) = before {
            let dt = (now - t0).max(1) as f64;
            let cpu_w = match (uj0, cpu_uj) {
                (Some(a), Some(b)) if b >= a => Some((b - a) as f64 / 1e6 / dt),
                _ => None,
            };
            let w = cpu_w.unwrap_or(0.0) + gpu_w.unwrap_or(0.0);
            // each app's share of the processor's time in this minute; the graphics card's draw goes with it
            let deltas: Vec<(String, u64)> = times.iter().map(|(n, t)| (n.clone(), t.saturating_sub(*times0.get(n).unwrap_or(t)))).filter(|(_, d)| *d > 0).collect();
            let total: u64 = deltas.iter().map(|(_, d)| d).sum();
            let wh = w * dt / 3600.0;
            let apps: Vec<(String, f64)> = if total > 0 { deltas.iter().map(|(n, d)| (n.clone(), wh * *d as f64 / total as f64)).filter(|(_, x)| *x > 0.001).collect() } else { vec![] };
            if w > 0.0 {
                let row = json!({"t": now, "w": (w * 10.0).round() / 10.0, "wh": (wh * 10_000.0).round() / 10_000.0, "cpu": cpu_w.is_some(), "apps": apps.iter().map(|(n, x)| (n.clone(), json!((x * 1000.0).round() / 1000.0))).collect::<serde_json::Map<_, _>>()});
                // the file keeps a day (and a bit): older lines go when it is rewritten
                let p = history_path();
                let keep = parse_history(&std::fs::read_to_string(&p).unwrap_or_default(), now - 26 * 3600);
                let mut text: String = keep.iter().map(|a| json!({"t": a.t, "w": a.w, "wh": a.wh, "apps": a.apps.iter().map(|(n, x)| (n.clone(), json!(x))).collect::<serde_json::Map<_, _>>()}).to_string() + "\n").collect();
                text += &(row.to_string() + "\n");
                if let Some(d) = p.parent() {
                    let _ = std::fs::create_dir_all(d);
                }
                let _ = std::fs::write(&p, text);
                sample = Some(w);
            }
        }
        (now, cpu_uj, times, sample)
    }, |(now, uj, times, w)| {
        LAST_COUNTERS.with(|a| *a.borrow_mut() = Some((now, uj, times)));
        if depurando() {
            if let Some(w) = w {
                eprintln!("debug energia: amostra {w:.1} W");
            }
        }
    });
}

// ------------------------------------------------------------ idle, agents awake, the agent choosing the mode

thread_local! {
    static IDLE_WATCH: RefCell<Option<(std::sync::mpsc::Receiver<(u64, crate::idle::IdleEvent)>, u64)>> = RefCell::default();
    /// logind's inhibitor while an agent works: (kept awake, the inhibitor's process)
    static INHIBITOR: RefCell<(bool, Option<std::process::Child>)> = RefCell::default();
    /// the mode before the agent chose performance, to give back
    static MODE_BEFORE: RefCell<Option<String>> = RefCell::default();
}

/// Minutes of the idle times (0: never).
fn minutes(key: &str, default_min: u64) -> u64 {
    ajustes()[key].as_u64().unwrap_or(default_min)
}

/// Whether this is the live session of the install media (the kernel line has casper's `boot=casper`).
pub(crate) fn live_session() -> bool {
    is_live_cmdline(&std::fs::read_to_string("/proc/cmdline").unwrap_or_default())
}

fn is_live_cmdline(cmdline: &str) -> bool {
    cmdline.split_whitespace().any(|w| w == "boot=casper")
}

/// Watches the idle times chosen (again when they change).
pub fn watch_idle() {
    let live = live_session();
    let (screen_off, suspend_after) = (minutes("screen_off_min", 10), minutes("suspend_min", 30));
    // Privacy › automatic lock (0: never); 5 minutes unless chosen — but not in the development and test sessions,
    // whose minutes are seconds and whose screen is someone's desktop, nor in the live session of the install media:
    // its account has no password (the lock screen could not be opened again) and an install would go on behind it
    let lock_after = if live {
        0
    } else {
        minutes("bloqueio_apos_min", if std::env::var_os("KALANGO_SHELL_DEV").is_some() { 0 } else { 5 })
    };
    let generation = (lock_after << 32) | (screen_off << 16) | suspend_after;
    if IDLE_WATCH.with(|o| o.borrow().as_ref().map(|(_, g)| *g)) == Some(generation) {
        return;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let mut times = vec![(1, (screen_off * minute_ms()) as u32)];
    // (live, never suspend: an install copying to the disk would stop halfway)
    if !live {
        times.push((2, (suspend_after * minute_ms()) as u32));
    }
    if lock_after > 0 {
        times.push((3, (lock_after * minute_ms()) as u32));
    }
    match crate::idle::watch(times, generation, tx) {
        Ok(()) => {
            IDLE_WATCH.with(|o| *o.borrow_mut() = Some((rx, generation)));
            if depurando() {
                eprintln!("debug energia: ocioso em {screen_off} min (tela) e {suspend_after} min (suspender), bloqueio em {lock_after} min");
            }
        }
        Err(e) => eprintln!("kalango-shell: notificações de ociosidade: {e}"),
    }
}

/// Twice a second: the idle events, the agents keeping the computer awake, the agent choosing the mode.
pub fn watch(shell: &crate::Shell) {
    let events: Vec<crate::idle::IdleEvent> = IDLE_WATCH.with(|o| {
        let o = o.borrow();
        let Some((rx, g)) = o.as_ref() else { return vec![] };
        rx.try_iter().filter(|(gen, _)| gen == g).map(|(_, e)| e).collect()
    });
    let working = shell.estado.agent_task.borrow().is_some();
    for e in events {
        match e {
            crate::idle::IdleEvent::Idle(1) => {
                let _ = crate::servicos::niri_acao(json!({"PowerOffMonitors": {}}));
                if depurando() {
                    eprintln!("debug energia: ocioso, telas apagadas");
                }
            }
            crate::idle::IdleEvent::Idle(3) => {
                if depurando() {
                    eprintln!("debug energia: ocioso, bloqueando");
                }
                shell.bloqueio().bloquear();
            }
            crate::idle::IdleEvent::Idle(2) => {
                if working && ajustes()["agents_keep_awake"].as_bool().unwrap_or(true) {
                    if depurando() {
                        eprintln!("debug energia: ocioso, mas um agente trabalha: não suspende");
                    }
                } else {
                    if depurando() {
                        eprintln!("debug energia: ocioso, suspendendo");
                    }
                    crate::energia::Energia::fazer("suspender", false);
                }
            }
            _ => {}
        }
    }
    // the agents keep it awake: logind's inhibitor while one works
    let keep = working && ajustes()["agents_keep_awake"].as_bool().unwrap_or(true);
    INHIBITOR.with(|i| {
        let mut i = i.borrow_mut();
        match (keep, i.0) {
            (true, false) => {
                i.0 = true;
                if std::env::var_os("KALANGO_SHELL_DEV").is_some() {
                    // (the development session runs on someone's desktop: its sleep stays its own)
                    eprintln!("kalango-shell: [sessão de teste] o inibidor de suspensão não foi pedido ao logind");
                } else {
                    i.1 = std::process::Command::new("systemd-inhibit").args(["--what=sleep:idle", "--who=KalangoOS", &format!("--why={}", tr("An agent is working")), "--mode=block", "sleep", "infinity"])
                        .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().ok();
                }
                if depurando() {
                    eprintln!("debug energia: agente trabalhando, computador acordado");
                }
            }
            (false, true) => {
                i.0 = false;
                if let Some(mut c) = i.1.take() {
                    let _ = c.kill();
                    let _ = c.wait();
                }
                if depurando() {
                    eprintln!("debug energia: agente parou, suspensão liberada");
                }
            }
            _ => {}
        }
    });
    // the agent chooses the mode: performance while the local model works, the person's mode back after
    if ajustes()["agent_picks_mode"].as_bool().unwrap_or(true) {
        let before = MODE_BEFORE.with(|m| m.borrow().clone());
        match (working, before) {
            (true, None) => {
                let current = std::process::Command::new(powerprofilesctl()).arg("get").output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
                if !current.is_empty() && current != "performance" {
                    MODE_BEFORE.with(|m| *m.borrow_mut() = Some(current));
                    if on_host_power() {
                        // (the development session runs on someone's desktop: its power mode stays its own)
                        eprintln!("kalango-shell: [sessão de teste] o agente escolheria desempenho");
                        return;
                    }
                    std::thread::spawn(|| crate::sistema::perfil("performance"));
                    crate::osd::modo_de_energia(kalango_i18n::tr("Performance"));
                    if depurando() {
                        eprintln!("debug energia: o agente escolheu desempenho");
                    }
                }
            }
            (false, Some(m)) => {
                MODE_BEFORE.with(|x| *x.borrow_mut() = None);
                let m2 = m.clone();
                if !on_host_power() {
                    std::thread::spawn(move || crate::sistema::perfil(&m2));
                }
                if depurando() {
                    eprintln!("debug energia: o modo da pessoa voltou ({m})");
                }
            }
            _ => {}
        }
    }
}

/// A development session using the computer's real power-profiles-daemon (no stand-in): the agent's own choices
/// stay out of it.
fn on_host_power() -> bool {
    std::env::var_os("KALANGO_SHELL_DEV").is_some() && std::env::var_os("KALANGO_POWERPROFILESCTL").is_none_or(|v| v.is_empty())
}

// ------------------------------------------------------------ the page

/// (power-profiles-daemon's id, name, what it does); each has its colour in the style sheet
const MODES: &[(&str, &str, &str)] = &[
    ("power-saver", n_("Saver"), n_("quieter, draws less")),
    ("balanced", n_("Balanced"), n_("adjusts itself to the use")),
    ("performance", n_("Performance"), n_("everything at most: games and AI models")),
];

const TIMEOUTS: &[u64] = &[0, 1, 2, 5, 10, 15, 30, 60];

fn timeout_name(min: u64) -> String {
    match min {
        0 => tr("never").to_string(),
        60 => tr("after 1 h").to_string(),
        m => trf("after {n} min", &[("n", m.to_string())]),
    }
}

pub struct PowerPage {
    pub frame: Rc<Moldura>,
    modes: Vec<(&'static str, gtk::Button)>,
    agent_switch: gtk::Switch,
    total: gtk::Label,
    mean_label: gtk::Label,
    chart: gtk::DrawingArea,
    apps: gtk::Box,
    battery_card: gtk::Box,
    battery_ring: gtk::DrawingArea,
    battery_text: gtk::Label,
    battery_sub: gtk::Label,
    charge_limit: gtk::Switch,
    charge_limit_row: gtk::Box,
    screen_off: gtk::DropDown,
    lid: gtk::DropDown,
    lid_row: gtk::Box,
    suspend_after: gtk::DropDown,
    keep_awake: gtk::Switch,
    gpu: Vec<(&'static str, gtk::ToggleButton)>,
    gpu_card: gtk::Box,
    gpu_use: gtk::Label,
    gpu_note: gtk::Label,
    samples: RefCell<Vec<Sample>>,
    battery_pct: Cell<f64>,
    loading: Cell<bool>,
    timer: RefCell<Option<glib::SourceId>>,
}

fn left_label(t: &str, c: &str) -> gtk::Label {
    let l = rotulo(t, c);
    l.set_xalign(0.0);
    l
}

fn label16(glifo: &str, text: &str) -> gtk::Box {
    let r = caixa(Orientation::Horizontal, 6, "");
    r.append(&ui_icon(glifo, "#8E8F99", 16));
    r.append(&rotulo(text, "cfg-rotulo"));
    r
}

fn row(titulo: &str, sub: Option<&str>, controle: &impl IsA<gtk::Widget>) -> gtk::Box {
    let r = caixa(Orientation::Horizontal, 12, "cfg-tela-opcao");
    let t = caixa(Orientation::Vertical, 1, "");
    t.set_hexpand(true);
    t.set_valign(Align::Center);
    t.append(&left_label(titulo, "cfg-texto-13"));
    if let Some(s) = sub {
        t.append(&left_label(s, "cfg-sub"));
        r.add_css_class("alta");
    }
    r.append(&t);
    controle.set_valign(Align::Center);
    r.append(controle);
    r
}

fn timeout_selector() -> gtk::DropDown {
    let nomes: Vec<String> = TIMEOUTS.iter().map(|m| timeout_name(*m)).collect();
    let d = gtk::DropDown::new(Some(gtk::StringList::new(&nomes.iter().map(String::as_str).collect::<Vec<_>>())), None::<gtk::Expression>);
    d.add_css_class("cfg-seletor");
    d.set_size_request(160, 34);
    d
}

impl PowerPage {
    pub fn new(c: &Rc<crate::configuracoes::Configuracoes>) -> Rc<PowerPage> {
        let m = Moldura::new(tr("Power"), &[], tr("Battery and suspend"));
        m.busca_grande();
        // the modes
        let mode_card = caixa(Orientation::Vertical, 12, "cfg-cartao");
        mode_card.append(&label16("desligar-d", tr("POWER MODE")));
        let mode_row = caixa(Orientation::Horizontal, 10, "");
        mode_row.set_homogeneous(true);
        let mut modes = Vec::new();
        for (id, nome, sub) in MODES {
            let b = gtk::Button::new();
            b.add_css_class("cfg-som-cartao");
            b.set_valign(Align::Start);
            let column = caixa(Orientation::Vertical, 6, "");
            let p = caixa(Orientation::Horizontal, 0, "cfg-energia-ponto");
            p.set_size_request(8, 8);
            p.set_halign(Align::Start);
            p.add_css_class(match *id { "power-saver" => "economia", "performance" => "desempenho", _ => "equilibrado" });
            column.append(&p);
            column.append(&left_label(tr(nome), "cfg-nome-13"));
            // (on a small screen the three cards narrow: the line ends in "…")
            let what = left_label(tr(sub), "cfg-sub");
            what.set_ellipsize(gtk::pango::EllipsizeMode::End);
            column.append(&what);
            b.set_child(Some(&column));
            mode_row.append(&b);
            modes.push((*id, b));
        }
        mode_card.append(&mode_row);
        let agent_switch = interruptor();
        let agent_row = caixa(Orientation::Horizontal, 12, "cfg-tela-opcao");
        agent_row.add_css_class("alta");
        let spark_box = caixa(Orientation::Horizontal, 0, "");
        spark_box.set_size_request(16, 16);
        spark_box.set_valign(Align::Center);
        spark_box.set_hexpand(false);
        let spark = estrela(13, "#A5A1E2");
        // (the design's spark starts a pixel in from its 16 px frame)
        spark.set_halign(Align::Start);
        spark.set_margin_start(1);
        spark.set_valign(Align::Center);
        spark_box.append(&spark);
        agent_row.append(&spark_box);
        let agent_texts = caixa(Orientation::Vertical, 1, "");
        agent_texts.set_hexpand(true);
        agent_texts.set_valign(Align::Center);
        agent_texts.append(&left_label(tr("Let the agent choose the mode"), "cfg-texto-13"));
        agent_texts.append(&left_label(tr("performance while a local model is working"), "cfg-sub"));
        agent_row.append(&agent_texts);
        agent_switch.set_valign(Align::Center);
        agent_row.append(&agent_switch);
        mode_card.append(&agent_row);
        m.corpo.append(&mode_card);
        // the last 24 h
        let usage_card = caixa(Orientation::Vertical, 10, "cfg-cartao");
        usage_card.append(&label16("raio-e", tr("DRAW IN THE LAST 24 H")));
        let total_row = caixa(Orientation::Horizontal, 8, "");
        let total = rotulo("", "cfg-energia-total");
        total_row.append(&total);
        let mean_label = rotulo("", "cfg-sub");
        mean_label.set_valign(Align::End);
        total_row.append(&mean_label);
        usage_card.append(&total_row);
        let chart = gtk::DrawingArea::new();
        chart.set_content_height(135);
        chart.set_hexpand(true);
        usage_card.append(&chart);
        let apps = caixa(Orientation::Vertical, 10, "");
        usage_card.append(&apps);
        m.corpo.append(&usage_card);
        // the panel: battery
        let battery_card = caixa(Orientation::Vertical, 8, "cfg-cartao");
        battery_card.append(&label16("bateria-d", tr("BATTERY")));
        let battery_row = caixa(Orientation::Horizontal, 12, "");
        let battery_ring = gtk::DrawingArea::new();
        battery_ring.set_content_width(64);
        battery_ring.set_content_height(64);
        battery_row.append(&battery_ring);
        let battery_texts = caixa(Orientation::Vertical, 2, "");
        battery_texts.set_valign(Align::Center);
        let battery_text = left_label("", "cfg-nome-13");
        battery_texts.append(&battery_text);
        let battery_sub = left_label("", "cfg-sub");
        battery_texts.append(&battery_sub);
        battery_row.append(&battery_texts);
        battery_card.append(&battery_row);
        let charge_limit = interruptor();
        let charge_limit_row = row(tr("Limit the charge to 80%"), Some(tr("makes the battery last more years")), &charge_limit);
        battery_card.append(&charge_limit_row);
        let rules = caixa(Orientation::Vertical, 16, "");
        m.painel.append(&rules);
        rules.append(&battery_card);
        // suspend
        let suspend_card = caixa(Orientation::Vertical, 2, "cfg-cartao");
        suspend_card.append(&label16("lua-d", tr("SUSPEND")));
        let screen_off = timeout_selector();
        suspend_card.append(&row(tr("Turn off the screen"), None, &screen_off));
        let suspend_after = timeout_selector();
        suspend_card.append(&row(tr("Suspend"), None, &suspend_after));
        let lid = gtk::DropDown::new(Some(gtk::StringList::new(&[tr("suspend"), tr("power off"), tr("do nothing")])), None::<gtk::Expression>);
        lid.add_css_class("cfg-seletor");
        lid.set_size_request(160, 34);
        let lid_row = row(tr("When closing the lid"), None, &lid);
        suspend_card.append(&lid_row);
        let keep_awake = interruptor();
        suspend_card.append(&row(tr("Agents keep the computer awake"), Some(tr("only until the task at hand ends")), &keep_awake));
        rules.append(&suspend_card);
        // the graphics card
        let gpu_card = caixa(Orientation::Vertical, 8, "cfg-cartao");
        gpu_card.append(&label16("cpu-d", tr("GRAPHICS CARD")));
        let segmented = caixa(Orientation::Horizontal, 2, "cfg-segmentado");
        segmented.add_css_class("escuro");
        segmented.set_homogeneous(true);
        let mut gpu: Vec<(&'static str, gtk::ToggleButton)> = Vec::new();
        for (id, nome) in [("hibrido", tr("Hybrid")), ("dedicado", tr("Dedicated")), ("integrado", tr("Integrated"))] {
            let b = gtk::ToggleButton::with_label(nome);
            b.add_css_class("cfg-seg");
            if let Some((_, p)) = gpu.first() {
                b.set_group(Some(p));
            }
            segmented.append(&b);
            gpu.push((id, b));
        }
        gpu_card.append(&segmented);
        let gpu_row = caixa(Orientation::Horizontal, 8, "");
        let gpu_label = left_label(tr("In use now"), "cfg-sub");
        gpu_label.set_hexpand(true);
        gpu_row.append(&gpu_label);
        let gpu_use = rotulo("", "cfg-valor-pequeno");
        gpu_row.append(&gpu_use);
        gpu_card.append(&gpu_row);
        let gpu_note = left_label("", "cfg-sub");
        gpu_note.set_wrap(true);
        gpu_note.set_visible(false);
        gpu_card.append(&gpu_note);
        rules.append(&gpu_card);
        let e = Rc::new(PowerPage { frame: m, modes, agent_switch, total, mean_label, chart, apps, battery_card, battery_ring, battery_text, battery_sub, charge_limit, charge_limit_row, screen_off, lid, lid_row, suspend_after, keep_awake, gpu, gpu_card,
            gpu_use, gpu_note, samples: RefCell::default(), battery_pct: Cell::new(0.0), loading: Cell::new(false), timer: RefCell::default() });
        e.connect_signals(c);
        e
    }

    fn connect_signals(self: &Rc<Self>, c: &Rc<crate::configuracoes::Configuracoes>) {
        for (id, b) in &self.modes {
            let (w, id) = (Rc::downgrade(self), *id);
            b.connect_clicked(move |_| {
                let Some(e) = w.upgrade() else { return };
                let w = Rc::downgrade(&e);
                crate::lateral::em_segundo_plano(move || crate::sistema::perfil(id), move |_| {
                    if depurando() {
                        eprintln!("debug energia: escolhido {id}");
                    }
                    if let Some(e) = w.upgrade() {
                        e.refresh();
                    }
                });
            });
            crate::configuracoes::registrar_posicao(c, &format!("cfg energia modo {id}"), b.upcast_ref());
        }
        for (sw, key) in [(&self.agent_switch, "agent_picks_mode"), (&self.keep_awake, "agents_keep_awake")] {
            let w = Rc::downgrade(self);
            sw.connect_state_set(move |_, on| {
                if w.upgrade().is_some_and(|e| !e.loading.get()) {
                    salvar_ajuste(key, json!(on));
                    if depurando() {
                        eprintln!("debug energia: {key} {on}");
                    }
                }
                glib::Propagation::Proceed
            });
        }
        for (d, key) in [(&self.screen_off, "screen_off_min"), (&self.suspend_after, "suspend_min")] {
            let w = Rc::downgrade(self);
            d.connect_selected_notify(move |d| {
                if w.upgrade().is_some_and(|e| !e.loading.get()) {
                    let m = TIMEOUTS.get(d.selected() as usize).copied().unwrap_or(0);
                    salvar_ajuste(key, json!(m));
                    watch_idle();
                    if depurando() {
                        eprintln!("debug energia: {key} {m}");
                    }
                }
            });
        }
        let w = Rc::downgrade(self);
        self.charge_limit.connect_state_set(move |sw, on| {
            let Some(e) = w.upgrade().filter(|e| !e.loading.get()) else { return glib::Propagation::Proceed };
            // through kalangod: the system asks the person
            let (w, sw) = (Rc::downgrade(&e), sw.clone());
            crate::lateral::em_segundo_plano(move || crate::servicos::kalangod_chamar("energia.limite_carga", json!({"ligado": on})), move |r| {
                if depurando() {
                    eprintln!("debug energia: limite de carga {} ({})", if on { 80 } else { 100 }, if r.is_ok() { "pedido aprovado" } else { "recusado" });
                }
                if let (Some(e), Err(x)) = (w.upgrade(), r) {
                    e.loading.set(true);
                    sw.set_active(!on);
                    sw.set_state(!on);
                    e.loading.set(false);
                    e.frame.sub.set_text(&x);
                }
            });
            glib::Propagation::Proceed
        });
        let w = Rc::downgrade(self);
        self.lid.connect_selected_notify(move |d| {
            let Some(e) = w.upgrade().filter(|e| !e.loading.get()) else { return };
            let acao = ["suspender", "desligar", "nada"].get(d.selected() as usize).copied().unwrap_or("suspender");
            let w = Rc::downgrade(&e);
            crate::lateral::em_segundo_plano(move || crate::servicos::kalangod_chamar("energia.tampa", json!({"acao": acao})), move |r| {
                if depurando() {
                    eprintln!("debug energia: tampa {acao} ({})", if r.is_ok() { "pedido aprovado" } else { "recusado" });
                }
                if let (Some(e), Err(x)) = (w.upgrade(), r) {
                    e.frame.sub.set_text(&x);
                }
            });
        });
        for (id, b) in &self.gpu {
            let (w, id) = (Rc::downgrade(self), *id);
            b.connect_toggled(move |b| {
                if !b.is_active() {
                    return;
                }
                let Some(e) = w.upgrade().filter(|e| !e.loading.get()) else { return };
                // through kalangod: the system asks the person; it takes effect after restarting
                e.gpu_note.set_text(tr("Waiting for your approval…"));
                e.gpu_note.set_visible(true);
                let w = Rc::downgrade(&e);
                crate::lateral::em_segundo_plano(move || crate::servicos::kalangod_chamar("grafico.modo", json!({"modo": id})), move |r| {
                    if depurando() {
                        eprintln!("debug energia: placa de vídeo {id} ({})", if r.is_ok() { "pedido aprovado" } else { "recusado" });
                    }
                    let Some(e) = w.upgrade() else { return };
                    e.gpu_note.set_text(&match r {
                        Ok(_) => tr("Takes effect after restarting the computer.").to_string(),
                        Err(x) => x,
                    });
                });
            });
        }
        let w = Rc::downgrade(self);
        self.chart.set_draw_func(move |_, cr, w_, h_| if let Some(e) = w.upgrade() { e.draw_chart(cr, f64::from(w_), f64::from(h_)) });
        let w = Rc::downgrade(self);
        self.battery_ring.set_draw_func(move |_, cr, w_, h_| {
            let Some(e) = w.upgrade() else { return };
            let (cx, cy, r) = (f64::from(w_) / 2.0, f64::from(h_) / 2.0, 29.0);
            cr.set_line_width(6.0);
            let (a, b, c) = crate::tema::rgb("#FFFFFF");
            cr.set_source_rgba(a, b, c, 0.08);
            cr.arc(cx, cy, r, 0.0, std::f64::consts::TAU);
            let _ = cr.stroke();
            let (a, b, c) = crate::tema::rgb("#8ED2C3");
            cr.set_source_rgb(a, b, c);
            cr.set_line_cap(gtk::cairo::LineCap::Round);
            let start = -std::f64::consts::FRAC_PI_2;
            cr.arc(cx, cy, r, start, start + e.battery_pct.get() * std::f64::consts::TAU);
            let _ = cr.stroke();
            let t = format!("{}%", (e.battery_pct.get() * 100.0).round());
            cr.select_font_face(crate::tema::fonte_display(), gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Bold);
            cr.set_font_size(14.0);
            if let Ok(x) = cr.text_extents(&t) {
                let (a, b, c) = crate::tema::rgb("#E2E2E6");
                cr.set_source_rgb(a, b, c);
                cr.move_to(cx - x.width() / 2.0 - x.x_bearing(), cy + x.height() / 2.0);
                let _ = cr.show_text(&t);
            }
        });
        crate::configuracoes::registrar_posicao(c, "cfg energia agente", self.agent_switch.upcast_ref());
        crate::configuracoes::registrar_posicao(c, "cfg energia acordado", self.keep_awake.upcast_ref());
        crate::configuracoes::registrar_posicao(c, "cfg energia limite", self.charge_limit.upcast_ref());
        crate::configuracoes::registrar_posicao(c, "cfg energia tela", self.screen_off.upcast_ref());
        crate::configuracoes::registrar_posicao(c, "cfg energia suspender", self.suspend_after.upcast_ref());
        crate::configuracoes::registrar_posicao(c, "cfg energia tampa", self.lid.upcast_ref());
        for (id, b) in &self.gpu {
            crate::configuracoes::registrar_posicao(c, &format!("cfg energia gpu {id}"), b.upcast_ref());
        }
    }

    pub fn compactar(&self, c: bool) {
        self.frame.compactar(c);
    }

    pub fn mostrar(self: &Rc<Self>) {
        self.refresh();
        if self.timer.borrow().is_some() {
            return;
        }
        let w = Rc::downgrade(self);
        *self.timer.borrow_mut() = Some(glib::timeout_add_local(Duration::from_secs(5), move || {
            let Some(e) = w.upgrade() else { return glib::ControlFlow::Break };
            if !e.frame.raiz.is_mapped() {
                e.timer.borrow_mut().take();
                return glib::ControlFlow::Break;
            }
            e.refresh();
            glib::ControlFlow::Continue
        }));
    }

    fn refresh(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(|| {
            let s = crate::sistema::ler();
            let now = glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0);
            let samples = parse_history(&std::fs::read_to_string(history_path()).unwrap_or_default(), now - 24 * 3600);
            let gpu = crate::sistema::gpu();
            let battery_card = read_battery();
            let extra = (charge_limit_now(), has_lid().then(lid_action_now));
            let prime = std::process::Command::new("prime-select").arg("query").output().ok().filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
            (s, samples, gpu, prime, battery_card, extra)
        }, move |(s, samples, gpu, prime, battery_card, (charge_limit, lid))| {
            let Some(e) = w.upgrade() else { return };
            e.loading.set(true);
            for (id, b) in &e.modes {
                b.set_sensitive(s.perfis.iter().any(|p| p == id));
                if s.perfil == *id {
                    b.add_css_class("escolhido");
                } else {
                    b.remove_css_class("escolhido");
                }
            }
            let now_w = samples.last().map(|a| a.w);
            let mode = MODES.iter().find(|(id, ..)| *id == s.perfil).map(|(_, n, _)| tr(n).to_lowercase()).unwrap_or_default();
            let mut sub = match &battery_card {
                Some(b) if !b.charging && !b.full => trf("On battery · {n}%", &[("n", format!("{}", b.pct.round()))]),
                _ => tr("Plugged in").to_string(),
            };
            if !mode.is_empty() {
                sub += &format!(" · {}", trf("{mode} mode", &[("mode", mode)]));
            }
            if let Some(w) = now_w {
                sub += &format!(" · {}", trf("{w} W now", &[("w", format!("{}", w.round()))]));
            }
            e.frame.sub.set_text(&sub);
            e.agent_switch.set_active(ajustes()["agent_picks_mode"].as_bool().unwrap_or(true));
            e.agent_switch.set_state(e.agent_switch.is_active());
            e.keep_awake.set_active(ajustes()["agents_keep_awake"].as_bool().unwrap_or(true));
            e.keep_awake.set_state(e.keep_awake.is_active());
            // (only a change touches a drop-down: setting it again would close its list while the person is choosing)
            for (d, key, default_min) in [(&e.screen_off, "screen_off_min", 10), (&e.suspend_after, "suspend_min", 30)] {
                let m = minutes(key, default_min);
                let i = TIMEOUTS.iter().position(|x| *x == m).unwrap_or(0) as u32;
                if d.selected() != i {
                    d.set_selected(i);
                }
            }
            // the battery (the card only where there is one)
            e.battery_card.set_visible(battery_card.is_some());
            if let Some(b) = &battery_card {
                e.battery_pct.set(b.pct / 100.0);
                let duration = |m: u32| if m >= 60 { trf("{h} h {m} min", &[("h", (m / 60).to_string()), ("m", (m % 60).to_string())]) } else { trf("{n} min", &[("n", m.to_string())]) };
                e.battery_text.set_text(&match (b.charging, b.full, b.minutes) {
                    (_, true, _) => tr("Charged").to_string(),
                    (true, _, Some(m)) => trf("Charging · full in {t}", &[("t", duration(m))]),
                    (true, _, None) => tr("Charging").to_string(),
                    (false, _, Some(m)) => trf("On battery · {t} left", &[("t", duration(m))]),
                    (false, _, None) => tr("On battery").to_string(),
                });
                e.battery_sub.set_text(&b.health.map(|h| trf("battery health: {n}%", &[("n", h.to_string())])).unwrap_or_default());
                e.battery_sub.set_visible(b.health.is_some());
                e.battery_ring.queue_draw();
            }
            e.charge_limit_row.set_visible(charge_limit.is_some());
            if let Some(on) = charge_limit {
                e.charge_limit.set_active(on);
                e.charge_limit.set_state(on);
            }
            e.lid_row.set_visible(lid.is_some());
            if let Some(t) = lid {
                let i = ["suspender", "desligar", "nada"].iter().position(|x| *x == t).unwrap_or(0) as u32;
                if e.lid.selected() != i {
                    e.lid.set_selected(i);
                }
            }
            // the graphics card: in use now, and the mode where prime-select chooses it
            e.gpu_card.set_visible(gpu.is_some());
            if let Some((uso, temp, nome)) = &gpu {
                e.gpu_use.set_text(&format!("{} · {uso}% · {temp} °C", nome.split_whitespace().next().unwrap_or(nome)));
            }
            let gpu_mode = prime.as_deref().map(|p| match p { "nvidia" => "dedicado", "intel" => "integrado", _ => "hibrido" });
            for (id, b) in &e.gpu {
                b.set_sensitive(prime.is_some());
                if Some(*id) == gpu_mode {
                    b.set_active(true);
                }
            }
            if depurando() {
                eprintln!("debug energia: modo {}, {} amostras, bateria {}", s.perfil, samples.len(), battery_card.as_ref().map(|b| format!("{}%", b.pct.round())).unwrap_or("nenhuma".into()));
            }
            e.show_usage(samples);
            e.loading.set(false);
        });
    }

    fn show_usage(&self, samples: Vec<Sample>) {
        let (kwh, mean_label, pico) = summarize(&samples);
        self.total.set_text(&format!("{} kWh", format!("{kwh:.1}").replace('.', ",")));
        self.mean_label.set_text(&match pico {
            Some((w, t)) => trf("mean {m} W · peak {p} W at {h}", &[("m", format!("{}", mean_label.round())), ("p", format!("{}", w.round())),
                ("h", glib::DateTime::from_unix_local(t).ok().and_then(|d| d.format("%H:%M").ok()).map(|s| s.to_string()).unwrap_or_default())]),
            None => tr("measuring: the first hour shows soon").to_string(),
        });
        while let Some(x) = self.apps.first_child() {
            self.apps.remove(&x);
        }
        let top_apps = top_apps(&samples);
        // each bar is the app's share of all the computer drew
        let scale_max = kwh.max(1e-6);
        let all_apps = crate::apps::todos();
        for (nome, k) in top_apps {
            let r = caixa(Orientation::Horizontal, 12, "");
            // the app's icon (18 in a 22 frame, as the dock's tiles)
            let icon_box = caixa(Orientation::Horizontal, 0, "");
            icon_box.set_size_request(22, 22);
            icon_box.set_valign(Align::Center);
            let img = if nome == tr("Local model") {
                crate::apps::imagem_kalango("agentes", 18)
            } else {
                let n = nome.to_lowercase();
                match all_apps.iter().find(|a| a.nome.to_lowercase() == n || crate::apps::executable_name(&a.info).is_some_and(|f| f.to_lowercase() == n)) {
                    Some(a) => crate::apps::imagem(a, 18),
                    None => {
                        let i = gtk::Image::from_icon_name("application-x-executable");
                        i.set_pixel_size(18);
                        i
                    }
                }
            };
            icon_box.set_hexpand(false);
            img.set_margin_start(2);
            img.set_halign(Align::Start);
            img.set_valign(Align::Center);
            icon_box.append(&img);
            r.append(&icon_box);
            let n = left_label(&nome, "cfg-texto");
            n.set_size_request(220, -1);
            n.set_ellipsize(gtk::pango::EllipsizeMode::End);
            n.set_max_width_chars(1);
            r.append(&n);
            let bar = gtk::LevelBar::for_interval(0.0, 1.0);
            bar.add_css_class("cfg-energia-barra");
            bar.set_value(k / scale_max);
            bar.set_hexpand(true);
            bar.set_valign(Align::Center);
            r.append(&bar);
            let v = rotulo(&format!("{} kWh", format!("{k:.1}").replace('.', ",")), "cfg-energia-valor");
            v.set_xalign(1.0);
            v.set_size_request(51, -1);
            r.append(&v);
            self.apps.append(&r);
        }
        *self.samples.borrow_mut() = samples;
        self.chart.queue_draw();
    }

    /// The day's draw: the area and its line, the hours under it.
    fn draw_chart(&self, cr: &gtk::cairo::Context, w: f64, h: f64) {
        let a = self.samples.borrow();
        let now = glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0);
        let (gw, gh) = (w - 46.0, h - 25.0);
        let max_w = a.iter().map(|x| x.w).fold(1.0f64, f64::max);
        let px = |t: i64| (1.0 - (now - t) as f64 / 86_400.0) * gw;
        let py = |v: f64| 14.0 + (gh - 14.0) * (1.0 - v / max_w);
        if a.len() >= 2 {
            cr.move_to(px(a[0].t), gh);
            for x in a.iter() {
                cr.line_to(px(x.t), py(x.w));
            }
            cr.line_to(px(a[a.len() - 1].t), gh);
            cr.close_path();
            let (r, g, b) = crate::tema::rgb(crate::tema::ACENTO);
            cr.set_source_rgba(r, g, b, 0.12);
            let _ = cr.fill();
            cr.move_to(px(a[0].t), py(a[0].w));
            for x in a.iter().skip(1) {
                cr.line_to(px(x.t), py(x.w));
            }
            cr.set_source_rgb(r, g, b);
            cr.set_line_width(1.5);
            cr.set_line_join(gtk::cairo::LineJoin::Round);
            let _ = cr.stroke();
        }
        // the hours: 18, 12 and 6 hours ago, and now
        cr.select_font_face("JetBrains Mono", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Normal);
        cr.set_font_size(11.0);
        let (r, g, b) = crate::tema::rgb("#7A7B88");
        cr.set_source_rgb(r, g, b);
        for k in 0..5 {
            let t = now - 86_400 + k * 21_600;
            let txt = if k == 4 { tr("now").to_string() } else { glib::DateTime::from_unix_local(t).ok().and_then(|d| d.format("%H:00").ok()).map(|s| s.to_string()).unwrap_or_default() };
            let x = (w - 33.0) * k as f64 / 4.0;
            cr.move_to(x, h - 4.0);
            let _ = cr.show_text(&txt);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upower_tells_the_charge_the_time_and_the_health() {
        let t = "  native-path:          BAT0\n  battery\n    present:             yes\n    state:               charging\n    time to full:        35,0 minutes\n    percentage:          82%\n    capacity:            96,1%\n";
        assert_eq!(parse_upower(t), Some(Battery { pct: 82.0, charging: true, full: false, minutes: Some(35), health: Some(96) }));
        let d = t.replace("charging", "discharging").replace("time to full:        35,0 minutes", "time to empty:       2,5 hours");
        assert_eq!(parse_upower(&d).unwrap().minutes, Some(150));
        assert_eq!(parse_upower("    present:             no\n"), None);
    }

    #[test]
    fn a_days_samples_make_the_total_the_mean_the_peak_and_the_apps() {
        let t0 = 1_000_000;
        let text = (0..60).map(|i| json!({"t": t0 + i * 60, "w": if i == 30 { 412.0 } else { 60.0 }, "wh": if i == 30 { 412.0 / 60.0 } else { 1.0 }, "apps": {"Modelo local": 0.5, "Chrome": 0.2}}).to_string()).collect::<Vec<_>>().join("\n");
        let a = parse_history(&text, t0 + 60);
        assert_eq!(a.len(), 59);
        let (kwh, mean_label, pico) = summarize(&a);
        assert!((kwh - (58.0 * 60.0 + 412.0) / 60.0 / 1000.0).abs() < 1e-9);
        assert!(mean_label > 60.0 && mean_label < 70.0);
        assert_eq!(pico, Some((412.0, t0 + 30 * 60)));
        let m = top_apps(&a);
        assert_eq!(m[0].0, "Modelo local");
        assert!((m[0].1 - 59.0 * 0.5 / 1000.0).abs() < 1e-9);
    }
}

#[cfg(test)]
mod live_tests {
    use super::is_live_cmdline;

    #[test]
    fn the_install_media_is_the_live_session() {
        assert!(is_live_cmdline("BOOT_IMAGE=/casper/vmlinuz boot=casper quiet splash ---"));
        assert!(!is_live_cmdline("BOOT_IMAGE=/boot/vmlinuz root=UUID=1234 ro quiet splash"));
        assert!(!is_live_cmdline("boot=casperx"));
    }
}
