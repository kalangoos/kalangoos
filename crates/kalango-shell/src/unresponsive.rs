//! "App não está respondendo" (design 54053): instead of only "wait or quit", the system says what is stuck and
//! offers the least destructive way out first.
//!
//! How an app is found stuck: the compositor (niri) has no ping, so the shell looks at what every window's process
//! leaves unread on its Wayland connection — the events the compositor sent it (the pointer, the keys, a resize). A
//! working app reads them in milliseconds; one whose interface thread is stuck leaves them in its socket. Bytes waiting
//! on the app's end of the connection (`ss`, by the socket's inode in /proc/PID/fd) for 5 seconds = it is not
//! responding. It answers again as soon as it reads them.
//!
//! The dialog, over the dimmed screen: the app, for how long, and what its process is doing (busy on the processor,
//! waiting for the disk, suspended, idle); the agent's diagnosis (the local model, from the app's processes with their
//! memory and processor — or the numbers alone, when the agents' suggestions are off) with the system's memory, and
//! the least destructive action: resume a suspended app, or end only the process that stands out (a browser's tab, a
//! calculation) — the app often comes back; then "Esperar" (asks again in 30 s if still stuck) and "Forçar
//! encerramento". Its windows stay at 60 % (niri's window rule) until it answers again; the dialog closes by itself
//! when it does.

use crate::conversa::ui_icon;
use crate::superficies::{caixa, camada, rotulo};
use crate::Shell;
use gtk::prelude::*;
use gtk::{gio, glib, Align, Orientation};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kalango_i18n::{tr, trf};
use kalango_movimento::Animavel;
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

/// Unread this long, an app is not responding.
const STUCK: Duration = Duration::from_secs(5);
/// After "Esperar", asked again if it is still stuck.
const ASK_AGAIN: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, PartialEq)]
struct Win {
    id: u64,
    pid: i32,
    app_id: String,
    title: String,
    focused: bool,
}

/// `ss -xn`: the compositor's ends of its connections (the peers' inodes) and every socket's unread bytes.
pub fn parse_ss(out: &str, path: &str) -> (HashSet<u64>, HashMap<u64, u64>) {
    let (mut peers, mut queue) = (HashSet::new(), HashMap::new());
    for l in out.lines() {
        let c: Vec<&str> = l.split_whitespace().collect();
        // u_str ESTAB Recv-Q Send-Q Local inode Peer inode
        if c.len() < 8 || c[0] != "u_str" {
            continue;
        }
        let (Ok(q), Ok(inode), Ok(peer)) = (c[2].parse::<u64>(), c[5].parse::<u64>(), c[7].parse::<u64>()) else { continue };
        if c[4] == path {
            peers.insert(peer);
        }
        queue.insert(inode, q);
    }
    (peers, queue)
}

/// The socket inodes a process holds.
fn sockets(pid: i32) -> Vec<u64> {
    std::fs::read_dir(format!("/proc/{pid}/fd")).into_iter().flatten().flatten()
        .filter_map(|e| std::fs::read_link(e.path()).ok())
        .filter_map(|l| l.to_str().and_then(|s| s.strip_prefix("socket:[")).and_then(|s| s.strip_suffix(']')).and_then(|s| s.parse().ok()))
        .collect()
}

fn wayland_path() -> Option<String> {
    let d = std::env::var("WAYLAND_DISPLAY").ok()?;
    if d.starts_with('/') {
        return Some(d);
    }
    Some(glib::user_runtime_dir().join(d).display().to_string())
}

