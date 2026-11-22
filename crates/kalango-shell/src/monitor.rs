//! KalangoOS's System Monitor (designs "Monitor do sistema" and its "· Processos", "· Agentes", "· Discos" and
//! "· Rede"; app id os.kalango.Monitor; `kalango-shell monitor [page]`), in place of GNOME's.
//! - **Visão geral:** the processor, the memory, the GPU and the network as they are now, with a minute of history
//!   that slides left once a second; the apps that weigh most; "O que está pesando" — the local model explains the
//!   spike instead of only showing it; the chosen app's details, "Suspender" and "Encerrar".
//! - **Processos:** every process, grouped by the app whose window it belongs to (an app opens to show its own),
//!   filtered (apps, the system's, the agents'), sorted by the column clicked; the list reorders with a spring.
//! - **Agentes, Discos, Rede:** monitor_paginas.rs.

use crate::conversa::ui_icon;
use crate::monitor_dados::{self as dados, Snap};
use crate::superficies::{caixa, rotulo};
use crate::Shell;
use gtk::prelude::*;
use gtk::{gdk, glib, pango, Align, Orientation};
use kalango_i18n::{tr, trf};
use kalango_movimento::{Animavel, Canal, SUAVE};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::{c_char, CString};
use std::rc::{Rc, Weak};

extern "C" {
    fn gdk_wayland_toplevel_set_application_id(toplevel: *mut gdk::ffi::GdkToplevel, id: *const c_char);
    fn kill(pid: i32, sig: i32) -> i32;
}

pub(crate) fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

/// A minute of history.
pub(crate) const HIST: usize = 60;
/// A process table's row height (the design's 46).
const ROW_H: f64 = 46.0;

// ------------------------------------------------------------------------------------------------- formats

pub(crate) fn bytes(b: f64) -> String {
    let gb = b / 1e9;
    if gb >= 100.0 {
        format!("{:.0} GB", gb)
    } else if gb >= 1.0 {
        format!("{:.1} GB", gb).replace('.', ",")
    } else if b >= 1e6 {
        format!("{:.0} MB", b / 1e6)
    } else if b >= 1e3 {
        format!("{:.0} KB", b / 1e3)
    } else if b > 0.0 {
        format!("{:.0} B", b)
    } else {
        "0".into()
    }
}

pub(crate) fn rate(b: f64) -> String {
    if b < 1.0 {
        return "0 B/s".into();
    }
    let s = bytes(b);
    format!("{s}/s")
}

pub(crate) fn pct(v: f64) -> String {
    format!("{:.0}%", v.clamp(0.0, 100.0))
}

/// "hoje, 00:12", "ontem, 22:10", "12/11, 09:40".
pub(crate) fn when(unix: i64) -> String {
    let Ok(t) = glib::DateTime::from_unix_local(unix) else { return String::new() };
    let Ok(now) = glib::DateTime::now_local() else { return String::new() };
    let hm = t.format("%H:%M").map(|s| s.to_string()).unwrap_or_default();
    let days = (now.ymd().0 * 400 + now.day_of_year()) - (t.ymd().0 * 400 + t.day_of_year());
    match days {
        0 => trf("today, {time}", &[("time", hm)]),
        1 => trf("yesterday, {time}", &[("time", hm)]),
        _ => format!("{}, {hm}", t.format("%d/%m").map(|s| s.to_string()).unwrap_or_default()),
    }
}

// ------------------------------------------------------------------------------------------------- a card with history

/// A resource card (design "CPU", "Memória"…): its caption, the value now, a line under it and a minute of history.
pub(crate) struct Card {
    pub root: gtk::Box,
    pub value: gtk::Label,
    pub sub: gtk::Label,
    pub graph: gtk::DrawingArea,
    pub data: Rc<RefCell<VecDeque<f64>>>,
}

impl Card {
    pub(crate) fn new(icon: &str, caption: &str, color: &'static str) -> Card {
        let root = caixa(Orientation::Vertical, 6, "mon-cartao");
        root.set_hexpand(true);
        let top = caixa(Orientation::Horizontal, 6, "");
        top.append(&ui_icon(icon, color, 14));
        top.append(&rotulo(caption, "mon-legenda"));
        root.append(&top);
        let value = rotulo("—", "mon-valor");
        value.set_xalign(0.0);
        root.append(&value);
        let sub = rotulo("", "mon-sub");
        sub.set_xalign(0.0);
        sub.set_ellipsize(pango::EllipsizeMode::End);
        root.append(&sub);
        let graph = gtk::DrawingArea::new();
        graph.set_content_height(54);
        graph.set_hexpand(true);
        let data: Rc<RefCell<VecDeque<f64>>> = Rc::default();
        let (d, rgba) = (data.clone(), gdk::RGBA::parse(color).unwrap_or(gdk::RGBA::WHITE));
        graph.set_draw_func(move |_, cr, w, h| {
            let d = d.borrow();
            if d.len() < 2 {
                return;
            }
            let (w, h) = (f64::from(w), f64::from(h));
            let max = d.iter().cloned().fold(1e-9, f64::max).max(1e-9);
            let step = w / (HIST - 1) as f64;
            // (the newest at the right edge: the rest has slid left one step a second)
            let x0 = w - step * (d.len() - 1) as f64;
            let pt = |i: usize, v: f64| (x0 + step * i as f64, h - 2.0 - (v / max).clamp(0.0, 1.0) * (h - 6.0));
            cr.move_to(x0, h);
            for (i, v) in d.iter().enumerate() {
                let (x, y) = pt(i, *v);
                cr.line_to(x, y);
            }
            cr.line_to(w, h);
            cr.close_path();
            let grad = gtk::cairo::LinearGradient::new(0.0, 0.0, 0.0, h);
            grad.add_color_stop_rgba(0.0, rgba.red().into(), rgba.green().into(), rgba.blue().into(), 0.28);
            grad.add_color_stop_rgba(1.0, rgba.red().into(), rgba.green().into(), rgba.blue().into(), 0.02);
            let _ = cr.set_source(&grad);
            let _ = cr.fill();
            for (i, v) in d.iter().enumerate() {
                let (x, y) = pt(i, *v);
                if i == 0 { cr.move_to(x, y) } else { cr.line_to(x, y) }
            }
            cr.set_source_rgba(rgba.red().into(), rgba.green().into(), rgba.blue().into(), 1.0);
            cr.set_line_width(1.5);
            cr.set_line_join(gtk::cairo::LineJoin::Round);
            let _ = cr.stroke();
        });
        root.append(&graph);
        Card { root, value, sub, graph, data }
    }

    pub(crate) fn push(&self, v: f64) {
        let mut d = self.data.borrow_mut();
        d.push_back(v);
        while d.len() > HIST {
            d.pop_front();
        }
        drop(d);
        self.graph.queue_draw();
    }
}

// ------------------------------------------------------------------------------------------------- what adapts to the window

/// A row of cards that wraps: four side by side on a wide window, two by two or one under the other on a narrow one.
pub(crate) fn card_row(cards: &[&gtk::Box]) -> gtk::FlowBox {
    let f = gtk::FlowBox::new();
    f.set_selection_mode(gtk::SelectionMode::None);
    f.set_homogeneous(true);
    f.set_min_children_per_line(1);
    f.set_max_children_per_line(cards.len().max(1) as u32);
    f.set_column_spacing(14);
    f.set_row_spacing(14);
    f.set_hexpand(true);
    for c in cards {
        c.set_size_request(240, -1);
        f.insert(*c, -1);
    }
    f
}

/// The table and its side (the note, the details): side by side from 1100 px, the side under the table below that.
pub(crate) fn body(table: &gtk::Widget, side: &gtk::Box) -> gtk::Box {
    let b = caixa(Orientation::Horizontal, 14, "");
    b.set_vexpand(true);
    table.set_hexpand(true);
    side.set_size_request(400, -1);
    side.set_hexpand(false);
    b.append(table);
    b.append(side);
    b
}

/// Lays the bodies out for this width (a meter on the window calls it as it changes).
pub(crate) fn fit_bodies(bodies: &[(gtk::Box, gtk::Box)], width: i32) {
    let narrow = width < 1100;
    for (b, side) in bodies {
        let want = if narrow { Orientation::Vertical } else { Orientation::Horizontal };
        if b.orientation() != want {
            b.set_orientation(want);
            side.set_size_request(if narrow { -1 } else { 400 }, -1);
            side.set_hexpand(narrow);
        }
    }
}

// ------------------------------------------------------------------------------------------------- a table that reorders with a spring

