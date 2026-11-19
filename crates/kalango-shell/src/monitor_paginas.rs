//! The System Monitor's "Agentes", "Discos" and "Rede" pages (designs "Monitor do sistema · Agentes", "· Discos",
//! "· Rede").
//! - **Agentes:** the local models' servers (llama.cpp: text, vision, voice) — in use, idle or off, their video
//!   memory and speed; the day's tokens (the engine's usage log: all on this computer); the agents' tasks (the bar's
//!   conversation and the routines) with what they use; "Liberar a memória agora" ends the idle servers.
//! - **Discos:** the mounted file systems with their use and activity; who reads and writes; what takes the space
//!   (the home's biggest folders); restore points (kalangod) and health (udisks' SMART).
//! - **Rede:** the connections (Wi-Fi, cable, VPN) with their traffic and the day's total; the apps with open
//!   connections and the ports they listen on (the kernel's tables), and where they connect to (reverse names).

use crate::monitor::{body as body_of, bytes, card_row, pct, rate, when, Card, Detail, Group, Kind, Monitor, Note, RowData, RowIcon, Table};
use crate::monitor_dados::{self as dados, Snap};
use crate::superficies::caixa;
use gtk::prelude::*;
use gtk::{gio, glib, Orientation};
use kalango_i18n::{tr, trf};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::{Rc, Weak};

/// $XDG_STATE_HOME or ~/.local/state.
fn state_dir() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| glib::home_dir().join(".local/state"))
}

fn now() -> i64 {
    glib::DateTime::now_local().map(|t| t.to_unix()).unwrap_or(0)
}

fn today_start() -> i64 {
    let n = glib::DateTime::now_local().ok();
    n.and_then(|n| glib::DateTime::from_local(n.year(), n.month(), n.day_of_month(), 0, 0, 0.0).ok()).map(|t| t.to_unix()).unwrap_or(0)
}

// ------------------------------------------------------------------------------------------------- the network's kind

thread_local! {
    static KIND: RefCell<(i64, Option<String>)> = const { RefCell::new((0, None)) };
}

/// "Wi-Fi", "cabo" or "VPN": what the computer is connected through (asked to NetworkManager every 10 s).
pub(crate) fn connection_kind() -> Option<String> {
    KIND.with(|k| {
        let (t, v) = k.borrow().clone();
        if now() - t > 10 {
            k.borrow_mut().0 = now();
            crate::lateral::em_segundo_plano(nm_devices, |d: Vec<NmDevice>| {
                let v = d.iter().find(|x| x.state == "connected").map(|x| match x.kind.as_str() {
                    "wifi" => "Wi-Fi".to_string(),
                    "ethernet" => tr("cable").to_string(),
                    other => other.to_string(),
                });
                KIND.with(|k| k.borrow_mut().1 = v);
            });
        }
        v
    })
}

#[derive(Clone, Default)]
struct NmDevice {
    device: String,
    kind: String,
    state: String,
    connection: String,
}

fn nmcli(args: &[&str]) -> String {
    std::process::Command::new(crate::configuracoes_rede::nmcli()).args(args).output().ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default()
}