/// One look (off the interface's thread): the windows, and for each window's process the bytes the compositor sent it
/// that it has not read.
fn look() -> (Vec<Win>, HashMap<i32, u64>) {
    let wins: Vec<Win> = crate::servicos::niri_pedido(json!("Windows")).ok().and_then(|v| v["Windows"].as_array().cloned()).unwrap_or_default()
        .iter().filter_map(|w| Some(Win {
            id: w["id"].as_u64()?, pid: w["pid"].as_i64()? as i32, app_id: w["app_id"].as_str().unwrap_or("").to_string(),
            title: w["title"].as_str().unwrap_or("").to_string(), focused: w["is_focused"].as_bool().unwrap_or(false),
        })).collect();
    let me = std::process::id() as i32;
    let mut out = HashMap::new();
    let Some(path) = wayland_path() else { return (wins, out) };
    let Ok(o) = std::process::Command::new("ss").arg("-xn").stderr(std::process::Stdio::null()).output() else { return (wins, out) };
    let (peers, queue) = parse_ss(&String::from_utf8_lossy(&o.stdout), &path);
    for pid in wins.iter().map(|w| w.pid).filter(|p| *p > 0 && *p != me).collect::<HashSet<_>>() {
        if let Some(q) = sockets(pid).into_iter().find(|i| peers.contains(i)).and_then(|i| queue.get(&i).copied()) {
            out.insert(pid, q);
        }
    }
    (wins, out)
}

/// A process of the app's tree.
#[derive(Clone, Debug, Default)]
pub struct Proc {
    pub pid: i32,
    pub name: String,
    pub rss: u64,
    pub ticks: u64,
    pub state: char,
    /// a browser's tab (Chrome's renderers, Firefox's content processes)
    pub tab: bool,
}

fn read_proc(pid: i32) -> Option<(Proc, i32)> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (_, rest) = stat.rsplit_once(") ")?;
    let f: Vec<&str> = rest.split_whitespace().collect();
    let state = f.first()?.chars().next()?;
    let ppid: i32 = f.get(1)?.parse().ok()?;
    let ticks = f.get(11)?.parse::<u64>().ok()? + f.get(12)?.parse::<u64>().ok()?;
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap_or_default();
    let rss = status.lines().find(|l| l.starts_with("VmRSS:")).and_then(|l| l.split_whitespace().nth(1)).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0) * 1024;
    let cmd = std::fs::read(format!("/proc/{pid}/cmdline")).map(|b| String::from_utf8_lossy(&b).replace('\0', " ")).unwrap_or_default();
    let first = cmd.split_whitespace().next().unwrap_or("");
    let name = std::path::Path::new(first).file_name().map(|n| n.to_string_lossy().into_owned()).filter(|n| !n.is_empty())
        .unwrap_or_else(|| stat.split_once('(').and_then(|(_, r)| r.rsplit_once(')')).map(|(n, _)| n.to_string()).unwrap_or_default());
    let tab = cmd.contains("--type=renderer") || (cmd.contains("-contentproc") && cmd.contains(" tab"));
    Some((Proc { pid, name, rss, ticks, state, tab }, ppid))
}

/// The app's process and everything under it.
pub fn tree(root: i32) -> Vec<Proc> {
    let mut all: Vec<(Proc, i32)> = std::fs::read_dir("/proc").into_iter().flatten().flatten()
        .filter_map(|e| e.file_name().to_str().and_then(|s| s.parse::<i32>().ok())).filter_map(read_proc).collect();
    let mut keep: Vec<Proc> = Vec::new();
    let mut frontier = vec![root];
    if let Some(i) = all.iter().position(|(p, _)| p.pid == root) {
        keep.push(all.remove(i).0);
    }
    while let Some(parent) = frontier.pop() {
        let (kids, rest): (Vec<_>, Vec<_>) = all.into_iter().partition(|(_, pp)| *pp == parent);
        all = rest;
        for (k, _) in kids {
            frontier.push(k.pid);
            keep.push(k);
        }
    }
    keep
}