/// A row's look: what it shows, and how.
#[derive(Clone)]
pub(crate) struct RowData {
    pub key: String,
    pub name: String,
    pub sub: String,
    pub icon: RowIcon,
    pub values: Vec<String>,
    /// the last value's colour ("alta" in coral, "trabalhando" in the accent)
    pub last_css: Option<&'static str>,
    pub indent: bool,
    /// a group that opens (Processos): its chevron, open or closed
    pub opens: Option<bool>,
}

#[derive(Clone)]
pub(crate) enum RowIcon {
    App(crate::apps::App),
    Glyph(&'static str),
}

struct Row {
    anim: Animavel,
    root: gtk::Box,
    icon: gtk::Box,
    chevron: gtk::Button,
    name: gtk::Label,
    sub: gtk::Label,
    values: Vec<gtk::Label>,
    data: RefCell<Option<RowData>>,
}

/// A table (design "Processos"): a header, rows that keep their place by key and slide to a new one with a spring
/// when the order changes (FLIP: the row is laid out where it goes at once, then drawn from where it was).
pub(crate) struct Table {
    pub root: gtk::Box,
    header: Vec<gtk::Button>,
    list: gtk::Box,
    rows: RefCell<HashMap<String, Rc<Row>>>,
    order: RefCell<Vec<String>>,
    selected: RefCell<Option<String>>,
    on_select: RefCell<Option<Box<dyn Fn(&str)>>>,
    on_open: RefCell<Option<Box<dyn Fn(&str)>>>,
    on_sort: RefCell<Option<Box<dyn Fn(usize)>>>,
}

impl Table {
    pub(crate) fn new(columns: &[&str], name: &str) -> Rc<Table> {
        let root = caixa(Orientation::Vertical, 0, "mon-tabela");
        let head = caixa(Orientation::Horizontal, 8, "mon-cabecalho");
        let mut header = Vec::new();
        for (i, c) in std::iter::once(name).chain(columns.iter().copied()).enumerate() {
            let b = gtk::Button::with_label(c);
            b.add_css_class("mon-coluna");
            if i == 0 {
                b.set_hexpand(true);
                if let Some(l) = b.child().and_downcast::<gtk::Label>() {
                    l.set_xalign(0.0);
                }
            } else {
                b.set_size_request(96, -1);
                if let Some(l) = b.child().and_downcast::<gtk::Label>() {
                    l.set_xalign(1.0);
                }
            }
            head.append(&b);
            header.push(b);
        }
        root.append(&head);
        let list = caixa(Orientation::Vertical, 0, "");
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
        scroll.set_vexpand(true);
        scroll.set_propagate_natural_height(true);
        scroll.set_child(Some(&list));
        root.append(&scroll);
        let t = Rc::new(Table {
            root, header, list, rows: RefCell::default(), order: RefCell::default(), selected: RefCell::default(),
            on_select: RefCell::default(), on_open: RefCell::default(), on_sort: RefCell::default(),
        });
        for (i, b) in t.header.iter().enumerate() {
            let w = Rc::downgrade(&t);
            b.connect_clicked(move |_| {
                if let Some(t) = w.upgrade() {
                    if let Some(f) = t.on_sort.borrow().as_ref() {
                        f(i);
                    }
                }
            });
        }
        t
    }

    pub(crate) fn set_header(&self, i: usize, text: &str) {
        if let Some(b) = self.header.get(i) {
            b.set_label(text);
        }
    }

    pub(crate) fn on_select(&self, f: impl Fn(&str) + 'static) {
        *self.on_select.borrow_mut() = Some(Box::new(f));
    }
    pub(crate) fn on_open(&self, f: impl Fn(&str) + 'static) {
        *self.on_open.borrow_mut() = Some(Box::new(f));
    }
    pub(crate) fn on_sort(&self, f: impl Fn(usize) + 'static) {
        *self.on_sort.borrow_mut() = Some(Box::new(f));
    }

    pub(crate) fn select(&self, key: Option<&str>) {
        *self.selected.borrow_mut() = key.map(str::to_string);
        for (k, r) in self.rows.borrow().iter() {
            if Some(k.as_str()) == key { r.root.add_css_class("escolhida") } else { r.root.remove_css_class("escolhida") }
        }
    }

    fn row(self: &Rc<Self>, key: &str, columns: usize) -> Rc<Row> {
        if let Some(r) = self.rows.borrow().get(key) {
            return r.clone();
        }
        let root = caixa(Orientation::Horizontal, 8, "mon-linha");
        root.set_size_request(-1, ROW_H as i32);
        let who = caixa(Orientation::Horizontal, 10, "");
        who.set_hexpand(true);
        let chevron = gtk::Button::new();
        chevron.add_css_class("mon-abrir");
        chevron.set_child(Some(&ui_icon("chevron", "#8E8F99", 10)));
        chevron.set_valign(Align::Center);
        chevron.set_visible(false);
        who.append(&chevron);
        let icon = caixa(Orientation::Horizontal, 0, "");
        icon.set_size_request(24, 24);
        icon.set_valign(Align::Center);
        // (24 × 24 as drawn: the image centred in it does not widen it)
        icon.set_hexpand(false);
        who.append(&icon);
        let names = caixa(Orientation::Vertical, 1, "");
        names.set_valign(Align::Center);
        names.set_hexpand(true);
        let name = rotulo("", "mon-nome");
        name.set_xalign(0.0);
        name.set_ellipsize(pango::EllipsizeMode::End);
        let sub = rotulo("", "mon-sub");
        sub.set_xalign(0.0);
        sub.set_ellipsize(pango::EllipsizeMode::End);
        names.append(&name);
        names.append(&sub);
        who.append(&names);
        root.append(&who);
        let mut values = Vec::new();
        for _ in 0..columns {
            let l = rotulo("", "mon-numero");
            l.set_xalign(1.0);
            l.set_size_request(96, -1);
            l.set_valign(Align::Center);
            root.append(&l);
            values.push(l);
        }
        let anim = Animavel::new(&root);
        let r = Rc::new(Row { anim, root, icon, chevron, name, sub, values, data: RefCell::default() });
        let click = gtk::GestureClick::new();
        let (w, k) = (Rc::downgrade(self), key.to_string());
        click.connect_released(move |_, _, _, _| {
            if let Some(t) = w.upgrade() {
                t.select(Some(&k));
                if let Some(f) = t.on_select.borrow().as_ref() {
                    f(&k);
                }
            }
        });
        r.root.add_controller(click);
        let (w, k) = (Rc::downgrade(self), key.to_string());
        r.chevron.connect_clicked(move |_| {
            if let Some(t) = w.upgrade() {
                if let Some(f) = t.on_open.borrow().as_ref() {
                    f(&k);
                }
            }
        });
        self.rows.borrow_mut().insert(key.to_string(), r.clone());
        r
    }