/// nmcli's terse fields ("a\:b" is a colon inside a field).
fn fields(l: &str) -> Vec<String> {
    let (mut out, mut cur, mut esc) = (Vec::new(), String::new(), false);
    for c in l.chars() {
        if esc {
            cur.push(c);
            esc = false;
        } else if c == '\\' {
            esc = true;
        } else if c == ':' {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    out.push(cur);
    out
}

fn nm_devices() -> Vec<NmDevice> {
    nmcli(&["-t", "-f", "DEVICE,TYPE,STATE,CONNECTION", "device"]).lines().map(fields)
        .map(|f| NmDevice { device: f.first().cloned().unwrap_or_default(), kind: f.get(1).cloned().unwrap_or_default(),
            state: f.get(2).cloned().unwrap_or_default(), connection: f.get(3).cloned().unwrap_or_default() }).collect()
}

/// What NetworkManager says, every 5 s: the devices, the Wi-Fi in use, the VPNs.
#[derive(Clone, Default)]
struct Nm {
    devices: Vec<NmDevice>,
    /// ssid, signal %, MHz
    wifi: Option<(String, u32, u32)>,
    /// name, active
    vpns: Vec<(String, bool)>,
}

fn nm_read() -> Nm {
    let devices = nm_devices();
    let wifi = nmcli(&["-t", "-f", "IN-USE,SSID,SIGNAL,FREQ", "device", "wifi", "list", "--rescan", "no"]).lines().map(fields)
        .find(|f| f.first().map(|s| s == "*").unwrap_or(false))
        .map(|f| (f.get(1).cloned().unwrap_or_default(), f.get(2).and_then(|s| s.parse().ok()).unwrap_or(0),
            f.get(3).and_then(|s| s.split_whitespace().next()?.parse().ok()).unwrap_or(0)));
    let vpns = nmcli(&["-t", "-f", "NAME,TYPE,ACTIVE", "connection", "show"]).lines().map(fields)
        .filter(|f| f.get(1).map(|t| t == "vpn" || t == "wireguard").unwrap_or(false))
        .map(|f| (f.first().cloned().unwrap_or_default(), f.get(2).map(|a| a == "yes").unwrap_or(false))).collect();
    Nm { devices, wifi, vpns }
}

// ------------------------------------------------------------------------------------------------- the pages

/// The day's network totals: what the counters said at the day's first sample (state/kalango/rede-hoje.json).
#[derive(Default)]
struct NetDay {
    day: i64,
    base_rx: u64,
    base_tx: u64,
    /// counted before a restart (the counters start again at boot)
    carried_rx: u64,
    carried_tx: u64,
    last_rx: u64,
    last_tx: u64,
}

fn netday_path() -> PathBuf {
    state_dir().join("kalango/rede-hoje.json")
}

pub(crate) struct Pages {
    pub agents: gtk::Box,
    pub disks: gtk::Box,
    pub network: gtk::Box,
    monitor: Weak<Monitor>,
    // Agentes
    models: Vec<Card>,
    tokens: Card,
    tasks: Rc<Table>,
    ag_note: Note,
    ag_detail: Detail,
    usage: RefCell<(u64, HashMap<String, (i64, f64)>, u64, u64)>,
    task_sel: RefCell<Option<String>>,
    task_pids: RefCell<HashMap<String, Vec<i32>>>,
    // Discos
    fs_cards: Vec<Card>,
    activity: Card,
    fs: RefCell<Vec<(String, String, String, String)>>,
    fs_sel: Cell<usize>,
    io_table: Rc<Table>,
    dk_note: Note,
    dk_detail: Detail,
    biggest: RefCell<Option<(i64, Vec<(PathBuf, u64)>)>>,
    dk_fine: Cell<bool>,
    points: RefCell<Option<(i64, String)>>,
    health: RefCell<HashMap<String, String>>,
    // Rede
    net_cards: Vec<Card>,
    net_table: Rc<Table>,
    nt_note: Note,
    nt_detail: Detail,
    nm: RefCell<(i64, Nm)>,
    socks: RefCell<HashMap<String, (usize, Vec<u16>, Vec<String>)>>,
    socks_at: Cell<i64>,
    names: RefCell<HashMap<String, String>>,
    netday: RefCell<NetDay>,
    nt_fine: Cell<bool>,
}

impl Pages {
    pub(crate) fn new(m: &Rc<Monitor>) -> Rc<Pages> {
        let page = || caixa(Orientation::Vertical, 16, "mon-pagina-conteudo");
        let row = |cards: &[&gtk::Box]| card_row(cards);
        let body = |table: &gtk::Box, note: &gtk::Box, detail: &gtk::Box| {
            let side = caixa(Orientation::Vertical, 14, "");
            side.append(note);
            side.append(detail);
            table.set_valign(gtk::Align::Start);
            let b = body_of(table.upcast_ref(), &side);
            m.bodies.borrow_mut().push((b.clone(), side));
            b
        };
        // ---- Agentes
        let models = vec![Card::new("spark", "LFM3 1.7B · TEXTO", "#8ED2C3"), Card::new("spark", "LFM3-VL 2B · VISÃO", "#7CC3DF"),
            Card::new("spark", "LFM3-AUDIO · VOZ", "#A5A1E2")];
        let tokens = Card::new("relogio", tr("TODAY"), "#E5C98C");
        let tasks = Table::new(&[tr("CPU"), tr("Memory"), tr("Steps"), tr("State")], tr("Agent and task"));
        let ag_note = Note::new(tr("WHAT THE AGENTS USE"));
        let ag_detail = Detail::new(&[tr("Started"), tr("Steps"), tr("Request"), tr("Using now")], tr("Pause"), tr("End"), "mon-acao-perigo");
        let agents = page();
        agents.append(&row(&[&models[0].root, &models[1].root, &models[2].root, &tokens.root]));
        agents.append(&body(&tasks.root, &ag_note.root, &ag_detail.root));
        // ---- Discos
        let fs_cards = vec![Card::new("disco", "", "#8ED2C3"), Card::new("disco", "", "#7CC3DF"), Card::new("disco", "", "#A5A1E2")];
        let activity = Card::new("raio", tr("ACTIVITY"), "#E5C98C");
        let io_table = Table::new(&[tr("Reads"), tr("Writes"), tr("Read so far"), tr("Written")], tr("Process"));
        let dk_note = Note::new(tr("WHAT TAKES THE SPACE"));
        let dk_detail = Detail::new(&[tr("Device"), tr("Free"), tr("Restore points"), tr("Health")], tr("Open in Files"), tr("Free up space"), "mon-acao-acento");
        let disks = page();
        disks.append(&row(&[&fs_cards[0].root, &fs_cards[1].root, &fs_cards[2].root, &activity.root]));
        disks.append(&body(&io_table.root, &dk_note.root, &dk_detail.root));
        // ---- Rede
        let net_cards = vec![Card::new("wifi", "WI-FI", "#8ED2C3"), Card::new("rede-cabo", tr("CABLE"), "#7CC3DF"),
            Card::new("cadeado", "VPN", "#A5A1E2"), Card::new("relogio", tr("TODAY"), "#E5C98C")];
        let net_table = Table::new(&[tr("Connections"), tr("Listening"), "", tr("Since")], tr("App"));
        let nt_note = Note::new(tr("WHAT IS USING THE NETWORK"));
        let nt_detail = Detail::new(&[tr("Connections"), tr("Where to"), tr("Listening"), tr("Through")], tr("Suspend"), tr("End"), "mon-acao-perigo");
        let network = page();
        network.append(&row(&[&net_cards[0].root, &net_cards[1].root, &net_cards[2].root, &net_cards[3].root]));
        network.append(&body(&net_table.root, &nt_note.root, &nt_detail.root));
        let netday = std::fs::read_to_string(netday_path()).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()).map(|v| NetDay {
            day: v["dia"].as_i64().unwrap_or(0), base_rx: v["base_rx"].as_u64().unwrap_or(0), base_tx: v["base_tx"].as_u64().unwrap_or(0),
            carried_rx: v["antes_rx"].as_u64().unwrap_or(0), carried_tx: v["antes_tx"].as_u64().unwrap_or(0),
            last_rx: v["ultimo_rx"].as_u64().unwrap_or(0), last_tx: v["ultimo_tx"].as_u64().unwrap_or(0),
        }).unwrap_or_default();
        let p = Rc::new(Pages {
            agents, disks, network, monitor: Rc::downgrade(m), models, tokens, tasks, ag_note, ag_detail, usage: RefCell::default(),
            task_sel: RefCell::default(), task_pids: RefCell::default(), fs_cards, activity, fs: RefCell::default(), fs_sel: Cell::new(0), io_table, dk_note,
            dk_detail, biggest: RefCell::default(), dk_fine: Cell::new(false), points: RefCell::default(), health: RefCell::default(), net_cards, net_table,
            nt_note, nt_detail, nm: RefCell::default(), socks: RefCell::default(), socks_at: Cell::new(0), names: RefCell::default(), netday: RefCell::new(netday),
            nt_fine: Cell::new(false),
        });
        p.wire();
        p
    }

    fn wire(self: &Rc<Self>) {
        // Agentes
        let w = Rc::downgrade(self);
        self.tasks.on_select(move |k| {
            if let Some(p) = w.upgrade() {
                *p.task_sel.borrow_mut() = Some(k.to_string());
            }
        });
        let w = Rc::downgrade(self);
        self.ag_note.primary.connect_clicked(move |_| {
            if let Some(p) = w.upgrade() {
                p.free_models();
            }
        });
        let w = Rc::downgrade(self);
        self.ag_note.secondary.connect_clicked(move |_| {
            if w.upgrade().is_some() {
                let paused = crate::sistema::agent_pids().first().is_some_and(|p| crate::sistema::is_stopped(*p));
                crate::sistema::pausar_agentes(!paused);
                if crate::monitor::debug() {
                    eprintln!("debug monitor: agentes {}", if paused { "continuam" } else { "pausados" });
                }
            }
        });
        let w = Rc::downgrade(self);
        self.ag_detail.first.connect_clicked(move |_| {
            if let Some(p) = w.upgrade() {
                let key = p.task_sel.borrow().clone().unwrap_or_default();
                let pids = p.task_pids.borrow().get(&key).cloned().unwrap_or_default();
                let stop = !pids.first().is_some_and(|x| crate::sistema::is_stopped(*x));
                for pid in pids {
                    crate::sistema::pausar_processo(pid, stop);
                }
            }
        });
        let w = Rc::downgrade(self);
        self.ag_detail.second.connect_clicked(move |_| {
            let Some(p) = w.upgrade() else { return };
            let key = p.task_sel.borrow().clone().unwrap_or_default();
            if key == "barra" {
                if let Some(s) = p.monitor.upgrade().and_then(|m| m.shell()) {
                    s.right_bar().conversa.stop(true);
                }
            } else {
                for pid in p.task_pids.borrow().get(&key).cloned().unwrap_or_default() {
                    crate::monitor::signal(pid, 15);
                }
            }
            if crate::monitor::debug() {
                eprintln!("debug monitor: encerrou a tarefa «{key}»");
            }
        });
        // Discos
        for (i, c) in self.fs_cards.iter().enumerate() {
            let click = gtk::GestureClick::new();
            let w = Rc::downgrade(self);
            click.connect_released(move |_, _, _, _| {
                if let Some(p) = w.upgrade() {
                    p.fs_sel.set(i);
                    p.disk_detail();
                }
            });
            c.root.add_controller(click);
        }
        let w = Rc::downgrade(self);
        self.dk_detail.first.connect_clicked(move |_| {
            if let Some(p) = w.upgrade() {
                let mount = p.fs.borrow().get(p.fs_sel.get()).map(|f| f.1.clone());
                if let (Some(s), Some(mp)) = (p.monitor.upgrade().and_then(|m| m.shell()), mount) {
                    s.abrir_arquivos(Some(std::path::Path::new(&mp)));
                }
            }
        });
        let w = Rc::downgrade(self);
        self.dk_detail.second.connect_clicked(move |_| {
            if let Some(p) = w.upgrade() {
                let mount = p.fs.borrow().get(p.fs_sel.get()).map(|f| f.1.clone()).unwrap_or_else(|| "/".into());
                if let Some(s) = p.monitor.upgrade().and_then(|m| m.shell()) {
                    s.right_bar().ask_agent(&trf("Propose what to delete in {mount} to free up space: nothing is deleted before I approve it.", &[("mount", mount)]));
                }
            }
        });
        let w = Rc::downgrade(self);
        self.dk_note.primary.connect_clicked(move |_| {
            if let Some(p) = w.upgrade() {
                let big = p.biggest.borrow().as_ref().and_then(|(_, l)| l.first().map(|x| x.0.clone()));
                if let (Some(s), Some(dir)) = (p.monitor.upgrade().and_then(|m| m.shell()), big) {
                    s.abrir_arquivos(Some(&dir));
                }
            }
        });
        let w = Rc::downgrade(self);
        self.dk_note.secondary.connect_clicked(move |_| {
            if let Some(p) = w.upgrade() {
                p.dk_fine.set(true);
                p.dk_note.root.set_visible(false);
            }
        });
        // Rede
        let w = Rc::downgrade(self);
        self.nt_note.primary.connect_clicked(move |_| {
            if let Some(s) = w.upgrade().and_then(|p| p.monitor.upgrade()).and_then(|m| m.shell()) {
                s.abrir_configuracoes(Some("rede"));
            }
        });
        let w = Rc::downgrade(self);
        self.nt_note.secondary.connect_clicked(move |_| {
            if let Some(p) = w.upgrade() {
                p.nt_fine.set(true);
                p.nt_note.root.set_visible(false);
            }
        });
        let w = Rc::downgrade(self);
        self.net_table.on_select(move |k| {
            if let Some(m) = w.upgrade().and_then(|p| p.monitor.upgrade()) {
                m.choose(k);
            }
        });
        let w = Rc::downgrade(self);
        self.nt_detail.first.connect_clicked(move |_| {
            if let Some(m) = w.upgrade().and_then(|p| p.monitor.upgrade()) {
                m.suspend_selected();
            }
        });
        let w = Rc::downgrade(self);
        self.nt_detail.second.connect_clicked(move |_| {
            if let Some(m) = w.upgrade().and_then(|p| p.monitor.upgrade()) {
                m.end_selected();
            }
        });
        crate::central::posicao_debug("monitor liberar memória", self.ag_note.primary.upcast_ref());
    }

    pub(crate) fn update(self: &Rc<Self>, prev: &Snap, cur: &Snap) {
        let Some(m) = self.monitor.upgrade() else { return };
        let groups = m.groups.borrow().clone();
        self.update_agents(&m, &groups, prev, cur);
        self.update_disks(&groups, prev, cur);
        self.update_network(&m, &groups, prev, cur);
    }

    // ---------------------------------------------------------------------------------------------- Agentes

    /// The engine's usage log, read from where it was left: the day's tokens, and each model's last use and speed.
    fn read_usage(&self) {
        let path = state_dir().join("kalango/uso-tokens.jsonl");
        let Ok(mut f) = std::fs::File::open(&path) else { return };
        use std::io::{Read, Seek};
        let mut u = self.usage.borrow_mut();
        let len = f.metadata().map(|m| m.len()).unwrap_or(0);
        if len < u.0 {
            *u = Default::default();
        }
        if f.seek(std::io::SeekFrom::Start(u.0)).is_err() {
            return;
        }
        let mut text = String::new();
        if f.read_to_string(&mut text).is_err() {
            return;
        }
        let Some(end) = text.rfind('\n') else { return };
        u.0 += end as u64 + 1;
        let day = today_start();
        for l in text[..end].lines() {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(l) else { continue };
            let t = v["t"].as_i64().unwrap_or(0);
            let model = v["modelo"].as_str().unwrap_or("").to_string();
            u.1.insert(model, (t, v["tok_s"].as_f64().unwrap_or(0.0)));
            if t >= day {
                let n = v["tokens"].as_u64().unwrap_or(0);
                if v["tipo"].as_str() == Some("local") { u.2 += n } else { u.3 += n }
            }
        }
    }

    fn update_agents(self: &Rc<Self>, m: &Monitor, groups: &[Group], prev: &Snap, cur: &Snap) {
        self.read_usage();
        let (last, local, remote) = {
            let u = self.usage.borrow();
            (u.1.clone(), u.2, u.3)
        };
        let servers: Vec<&Group> = groups.iter().filter(|g| g.kind == Kind::Agent && g.name.contains("LFM3")).collect();
        let t = now();
        for (card, (want, off)) in self.models.iter().zip([("· ", tr("starts with the first request")), ("VL", tr("starts when you ask it to see the screen")),
            ("AUDIO", tr("starts when you dictate or speak"))]) {
            let server = servers.iter().find(|g| match want {
                "VL" => g.name.contains("VL"),
                "AUDIO" => g.name.to_uppercase().contains("AUDIO") || g.name.contains("Omni"),
                _ => !g.name.contains("VL") && !g.name.to_uppercase().contains("AUDIO") && !g.name.contains("Omni"),
            });
            match server {
                Some(g) => {
                    let use_ = last.iter().find(|(k, _)| model_matches(k, &g.name)).map(|(_, v)| *v);
                    let busy = g.cpu > 5.0 || use_.is_some_and(|(at, _)| t - at < 15);
                    card.value.set_text(if busy { tr("in use") } else { tr("idle") });
                    let mut sub = vec![trf("{size} of VRAM", &[("size", bytes(g.gpu_mb * 1e6))])];
                    if let Some((at, tok)) = use_ {
                        if busy && tok > 0.0 {
                            sub.push(trf("{n} tokens/s", &[("n", format!("{tok:.0}"))]));
                        } else if want != "· " {
                            let left = 10 - (t - at) / 60;
                            if left > 0 {
                                sub.push(trf("turns itself off in {n} min", &[("n", left.to_string())]));
                            }
                        }
                    }
                    card.sub.set_text(&sub.join(" · "));
                    card.graph.set_visible(true);
                    card.graph.set_opacity(if busy { 1.0 } else { 0.3 });
                    card.push(g.cpu);
                }
                None => {
                    card.value.set_text(tr("off"));
                    card.sub.set_text(off);
                    card.graph.set_visible(false);
                }
            }
        }
        self.tokens.value.set_text(&trf("{n} tokens", &[("n", thousands(local + remote))]));
        self.tokens.sub.set_text(&if remote == 0 { tr("all on this computer · nothing sent").to_string() } else { trf("{n} sent out", &[("n", thousands(remote))]) });
        let before = self.tokens.data.borrow().back().copied().unwrap_or(0.0);
        let _ = before;
        self.tokens.push((local + remote) as f64);
        // the tasks: the bar's conversation and the routines
        let Some(shell) = m.shell() else { return };
        let agent_pids = crate::sistema::agent_pids();
        let agent_use = groups.iter().filter(|g| g.kind == Kind::Agent && g.pids.iter().any(|p| agent_pids.contains(p)))
            .fold((0.0, 0.0), |(c, mm), g| (c + g.cpu, mm + g.mem));
        let mut rows = Vec::new();
        let mut pids = HashMap::new();
        let task = shell.estado.agent_task.borrow().clone();
        let paused = agent_pids.first().is_some_and(|p| crate::sistema::is_stopped(*p));
        rows.push(RowData {
            key: "barra".into(), name: tr("The agent's bar").into(),
            sub: task.as_ref().map(|(p, _)| p.chars().take(80).collect()).unwrap_or_else(|| tr("talks with you").to_string()),
            icon: RowIcon::Glyph("spark"),
            values: vec![pct(agent_use.0), if agent_use.1 > 0.0 { bytes(agent_use.1) } else { "—".into() },
                task.as_ref().map(|(_, n)| n.to_string()).unwrap_or_else(|| "0".into()),
                if paused { tr("paused").into() } else if task.is_some() { tr("working").into() } else { tr("idle").into() }],
            last_css: task.as_ref().map(|_| "acento"), indent: false, opens: None,
        });
        pids.insert("barra".to_string(), agent_pids.clone());
        let running = shell.agendador().map(|a| a.rodando.borrow().clone()).unwrap_or_default();
        let agora = glib::DateTime::now_local().ok();
        for r in crate::rotinas::Rotinas::carregar().lista.into_iter().filter(|r| r.ativa) {
            let key = format!("rotina:{}", r.id);
            let run = running.get(&r.id);
            let pid = run.and_then(|(_, p)| *p).map(|p| p as i32);
            let g = pid.and_then(|p| groups.iter().find(|g| g.pids.contains(&p)));
            let next = agora.as_ref().and_then(|a| r.proxima(a)).and_then(|d| d.format("%H:%M").ok()).map(|s| s.to_string());
            let estado = if run.is_some() { tr("working").to_string() } else if let Some(n) = &next { trf("at {time}", &[("time", n.clone())]) } else if r.ultima.is_some() { tr("finished").to_string() } else { "—".into() };
            let sub = match (&r.ultima, &next) {
                (Some((t, _, _)), _) if run.is_none() => trf("routine · last ran {when}", &[("when", when(*t))]),
                (_, Some(n)) => trf("routine · next at {time}", &[("time", n.clone())]),
                _ => tr("routine").to_string(),
            };
            rows.push(RowData { key: key.clone(), name: r.nome.clone(), sub, icon: RowIcon::Glyph("relogio"),
                values: vec![g.map(|g| pct(g.cpu)).unwrap_or_else(|| "0%".into()), g.map(|g| bytes(g.mem)).unwrap_or_else(|| "—".into()), "—".into(), estado],
                last_css: run.map(|_| "acento"), indent: false, opens: None });
            pids.insert(key, pid.into_iter().collect());
        }
        *self.task_pids.borrow_mut() = pids;
        self.tasks.update(&rows);
        if self.task_sel.borrow().is_none() {
            *self.task_sel.borrow_mut() = Some("barra".into());
            self.tasks.select(Some("barra"));
        }
        let sel = self.task_sel.borrow().clone().unwrap_or_default();
        if let Some(r) = rows.iter().find(|r| r.key == sel) {
            let d = &self.ag_detail;
            d.caption.set_text(&r.name.to_uppercase());
            let started = self.task_pids.borrow().get(&sel).and_then(|p| p.first().copied()).and_then(|pid| groups.iter().find(|g| g.pids.contains(&pid)).map(|g| g.start));
            d.lines[0].1.set_text(&started.map(when).unwrap_or_else(|| "—".into()));
            d.lines[1].1.set_text(&r.values[2]);
            d.lines[2].1.set_text(&r.sub);
            d.lines[3].1.set_text(&format!("{} · {}", r.values[0], r.values[1]));
            d.first.set_label(if paused && sel == "barra" { tr("Resume") } else { tr("Pause") });
        }
        // what the agents use now (counted, no model needed)
        let vram: f64 = groups.iter().filter(|g| g.kind == Kind::Agent).map(|g| g.gpu_mb).sum();
        let cpu: f64 = groups.iter().filter(|g| g.kind == Kind::Agent).map(|g| g.cpu).sum();
        let idle: Vec<String> = servers.iter().filter(|g| g.cpu < 2.0 && !last.iter().any(|(k, (at, _))| model_matches(k, &g.name) && t - at < 60))
            .map(|g| g.name.clone()).collect();
        let mut text = trf("Now {vram} of VRAM and {cpu} of the CPU, all on this computer.", &[("vram", bytes(vram * 1e6)), ("cpu", pct(cpu))]);
        if !idle.is_empty() {
            text.push(' ');
            text.push_str(&trf("Idle: {models}.", &[("models", idle.join(", "))]));
        }
        self.ag_note.show(&text, (!idle.is_empty()).then_some(tr("Free the memory now")), Some(if paused { tr("Resume the agents") } else { tr("Pause the agents") }));
        let _ = prev;
        let _ = cur;
    }

    /// "Liberar a memória agora": the models' servers not used in the last minute end (they start again when needed).
    fn free_models(&self) {
        let Some(m) = self.monitor.upgrade() else { return };
        let last = self.usage.borrow().1.clone();
        let t = now();
        let mut n = 0;
        for g in m.groups.borrow().iter().filter(|g| g.kind == Kind::Agent && g.name.contains("LFM3")) {
            if g.cpu < 2.0 && !last.iter().any(|(k, (at, _))| model_matches(k, &g.name) && t - at < 60) {
                for pid in &g.pids {
                    crate::monitor::signal(*pid, 15);
                    n += 1;
                }
            }
        }
        if crate::monitor::debug() {
            eprintln!("debug monitor: liberou a memória ({n} servidores)");
        }
    }

    // ---------------------------------------------------------------------------------------------- Discos

    fn update_disks(self: &Rc<Self>, groups: &[Group], prev: &Snap, cur: &Snap) {
        let dt = (cur.at - prev.at).max(0.2);
        // the file systems: what lives on disks, not the system's own (snaps, /boot)
        let mounts = std::fs::read_to_string("/proc/mounts").unwrap_or_default();
        let mut fs: Vec<(String, String, String, String)> = Vec::new();
        for l in mounts.lines() {
            let f: Vec<&str> = l.split_whitespace().collect();
            let (Some(dev), Some(mp), Some(kind)) = (f.first(), f.get(1), f.get(2)) else { continue };
            let mp = mp.replace("\\040", " ");
            if !dev.starts_with("/dev/") || !["ext4", "btrfs", "xfs", "f2fs", "vfat", "exfat", "ntfs", "ntfs3", "fuseblk"].contains(kind)
                || mp.starts_with("/boot") || mp.starts_with("/snap") || mp.starts_with("/var/") || fs.iter().any(|x| x.0 == *dev) {
                continue;
            }
            fs.push((dev.to_string(), mp, kind.to_string(), parent_disk(dev)));
        }
        fs.sort_by_key(|x| match x.1.as_str() { "/" => 0, "/home" => 1, _ => 2 });
        fs.truncate(3);
        let disk_rate = |d: &str| -> (f64, f64) {
            let c = cur.disks.iter().find(|x| x.0 == d);
            let p = prev.disks.iter().find(|x| x.0 == d);
            match (c, p) {
                (Some(c), Some(p)) => (c.1.saturating_sub(p.1) as f64 * 512.0 / dt, c.2.saturating_sub(p.2) as f64 * 512.0 / dt),
                _ => (0.0, 0.0),
            }
        };
        for (i, card) in self.fs_cards.iter().enumerate() {
            let Some((dev, mp, kind, disk)) = fs.get(i) else {
                card.root.set_visible(false);
                continue;
            };
            card.root.set_visible(true);
            let info = gio::File::for_path(mp).query_filesystem_info("filesystem::size,filesystem::free", gio::Cancellable::NONE).ok();
            let (size, free) = info.map(|i| (i.attribute_uint64("filesystem::size") as f64, i.attribute_uint64("filesystem::free") as f64)).unwrap_or((0.0, 0.0));
            let caption = match mp.as_str() {
                "/" => format!("{} · /", tr("SYSTEM")),
                "/home" => format!("{} · /home", tr("PERSONAL")),
                _ => format!("{} · {}", if removable(disk) { tr("REMOVABLE") } else { tr("DISK") }, std::path::Path::new(mp).file_name().map(|n| n.to_string_lossy().to_uppercase()).unwrap_or_default()),
            };
            if let Some(l) = card.root.first_child().and_then(|t| t.last_child()).and_downcast::<gtk::Label>() {
                l.set_text(&caption);
            }
            card.value.set_text(&bytes(size - free));
            card.sub.set_text(&format!("{} · {} · {kind}", trf("of {total}", &[("total", bytes(size))]), bus(disk)));
            let (r, w) = disk_rate(disk);
            card.push(r + w);
            if i == self.fs_sel.get() { card.root.add_css_class("escolhido") } else { card.root.remove_css_class("escolhido") }
            let _ = dev;
        }
        *self.fs.borrow_mut() = fs;
        let (r, w) = cur.disks.iter().map(|d| disk_rate(&d.0)).fold((0.0, 0.0), |a, b| (a.0 + b.0, a.1 + b.1));
        self.activity.value.set_text(&trf("reads {rate}", &[("rate", rate(r))]));
        self.activity.sub.set_text(&trf("writes {rate} · all disks", &[("rate", rate(w))]));
        self.activity.push(r + w);
        // who reads and writes
        let mut g: Vec<&Group> = groups.iter().filter(|g| g.read + g.written > 0.0 || g.read_total.unwrap_or(0.0) + g.written_total.unwrap_or(0.0) > 1e6).collect();
        g.sort_by(|a, b| (b.read + b.written).total_cmp(&(a.read + a.written)).then(b.read_total.unwrap_or(0.0).total_cmp(&a.read_total.unwrap_or(0.0))));
        let rows: Vec<RowData> = g.into_iter().take(12).map(|g| RowData { key: g.key.clone(), name: g.name.clone(), sub: g.sub.clone(), icon: g.icon.clone(),
            values: vec![rate(g.read), rate(g.written), g.read_total.map(bytes).unwrap_or_else(|| "—".into()), g.written_total.map(bytes).unwrap_or_else(|| "—".into())],
            last_css: None, indent: false, opens: None }).collect();
        self.io_table.update(&rows);
        self.space_note();
        self.disk_detail();
    }

    fn disk_detail(self: &Rc<Self>) {
        let Some((dev, mp, kind, disk)) = self.fs.borrow().get(self.fs_sel.get()).cloned() else { return };
        let d = &self.dk_detail;
        d.caption.set_text(&self.fs_cards.get(self.fs_sel.get()).and_then(|c| c.root.first_child()).and_then(|t| t.last_child()).and_downcast::<gtk::Label>()
            .map(|l| l.text().to_string()).unwrap_or_default());
        d.lines[0].1.set_text(&format!("{} · {kind}", dev.trim_start_matches("/dev/")));
        let free = gio::File::for_path(&mp).query_filesystem_info("filesystem::free", gio::Cancellable::NONE).ok().map(|i| i.attribute_uint64("filesystem::free") as f64);
        d.lines[1].1.set_text(&free.map(bytes).unwrap_or_else(|| "—".into()));
        // restore points (kalangod, once a minute) and health (udisks, once per disk)
        let stale = self.points.borrow().as_ref().map_or(true, |(t, _)| now() - t > 60);
        if stale {
            // (the value read first: a borrow inside the assignment would still be held while writing)
            let shown = self.points.borrow().as_ref().map(|p| p.1.clone()).unwrap_or_else(|| "…".into());
            *self.points.borrow_mut() = Some((now(), shown));
            let w = Rc::downgrade(self);
            crate::lateral::em_segundo_plano(|| crate::servicos::kalangod_chamar("pontos.listar", serde_json::json!({})).ok(), move |v: Option<serde_json::Value>| {
                let Some(p) = w.upgrade() else { return };
                let list = v.as_ref().and_then(|v| v["pontos"].as_array().cloned()).unwrap_or_default();
                let text = match list.iter().filter_map(|x| x["time"].as_i64()).max() {
                    Some(t) => trf("{n} · the last {when}", &[("n", list.len().to_string()), ("when", when(t))]),
                    None if v.is_some() => tr("none yet").to_string(),
                    None => tr("kalangod is not running").to_string(),
                };
                *p.points.borrow_mut() = Some((now(), text));
            });
        }
        d.lines[2].1.set_text(&self.points.borrow().as_ref().map(|p| p.1.clone()).unwrap_or_default());
        if !self.health.borrow().contains_key(&disk) {
            self.health.borrow_mut().insert(disk.clone(), "…".into());
            let (w, dk, key) = (Rc::downgrade(self), disk.clone(), disk.clone());
            crate::lateral::em_segundo_plano(move || smart_health(&dk), move |h: String| {
                if let Some(p) = w.upgrade() {
                    p.health.borrow_mut().insert(key, h);
                }
            });
        }
        let h = self.health.borrow().get(&disk).cloned().unwrap_or_default();
        d.lines[3].1.set_text(&h);
        if h == tr("good") { d.lines[3].1.add_css_class("bom") } else { d.lines[3].1.remove_css_class("bom") }
    }

    /// "O que ocupa espaço": the home's biggest folders (measured off the interface's thread, once in ten minutes).
    fn space_note(self: &Rc<Self>) {
        if self.dk_fine.get() {
            return;
        }
        let stale = self.biggest.borrow().as_ref().map_or(true, |(t, _)| now() - t > 600);
        if stale {
            let shown = self.biggest.borrow().as_ref().map(|b| b.1.clone()).unwrap_or_default();
            *self.biggest.borrow_mut() = Some((now(), shown));
            let w = Rc::downgrade(self);
            crate::lateral::em_segundo_plano(biggest_folders, move |l: Vec<(PathBuf, u64)>| {
                if let Some(p) = w.upgrade() {
                    *p.biggest.borrow_mut() = Some((now(), l));
                }
            });
        }
        let list = self.biggest.borrow().as_ref().map(|b| b.1.clone()).unwrap_or_default();
        let name = |p: &PathBuf| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let text = match list.as_slice() {
            [] => tr("Measuring the folders…").to_string(),
            [a] => trf("{folder} has {size}.", &[("folder", name(&a.0)), ("size", bytes(a.1 as f64))]),
            [a, b, ..] => trf("{folder} has {size}; {folder2}, {size2}.", &[("folder", name(&a.0)), ("size", bytes(a.1 as f64)), ("folder2", name(&b.0)), ("size2", bytes(b.1 as f64))]),
        };
        self.dk_note.show(&text, (!list.is_empty()).then_some(tr("See in Files")), Some(tr("It is fine")));
    }

    // ---------------------------------------------------------------------------------------------- Rede

    fn update_network(self: &Rc<Self>, m: &Monitor, groups: &[Group], prev: &Snap, cur: &Snap) {
        let dt = (cur.at - prev.at).max(0.2);
        // NetworkManager every 5 s
        if now() - self.nm.borrow().0 >= 5 {
            self.nm.borrow_mut().0 = now();
            let w = Rc::downgrade(self);
            crate::lateral::em_segundo_plano(nm_read, move |n: Nm| {
                if let Some(p) = w.upgrade() {
                    p.nm.borrow_mut().1 = n;
                }
            });
        }
        let nm = self.nm.borrow().1.clone();
        let iface_rate = |dev: &str| -> (f64, f64) {
            let c = cur.net.iter().find(|x| x.0 == dev);
            let p = prev.net.iter().find(|x| x.0 == dev);
            match (c, p) {
                (Some(c), Some(p)) => (c.1.saturating_sub(p.1) as f64 / dt, c.2.saturating_sub(p.2) as f64 / dt),
                _ => (0.0, 0.0),
            }
        };
        // Wi-Fi
        let wifi_dev = nm.devices.iter().find(|d| d.kind == "wifi");
        let wc = &self.net_cards[0];
        wc.root.set_visible(wifi_dev.is_some());
        if let Some(d) = wifi_dev {
            let (r, t) = iface_rate(&d.device);
            set_caption(wc, &format!("WI-FI{}", nm.wifi.as_ref().map(|w| format!(" · {}", w.0.to_uppercase())).unwrap_or_default()));
            if d.state == "connected" {
                wc.value.set_text(&rate(r));
                let mut sub = vec![format!("↑ {}", rate(t))];
                if let Some((_, sig, freq)) = &nm.wifi {
                    sub.push(trf("signal {quality}", &[("quality", if *sig >= 70 { tr("good") } else if *sig >= 40 { tr("fair") } else { tr("weak") }.to_string())]));
                    sub.push(if *freq >= 5000 { "5 GHz".into() } else { "2,4 GHz".into() });
                }
                wc.sub.set_text(&sub.join(" · "));
                wc.graph.set_visible(true);
            } else {
                wc.value.set_text(tr("disconnected"));
                wc.sub.set_text(tr("no network in use"));
                wc.graph.set_visible(false);
            }
            wc.push(r + t);
        }
        // cable
        let eth = nm.devices.iter().find(|d| d.kind == "ethernet");
        let ec = &self.net_cards[1];
        ec.root.set_visible(eth.is_some());
        if let Some(d) = eth {
            let (r, t) = iface_rate(&d.device);
            let on = d.state == "connected";
            ec.value.set_text(&if on { rate(r) } else { tr("disconnected").to_string() });
            ec.sub.set_text(&if on { format!("↑ {} · {}", rate(t), d.connection) } else { tr("no cable in the port").to_string() });
            ec.graph.set_visible(on);
            ec.push(r + t);
        }
        // VPN
        let vc = &self.net_cards[2];
        match nm.vpns.iter().find(|v| v.1).or(nm.vpns.first()) {
            Some((name, true)) => {
                vc.value.set_text(tr("VPN on"));
                vc.sub.set_text(name);
            }
            Some((name, false)) => {
                vc.value.set_text(tr("VPN off"));
                vc.sub.set_text(&trf("{name} ready", &[("name", name.clone())]));
            }
            None => {
                vc.value.set_text(tr("VPN off"));
                vc.sub.set_text(tr("none set up"));
            }
        }
        vc.graph.set_visible(false);
        // today
        let (rx, tx) = cur.net.iter().fold((0u64, 0u64), |a, n| (a.0 + n.1, a.1 + n.2));
        let (drx, dtx) = self.today(rx, tx);
        let tc = &self.net_cards[3];
        tc.value.set_text(&bytes(drx as f64));
        tc.sub.set_text(&trf("downloaded · {sent} sent", &[("sent", bytes(dtx as f64))]));
        let (r, t) = cur.net.iter().map(|n| iface_rate(&n.0)).fold((0.0, 0.0), |a, b| (a.0 + b.0, a.1 + b.1));
        tc.push(r + t);
        // who uses the network (the sockets of each group, every 3 s, off the interface's thread)
        if now() - self.socks_at.get() >= 3 {
            self.socks_at.set(now());
            let list: Vec<(String, Vec<i32>)> = groups.iter().map(|g| (g.key.clone(), g.pids.clone())).collect();
            let w = Rc::downgrade(self);
            crate::lateral::em_segundo_plano(move || list.into_iter().map(|(k, p)| (k, dados::sockets(&p))).filter(|(_, s)| s.0 > 0 || !s.1.is_empty()).collect::<HashMap<_, _>>(),
                move |s: HashMap<String, (usize, Vec<u16>, Vec<String>)>| {
                    if let Some(p) = w.upgrade() {
                        p.resolve(&s);
                        *p.socks.borrow_mut() = s;
                    }
                });
        }
        let socks = self.socks.borrow().clone();
        let names = self.names.borrow().clone();
        let mut using: Vec<(&Group, &(usize, Vec<u16>, Vec<String>))> = groups.iter().filter_map(|g| socks.get(&g.key).map(|s| (g, s))).collect();
        using.sort_by(|a, b| b.1 .0.cmp(&a.1 .0).then(b.1 .1.len().cmp(&a.1 .1.len())));
        let hosts = |ips: &[String]| -> String {
            let mut n: Vec<String> = Vec::new();
            for ip in ips {
                let h = names.get(ip).cloned().unwrap_or_else(|| ip.clone());
                if !n.contains(&h) {
                    n.push(h);
                }
            }
            match n.len() {
                0 => String::new(),
                1 | 2 => n.join(", "),
                k => trf("{a}, {b} and {n} more", &[("a", n[0].clone()), ("b", n[1].clone()), ("n", (k - 2).to_string())]),
            }
        };
        let rows: Vec<RowData> = using.iter().take(14).map(|(g, s)| RowData { key: g.key.clone(), name: g.name.clone(),
            sub: if s.2.is_empty() { g.sub.clone() } else { hosts(&s.2) }, icon: g.icon.clone(),
            values: vec![s.0.to_string(), if s.1.is_empty() { "—".into() } else { trf("port {port}", &[("port", s.1.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", "))]) },
                String::new(), glib::DateTime::from_unix_local(g.start).ok().and_then(|t| t.format("%H:%M").ok()).map(|s| s.to_string()).unwrap_or_default()],
            last_css: None, indent: false, opens: None }).collect();
        self.net_table.update(&rows);
        // the note: who talks the most, and the ports open on this computer (counted, no model)
        if !self.nt_fine.get() {
            let mut text = match using.first() {
                Some((g, s)) if s.0 > 0 => {
                    let h = hosts(&s.2);
                    if h.is_empty() { trf("{app} has {n} connections.", &[("app", g.name.clone()), ("n", s.0.to_string())]) }
                    else { trf("{app} has {n} connections, with {hosts}.", &[("app", g.name.clone()), ("n", s.0.to_string()), ("hosts", h)]) }
                }
                _ => tr("No app is talking to the network now.").to_string(),
            };
            let open: Vec<(String, u16)> = using.iter().flat_map(|(g, s)| s.1.iter().map(move |p| (g.name.clone(), *p))).collect();
            text.push(' ');
            text.push_str(&match open.as_slice() {
                [] => tr("No port is open on this computer.").to_string(),
                [(app, port)] => trf("One port is open on this computer: {port}, of {app}.", &[("port", port.to_string()), ("app", app.clone())]),
                more => trf("{n} ports are open on this computer.", &[("n", more.len().to_string())]),
            });
            self.nt_note.show(&text, Some(tr("See the firewall")), Some(tr("It is fine")));
        }
        // the chosen one (shared with the other pages)
        let sel = m.selected_group();
        let d = &self.nt_detail;
        match sel {
            Some(g) => {
                let s = socks.get(&g.key).cloned().unwrap_or_default();
                d.caption.set_text(&g.name.to_uppercase());
                d.lines[0].1.set_text(&s.0.to_string());
                d.lines[1].1.set_text(&if s.2.is_empty() { "—".into() } else { hosts(&s.2) });
                d.lines[2].1.set_text(&if s.1.is_empty() { tr("no port").to_string() } else { trf("port {port}", &[("port", s.1.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", "))]) });
                let through = nm.devices.iter().find(|x| x.state == "connected").map(|x| if x.kind == "wifi" { format!("Wi-Fi · {}", x.connection) } else { x.connection.clone() });
                d.lines[3].1.set_text(&through.unwrap_or_else(|| "—".into()));
                d.first.set_sensitive(!g.own);
                d.second.set_sensitive(!g.own);
            }
            None => d.caption.set_text(""),
        }
    }

    /// The day's totals: the counters' growth since the day's first sample (kept across restarts of the shell).
    fn today(&self, rx: u64, tx: u64) -> (u64, u64) {
        let mut d = self.netday.borrow_mut();
        let day = today_start();
        if d.day != day {
            *d = NetDay { day, base_rx: rx, base_tx: tx, ..Default::default() };
        } else if rx < d.last_rx || tx < d.last_tx {
            // (the computer restarted: the counters start from zero again)
            d.carried_rx += d.last_rx.saturating_sub(d.base_rx);
            d.carried_tx += d.last_tx.saturating_sub(d.base_tx);
            d.base_rx = 0;
            d.base_tx = 0;
        }
        d.last_rx = rx;
        d.last_tx = tx;
        let v = (d.carried_rx + rx.saturating_sub(d.base_rx), d.carried_tx + tx.saturating_sub(d.base_tx));
        let p = netday_path();
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&p, serde_json::json!({"dia": d.day, "base_rx": d.base_rx, "base_tx": d.base_tx, "antes_rx": d.carried_rx,
            "antes_tx": d.carried_tx, "ultimo_rx": d.last_rx, "ultimo_tx": d.last_tx}).to_string());
        v
    }

    /// The addresses' names (the resolver's reverse lookup, once each, off the interface's thread).
    fn resolve(self: &Rc<Self>, socks: &HashMap<String, (usize, Vec<u16>, Vec<String>)>) {
        let todo: Vec<String> = socks.values().flat_map(|s| s.2.iter().cloned()).filter(|ip| !self.names.borrow().contains_key(ip)).take(12).collect();
        if todo.is_empty() {
            return;
        }
        for ip in &todo {
            self.names.borrow_mut().insert(ip.clone(), ip.clone());
        }
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(move || {
            let r = gio::Resolver::default();
            todo.into_iter().filter_map(|ip| {
                let a = gio::InetAddress::from_string(&ip)?;
                let n = r.lookup_by_address(&a, gio::Cancellable::NONE).ok()?;
                Some((ip, short_host(&n)))
            }).collect::<Vec<_>>()
        }, move |l: Vec<(String, String)>| {
            if let Some(p) = w.upgrade() {
                for (ip, n) in l {
                    p.names.borrow_mut().insert(ip, n);
                }
            }
        });
    }
}