/// The process to end first: one under the app that stands out by memory (a quarter of the tree's and 100 MB) or by
/// processor (half a core) — never the app itself.
pub fn culprit(procs: &[Proc], cpu: &HashMap<i32, f64>) -> Option<Proc> {
    let total: u64 = procs.iter().map(|p| p.rss).sum();
    let kids = procs.iter().skip(1);
    let by_mem = kids.clone().max_by_key(|p| p.rss).filter(|p| p.rss >= 100_000_000 && p.rss * 4 >= total);
    let by_cpu = kids.clone().filter(|p| cpu.get(&p.pid).copied().unwrap_or(0.0) >= 50.0)
        .max_by(|a, b| cpu.get(&a.pid).unwrap_or(&0.0).total_cmp(cpu.get(&b.pid).unwrap_or(&0.0)));
    by_mem.or(by_cpu).cloned()
}

fn memory() -> (f64, f64) {
    let s = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let kb = |k: &str| s.lines().find(|l| l.starts_with(k)).and_then(|l| l.split_whitespace().nth(1)).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0) * 1024.0;
    let total = kb("MemTotal:");
    (total - kb("MemAvailable:"), total)
}

/// "27,5 / 32 GB" (binary gigabytes, as memory is sold).
fn mem_pair(used: f64, total: f64) -> String {
    format!("{} / {:.0} GB", format!("{:.1}", used / 1073741824.0).replace('.', ","), total / 1073741824.0)
}

fn size(b: u64) -> String {
    if b >= 1_073_741_824 { format!("{} GB", format!("{:.1}", b as f64 / 1073741824.0).replace('.', ",")) } else { format!("{} MB", b / 1_048_576) }
}

/// What the app's tree is doing: the processes, each one's processor (% of a core, over half a second), the memory.
struct Facts {
    procs: Vec<Proc>,
    cpu: HashMap<i32, f64>,
    mem: (f64, f64),
}

fn facts(pid: i32) -> Facts {
    let a = tree(pid);
    std::thread::sleep(Duration::from_millis(500));
    let b = tree(pid);
    let hz = 100.0;
    let cpu = b.iter().map(|p| {
        let before = a.iter().find(|q| q.pid == p.pid).map(|q| q.ticks).unwrap_or(p.ticks);
        (p.pid, (p.ticks.saturating_sub(before)) as f64 / hz / 0.5 * 100.0)
    }).collect();
    Facts { procs: b, cpu, mem: memory() }
}

fn kill(pid: i32, sig: i32) {
    extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    if pid > 0 {
        unsafe {
            kill(pid, sig);
        }
    }
}

/// SIGTERM, then SIGKILL 2 s later to whatever is still there.
fn end_all(pids: Vec<i32>) {
    for p in &pids {
        kill(*p, 15);
    }
    glib::timeout_add_local_once(Duration::from_secs(2), move || {
        for p in pids.iter().filter(|p| std::path::Path::new(&format!("/proc/{p}")).exists()) {
            kill(*p, 9);
        }
    });
}

fn end(pid: i32) {
    end_all(vec![pid]);
}

/// "Forçar encerramento": the app and every process under it (a child left behind would keep its memory).
fn end_tree(pid: i32) {
    end_all(tree(pid).into_iter().map(|p| p.pid).collect());
}

#[derive(Clone)]
struct Stuck {
    win: Win,
    name: String,
    since: Instant,
}

struct Dialog {
    window: gtk::ApplicationWindow,
    card: Animavel,
    icon: gtk::Box,
    title: gtk::Label,
    sub: gtk::Label,
    diag: gtk::Label,
    bar: gtk::LevelBar,
    mem: gtk::Label,
    first: gtk::Button,
    first_label: gtk::Label,
    shown: RefCell<Option<Stuck>>,
    /// what the first button does: resume (SIGCONT) or end that process
    action: RefCell<Option<(i32, bool)>>,
    state_text: RefCell<String>,
}

pub struct Watcher {
    shell: Weak<Shell>,
    since: RefCell<HashMap<i32, Instant>>,
    asked: RefCell<HashMap<i32, Instant>>,
    dimmed: RefCell<HashMap<i32, String>>,
    dialog: RefCell<Option<Rc<Dialog>>>,
    busy: Cell<bool>,
}