    /// Shows these rows in this order: new ones come in, gone ones go, moved ones slide.
    pub(crate) fn update(self: &Rc<Self>, data: &[RowData]) {
        let keys: Vec<String> = data.iter().map(|d| d.key.clone()).collect();
        let old: HashMap<String, usize> = self.order.borrow().iter().enumerate().map(|(i, k)| (k.clone(), i)).collect();
        // gone
        let gone: Vec<String> = self.rows.borrow().keys().filter(|k| !keys.contains(k)).cloned().collect();
        for k in gone {
            if let Some(r) = self.rows.borrow_mut().remove(&k) {
                self.list.remove(&r.anim);
            }
        }
        let mut prev: Option<gtk::Widget> = None;
        for (i, d) in data.iter().enumerate() {
            let r = self.row(&d.key, d.values.len());
            if r.anim.parent().is_none() {
                self.list.append(&r.anim);
            }
            self.list.reorder_child_after(&r.anim, prev.as_ref());
            prev = Some(r.anim.clone().upcast());
            // where it was: it is drawn from there and slides to its new place
            if let Some(&o) = old.get(&d.key) {
                if o != i {
                    let from = (o as f64 - i as f64) * ROW_H + r.anim.valor(Canal::Y);
                    r.anim.definir(Canal::Y, from);
                    r.anim.ir(Canal::Y, 0.0, SUAVE);
                }
            }
            let changed = r.data.borrow().as_ref().map(|x| x.name != d.name || x.indent != d.indent).unwrap_or(true);
            if changed {
                while let Some(c) = r.icon.first_child() {
                    r.icon.remove(&c);
                }
                let img: gtk::Widget = match &d.icon {
                    RowIcon::App(a) => crate::apps::imagem(a, 24).upcast(),
                    RowIcon::Glyph(g) => ui_icon(g, "#8E8F99", 18).upcast(),
                };
                img.set_hexpand(true);
                r.icon.append(&img);
                r.root.set_margin_start(if d.indent { 34 } else { 0 });
            }
            r.name.set_text(&d.name);
            r.sub.set_text(&d.sub);
            r.sub.set_visible(!d.sub.is_empty());
            for (l, v) in r.values.iter().zip(&d.values) {
                l.set_text(v);
            }
            if let Some(last) = r.values.last() {
                for c in ["alta", "acento", "fraco"] {
                    last.remove_css_class(c);
                }
                if let Some(c) = d.last_css {
                    last.add_css_class(c);
                }
            }
            r.chevron.set_visible(d.opens.is_some());
            if let Some(open) = d.opens {
                if open { r.chevron.add_css_class("aberta") } else { r.chevron.remove_css_class("aberta") }
            }
            *r.data.borrow_mut() = Some(d.clone());
        }
        *self.order.borrow_mut() = keys;
        let sel = self.selected.borrow().clone();
        self.select(sel.as_deref());
    }
}

// ------------------------------------------------------------------------------------------------- the apps and their processes

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Kind {
    App,
    System,
    Agent,
}

/// What weighs: an app with its processes, a command in the Terminal, an agent, or one of the system's processes.
#[derive(Clone)]
pub(crate) struct Group {
    pub key: String,
    pub name: String,
    pub sub: String,
    pub icon: RowIcon,
    pub kind: Kind,
    pub pids: Vec<i32>,
    pub cpu: f64,
    pub mem: f64,
    pub gpu_mb: f64,
    pub read: f64,
    pub written: f64,
    pub read_total: Option<f64>,
    pub written_total: Option<f64>,
    pub threads: u32,
    pub start: i64,
    pub stopped: bool,
    /// the shell itself (never suspended nor ended from here)
    pub own: bool,
}

impl Group {
    /// How much it weighs, for the order of the overview: the processor, the GPU's memory, the memory.
    pub(crate) fn score(&self, mem_total: f64, vram_total: f64) -> f64 {
        self.cpu + 100.0 * self.gpu_mb / vram_total.max(1.0) * 0.6 + 100.0 * self.mem / mem_total.max(1.0) * 0.3
    }
    pub(crate) fn energy(&self, vram_total: f64) -> (&'static str, Option<&'static str>) {
        let gpu = 100.0 * self.gpu_mb / vram_total.max(1.0);
        if self.cpu >= 15.0 || gpu >= 30.0 {
            (tr("high"), Some("alta"))
        } else if self.cpu >= 3.0 || gpu >= 5.0 {
            (tr("medium"), None)
        } else {
            (tr("low"), Some("fraco"))
        }
    }
}

/// The local models' servers and the engines: "LFM3 1.7B · texto" from llama-server's --model.
pub(crate) fn agent_label(p: &dados::Proc) -> Option<(String, String)> {
    if p.cmd.contains("llama-server") {
        let model = p.cmd.split_whitespace().skip_while(|a| *a != "--model" && *a != "-m").nth(1).unwrap_or("");
        let file = std::path::Path::new(model).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let kind = if file.contains("VL") { tr("vision") } else if file.contains("Audio") || file.contains("Omni") { tr("voice") } else { tr("text") };
        let short = file.split("-Q").next().unwrap_or(&file).replace('-', " ").replace("LFM3 ", "LFM3-").replacen("LFM3-", "LFM3 ", 1);
        return Some((format!("{short} · {kind}"), tr("local model").to_string()));
    }
    // (the engine itself — not a shell whose command line mentions it)
    let prog = p.cmd.split_whitespace().next().map(|a| a.rsplit('/').next().unwrap_or(a)).unwrap_or("");
    if p.name == "kalango-agente" || prog == "kalango-agente" {
        let what = p.cmd.split_whitespace().skip(1).find(|a| !a.starts_with('-')).map(|a| a.replace('-', " "));
        return Some((tr("Agent").to_string(), what.unwrap_or_else(|| tr("conversation").to_string())));
    }
    if p.name == "kalangod" {
        return Some(("kalangod".into(), tr("the agents' service (root)").to_string()));
    }
    None
}

/// Groups the processes: each app's (its windows' processes and their children), each command run in the Terminal
/// (its folder under it), the agents', and the rest one by one (the system's).
pub(crate) fn group(shell: &Shell, prev: &Snap, cur: &Snap) -> Vec<Group> {
    let dt = (cur.at - prev.at).max(0.2);
    let cores = cur.cores.max(1) as f64;
    let before: HashMap<i32, &dados::Proc> = prev.procs.iter().map(|p| (p.pid, p)).collect();
    let by_pid: HashMap<i32, &dados::Proc> = cur.procs.iter().map(|p| (p.pid, p)).collect();
    let me = std::process::id() as i32;
    let my_uid = cur.procs.iter().find(|p| p.pid == me).map(|p| p.uid).unwrap_or(1000);
    let boot = glib::DateTime::now_local().map(|n| n.to_unix()).unwrap_or(0) - cur.uptime as i64;
    // the windows' processes → their app (KalangoOS's own windows are the shell's: not an app's)
    let mut window_app: HashMap<i32, (String, String, Option<crate::apps::App>)> = HashMap::new();
    for j in shell.estado.janelas.borrow().values() {
        let Some(pid) = j.pid.filter(|p| *p != me) else { continue };
        let app = crate::apps::da_janela(&j.app_id, &shell.todos).cloned();
        let name = app.as_ref().map(|a| a.nome.clone()).unwrap_or_else(|| j.app_id.clone());
        let e = window_app.entry(pid).or_insert((j.app_id.clone(), name, app));
        let _ = e;
    }
    let titles: HashMap<String, Vec<String>> = shell.estado.janelas.borrow().values().fold(HashMap::new(), |mut m, j| {
        m.entry(j.app_id.clone()).or_default().push(j.titulo.clone());
        m
    });
    // the Terminal's shells: what runs under them is a command of its own ("cargo build", its folder)
    let shells: HashMap<i32, std::path::PathBuf> = crate::terminal::shells().into_iter().collect();
    let mut groups: HashMap<String, Group> = HashMap::new();
    for p in &cur.procs {
        // its place: walk up to a window's process, a terminal's shell, the shell itself
        let (mut a, mut child_of_shell, mut key_kind) = (p.pid, None::<i32>, None::<String>);
        let mut steps = 0;
        while a > 1 && steps < 64 {
            if window_app.contains_key(&a) {
                key_kind = Some(format!("app:{a}"));
                break;
            }
            if shells.contains_key(&a) && a != p.pid {
                key_kind = child_of_shell.map(|c| format!("cmd:{c}"));
                break;
            }
            if a == me {
                key_kind = Some("kalango".into());
                break;
            }
            child_of_shell = Some(a);
            a = by_pid.get(&a).map(|x| x.ppid).unwrap_or(0);
            steps += 1;
        }
        let (key, name, sub, icon, kind, own) = match key_kind.as_deref() {
            Some(k) if k.starts_with("app:") => {
                let (app_id, name, app) = window_app[&a].clone();
                let t = titles.get(&app_id).cloned().unwrap_or_default();
                let sub = if t.len() > 1 { trf("{n} windows", &[("n", t.len().to_string())]) } else { t.first().cloned().unwrap_or_default() };
                (format!("app:{app_id}"), name, sub, app.map(RowIcon::App).unwrap_or(RowIcon::Glyph("apps-n")), Kind::App, false)
            }
            Some(k) if k.starts_with("cmd:") => {
                let c: i32 = k[4..].parse().unwrap_or(0);
                let cmdp = by_pid.get(&c).copied();
                let cmd = cmdp.map(|x| short_cmd(&x.cmd, &x.name)).unwrap_or_default();
                let dir = shells.get(&a).and_then(|d| d.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                (k.to_string(), cmd, dir, RowIcon::Glyph("terminal"), Kind::App, false)
            }
            _ => {
                if let Some((name, sub)) = agent_label(p) {
                    (format!("agente:{}", p.pid), name, sub, RowIcon::Glyph("spark"), Kind::Agent, false)
                } else if key_kind.as_deref() == Some("kalango") || p.pid == me {
                    // (the shell's helpers and the shells of the Terminal's tabs themselves)
                    let terminal = shells.contains_key(&p.pid);
                    if terminal {
                        ("app:os.kalango.Terminal".into(), tr("Terminal").to_string(), String::new(), app_icon(shell, "os.kalango.Terminal"), Kind::App, false)
                    } else {
                        ("kalango".into(), "KalangoOS".into(), tr("bars, dock and the system's apps").to_string(), app_icon(shell, "os.kalango.Agentes"), Kind::System, true)
                    }
                } else {
                    let kind = if p.uid != my_uid { Kind::System } else { Kind::System };
                    let sub = short_cmd(&p.cmd, &p.name);
                    (format!("proc:{}", p.pid), p.name.clone(), if sub == p.name { String::new() } else { sub }, RowIcon::Glyph("engrenagem"), kind, false)
                }
            }
        };
        let was = before.get(&p.pid);
        let cpu = was.map(|w| p.ticks.saturating_sub(w.ticks) as f64 / dados::TICKS / dt / cores * 100.0).unwrap_or(0.0);
        let (rd, wr) = match (was.and_then(|w| w.io), p.io) {
            (Some((r0, w0)), Some((r1, w1))) => (r1.saturating_sub(r0) as f64 / dt, w1.saturating_sub(w0) as f64 / dt),
            _ => (0.0, 0.0),
        };
        let g = groups.entry(key.clone()).or_insert_with(|| Group {
            key, name, sub, icon, kind, pids: Vec::new(), cpu: 0.0, mem: 0.0, gpu_mb: 0.0, read: 0.0, written: 0.0,
            read_total: Some(0.0), written_total: Some(0.0), threads: 0, start: i64::MAX, stopped: true, own,
        });
        g.pids.push(p.pid);
        g.cpu += cpu;
        g.mem += p.rss as f64;
        g.gpu_mb += cur.gpu_mem.get(&p.pid).copied().unwrap_or(0.0);
        g.read += rd;
        g.written += wr;
        g.read_total = match (g.read_total, p.io) { (Some(t), Some((r, _))) => Some(t + r as f64), _ => None };
        g.written_total = match (g.written_total, p.io) { (Some(t), Some((_, w))) => Some(t + w as f64), _ => None };
        g.threads += p.threads;
        g.start = g.start.min(boot + (p.start as f64 / dados::TICKS) as i64);
        g.stopped &= p.stopped;
    }
    groups.into_values().collect()
}

/// "cargo build --release" → "cargo build"; a path's program by its name.
fn short_cmd(cmd: &str, name: &str) -> String {
    let mut w = cmd.split_whitespace();
    let Some(first) = w.next() else { return name.to_string() };
    let prog = std::path::Path::new(first).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| first.to_string());
    match w.next() {
        Some(a) if !a.starts_with('-') && !a.contains('/') && a.len() < 24 && ["cargo", "npm", "git", "make", "python3", "go", "docker"].contains(&prog.as_str()) => format!("{prog} {a}"),
        _ => prog,
    }
}

fn app_icon(shell: &Shell, id: &str) -> RowIcon {
    shell.todos.iter().find(|a| a.id == id).cloned().map(RowIcon::App).unwrap_or(RowIcon::Glyph("apps-n"))
}

// ------------------------------------------------------------------------------------------------- the side: what weighs, and the chosen one

/// "O que está pesando" (and the other pages' "O que…" cards): a caption, a sentence, two actions.
pub(crate) struct Note {
    pub root: gtk::Box,
    pub text: gtk::Label,
    pub primary: gtk::Button,
    pub secondary: gtk::Button,
}

impl Note {
    pub(crate) fn new(caption: &str) -> Note {
        let root = caixa(Orientation::Vertical, 8, "mon-nota");
        let top = caixa(Orientation::Horizontal, 6, "");
        top.append(&ui_icon("spark", "#A5A1E2", 13));
        let caption = rotulo(caption, "mon-nota-legenda");
        top.append(&caption);
        root.append(&top);
        let text = rotulo("", "mon-nota-texto");
        text.set_xalign(0.0);
        text.set_wrap(true);
        text.set_max_width_chars(1);
        text.set_hexpand(true);
        root.append(&text);
        let acts = caixa(Orientation::Horizontal, 8, "");
        let primary = gtk::Button::with_label("");
        primary.add_css_class("mon-nota-acao");
        let secondary = gtk::Button::with_label("");
        secondary.add_css_class("mon-nota-texto-acao");
        acts.append(&primary);
        acts.append(&secondary);
        root.append(&acts);
        Note { root, text, primary, secondary }
    }