fn set_caption(c: &Card, text: &str) {
    if let Some(l) = c.root.first_child().and_then(|t| t.last_child()).and_downcast::<gtk::Label>() {
        l.set_text(text);
    }
}

/// "lb-140-82-112-3-iad.github.com" → "github.com".
pub(crate) fn short_host(n: &str) -> String {
    let parts: Vec<&str> = n.trim_end_matches('.').split('.').collect();
    let two_level = parts.len() >= 3 && ["com", "net", "org", "gov", "edu"].contains(&parts[parts.len() - 2]) && parts[parts.len() - 1].len() == 2;
    let keep = if two_level { 3 } else { 2 };
    parts[parts.len().saturating_sub(keep)..].join(".")
}

fn thousands(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1} mi", n as f64 / 1e6).replace('.', ",")
    } else if n >= 1000 {
        format!("{:.1} mil", n as f64 / 1e3).replace('.', ",")
    } else {
        n.to_string()
    }
}

/// The usage log's model name ("LFM3-1.7B-Q4_K_M") and a server's label ("LFM3 1.7B · texto").
fn model_matches(log: &str, label: &str) -> bool {
    let a = log.to_uppercase().replace(['-', '_'], " ");
    let b = label.to_uppercase().replace(['-', '_'], " ");
    let first = b.split(" · ").next().unwrap_or("").trim().to_string();
    !first.is_empty() && a.starts_with(&first)
}