thread_local! {
    static WATCHER: RefCell<Option<Rc<Watcher>>> = const { RefCell::new(None) };
}

/// Starts looking (every 2 s, off the interface's thread).
pub fn start(shell: &Rc<Shell>) {
    let w = Rc::new(Watcher { shell: Rc::downgrade(shell), since: RefCell::default(), asked: RefCell::default(), dimmed: RefCell::default(), dialog: RefCell::default(), busy: Cell::new(false) });
    WATCHER.with(|x| *x.borrow_mut() = Some(w.clone()));
    glib::timeout_add_local(Duration::from_secs(2), move || {
        w.tick();
        glib::ControlFlow::Continue
    });
}

impl Watcher {
    fn tick(self: &Rc<Self>) {
        if self.busy.replace(true) {
            return;
        }
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(look, move |(wins, queues)| {
            let Some(s) = w.upgrade() else { return };
            s.busy.set(false);
            s.seen(&wins, &queues);
        });
    }

    fn seen(self: &Rc<Self>, wins: &[Win], queues: &HashMap<i32, u64>) {
        let now = Instant::now();
        let pids: HashSet<i32> = wins.iter().map(|w| w.pid).collect();
        // answering (or gone): forget, bring its windows back, close its dialog
        let answering: Vec<i32> = self.since.borrow().keys().copied().filter(|p| queues.get(p).copied().unwrap_or(0) == 0).collect();
        for pid in answering {
            self.since.borrow_mut().remove(&pid);
            self.asked.borrow_mut().remove(&pid);
            if let Some(app) = self.dimmed.borrow_mut().remove(&pid) {
                crate::window_prefs::set_dimmed(&app, false);
            }
            let shown = self.dialog.borrow().as_ref().and_then(|d| d.shown.borrow().as_ref().map(|x| x.win.pid));
            if shown == Some(pid) {
                if let Some(d) = self.dialog.borrow().clone() {
                    d.close();
                }
            }
            if debug() {
                eprintln!("debug travado: {} {pid}", if pids.contains(&pid) { "voltou a responder:" } else { "fechado:" });
            }
        }
        for (pid, q) in queues.iter().filter(|(_, q)| **q > 0) {
            let first = *self.since.borrow_mut().entry(*pid).or_insert(now);
            if now.duration_since(first) < STUCK {
                continue;
            }
            let Some(win) = wins.iter().filter(|w| w.pid == *pid).max_by_key(|w| w.focused).cloned() else { continue };
            // its windows at 60 % until it answers
            if !self.dimmed.borrow().contains_key(pid) {
                self.dimmed.borrow_mut().insert(*pid, win.app_id.clone());
                crate::window_prefs::set_dimmed(&win.app_id, true);
                if debug() {
                    eprintln!("debug travado: janelas de {} a 60%", win.app_id);
                }
            }
            let open = self.dialog.borrow().as_ref().is_some_and(|d| d.shown.borrow().is_some());
            let asked = self.asked.borrow().get(pid).copied();
            if open || asked.is_some_and(|t| now.duration_since(t) < ASK_AGAIN) {
                continue;
            }
            self.asked.borrow_mut().insert(*pid, now);
            if debug() {
                eprintln!("debug travado: {} ({}) sem ler os eventos há {} s ({q} bytes esperando)", win.app_id, pid, now.duration_since(first).as_secs());
            }
            self.ask(win, first);
        }
    }

    fn name_of(&self, win: &Win) -> String {
        self.shell.upgrade().and_then(|s| crate::apps::da_janela(&win.app_id, &s.todos).map(|a| a.nome.clone()))
            .unwrap_or_else(|| win.title.split(" — ").next().unwrap_or(&win.title).trim().to_string())
    }