    pub(crate) fn show(&self, text: &str, primary: Option<&str>, secondary: Option<&str>) {
        self.text.set_text(text);
        self.primary.set_visible(primary.is_some());
        self.primary.set_label(primary.unwrap_or(""));
        self.secondary.set_visible(secondary.is_some());
        self.secondary.set_label(secondary.unwrap_or(""));
    }
}

/// The chosen one's details (design "Selecionado"): its caption, four lines, two actions.
pub(crate) struct Detail {
    pub root: gtk::Box,
    pub caption: gtk::Label,
    pub lines: Vec<(gtk::Label, gtk::Label)>,
    pub first: gtk::Button,
    pub second: gtk::Button,
}

impl Detail {
    pub(crate) fn new(labels: &[&str], first: &str, second: &str, second_css: &str) -> Detail {
        let root = caixa(Orientation::Vertical, 9, "mon-detalhe");
        let caption = rotulo("", "mon-legenda");
        caption.set_xalign(0.0);
        root.append(&caption);
        let mut lines = Vec::new();
        for l in labels {
            let row = caixa(Orientation::Horizontal, 8, "");
            let k = rotulo(l, "mon-detalhe-chave");
            k.set_xalign(0.0);
            k.set_hexpand(true);
            let v = rotulo("", "mon-detalhe-valor");
            v.set_xalign(1.0);
            v.set_ellipsize(pango::EllipsizeMode::End);
            row.append(&k);
            row.append(&v);
            root.append(&row);
            lines.push((k, v));
        }
        let acts = caixa(Orientation::Horizontal, 8, "");
        acts.set_homogeneous(true);
        let a = gtk::Button::with_label(first);
        a.add_css_class("mon-acao");
        let b = gtk::Button::with_label(second);
        b.add_css_class(second_css);
        acts.append(&a);
        acts.append(&b);
        root.append(&acts);
        Detail { root, caption, lines, first: a, second: b }
    }
}

// ------------------------------------------------------------------------------------------------- the window

pub struct Monitor {
    pub window: gtk::ApplicationWindow,
    shell: Weak<Shell>,
    rail: Vec<gtk::Button>,
    stack: gtk::Stack,
    search: gtk::Entry,
    pub(crate) cards: Vec<Card>,
    ov_table: Rc<Table>,
    ov_note: Note,
    ov_detail: Detail,
    chips: Vec<gtk::ToggleButton>,
    filter: Cell<usize>,
    pr_table: Rc<Table>,
    pr_detail: Detail,
    pr_note: Note,
    pub(crate) pages: RefCell<Option<Rc<crate::monitor_paginas::Pages>>>,
    /// every page's table-and-side, laid out for the window's width
    pub(crate) bodies: RefCell<Vec<(gtk::Box, gtk::Box)>>,
    pub(crate) prev: RefCell<Option<Snap>>,
    pub(crate) cur: RefCell<Option<Snap>>,
    pub(crate) groups: RefCell<Vec<Group>>,
    selected: RefCell<Option<String>>,
    open_groups: RefCell<HashSet<String>>,
    sort: Cell<usize>,
    reading: Cell<bool>,
    ticks: Cell<u64>,
    /// what weighs, for how many seconds; what was explained, and what the person said is fine
    heavy: RefCell<(String, u32)>,
    explained: RefCell<HashMap<String, (i64, String, bool)>>,
    fine: RefCell<HashMap<String, i64>>,
    ending: RefCell<Option<(String, i64)>>,
}

thread_local! {
    static MONITOR: RefCell<Option<Rc<Monitor>>> = const { RefCell::new(None) };
}

const PAGES: [(&str, &str); 5] = [("visao", "cpu"), ("processos", "lista"), ("agentes", "spark"), ("discos", "pasta"), ("rede", "rede-cabo")];

/// Opens the Monitor (on a page: "processos", "agentes", "discos", "rede").
pub fn open(shell: &Rc<Shell>, page: Option<&str>) {
    let m = MONITOR.with(|m| m.borrow().clone()).unwrap_or_else(|| {
        let app = gtk::gio::Application::default().and_downcast::<gtk::Application>().expect("app");
        let m = Monitor::new(&app, shell);
        MONITOR.with(|x| *x.borrow_mut() = Some(m.clone()));
        m
    });
    if !m.window.is_visible() {
        let (w, h) = crate::superficies::tamanho_de_janela(1500, 910);
        m.window.set_default_size(w, h);
    }
    if let Some(p) = page {
        m.go(p);
    }
    m.window.present();
    m.start();
    if debug() {
        eprintln!("debug monitor: aberto ({})", m.stack.visible_child_name().unwrap_or_default());
    }
}

impl Monitor {
    fn new(app: &gtk::Application, shell: &Rc<Shell>) -> Rc<Monitor> {
        let (w, h) = crate::superficies::tamanho_de_janela(1500, 910);
        let window = gtk::ApplicationWindow::builder().application(app).title(tr("System Monitor")).default_width(w).default_height(h).build();
        // (its app id as its name: the state written by `kalango-shell estado` finds it by it)
        window.set_widget_name("os.kalango.Monitor");
        // (closed for a while, the window lets go of its surface and renderer and keeps its widgets: see
        // superficies::release_resources_when_hidden)
        crate::superficies::release_resources_when_hidden(&window, || true);
        window.add_css_class("ed-janela");
        let sem_barra = gtk::Box::new(Orientation::Horizontal, 0);
        sem_barra.set_visible(false);
        window.set_titlebar(Some(&sem_barra));
        // the rail: the app, then its pages
        let rail_box = caixa(Orientation::Vertical, 4, "mon-trilho");
        if let Some(a) = shell.todos.iter().find(|a| a.id == "os.kalango.Monitor") {
            let i = crate::apps::imagem(a, 28);
            i.set_margin_bottom(10);
            rail_box.append(&i);
        }
        let tips = [tr("Overview"), tr("Processes"), tr("Agents"), tr("Disks"), tr("Network")];
        let mut rail = Vec::new();
        for ((_, icon), tip) in PAGES.iter().zip(tips) {
            let b = gtk::Button::new();
            b.add_css_class("mon-pagina");
            b.set_child(Some(&ui_icon(icon, "#B9BAC4", 16)));
            b.set_tooltip_text(Some(tip));
            rail_box.append(&b);
            rail.push(b);
        }
        // the toolbar: the title, the search, the menu, the window's buttons
        let bar = caixa(Orientation::Horizontal, 10, "ed-barra");
        let title = rotulo(tr("System Monitor"), "mon-titulo");
        bar.append(&title);
        let gap = caixa(Orientation::Horizontal, 0, "");
        gap.set_hexpand(true);
        bar.append(&gap);
        let search = gtk::Entry::new();
        search.add_css_class("ed-pedir");
        search.set_size_request(300, 32);
        search.set_valign(Align::Center);
        search.set_placeholder_text(Some(tr("Search a process or ask: “what is using the internet?”")));
        search.set_primary_icon_paintable(ui_icon("spark", "#A5A1E2", 13).paintable().as_ref());
        let sbox = gtk::Overlay::new();
        sbox.set_child(Some(&search));
        let key = rotulo("Ctrl K", "tb-atalho");
        key.set_halign(Align::End);
        key.set_valign(Align::Center);
        key.set_margin_end(8);
        key.set_can_target(false);
        sbox.add_overlay(&key);
        bar.append(&sbox);
        bar.append(&crate::configuracoes::controles_janela());
        let handle = gtk::WindowHandle::new();
        handle.set_child(Some(&bar));
        let stack = gtk::Stack::new();
        stack.set_transition_type(gtk::StackTransitionType::Crossfade);
        stack.set_transition_duration(160);
        stack.set_vexpand(true);
        // ---- Visão geral
        let cards = vec![
            Card::new("cpu", tr("CPU"), "#8ED2C3"),
            Card::new("lista", tr("MEMORY"), "#7CC3DF"),
            Card::new("monitor", tr("GPU"), "#A5A1E2"),
            Card::new("rede-cabo", tr("NETWORK"), "#E5C98C"),
        ];
        let res = card_row(&cards.iter().map(|c| &c.root).collect::<Vec<_>>());
        let cols = [tr("CPU"), tr("Memory"), tr("GPU"), tr("Energy")];
        let ov_table = Table::new(&cols, tr("Process"));
        ov_table.root.set_hexpand(true);
        ov_table.root.set_valign(Align::Start);
        let ov_note = Note::new(tr("WHAT IS WEIGHING"));
        let ov_detail = Detail::new(&[tr("Started"), tr("Threads"), tr("Disk"), tr("Network")], tr("Suspend"), tr("End"), "mon-acao-perigo");
        let side = caixa(Orientation::Vertical, 14, "");
        side.append(&ov_note.root);
        side.append(&ov_detail.root);
        let ov_body = body(ov_table.root.upcast_ref(), &side);
        ov_body.set_vexpand(false);
        let ov = caixa(Orientation::Vertical, 16, "mon-pagina-conteudo");
        ov.append(&res);
        ov.append(&ov_body);
        stack.add_named(&ov, Some("visao"));
        // ---- Processos
        let chip_row = caixa(Orientation::Horizontal, 8, "");
        let mut chips = Vec::new();
        for _ in 0..4 {
            let c = gtk::ToggleButton::new();
            c.add_css_class("mon-chip");
            chip_row.append(&c);
            chips.push(c);
        }
        for c in &chips[1..] {
            c.set_group(Some(&chips[0]));
        }
        chips[0].set_active(true);
        let pr_table = Table::new(&cols, tr("Process"));
        pr_table.root.set_hexpand(true);
        let pr_note = Note::new(tr("WHAT IS WEIGHING"));
        let pr_detail = Detail::new(&[tr("Started"), tr("Threads"), tr("Disk"), tr("Network")], tr("Suspend"), tr("End"), "mon-acao-perigo");
        let pside = caixa(Orientation::Vertical, 14, "");
        pside.append(&pr_note.root);
        pside.append(&pr_detail.root);
        let pbody = body(pr_table.root.upcast_ref(), &pside);
        let pr = caixa(Orientation::Vertical, 16, "mon-pagina-conteudo");
        pr.append(&chip_row);
        pr.append(&pbody);
        stack.add_named(&pr, Some("processos"));
        let content = caixa(Orientation::Vertical, 0, "");
        content.set_hexpand(true);
        // (measures the content: the cards wrap and the sides go under the tables on a narrow window)
        let meter = gtk::DrawingArea::new();
        meter.set_can_target(false);
        meter.set_content_height(1);
        meter.set_hexpand(true);
        content.append(&meter);
        content.append(&handle);
        // (each page scrolls when the window is shorter than it)
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
        scroll.set_vexpand(true);
        scroll.set_child(Some(&stack));
        content.append(&scroll);
        let root = caixa(Orientation::Horizontal, 0, "");
        root.append(&rail_box);
        root.append(&content);
        window.set_child(Some(&root));

        let m = Rc::new(Monitor {
            window, shell: Rc::downgrade(shell), rail, stack, search, cards, ov_table, ov_note, ov_detail, chips, filter: Cell::new(0),
            pr_table, pr_detail, pr_note, pages: RefCell::default(), bodies: RefCell::new(vec![(ov_body.clone(), side.clone()), (pbody.clone(), pside.clone())]), prev: RefCell::default(), cur: RefCell::default(), groups: RefCell::default(),
            selected: RefCell::default(), open_groups: RefCell::default(), sort: Cell::new(1), reading: Cell::new(false), ticks: Cell::new(0),
            heavy: RefCell::default(), explained: RefCell::default(), fine: RefCell::default(), ending: RefCell::default(),
        });
        let w = Rc::downgrade(&m);
        meter.connect_resize(move |_, width, _| {
            if let Some(m) = w.upgrade() {
                let bodies = m.bodies.borrow().clone();
                glib::idle_add_local_once(move || fit_bodies(&bodies, width));
            }
        });
        let pages = crate::monitor_paginas::Pages::new(&m);
        m.stack.add_named(&pages.agents, Some("agentes"));
        m.stack.add_named(&pages.disks, Some("discos"));
        m.stack.add_named(&pages.network, Some("rede"));
        *m.pages.borrow_mut() = Some(pages);
        for (i, b) in m.rail.iter().enumerate() {
            let w = Rc::downgrade(&m);
            b.connect_clicked(move |_| {
                if let Some(m) = w.upgrade() {
                    m.go(PAGES[i].0);
                }
            });
            crate::central::posicao_debug(&format!("monitor página {}", PAGES[i].0), b.upcast_ref());
        }
        m.go("visao");
        // choosing a row
        for t in [&m.ov_table, &m.pr_table] {
            let w = Rc::downgrade(&m);
            t.on_select(move |k| {
                if let Some(m) = w.upgrade() {
                    *m.selected.borrow_mut() = Some(k.to_string());
                    m.ov_table.select(Some(k));
                    m.pr_table.select(Some(k));
                    m.ending.borrow_mut().take();
                    m.refresh_details();
                    if debug() {
                        eprintln!("debug monitor: escolhido {k}");
                    }
                }
            });
        }
        let w = Rc::downgrade(&m);
        m.pr_table.on_open(move |k| {
            if let Some(m) = w.upgrade() {
                let mut o = m.open_groups.borrow_mut();
                if !o.remove(k) {
                    o.insert(k.to_string());
                }
                drop(o);
                m.refresh_tables();
            }
        });
        let w = Rc::downgrade(&m);
        m.pr_table.on_sort(move |i| {
            if let Some(m) = w.upgrade() {
                m.sort.set(i);
                m.refresh_tables();
            }
        });
        for (i, c) in m.chips.iter().enumerate() {
            let w = Rc::downgrade(&m);
            c.connect_toggled(move |c| {
                if let (Some(m), true) = (w.upgrade(), c.is_active()) {
                    m.filter.set(i);
                    m.refresh_tables();
                }
            });
        }
        // the actions on the chosen one
        for d in [&m.ov_detail, &m.pr_detail] {
            let w = Rc::downgrade(&m);
            d.first.connect_clicked(move |_| {
                if let Some(m) = w.upgrade() {
                    m.suspend_selected();
                }
            });
            let w = Rc::downgrade(&m);
            d.second.connect_clicked(move |_| {
                if let Some(m) = w.upgrade() {
                    m.end_selected();
                }
            });
            crate::central::posicao_debug("monitor encerrar", d.second.upcast_ref());
            crate::central::posicao_debug("monitor suspender", d.first.upcast_ref());
        }
        for n in [&m.ov_note, &m.pr_note] {
            let w = Rc::downgrade(&m);
            n.primary.connect_clicked(move |_| {
                if let Some(m) = w.upgrade() {
                    m.note_primary();
                }
            });
            let w = Rc::downgrade(&m);
            n.secondary.connect_clicked(move |_| {
                if let Some(m) = w.upgrade() {
                    m.note_fine();
                }
            });
        }
        // the search: a name filters, a question goes to the agent
        let w = Rc::downgrade(&m);
        m.search.connect_changed(move |_| {
            if let Some(m) = w.upgrade() {
                m.refresh_tables();
            }
        });
        let w = Rc::downgrade(&m);
        m.search.connect_activate(move |e| {
            if let Some(m) = w.upgrade() {
                let q = e.text().trim().to_string();
                if q.split_whitespace().count() >= 3 {
                    m.ask_agent(&q);
                    e.set_text("");
                }
            }
        });
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let w = Rc::downgrade(&m);
        keys.connect_key_pressed(move |_, k, _, mods| {
            let Some(m) = w.upgrade() else { return glib::Propagation::Proceed };
            if mods.contains(gdk::ModifierType::CONTROL_MASK) && k.to_lower() == gdk::Key::k {
                m.search.grab_focus();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        m.window.add_controller(keys);
        let app_id = |j: &gtk::ApplicationWindow| {
            if let Some(s) = j.surface().filter(|s| s.type_().name().contains("Wayland")) {
                let id = CString::new("os.kalango.Monitor").unwrap();
                unsafe { gdk_wayland_toplevel_set_application_id(s.as_ptr() as *mut gdk::ffi::GdkToplevel, id.as_ptr()) };
            }
        };
        m.window.connect_realize(move |j| app_id(j));
        m.window.connect_map(move |j| app_id(j));
        m.window.connect_close_request(|win| {
            win.set_visible(false);
            if debug() {
                eprintln!("debug monitor: fechado");
            }
            glib::Propagation::Stop
        });
        m
    }

    pub(crate) fn shell(&self) -> Option<Rc<Shell>> {
        self.shell.upgrade()
    }

    fn go(self: &Rc<Self>, page: &str) {
        let i = PAGES.iter().position(|(p, _)| *p == page).unwrap_or(0);
        self.stack.set_visible_child_name(PAGES[i].0);
        for (n, b) in self.rail.iter().enumerate() {
            if n == i { b.add_css_class("ativa") } else { b.remove_css_class("ativa") }
        }
        if debug() {
            eprintln!("debug monitor: página {}", PAGES[i].0);
        }
    }

    /// Reads once a second while the window is open (off the interface's thread; the GPU every other second).
    fn start(self: &Rc<Self>) {
        if self.ticks.get() > 0 && self.window.is_visible() {
            return;
        }
        self.sample();
        let w = Rc::downgrade(self);
        glib::timeout_add_local(std::time::Duration::from_secs(1), move || {
            let Some(m) = w.upgrade() else { return glib::ControlFlow::Break };
            if !m.window.is_visible() {
                m.ticks.set(0);
                return glib::ControlFlow::Break;
            }
            m.sample();
            glib::ControlFlow::Continue
        });
    }

    fn sample(self: &Rc<Self>) {
        if self.reading.replace(true) {
            return;
        }
        let n = self.ticks.get();
        self.ticks.set(n + 1);
        let gpu = n % 2 == 0 || self.cur.borrow().as_ref().map(|s| s.gpu.is_none()).unwrap_or(true);
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(move || dados::snap(gpu), move |mut s: Snap| {
            let Some(m) = w.upgrade() else { return };
            m.reading.set(false);
            // (the GPU read every other second: in between, the last one)
            if s.gpu.is_none() {
                if let Some(c) = m.cur.borrow().as_ref() {
                    s.gpu = c.gpu.clone();
                    s.gpu_mem = c.gpu_mem.clone();
                }
            }
            let prev = m.cur.replace(Some(s));
            *m.prev.borrow_mut() = prev;
            m.update();
        });
    }

    fn update(self: &Rc<Self>) {
        let (Some(prev), Some(cur)) = (self.prev.borrow().clone(), self.cur.borrow().clone()) else { return };
        let Some(shell) = self.shell.upgrade() else { return };
        let dt = (cur.at - prev.at).max(0.2);
        // the cards
        let cpu = 100.0 * (1.0 - (cur.cpu_idle.saturating_sub(prev.cpu_idle)) as f64 / (cur.cpu_total.saturating_sub(prev.cpu_total)).max(1) as f64);
        self.cards[0].value.set_text(&pct(cpu));
        let mut cpu_sub = vec![trf("{n} cores", &[("n", cur.cores.to_string())]), format!("{:.1} GHz", cur.ghz).replace('.', ",")];
        if let Some(t) = cur.temp {
            cpu_sub.push(format!("{t:.0} °C"));
        }
        self.cards[0].sub.set_text(&cpu_sub.join(" · "));
        self.cards[0].push(cpu);
        let used = cur.mem_total.saturating_sub(cur.mem_avail) as f64;
        self.cards[1].value.set_text(&bytes(used));
        self.cards[1].sub.set_text(&trf("of {total} · {cache} in cache", &[("total", bytes(cur.mem_total as f64)), ("cache", bytes(cur.mem_cached as f64))]));
        self.cards[1].push(used);
        match &cur.gpu {
            Some(g) => {
                self.cards[2].value.set_text(&pct(g.util));
                self.cards[2].sub.set_text(&format!("{} · {:.0} °C", trf("VRAM {used} of {total}", &[("used", bytes(g.vram_used_mb * 1e6)), ("total", bytes(g.vram_total_mb * 1e6))]), g.temp));
                self.cards[2].push(g.util);
            }
            None => {
                self.cards[2].value.set_text("—");
                self.cards[2].sub.set_text(tr("no GPU this monitor can read"));
            }
        }
        let (rx, tx) = cur.net.iter().fold((0.0, 0.0), |(r, t), (n, rx, tx)| {
            let p = prev.net.iter().find(|(m, _, _)| m == n);
            p.map(|(_, r0, t0)| (r + rx.saturating_sub(*r0) as f64 / dt, t + tx.saturating_sub(*t0) as f64 / dt)).unwrap_or((r, t))
        });
        self.cards[3].value.set_text(&rate(rx));
        let via = crate::monitor_paginas::connection_kind();
        self.cards[3].sub.set_text(&format!("↑ {}{}", rate(tx), via.map(|v| format!(" · {v}")).unwrap_or_default()));
        self.cards[3].push(rx + tx);
        // the groups
        *self.groups.borrow_mut() = group(&shell, &prev, &cur);
        self.refresh_tables();
        self.refresh_details();
        self.weigh();
        if let Some(p) = self.pages.borrow().as_ref() {
            p.update(&prev, &cur);
        }
        if debug() && self.ticks.get() % 5 == 1 {
            let g = self.groups.borrow();
            let top: Vec<String> = self.overview_rows(&g, &cur).iter().take(3).map(|r| format!("{} {}", r.name, r.values[0])).collect();
            eprintln!("debug monitor: cpu {} memória {} · {} grupos · {}", pct(cpu), bytes(used), g.len(), top.join(", "));
        }
    }

    fn vram_total(cur: &Snap) -> f64 {
        cur.gpu.as_ref().map(|g| g.vram_total_mb).unwrap_or(1.0)
    }

    fn row_of(g: &Group, cur: &Snap) -> RowData {
        let vt = Self::vram_total(cur);
        let (energy, css) = g.energy(vt);
        let gpu = if cur.gpu.is_some() && g.gpu_mb > 0.0 { pct(100.0 * g.gpu_mb / vt) } else { "0%".into() };
        let sub = if g.stopped { format!("{} · {}", tr("suspended"), g.sub) } else { g.sub.clone() };
        RowData { key: g.key.clone(), name: g.name.clone(), sub, icon: g.icon.clone(), values: vec![pct(g.cpu), bytes(g.mem), gpu, energy.into()],
            last_css: css, indent: false, opens: None }
    }

    fn overview_rows(&self, groups: &[Group], cur: &Snap) -> Vec<RowData> {
        let (mt, vt) = (cur.mem_total as f64, Self::vram_total(cur));
        // (the apps and the agents; one of the system's only when it works hard)
        let mut g: Vec<&Group> = groups.iter().filter(|g| g.kind != Kind::System || g.own || g.cpu >= 5.0).collect();
        g.sort_by(|a, b| b.score(mt, vt).total_cmp(&a.score(mt, vt)));
        g.into_iter().take(7).map(|g| Self::row_of(g, cur)).collect()
    }

    fn refresh_tables(self: &Rc<Self>) {
        let Some(cur) = self.cur.borrow().clone() else { return };
        let groups = self.groups.borrow().clone();
        let q = self.search.text().trim().to_lowercase();
        let filtering = !q.is_empty() && q.split_whitespace().count() < 3;
        let matches = |g: &Group| !filtering || g.name.to_lowercase().contains(&q) || g.sub.to_lowercase().contains(&q);
        let ov: Vec<RowData> = self.overview_rows(&groups.iter().filter(|g| matches(g)).cloned().collect::<Vec<_>>(), &cur);
        self.ov_table.update(&ov);
        // Processos: every group, filtered and sorted; an open one shows its processes under it
        let counts = [groups.len(), groups.iter().filter(|g| g.kind == Kind::App).count(), groups.iter().filter(|g| g.kind == Kind::System).count(),
            groups.iter().filter(|g| g.kind == Kind::Agent).count()];
        for (c, (label, n)) in self.chips.iter().zip([tr("Every one"), tr("Apps"), tr("The system's"), tr("The agents'")].iter().zip(counts)) {
            c.set_label(&format!("{label}  {n}"));
        }
        let want = match self.filter.get() {
            1 => Some(Kind::App),
            2 => Some(Kind::System),
            3 => Some(Kind::Agent),
            _ => None,
        };
        let mut list: Vec<&Group> = groups.iter().filter(|g| want.map_or(true, |k| g.kind == k) && matches(g)).collect();
        let vt = Self::vram_total(&cur);
        let s = self.sort.get();
        list.sort_by(|a, b| match s {
            0 => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            2 => b.mem.total_cmp(&a.mem),
            3 => b.gpu_mb.total_cmp(&a.gpu_mb),
            4 => b.score(cur.mem_total as f64, vt).total_cmp(&a.score(cur.mem_total as f64, vt)),
            _ => b.cpu.total_cmp(&a.cpu),
        });
        let names = [tr("Process"), tr("CPU"), tr("Memory"), tr("GPU"), tr("Energy")];
        for (i, n) in names.iter().enumerate() {
            self.pr_table.set_header(i, &if i == s { format!("{n} ▾") } else { n.to_string() });
        }
        let open = self.open_groups.borrow().clone();
        let by_pid: HashMap<i32, &dados::Proc> = cur.procs.iter().map(|p| (p.pid, p)).collect();
        let prev_by: HashMap<i32, dados::Proc> = self.prev.borrow().as_ref().map(|p| p.procs.iter().map(|x| (x.pid, x.clone())).collect()).unwrap_or_default();
        let dt = self.prev.borrow().as_ref().map(|p| (cur.at - p.at).max(0.2)).unwrap_or(1.0);
        let mut rows = Vec::new();
        for g in list.into_iter().take(400) {
            let mut r = Self::row_of(g, &cur);
            if g.pids.len() > 1 {
                r.opens = Some(open.contains(&g.key));
                if r.sub.is_empty() {
                    r.sub = trf("{n} processes", &[("n", g.pids.len().to_string())]);
                } else {
                    r.sub = format!("{} · {}", r.sub, trf("{n} processes", &[("n", g.pids.len().to_string())]));
                }
            }
            rows.push(r);
            if g.pids.len() > 1 && open.contains(&g.key) {
                let mut kids: Vec<(&dados::Proc, f64)> = g.pids.iter().filter_map(|p| by_pid.get(p).copied()).map(|p| {
                    let c = prev_by.get(&p.pid).map(|w| p.ticks.saturating_sub(w.ticks) as f64 / dados::TICKS / dt / cur.cores.max(1) as f64 * 100.0).unwrap_or(0.0);
                    (p, c)
                }).collect();
                kids.sort_by(|a, b| b.1.total_cmp(&a.1));
                for (p, c) in kids {
                    let gpu = cur.gpu_mem.get(&p.pid).map(|mb| pct(100.0 * mb / vt)).unwrap_or_else(|| "0%".into());
                    rows.push(RowData { key: format!("pid:{}", p.pid), name: format!("{} · {}", g.name, p.name), sub: short_cmd(&p.cmd, &p.name),
                        icon: g.icon.clone(), values: vec![pct(c), bytes(p.rss as f64), gpu, format!("PID {}", p.pid)], last_css: Some("fraco"), indent: true, opens: None });
                }
            }
        }
        self.pr_table.update(&rows);
    }

    pub(crate) fn selected_group(&self) -> Option<Group> {
        let k = self.selected.borrow().clone()?;
        let groups = self.groups.borrow();
        if let Some(pid) = k.strip_prefix("pid:").and_then(|p| p.parse::<i32>().ok()) {
            let g = groups.iter().find(|g| g.pids.contains(&pid))?;
            let mut one = g.clone();
            one.key = k.clone();
            one.pids = vec![pid];
            return Some(one);
        }
        groups.iter().find(|g| g.key == k).cloned()
    }

    fn refresh_details(self: &Rc<Self>) {
        // (nothing chosen: the one that weighs most)
        if self.selected.borrow().is_none() {
            let Some(cur) = self.cur.borrow().clone() else { return };
            let first = self.overview_rows(&self.groups.borrow(), &cur).first().map(|r| r.key.clone());
            *self.selected.borrow_mut() = first.clone();
            self.ov_table.select(first.as_deref());
            self.pr_table.select(first.as_deref());
        }
        let g = self.selected_group();
        for d in [&self.ov_detail, &self.pr_detail] {
            let Some(g) = g.as_ref() else {
                d.caption.set_text("");
                continue;
            };
            d.caption.set_text(&g.name.to_uppercase());
            d.lines[0].1.set_text(&when(g.start));
            d.lines[1].1.set_text(&g.threads.to_string());
            d.lines[2].1.set_text(&if g.read + g.written < 1024.0 {
                if g.read_total.is_none() { tr("not readable (the system's)").to_string() } else { tr("idle").to_string() }
            } else if g.read >= g.written {
                trf("reads {rate}", &[("rate", rate(g.read))])
            } else {
                trf("writes {rate}", &[("rate", rate(g.written))])
            });
            let (n, ports, _) = dados::sockets(&g.pids);
            let net = if n == 0 && ports.is_empty() {
                tr("none").to_string()
            } else if ports.is_empty() {
                trf("{n} connections", &[("n", n.to_string())])
            } else {
                trf("{n} connections · port {port}", &[("n", n.to_string()), ("port", ports.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", "))])
            };
            d.lines[3].1.set_text(&net);
            if n == 0 && ports.is_empty() { d.lines[3].1.add_css_class("bom") } else { d.lines[3].1.remove_css_class("bom") }
            d.first.set_label(if g.stopped { tr("Resume") } else { tr("Suspend") });
            let ending = self.ending.borrow().as_ref().is_some_and(|(k, t)| *k == g.key && glib::monotonic_time() / 1_000_000 - t < 5);
            d.second.set_label(if ending { tr("Force") } else { tr("End") });
            // (the shell itself is not ended from inside it)
            d.first.set_sensitive(!g.own);
            d.second.set_sensitive(!g.own);
            d.second.set_tooltip_text(if g.own { Some(tr("This is KalangoOS itself.")) } else { None });
        }
    }

    /// A row chosen on another page (Rede): the same one everywhere.
    pub(crate) fn choose(self: &Rc<Self>, k: &str) {
        *self.selected.borrow_mut() = Some(k.to_string());
        self.ov_table.select(Some(k));
        self.pr_table.select(Some(k));
        self.ending.borrow_mut().take();
        self.refresh_details();
    }

    pub(crate) fn suspend_selected(self: &Rc<Self>) {
        let Some(g) = self.selected_group().filter(|g| !g.own) else { return };
        let stop = !g.stopped;
        for pid in &g.pids {
            crate::sistema::pausar_processo(*pid, stop);
        }
        if debug() {
            eprintln!("debug monitor: {} «{}» ({} processos)", if stop { "suspendeu" } else { "continuou" }, g.name, g.pids.len());
        }
    }

    /// "Encerrar": asks each process to end (SIGTERM, the app can save); a second click within 5 s forces it.
    pub(crate) fn end_selected(self: &Rc<Self>) {
        let Some(g) = self.selected_group().filter(|g| !g.own) else { return };
        let now = glib::monotonic_time() / 1_000_000;
        let force = self.ending.borrow().as_ref().is_some_and(|(k, t)| *k == g.key && now - t < 5);
        for pid in &g.pids {
            // (a suspended one is woken to hear it)
            unsafe {
                kill(*pid, if force { 9 } else { 15 });
                kill(*pid, 18);
            }
        }
        *self.ending.borrow_mut() = if force { None } else { Some((g.key.clone(), now)) };
        if debug() {
            eprintln!("debug monitor: {} «{}» ({} processos)", if force { "forçou o fim de" } else { "pediu o fim de" }, g.name, g.pids.len());
        }
        self.refresh_details();
    }

    /// "O que está pesando": the one that weighs, for three seconds in a row, is explained by the local model (once
    /// in two minutes each); with nothing heavy, a calm line (no model).
    fn weigh(self: &Rc<Self>) {
        let Some(cur) = self.cur.borrow().clone() else { return };
        let groups = self.groups.borrow().clone();
        let (mt, vt) = (cur.mem_total as f64, Self::vram_total(&cur));
        let now = glib::monotonic_time() / 1_000_000;
        let heavy = groups.iter().filter(|g| !g.own && self.fine.borrow().get(&g.key).map_or(true, |t| now - t > 1800))
            .filter(|g| g.cpu >= 25.0 || 100.0 * g.gpu_mb / vt >= 40.0 || 100.0 * g.mem / mt >= 20.0)
            .max_by(|a, b| a.score(mt, vt).total_cmp(&b.score(mt, vt))).cloned();
        let Some(g) = heavy else {
            self.heavy.borrow_mut().1 = 0;
            for n in [&self.ov_note, &self.pr_note] {
                n.show(tr("Nothing is weighing now: the computer has room to spare."), None, None);
            }
            return;
        };
        let mut h = self.heavy.borrow_mut();
        if h.0 == g.key { h.1 += 1 } else { *h = (g.key.clone(), 1) }
        let seconds = h.1;
        drop(h);
        if let Some((_, text, agent)) = self.explained.borrow().get(&g.key).cloned() {
            self.show_note(&text, agent);
            return;
        }
        if seconds < 3 || self.explained.borrow().get(&g.key).is_some_and(|(t, _, _)| now - t < 120) {
            return;
        }
        // (Agents › Settings › General › Suggestions while you work off: what weighs, in numbers, without the model)
        if !crate::agent_prefs::load().suggestions {
            let text = trf("{name} is using {cpu} of the processor and {mem} of memory.", &[("name", g.name.clone()), ("cpu", format!("{:.0}%", g.cpu)), ("mem", bytes(g.mem))]);
            self.explained.borrow_mut().insert(g.key.clone(), (now, text.clone(), g.kind == Kind::Agent));
            self.show_note(&text, g.kind == Kind::Agent);
            return;
        }
        // asked once: what it is, what it uses, and what the agents are doing
        self.explained.borrow_mut().insert(g.key.clone(), (now, tr("Looking into it…").to_string(), g.kind == Kind::Agent));
        let Some(shell) = self.shell.upgrade() else { return };
        let task = shell.estado.agent_task.borrow().clone().map(|(p, n)| format!("{p} ({n} passos)"));
        let cmd = cur.procs.iter().find(|p| g.pids.first() == Some(&p.pid)).map(|p| p.cmd.chars().take(200).collect::<String>()).unwrap_or_default();
        let input = serde_json::json!({"nome": g.name, "sub": g.sub, "comando": cmd, "cpu": format!("{:.0}%", g.cpu), "memoria": bytes(g.mem),
            "vram": if g.gpu_mb > 0.0 { bytes(g.gpu_mb * 1e6) } else { String::new() }, "agente": g.kind == Kind::Agent, "tarefa_do_agente": task,
            "minutos": seconds / 60}).to_string();
        let bin = crate::voz::agent_binary();
        let (w, key, agent) = (Rc::downgrade(self), g.key.clone(), g.kind == Kind::Agent);
        crate::lateral::em_segundo_plano(move || {
            use std::io::Write;
            let mut c = std::process::Command::new(&bin).arg("monitor").stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn().ok()?;
            c.stdin.take()?.write_all(input.as_bytes()).ok()?;
            let o = c.wait_with_output().ok()?;
            let v: serde_json::Value = serde_json::from_slice(&o.stdout).ok()?;
            Some(v["explicacao"].as_str()?.trim().to_string()).filter(|s| !s.is_empty())
        }, move |r: Option<String>| {
            let Some(m) = w.upgrade() else { return };
            let text = r.unwrap_or_else(|| tr("It is using a lot right now.").to_string());
            if debug() {
                eprintln!("debug monitor: pesando «{key}» — {text}");
            }
            m.explained.borrow_mut().insert(key, (glib::monotonic_time() / 1_000_000, text.clone(), agent));
            m.show_note(&text, agent);
        });
    }

    fn show_note(&self, text: &str, agent: bool) {
        for n in [&self.ov_note, &self.pr_note] {
            n.show(text, Some(if agent { tr("Pause the agent") } else { tr("Suspend it") }), Some(tr("It is fine")));
        }
        crate::central::posicao_debug("monitor nota principal", self.ov_note.primary.upcast_ref());
    }

    fn note_primary(self: &Rc<Self>) {
        let key = self.heavy.borrow().0.clone();
        let agent = self.explained.borrow().get(&key).map(|e| e.2).unwrap_or(false);
        if agent {
            crate::sistema::pausar_agentes(true);
            if debug() {
                eprintln!("debug monitor: agentes pausados");
            }
        } else if let Some(g) = self.groups.borrow().iter().find(|g| g.key == key) {
            for pid in &g.pids {
                crate::sistema::pausar_processo(*pid, true);
            }
        }
    }

    /// "Está tudo bem": that one is not pointed at again for half an hour.
    fn note_fine(self: &Rc<Self>) {
        let key = self.heavy.borrow().0.clone();
        self.fine.borrow_mut().insert(key.clone(), glib::monotonic_time() / 1_000_000);
        self.explained.borrow_mut().remove(&key);
        self.weigh();
    }

    /// A question in the search: to the agent, with what the Monitor sees now.
    fn ask_agent(&self, q: &str) {
        let Some(s) = self.shell.upgrade() else { return };
        let Some(cur) = self.cur.borrow().clone() else { return };
        let top: Vec<String> = self.overview_rows(&self.groups.borrow(), &cur).iter().take(6)
            .map(|r| format!("{} ({}): CPU {}, {} {}, GPU {}", r.name, r.sub, r.values[0], tr("memory"), r.values[1], r.values[2])).collect();
        let pedido = format!("{q}\n\n{}\n{}", tr("What the System Monitor shows now:"), top.join("\n"));
        s.right_bar().ask_agent(&pedido);
        if debug() {
            eprintln!("debug monitor: ao agente «{q}»");
        }
    }
}

/// Signals for the other pages (the Agents page's "Encerrar" ends a model's server).
pub(crate) fn signal(pid: i32, sig: i32) {
    unsafe {
        kill(pid, sig);
    }
}