/// "/dev/nvme0n1p2" → "nvme0n1"; "/dev/sda1" → "sda".
fn parent_disk(dev: &str) -> String {
    let name = dev.trim_start_matches("/dev/");
    let block = std::path::Path::new("/sys/class/block").join(name);
    // (a partition's sysfs folder is inside its disk's)
    std::fs::canonicalize(&block).ok().and_then(|p| {
        let parent = p.parent()?.file_name()?.to_string_lossy().into_owned();
        Some(if std::path::Path::new("/sys/block").join(&parent).exists() { parent } else { name.to_string() })
    }).unwrap_or_else(|| name.to_string())
}

fn removable(disk: &str) -> bool {
    std::fs::read_to_string(format!("/sys/block/{disk}/removable")).map(|s| s.trim() == "1").unwrap_or(false)
        || std::fs::canonicalize(format!("/sys/block/{disk}")).map(|p| p.to_string_lossy().contains("/usb")).unwrap_or(false)
}

fn bus(disk: &str) -> &'static str {
    if disk.starts_with("nvme") {
        "NVMe"
    } else if std::fs::canonicalize(format!("/sys/block/{disk}")).map(|p| p.to_string_lossy().contains("/usb")).unwrap_or(false) {
        "USB"
    } else if disk.starts_with("mmc") {
        "SD"
    } else {
        "SATA"
    }
}