    fn ask(self: &Rc<Self>, win: Win, since: Instant) {
        let Some(d) = self.dialog() else { return };
        let name = self.name_of(&win);
        // the app's icon
        while let Some(c) = d.icon.first_child() {
            d.icon.remove(&c);
        }
        let img = self.shell.upgrade().and_then(|s| crate::apps::da_janela(&win.app_id, &s.todos).map(|a| crate::apps::imagem(a, 36)))
            .unwrap_or_else(|| ui_icon("apps-n", "#8E8F99", 24));
        img.set_hexpand(true);
        d.icon.append(&img);
        d.title.set_text(&trf("{app} is not responding", &[("app", name.clone())]));
        d.diag.set_text(tr("The agent is looking at what is holding it…"));
        d.first.set_visible(false);
        *d.action.borrow_mut() = None;
        *d.state_text.borrow_mut() = String::new();
        *d.shown.borrow_mut() = Some(Stuck { win: win.clone(), name: name.clone(), since });
        d.refresh_sub();
        let (used, total) = memory();
        d.set_memory(used, total);
        d.window.present();
        d.window.set_keyboard_mode(KeyboardMode::Exclusive);
        d.window.add_css_class("aberto");
        d.card.entrar((0.0, 10.0), 0.96);
        // what it is doing, and the agent's reading of it
        let pid = win.pid;
        let w = Rc::downgrade(&d);
        let suggestions = crate::agent_prefs::load().suggestions;
        crate::lateral::em_segundo_plano(move || facts(pid), move |f: Facts| {
            let Some(d) = w.upgrade() else { return };
            if d.shown.borrow().as_ref().map(|s| s.win.pid) != Some(pid) {
                return;
            }
            d.show_facts(f, suggestions);
        });
    }