/// A disk's health from udisks (its SMART data): "boa", "atenção" or "—" (not readable).
fn smart_health(disk: &str) -> String {
    let Ok(bus) = gio::bus_get_sync(gio::BusType::System, gio::Cancellable::NONE) else { return "—".into() };
    let path = format!("/org/freedesktop/UDisks2/block_devices/{}", disk.replace('-', "_2d"));
    let get = |obj: &str, iface: &str, prop: &str| -> Option<glib::Variant> {
        let r = bus.call_sync(Some("org.freedesktop.UDisks2"), obj, "org.freedesktop.DBus.Properties", "Get",
            Some(&(iface, prop).to_variant()), None, gio::DBusCallFlags::NONE, 3000, gio::Cancellable::NONE).ok()?;
        r.child_value(0).as_variant()
    };
    let Some(drive) = get(&path, "org.freedesktop.UDisks2.Block", "Drive").and_then(|v| v.get::<glib::variant::ObjectPath>()).map(|p| p.to_string()) else { return "—".into() };
    if drive == "/" {
        return "—".into();
    }
    // NVMe: its critical warnings (none: good); ATA: SMART says it is failing or not
    if let Some(w) = get(&drive, "org.freedesktop.UDisks2.NVMe.Controller", "SmartCriticalWarning").and_then(|v| v.get::<Vec<String>>()) {
        return if w.is_empty() { tr("good").into() } else { tr("attention").into() };
    }
    if let Some(f) = get(&drive, "org.freedesktop.UDisks2.Drive.Ata", "SmartFailing").and_then(|v| v.get::<bool>()) {
        let enabled = get(&drive, "org.freedesktop.UDisks2.Drive.Ata", "SmartEnabled").and_then(|v| v.get::<bool>()).unwrap_or(false);
        return if !enabled { "—".into() } else if f { tr("attention").into() } else { tr("good").into() };
    }
    "—".into()
}

/// The home's top-level folders by size (walks for at most ~4 s and 300 000 entries; hidden ones count in their own).
fn biggest_folders() -> Vec<(PathBuf, u64)> {
    let home = glib::home_dir();
    let start = std::time::Instant::now();
    let mut budget = 300_000usize;
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(&home) else { return out };
    for e in rd.flatten() {
        let p = e.path();
        if !p.is_dir() || p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')) {
            continue;
        }
        let mut size = 0u64;
        let mut stack = vec![p.clone()];
        while let Some(d) = stack.pop() {
            if budget == 0 || start.elapsed().as_secs() > 4 {
                break;
            }
            let Ok(rd) = std::fs::read_dir(&d) else { continue };
            for e in rd.flatten() {
                budget = budget.saturating_sub(1);
                let Ok(m) = e.metadata() else { continue };
                if m.is_dir() && !m.file_type().is_symlink() {
                    stack.push(e.path());
                } else if m.is_file() {
                    use std::os::unix::fs::MetadataExt;
                    size += m.blocks() * 512;
                }
            }
        }
        if size > 0 {
            out.push((p, size));
        }
    }
    out.sort_by(|a, b| b.1.cmp(&a.1));
    out.truncate(3);
    out
}

#[cfg(test)]
mod tests {
    use super::{fields, model_matches, short_host};

    #[test]
    fn hosts_models_and_fields() {
        assert_eq!(short_host("lb-140-82-112-3-iad.github.com"), "github.com");
        assert_eq!(short_host("www.bb.com.br."), "bb.com.br");
        assert!(model_matches("LFM3-1.7B-Q4_K_M", "LFM3 1.7B · texto"));
        assert!(!model_matches("LFM3-VL-2B-Q4", "LFM3 1.7B · texto"));
        assert_eq!(fields(r"wlan0:wifi:connected:Casa\:5G"), vec!["wlan0", "wifi", "connected", "Casa:5G"]);
    }
}