    fn dialog(self: &Rc<Self>) -> Option<Rc<Dialog>> {
        if let Some(d) = self.dialog.borrow().clone() {
            return Some(d);
        }
        let app = gio::Application::default().and_downcast::<gtk::Application>()?;
        let window = camada(&app, "kalango-travado", Layer::Overlay, &[Edge::Top, Edge::Bottom, Edge::Left, Edge::Right]);
        window.add_css_class("wifi-fundo");
        window.set_exclusive_zone(-1);
        let body = caixa(Orientation::Vertical, 14, "wifi-cartao");
        body.set_size_request(500, -1);
        let head = caixa(Orientation::Horizontal, 14, "");
        let icon = caixa(Orientation::Horizontal, 0, "");
        icon.set_size_request(44, 44);
        icon.set_valign(Align::Start);
        icon.set_hexpand(false);
        head.append(&icon);
        let t = caixa(Orientation::Vertical, 4, "");
        t.set_hexpand(true);
        let title = rotulo("", "travado-titulo");
        title.set_xalign(0.0);
        title.set_wrap(true);
        title.set_max_width_chars(20);
        t.append(&title);
        let sub = rotulo("", "travado-texto");
        sub.set_xalign(0.0);
        sub.set_wrap(true);
        sub.set_max_width_chars(20);
        t.append(&sub);
        head.append(&t);
        body.append(&head);
        // the agent's diagnosis
        let card = caixa(Orientation::Vertical, 8, "travado-diagnostico");
        let lab = caixa(Orientation::Horizontal, 6, "");
        lab.append(&ui_icon("spark", "#A5A1E2", 16));
        lab.append(&rotulo(tr("AGENT'S DIAGNOSIS"), "travado-rotulo"));
        card.append(&lab);
        let diag = rotulo("", "travado-diag-texto");
        diag.set_xalign(0.0);
        diag.set_wrap(true);
        diag.set_max_width_chars(20);
        card.append(&diag);
        let row = caixa(Orientation::Horizontal, 8, "");
        let bar = gtk::LevelBar::for_interval(0.0, 1.0);
        bar.add_css_class("travado-barra");
        bar.set_hexpand(true);
        bar.set_valign(Align::Center);
        row.append(&bar);
        let mem = rotulo("", "travado-memoria");
        row.append(&mem);
        card.append(&row);
        let first = gtk::Button::new();
        first.add_css_class("travado-primeiro");
        first.set_halign(Align::Start);
        let first_label = rotulo("", "");
        first.set_child(Some(&first_label));
        card.append(&first);
        body.append(&card);
        let acts = caixa(Orientation::Horizontal, 8, "");
        let gap = caixa(Orientation::Horizontal, 0, "");
        gap.set_hexpand(true);
        acts.append(&gap);
        let wait = gtk::Button::with_label(tr("Wait"));
        wait.add_css_class("wifi-botao");
        acts.append(&wait);
        let force = gtk::Button::with_label(tr("Force quit"));
        force.add_css_class("travado-forcar");
        acts.append(&force);
        body.append(&acts);
        let anim = Animavel::new(&body);
        anim.origem(0.5, 0.5);
        anim.set_halign(Align::Center);
        anim.set_valign(Align::Center);
        window.set_child(Some(&anim));
        let d = Rc::new(Dialog {
            window, card: anim, icon, title, sub, diag, bar, mem, first: first.clone(), first_label, shown: RefCell::default(), action: RefCell::default(),
            state_text: RefCell::default(),
        });
        crate::central::posicao_debug_sempre("travado primeiro", first.upcast_ref());
        crate::central::posicao_debug_sempre("travado esperar", wait.upcast_ref());
        crate::central::posicao_debug_sempre("travado forcar", force.upcast_ref());
        let dw = Rc::downgrade(&d);
        first.connect_clicked(move |_| {
            let Some(d) = dw.upgrade() else { return };
            let Some((pid, resume)) = *d.action.borrow() else { return };
            if resume {
                crate::sistema::pausar_processo(pid, false);
            } else {
                end(pid);
            }
            if debug() {
                eprintln!("debug travado: {} {pid}", if resume { "retomar" } else { "encerrar só" });
            }
            d.first.set_sensitive(false);
            d.diag.set_text(tr("Done. Waiting for the app to answer again…"));
        });
        let (dw, me) = (Rc::downgrade(&d), Rc::downgrade(self));
        wait.connect_clicked(move |_| {
            if let (Some(d), Some(_)) = (dw.upgrade(), me.upgrade()) {
                if debug() {
                    eprintln!("debug travado: esperar");
                }
                d.close();
            }
        });
        let dw = Rc::downgrade(&d);
        force.connect_clicked(move |_| {
            let Some(d) = dw.upgrade() else { return };
            let Some(s) = d.shown.borrow().clone() else { return };
            if debug() {
                eprintln!("debug travado: forçar encerramento {} ({})", s.name, s.win.pid);
            }
            end_tree(s.win.pid);
            d.close();
        });
        // Esc waits
        let keys = gtk::EventControllerKey::new();
        let dw = Rc::downgrade(&d);
        keys.connect_key_pressed(move |_, k, _, _| {
            if k == gtk::gdk::Key::Escape {
                if let Some(d) = dw.upgrade() {
                    d.close();
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        d.window.add_controller(keys);
        // "há N segundos", every second while open
        let dw = Rc::downgrade(&d);
        glib::timeout_add_local(Duration::from_secs(1), move || {
            let Some(d) = dw.upgrade() else { return glib::ControlFlow::Break };
            if d.shown.borrow().is_some() {
                d.refresh_sub();
            }
            glib::ControlFlow::Continue
        });
        *self.dialog.borrow_mut() = Some(d.clone());
        Some(d)
    }
}

impl Dialog {
    /// The system's memory: red from three quarters up (the design's full bar), the accent below.
    fn set_memory(&self, used: f64, total: f64) {
        let f = if total > 0.0 { used / total } else { 0.0 };
        self.bar.set_value(f);
        if f >= 0.75 { self.bar.add_css_class("cheio") } else { self.bar.remove_css_class("cheio") }
        self.mem.set_text(&mem_pair(used, total));
    }

    fn refresh_sub(&self) {
        let Some(s) = self.shown.borrow().clone() else { return };
        let secs = s.since.elapsed().as_secs();
        let mut t = trf("No answer for {seconds} seconds.", &[("seconds", secs.to_string())]);
        let st = self.state_text.borrow();
        if !st.is_empty() {
            t = format!("{t} {st}");
        }
        self.sub.set_text(&t);
    }

    fn show_facts(self: &Rc<Self>, f: Facts, suggestions: bool) {
        let Some(s) = self.shown.borrow().clone() else { return };
        let main = f.procs.first().cloned().unwrap_or_default();
        let main_cpu = f.cpu.get(&main.pid).copied().unwrap_or(0.0);
        let culprit = culprit(&f.procs, &f.cpu);
        *self.state_text.borrow_mut() = match main.state {
            'T' | 't' => tr("It is suspended.").to_string(),
            'D' => tr("It is waiting for the disk or the network.").to_string(),
            'R' if main_cpu >= 50.0 => trf("It is busy on the processor ({cpu}%).", &[("cpu", format!("{main_cpu:.0}"))]),
            _ if culprit.is_some() => tr("It is waiting for one of its processes.").to_string(),
            _ => tr("It is stopped, waiting for something.").to_string(),
        };
        self.refresh_sub();
        // the least destructive way out
        let action = if matches!(main.state, 'T' | 't') {
            self.first_label.set_text(tr("Resume"));
            Some((main.pid, true))
        } else if let Some(c) = &culprit {
            self.first_label.set_text(&if c.tab { tr("Close just that tab").to_string() } else { trf("End just “{process}”", &[("process", c.name.clone())]) });
            Some((c.pid, false))
        } else {
            None
        };
        self.first.set_visible(action.is_some());
        self.first.set_sensitive(true);
        *self.action.borrow_mut() = action;
        let (used, total) = f.mem;
        self.set_memory(used, total);
        if debug() {
            eprintln!("debug travado: diálogo «{}» · estado {} · {} processos · destaque {}", s.name, main.state, f.procs.len(),
                culprit.as_ref().map(|c| format!("{} ({}, {:.0}%)", c.name, size(c.rss), f.cpu.get(&c.pid).copied().unwrap_or(0.0))).unwrap_or_else(|| "nenhum".into()));
        }
        // the numbers alone, or the local model's reading of them
        let numbers = match &culprit {
            Some(c) => trf("{process} is using {memory} of memory and {cpu}% of the processor; {app} waits for it.", &[
                ("process", c.name.clone()), ("memory", size(c.rss)), ("cpu", format!("{:.0}", f.cpu.get(&c.pid).copied().unwrap_or(0.0))), ("app", s.name.clone())]),
            None => trf("{app} uses {memory} of memory and {cpu}% of the processor.", &[("app", s.name.clone()), ("memory", size(main.rss)), ("cpu", format!("{main_cpu:.0}"))]),
        };
        if !suggestions {
            if debug() {
                eprintln!("debug travado: diagnóstico «{numbers}» (só os números)");
            }
            self.diag.set_text(&numbers);
            return;
        }
        let input = json!({
            "app": s.name, "segundos_sem_resposta": s.since.elapsed().as_secs(), "estado": main.state.to_string(),
            "processos": f.procs.iter().take(12).map(|p| {
                let mut j = json!({"nome": p.name, "memoria": size(p.rss), "processador": format!("{:.0}%", f.cpu.get(&p.pid).copied().unwrap_or(0.0)),
                    "principal": p.pid == main.pid});
                // only what is true: "a browser's tab: false" made the model talk of tabs
                if p.tab {
                    j["aba_do_navegador"] = json!(true);
                }
                j
            }).collect::<Vec<_>>(),
            "memoria_do_sistema": mem_pair(used, total),
            "destaque": culprit.as_ref().map(|c| json!({"nome": c.name, "memoria": size(c.rss), "tipo": if c.tab { "aba do navegador" } else { "processo" },
                "processador": format!("{:.0}%", f.cpu.get(&c.pid).copied().unwrap_or(0.0)),
                "parte_da_memoria": format!("{:.1}%", if total > 0.0 { c.rss as f64 / total * 100.0 } else { 0.0 }).replace('.', ","),
                "o_app_espera_por_ele": true})),
        }).to_string();
        let bin = crate::voz::agent_binary();
        let (w, pid) = (Rc::downgrade(self), s.win.pid);
        crate::lateral::em_segundo_plano(move || {
            use std::io::Write;
            let mut c = std::process::Command::new(&bin).arg("travado").stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn().ok()?;
            c.stdin.take()?.write_all(input.as_bytes()).ok()?;
            let o = c.wait_with_output().ok()?;
            let v: Value = serde_json::from_slice(&o.stdout).ok()?;
            Some(v["diagnostico"].as_str()?.trim().to_string()).filter(|s| !s.is_empty())
        }, move |r: Option<String>| {
            let Some(d) = w.upgrade() else { return };
            if d.shown.borrow().as_ref().map(|s| s.win.pid) != Some(pid) || !d.first.is_sensitive() && d.first.is_visible() {
                return;
            }
            let text = r.unwrap_or(numbers);
            if debug() {
                eprintln!("debug travado: diagnóstico «{text}»");
            }
            d.diag.set_text(&text);
        });
    }

    fn close(self: &Rc<Self>) {
        if self.shown.borrow_mut().take().is_none() {
            return;
        }
        self.window.set_keyboard_mode(KeyboardMode::None);
        self.window.remove_css_class("aberto");
        let w = Rc::downgrade(self);
        self.card.sair((0.0, 8.0), 0.98, move || {
            if let Some(d) = w.upgrade().filter(|d| d.shown.borrow().is_none()) {
                d.window.set_visible(false);
            }
        });
        if debug() {
            eprintln!("debug travado: diálogo fechado");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unread_bytes_on_the_apps_end_of_the_compositors_connection() {
        let out = "Netid State Recv-Q Send-Q Local Address:Port Peer Address:Port Process\n\
            u_str ESTAB 0      5376 /run/user/1000/wayland-3 109171380            * 109182107\n\
            u_str ESTAB 204    0                           * 109182107            * 109171380\n\
            u_str ESTAB 0      0    /run/user/1000/wayland-3 109171390            * 109182200\n\
            u_str ESTAB 0      0                           * 109182200            * 109171390\n\
            u_str ESTAB 9      0    /tmp/dbus-x 1            * 2\n";
        let (peers, q) = parse_ss(out, "/run/user/1000/wayland-3");
        assert_eq!(peers, [109182107, 109182200].into_iter().collect());
        assert_eq!(q.get(&109182107), Some(&204));
        assert_eq!(q.get(&109182200), Some(&0));
    }

    #[test]
    fn the_process_to_end_first_stands_out_and_is_never_the_app() {
        let p = |pid, rss, tab| Proc { pid, name: format!("p{pid}"), rss, ticks: 0, state: 'S', tab };
        let procs = vec![p(1, 50_000_000, false), p(2, 210_000_000, false), p(3, 20_000_000, true)];
        assert_eq!(culprit(&procs, &HashMap::new()).map(|c| c.pid), Some(2));
        let procs = vec![p(1, 900_000_000, false), p(2, 110_000_000, false)];
        assert_eq!(culprit(&procs, &HashMap::new()).map(|c| c.pid), None);
        let cpu: HashMap<i32, f64> = [(3, 97.0)].into_iter().collect();
        let procs = vec![p(1, 50_000_000, false), p(3, 20_000_000, true)];
        assert_eq!(culprit(&procs, &cpu).map(|c| (c.pid, c.tab)), Some((3, true)));
        assert_eq!(mem_pair(29_527_900_160.0, 34_359_738_368.0), "27,5 / 32 GB");
    }
}
