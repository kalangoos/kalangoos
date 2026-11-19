//! Privacy as visible state (design "Privacidade visível (quem está vendo o quê)"): chips in the top bar, before the
//! agents pill, whenever something is watching — coral for the microphone or the camera (the app's name), teal for an
//! agent reading files (its folder), blue for the screen being shared. No app can hide them: they come from PipeWire's
//! own graph and from kalangod's log of the agents' tools.
//!
//! A click opens "Privacidade agora": each access with a switch that cuts it at once (the microphone muted, a camera
//! or screen stream cut, the agents paused); what left this computer today (the agents' tokens, local and cloud, by
//! model); the rules (private folders, asking before the cloud, how long the agents' history stays); "Pausar tudo"
//! (Super+Shift+P) cuts every sensor and pauses the agents, and "Retomar" brings them back.

use crate::conversa::ui_icon;
use crate::superficies::{caixa, camada, rotulo};
use gtk::prelude::*;
use gtk::{glib, Align, Orientation};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kalango_i18n::{tr, trf};
use kalango_movimento::{Animavel, Canal, ELASTICA};
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Duration;

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord)]
enum Kind {
    Microphone,
    Camera,
    Screen,
    Agent,
}

/// Something watching now: what, who, and the PipeWire streams that carry it (to cut them).
#[derive(Clone, Debug, PartialEq)]
struct Access {
    kind: Kind,
    who: String,
    detail: String,
    streams: Vec<u32>,
    muted: bool,
}

/// The capture streams in PipeWire's graph (pw-dump): who records the microphone, who looks at the camera, who
/// receives the screen cast — a video consumer is told apart by the source it is linked to (niri's cast or a camera).
fn read_streams(v: &Value) -> Vec<Access> {
    let objs: Vec<&Value> = v.as_array().into_iter().flatten().collect();
    let props = |o: &Value| o["info"]["props"].clone();
    let screen_source = |p: &Value| p["node.name"].as_str().is_some_and(|n| n == "niri" || n.contains("screencast") || n.contains("xdg-desktop-portal"));
    let sources: BTreeMap<u64, bool> = objs.iter().filter(|o| o["type"] == "PipeWire:Interface:Node" && props(o)["media.class"] == "Video/Source")
        .filter_map(|o| Some((o["id"].as_u64()?, screen_source(&props(o))))).collect();
    let linked = |id: u64| -> Option<u64> {
        objs.iter().filter(|o| o["type"] == "PipeWire:Interface:Link").find(|o| o["info"]["input-node-id"].as_u64() == Some(id)).and_then(|o| o["info"]["output-node-id"].as_u64())
    };
    let mut by: BTreeMap<(Kind, String), Access> = BTreeMap::new();
    for o in objs.iter().filter(|o| o["type"] == "PipeWire:Interface:Node") {
        let p = props(o);
        let id = o["id"].as_u64().unwrap_or(0);
        let app = p["application.name"].as_str().or(p["node.description"].as_str()).unwrap_or("").to_string();
        let kind = match p["media.class"].as_str().unwrap_or("") {
            "Stream/Input/Audio" => Kind::Microphone,
            "Stream/Input/Video" => match linked(id).and_then(|s| sources.get(&s)) {
                Some(true) => Kind::Screen,
                _ => Kind::Camera,
            },
            _ => continue,
        };
        // (the shell's own recorders — dictation, captions — are the person's, said where they run)
        if app.to_lowercase().contains("kalango") || p["node.name"].as_str().is_some_and(|n| n.contains("kalango")) {
            continue;
        }
        let muted = o["info"]["params"]["Props"].as_array().is_some_and(|a| a.iter().any(|x| x["mute"].as_bool() == Some(true)));
        // a call (the stream's role) or a recording
        let detail = if p["media.role"] == "Communication" { "chamada" } else { "" }.to_string();
        let a = by.entry((kind, app.clone())).or_insert(Access { kind, who: app.clone(), detail, streams: vec![], muted: true });
        a.streams.push(id as u32);
        a.muted &= muted;
    }
    by.into_values().collect()
}

/// The agents reading files now: kalangod's log of the last minute (the folder, and only names and types when it
/// only listed).
fn read_agents() -> Vec<Access> {
    let agora = glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0) as u64;
    let Ok(v) = crate::servicos::kalangod_rpc("kalango/registros", json!({"desde": agora.saturating_sub(45), "limite": 40})) else { return vec![] };
    let mut by: BTreeMap<String, Access> = BTreeMap::new();
    for r in v["registros"].as_array().into_iter().flatten() {
        let tool = r["tool"].as_str().unwrap_or("");
        if !tool.starts_with("arquivos.") || r["status"] != "ok" {
            continue;
        }
        let caminho = r["args"]["caminho"].as_str().or(r["args"]["path"].as_str()).or(r["args"]["origem"].as_str()).unwrap_or("").to_string();
        let pasta = std::path::Path::new(&caminho);
        let pasta = if tool == "arquivos.listar" { pasta.to_path_buf() } else { pasta.parent().map(|x| x.to_path_buf()).unwrap_or_default() };
        let casa = glib::home_dir();
        let curta = pasta.strip_prefix(&casa).map(|x| format!("~/{}", x.display())).unwrap_or_else(|_| pasta.display().to_string());
        let quem = tr("Agent").to_string();
        let a = by.entry(quem.clone()).or_insert(Access { kind: Kind::Agent, who: quem, detail: curta.clone(), streams: vec![], muted: false });
        a.detail = curta;
        if tool != "arquivos.listar" {
            a.streams.push(1);
        }
    }
    let pausado = crate::sistema::agent_pids().iter().all(|p| std::fs::read_to_string(format!("/proc/{p}/stat")).is_ok_and(|s| s.rsplit_once(") ").is_some_and(|(_, r)| r.starts_with('T'))));
    by.into_values().map(|mut a| {
        a.muted = pausado && !crate::sistema::agent_pids().is_empty();
        a
    }).collect()
}

/// Today's tokens by model: (model, local?, tokens, replies).
fn tokens_today() -> Vec<(String, bool, u64, usize)> {
    let p = std::env::var_os("XDG_STATE_HOME").map(std::path::PathBuf::from).filter(|p| p.is_absolute())
        .unwrap_or_else(|| glib::home_dir().join(".local/state")).join("kalango/uso-tokens.jsonl");
    let hoje = glib::DateTime::now_local().ok().and_then(|d| glib::DateTime::from_local(d.year(), d.month(), d.day_of_month(), 0, 0, 0.0).ok()).map(|d| d.to_unix()).unwrap_or(0);
    let mut m: BTreeMap<(String, bool), (u64, usize)> = BTreeMap::new();
    for v in std::fs::read_to_string(p).unwrap_or_default().lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()) {
        if v["t"].as_i64().unwrap_or(0) < hoje {
            continue;
        }
        let local = v["tipo"] != "nuvem";
        let e = m.entry((v["modelo"].as_str().unwrap_or("?").to_string(), local)).or_default();
        e.0 += v["tokens"].as_u64().unwrap_or(0);
        e.1 += 1;
    }
    m.into_iter().map(|((n, l), (t, r))| (n, l, t, r)).collect()
}

struct Panel {
    window: gtk::ApplicationWindow,
    motion: Animavel,
    subtitle: gtk::Label,
    pause: gtk::Button,
    now: gtk::Box,
    today: gtk::Box,
    rules: gtk::Box,
}

struct State {
    chips: gtk::Box,
    accesses: RefCell<Vec<Access>>,
    /// When each access began (as first seen).
    since: RefCell<BTreeMap<(Kind, String), i64>>,
    paused: Cell<bool>,
    /// The microphone streams "Pausar tudo" muted (resuming unmutes only these).
    muted_by_pause: RefCell<Vec<u32>>,
    panel: RefCell<Option<Rc<Panel>>>,
    reading: Cell<bool>,
}

thread_local! {
    static STATE: RefCell<Option<Rc<State>>> = const { RefCell::new(None) };
}

fn state() -> Option<Rc<State>> {
    STATE.with(|s| s.borrow().clone())
}

fn glyph(k: Kind) -> (&'static str, &'static str, &'static str) {
    match k {
        Kind::Microphone => ("microfone", "#EFA2A6", "coral"),
        Kind::Camera => ("camera", "#EFA2A6", "coral"),
        Kind::Screen => ("monitor", "#7CC3DF", "azul"),
        Kind::Agent => ("olho", "#8ED2C3", "teal"),
    }
}

/// The chips' box for the top bar (before the agents pill).
pub fn chips() -> gtk::Box {
    if let Some(s) = state() {
        return s.chips.clone();
    }
    let chips = caixa(Orientation::Horizontal, 2, "privacidade-pilula");
    chips.set_valign(Align::Center);
    chips.set_visible(false);
    let s = Rc::new(State { chips: chips.clone(), accesses: RefCell::default(), since: RefCell::default(), paused: Cell::new(false), muted_by_pause: RefCell::default(), panel: RefCell::default(), reading: Cell::new(false) });
    STATE.with(|x| *x.borrow_mut() = Some(s));
    chips
}

fn render_chips(s: &State, before: &[Access]) {
    while let Some(c) = s.chips.first_child() {
        s.chips.remove(&c);
    }
    s.chips.set_visible(!s.accesses.borrow().is_empty());
    for a in s.accesses.borrow().iter() {
        let (g, cor, classe) = glyph(a.kind);
        let b = gtk::Button::new();
        b.add_css_class("privacidade-chip");
        if a.muted {
            b.add_css_class("mudo");
        }
        let r = caixa(Orientation::Horizontal, 5, "");
        let ponto = caixa(Orientation::Horizontal, 0, "privacidade-ponto");
        ponto.add_css_class(classe);
        ponto.set_size_request(6, 6);
        ponto.set_valign(Align::Center);
        r.append(&ponto);
        let ic = ui_icon(g, cor, 14);
        ic.set_valign(Align::Center);
        r.append(&ic);
        r.append(&rotulo(&a.who, "privacidade-chip-texto"));
        b.set_child(Some(&r));
        b.set_tooltip_text(Some(&describe(a)));
        b.connect_clicked(|_| toggle_panel());
        let anim = Animavel::new(&b);
        anim.origem(0.5, 0.5);
        s.chips.append(&anim);
        // a new one comes in with the elastic spring
        if !before.iter().any(|x| x.kind == a.kind && x.who == a.who) && !kalango_movimento::reduzido() {
            anim.definir(Canal::Escala, 0.6).definir(Canal::Opacidade, 0.0);
            anim.ir(Canal::Escala, 1.0, ELASTICA).ir(Canal::Opacidade, 1.0, ELASTICA);
        }
    }
}

fn describe(a: &Access) -> String {
    match a.kind {
        Kind::Microphone => trf("Microphone · {app}", &[("app", a.who.clone())]),
        Kind::Camera => trf("Camera · {app}", &[("app", a.who.clone())]),
        Kind::Screen => trf("Screen · {app}", &[("app", a.who.clone())]),
        Kind::Agent => trf("{agent} reads {folder}", &[("agent", a.who.clone()), ("folder", a.detail.clone())]),
    }
}

/// Reads who is watching (every 2 s, aside) and updates the chips and the open panel.
pub fn start() {
    glib::timeout_add_local(Duration::from_secs(2), || {
        let Some(s) = state() else { return glib::ControlFlow::Continue };
        if s.reading.replace(true) {
            return glib::ControlFlow::Continue;
        }
        crate::lateral::em_segundo_plano(|| {
            let streams = std::process::Command::new(crate::configuracoes_som::comando("pw-dump")).stderr(std::process::Stdio::null()).output().ok()
                .and_then(|o| serde_json::from_slice::<Value>(&o.stdout).ok()).map(|v| read_streams(&v)).unwrap_or_default();
            let mut all = streams;
            all.extend(read_agents());
            all
        }, move |all| {
            let Some(s) = state() else { return };
            s.reading.set(false);
            let before = s.accesses.borrow().clone();
            if before == all {
                return;
            }
            if debug() {
                eprintln!("debug privacidade agora: {}", if all.is_empty() { "nada em uso".to_string() } else { all.iter().map(describe).collect::<Vec<_>>().join("; ") });
            }
            {
                let agora = glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0);
                let mut since = s.since.borrow_mut();
                since.retain(|(k, w), _| all.iter().any(|a| a.kind == *k && &a.who == w));
                for a in &all {
                    since.entry((a.kind, a.who.clone())).or_insert(agora);
                }
            }
            if s.paused.get() {
                for a in all.iter().filter(|a| !before.iter().any(|b| b.kind == a.kind && b.who == a.who && b.streams == a.streams)) {
                    cut_while_paused(&s, a);
                }
            }
            *s.accesses.borrow_mut() = all;
            render_chips(&s, &before);
            let open = s.panel.borrow().clone().filter(|p| p.window.is_visible());
            if let Some(p) = open {
                fill(&p, &s);
            }
        });
        glib::ControlFlow::Continue
    });
}

fn panel() -> Option<Rc<Panel>> {
    let s = state()?;
    if let Some(p) = s.panel.borrow().clone() {
        return Some(p);
    }
    let app = gtk::gio::Application::default().and_downcast::<gtk::Application>()?;
    let window = camada(&app, "kalango-privacidade", Layer::Top, &[Edge::Top, Edge::Right]);
    window.add_css_class("transparente");
    window.set_resizable(false);
    window.set_margin(Edge::Top, 8);
    window.set_margin(Edge::Right, 8);
    window.set_keyboard_mode(KeyboardMode::OnDemand);
    let body = caixa(Orientation::Vertical, 12, "privacidade-cartao");
    body.set_size_request(460, -1);
    let head = caixa(Orientation::Horizontal, 10, "");
    let tile = caixa(Orientation::Horizontal, 0, "privacidade-icone");
    tile.set_size_request(32, 32);
    tile.set_hexpand(false);
    tile.set_valign(Align::Center);
    let ic = ui_icon("escudo", "#8ED2C3", 16);
    ic.set_hexpand(true);
    ic.set_halign(Align::Center);
    ic.set_valign(Align::Center);
    tile.append(&ic);
    head.append(&tile);
    let t = caixa(Orientation::Vertical, 1, "");
    t.set_hexpand(true);
    t.set_valign(Align::Center);
    let titulo = rotulo(tr("Privacy now"), "privacidade-titulo");
    titulo.set_xalign(0.0);
    t.append(&titulo);
    let subtitle = rotulo("", "privacidade-local");
    subtitle.set_xalign(0.0);
    t.append(&subtitle);
    head.append(&t);
    let pause = gtk::Button::new();
    pause.add_css_class("privacidade-pausar");
    pause.set_valign(Align::Center);
    head.append(&pause);
    body.append(&head);
    let now = caixa(Orientation::Vertical, 4, "");
    body.append(&now);
    let today = caixa(Orientation::Vertical, 6, "");
    body.append(&today);
    let rules = caixa(Orientation::Vertical, 4, "");
    body.append(&rules);
    let foot = caixa(Orientation::Horizontal, 6, "");
    foot.set_margin_top(4);
    for k in ["Super", "Shift", "P"] {
        foot.append(&rotulo(k, "tecla"));
    }
    foot.append(&rotulo(tr("pause everything"), "privacidade-sub"));
    let espaco = gtk::Box::new(Orientation::Horizontal, 0);
    espaco.set_hexpand(true);
    foot.append(&espaco);
    let rel = gtk::Button::with_label(tr("Full report ›"));
    rel.add_css_class("privacidade-link");
    rel.connect_clicked(|_| {
        hide_panel();
        crate::configuracoes::abrir_painel("privacidade");
    });
    foot.append(&rel);
    body.append(&foot);
    let motion = Animavel::new(&body);
    motion.origem(1.0, 0.0);
    window.set_child(Some(&motion));
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(|_, k, _, _| {
        if k == gtk::gdk::Key::Escape {
            hide_panel();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    window.add_controller(keys);
    pause.connect_clicked(|_| pause_all());
    let p = Rc::new(Panel { window, motion, subtitle, pause, now, today, rules });
    *s.panel.borrow_mut() = Some(p.clone());
    Some(p)
}

fn section(b: &gtk::Box, t: &str) {
    let l = rotulo(t, "grade-agente-secao");
    l.set_xalign(0.0);
    l.set_margin_top(4);
    b.append(&l);
}

fn row(icon: &str, color: &str, title: &str, sub: &str, switch: Option<(bool, Box<dyn Fn(bool)>)>, debug_name: &str) -> gtk::Box {
    let r = caixa(Orientation::Horizontal, 10, "privacidade-linha");
    let ic = ui_icon(icon, color, 16);
    ic.set_valign(Align::Center);
    r.append(&ic);
    let c = caixa(Orientation::Vertical, 1, "");
    c.set_hexpand(true);
    c.set_valign(Align::Center);
    let tl = rotulo(title, "privacidade-linha-titulo");
    tl.set_xalign(0.0);
    tl.set_ellipsize(gtk::pango::EllipsizeMode::End);
    tl.set_max_width_chars(1);
    tl.set_hexpand(true);
    c.append(&tl);
    if !sub.is_empty() {
        let sl = rotulo(sub, "privacidade-sub");
        sl.set_xalign(0.0);
        sl.set_ellipsize(gtk::pango::EllipsizeMode::End);
        sl.set_max_width_chars(1);
        sl.set_hexpand(true);
        c.append(&sl);
    }
    r.append(&c);
    if let Some((on, f)) = switch {
        let sw = gtk::Switch::new();
        sw.set_active(on);
        sw.set_valign(Align::Center);
        sw.connect_state_set(move |_, v| {
            f(v);
            glib::Propagation::Proceed
        });
        r.append(&sw);
        if debug() && !debug_name.is_empty() {
            log_place(debug_name, sw.upcast_ref());
        }
    }
    r
}

/// Cuts (or gives back) one access at once.
fn set_access(a: &Access, on: bool) {
    if debug() {
        eprintln!("debug privacidade agora: {} {}", describe(a), if on { "liberado" } else { "cortado" });
    }
    match a.kind {
        Kind::Microphone => {
            for id in &a.streams {
                let _ = std::process::Command::new(crate::configuracoes_som::comando("wpctl")).args(["set-mute", &id.to_string(), if on { "0" } else { "1" }]).status();
            }
        }
        Kind::Camera | Kind::Screen => {
            // a sharing the system's chooser started ends there (niri stops recording, the app is told)
            if !on && a.kind == Kind::Screen {
                crate::screen_share::stop_all();
            }
            // a video stream cannot be muted: cutting it is closing it (the app sees its stream end)
            if !on {
                for id in &a.streams {
                    let _ = std::process::Command::new(crate::configuracoes_som::comando("pw-cli")).args(["destroy", &id.to_string()]).status();
                }
            }
        }
        Kind::Agent => crate::sistema::pausar_agentes(!on),
    }
}

/// WirePlumber 0.4's camera monitors off: no camera node in PipeWire, so apps (through it and the camera portal) and
/// agents find none. Not written in the development session (it runs on someone's own system), except in a test
/// profile.
fn camera_rule() -> std::path::PathBuf {
    glib::user_config_dir().join("wireplumber/main.lua.d/51-kalango-camera.lua")
}

fn camera_blocked() -> bool {
    camera_rule().exists()
}

fn set_camera_blocked(block: bool) {
    let dev = std::env::var_os("KALANGO_SHELL_DEV").is_some();
    let perfil_de_teste = std::env::var_os("KALANGO_AREA_TESTE").is_some();
    if debug() {
        eprintln!("debug privacidade agora: câmera {}", if block { "bloqueada" } else { "liberada" });
    }
    if dev && !perfil_de_teste {
        return;
    }
    let p = camera_rule();
    if block {
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(&p, "-- KalangoOS (Privacidade agora › Câmera): a câmera desligada no sistema\nv4l2_monitor.enabled = false\nlibcamera_monitor.enabled = false\n");
        // and whoever has it now loses it
        if let Some(s) = state() {
            for a in s.accesses.borrow().iter().filter(|a| a.kind == Kind::Camera) {
                set_access(a, false);
            }
        }
    } else {
        let _ = std::fs::remove_file(&p);
    }
    if !dev {
        std::thread::spawn(|| {
            let _ = std::process::Command::new("systemctl").args(["--user", "restart", "wireplumber"]).status();
        });
    }
}

/// While everything is paused: a microphone is muted (and remembered, to give it back), a camera or a screen stream cut.
fn cut_while_paused(s: &State, a: &Access) {
    match a.kind {
        Kind::Agent => {}
        Kind::Microphone => {
            if !a.muted {
                s.muted_by_pause.borrow_mut().extend(a.streams.iter().copied());
                if debug() {
                    eprintln!("debug privacidade agora: a pausa silencia {:?} ({})", a.streams, describe(a));
                }
                set_access(a, false);
            } else if debug() {
                eprintln!("debug privacidade agora: {} já estava sem som (fica assim na volta)", describe(a));
            }
        }
        Kind::Camera | Kind::Screen => set_access(a, false),
    }
}

/// "Pausar tudo": every microphone muted, every camera and screen stream cut, the agents paused; again: back.
pub fn pause_all() {
    let Some(s) = state() else { return };
    let pausing = !s.paused.get();
    s.paused.set(pausing);
    // what is capturing right now (not the last poll's reading)
    let fresh = std::process::Command::new(crate::configuracoes_som::comando("pw-dump")).stderr(std::process::Stdio::null()).output().ok()
        .and_then(|o| serde_json::from_slice::<Value>(&o.stdout).ok()).map(|v| read_streams(&v));
    if let Some(mut f) = fresh {
        f.extend(s.accesses.borrow().iter().filter(|a| a.kind == Kind::Agent).cloned());
        *s.accesses.borrow_mut() = f;
    }
    let accesses = s.accesses.borrow().clone();
    if pausing {
        for a in &accesses {
            cut_while_paused(&s, a);
        }
    } else {
        // back: only the microphones the pause muted (one the person muted stays muted) — the streams it muted, by
        // their ids, whether or not this instant's reading lists them (a stream gone meanwhile is simply not there)
        let mut ours: Vec<u32> = std::mem::take(&mut *s.muted_by_pause.borrow_mut());
        ours.sort_unstable();
        ours.dedup();
        if debug() {
            eprintln!("debug privacidade agora: a volta devolve {ours:?} (microfones lidos: {:?})",
                accesses.iter().filter(|a| a.kind == Kind::Microphone).map(|a| a.streams.clone()).collect::<Vec<_>>());
        }
        for a in accesses.iter().filter(|a| a.kind == Kind::Microphone) {
            let back = Access { streams: a.streams.iter().copied().filter(|id| ours.contains(id)).collect(), ..a.clone() };
            if !back.streams.is_empty() {
                ours.retain(|id| !back.streams.contains(id));
                set_access(&back, true);
            }
        }
        if let Some(model) = accesses.iter().find(|a| a.kind == Kind::Microphone).cloned().or_else(|| s.accesses.borrow().iter().find(|a| a.kind == Kind::Microphone).cloned()) {
            if !ours.is_empty() {
                set_access(&Access { streams: ours, ..model }, true);
            }
        } else {
            for id in ours {
                let _ = std::process::Command::new(crate::configuracoes_som::comando("wpctl")).args(["set-mute", &id.to_string(), "0"]).status();
            }
        }
    }
    crate::sistema::pausar_agentes(pausing);
    if debug() {
        eprintln!("debug privacidade agora: {}", if pausing { "tudo pausado" } else { "retomado" });
    }
    let open = s.panel.borrow().clone().filter(|p| p.window.is_visible());
    if let Some(p) = open {
        fill(&p, &s);
    }
}

fn fill(p: &Rc<Panel>, s: &Rc<State>) {
    let accesses = s.accesses.borrow().clone();
    let active = accesses.iter().filter(|a| !a.muted).count();
    let today = tokens_today();
    let cloud: u64 = today.iter().filter(|(_, l, _, _)| !l).map(|(_, _, t, _)| t).sum();
    p.subtitle.set_text(&format!("{} · {}",
        match active { 0 => tr("nothing in use").to_string(), 1 => tr("1 access in use").to_string(), n => trf("{n} accesses in use", &[("n", n.to_string())]) },
        if cloud == 0 { tr("nothing sent to the cloud").to_string() } else { trf("{n} tokens to the cloud today", &[("n", crate::central_uso::mil(cloud))]) }));
    let conteudo = caixa(Orientation::Horizontal, 6, "");
    let quadrado = caixa(Orientation::Horizontal, 0, "privacidade-parar");
    quadrado.set_size_request(8, 8);
    quadrado.set_valign(Align::Center);
    if !s.paused.get() {
        conteudo.append(&quadrado);
    } else {
        conteudo.append(&ui_icon("tocar", "#8ED2C3", 10));
    }
    conteudo.append(&gtk::Label::new(Some(if s.paused.get() { tr("Resume") } else { tr("Pause everything") })));
    p.pause.set_child(Some(&conteudo));
    if s.paused.get() {
        p.pause.add_css_class("retomar");
    } else {
        p.pause.remove_css_class("retomar");
    }
    for b in [&p.now, &p.today, &p.rules] {
        while let Some(c) = b.first_child() {
            b.remove(&c);
        }
    }
    // in use now: what is watching, and the sensors that are not
    section(&p.now, tr("IN USE RIGHT NOW"));
    for a in &accesses {
        let (g, cor, _) = glyph(a.kind);
        let what = match a.kind {
            Kind::Agent => tr("only names and types of files · local model").to_string(),
            Kind::Microphone if a.muted => tr("muted by you").to_string(),
            Kind::Microphone if a.detail == "chamada" => tr("voice call").to_string(),
            Kind::Microphone => tr("recording now").to_string(),
            Kind::Camera => tr("the camera is on").to_string(),
            Kind::Screen => tr("your screen is being shared").to_string(),
        };
        let desde = s.since.borrow().get(&(a.kind, a.who.clone())).and_then(|t| glib::DateTime::from_unix_local(*t).ok()).and_then(|d| d.format("%H:%M").ok());
        let sub = match (a.kind, desde) {
            (Kind::Agent, _) | (_, None) => what,
            (_, Some(h)) => format!("{} · {what}", trf("since {time}", &[("time", h.to_string())])),
        };
        let a2 = a.clone();
        // (the switch's change counts at once: "Pausar tudo" must not decide from a reading 2 s old)
        let nome = format!("privacidade chave {}", match a.kind { Kind::Microphone => "microfone", Kind::Camera => "camera", Kind::Screen => "tela", Kind::Agent => "agente" });
        p.now.append(&row(g, cor, &describe(a), &sub, Some((!a.muted, Box::new(move |on| {
            set_access(&a2, on);
            if let Some(s) = state() {
                for x in s.accesses.borrow_mut().iter_mut().filter(|x| x.kind == a2.kind && x.who == a2.who) {
                    x.muted = !on;
                }
            }
        }))), &nome));
    }
    for (k, titulo, sub) in [(Kind::Microphone, tr("Microphone · nobody"), tr("no app is recording")), (Kind::Screen, tr("Screen · nobody"), tr("no app or agent is seeing your screen"))] {
        if !accesses.iter().any(|a| a.kind == k) {
            let (g, _, _) = glyph(k);
            p.now.append(&row(g, "#8E8F99", titulo, sub, None, ""));
        }
    }
    // the camera: blocked in the system (its switch), or nobody using it
    if !accesses.iter().any(|a| a.kind == Kind::Camera) {
        let bloqueada = camera_blocked();
        let (titulo, sub) = if bloqueada {
            (tr("Camera · blocked"), tr("off in the system: apps and agents cannot find it"))
        } else {
            (tr("Camera · nobody"), tr("no app is using the camera"))
        };
        p.now.append(&row("camera", "#8E8F99", titulo, sub, Some((!bloqueada, Box::new(|on| set_camera_blocked(!on)))), "privacidade chave bloquear camera"));
    }
    // what left this computer today
    section(&p.today, tr("WHAT LEFT THIS COMPUTER TODAY"));
    let bloco = caixa(Orientation::Vertical, 6, "privacidade-bloco");
    p.today.append(&bloco);
    let local: u64 = today.iter().filter(|(_, l, _, _)| *l).map(|(_, _, t, _)| t).sum();
    let resumo = caixa(Orientation::Horizontal, 8, "");
    let n = rotulo(&format!("{} {}", crate::central_uso::mil(cloud), tr("tokens")), "privacidade-numero");
    resumo.append(&n);
    let l = rotulo(tr("to the cloud"), "privacidade-sub");
    l.set_valign(Align::Baseline);
    resumo.append(&l);
    let espaco = gtk::Box::new(Orientation::Horizontal, 0);
    espaco.set_hexpand(true);
    resumo.append(&espaco);
    resumo.append(&rotulo(&trf("{n} handled only here", &[("n", crate::central_uso::mil(local))]), "privacidade-local"));
    bloco.append(&resumo);
    // the cloud's share on the left (gold), what stayed here after it (teal)
    let barra = gtk::LevelBar::for_interval(0.0, 1.0);
    barra.add_css_class("privacidade-barra");
    barra.set_inverted(true);
    barra.set_value(if local + cloud == 0 { 1.0 } else { local as f64 / (local + cloud) as f64 });
    bloco.append(&barra);
    if today.is_empty() {
        let v = rotulo(tr("The agents have not worked today."), "privacidade-sub");
        v.set_xalign(0.0);
        bloco.append(&v);
    }
    for (modelo, local, tokens, vezes) in today {
        let r = caixa(Orientation::Horizontal, 8, "");
        let ponto = caixa(Orientation::Horizontal, 0, "privacidade-ponto");
        ponto.add_css_class(if local { "teal" } else { "ouro" });
        ponto.set_size_request(6, 6);
        ponto.set_valign(Align::Center);
        r.append(&ponto);
        let t = rotulo(&format!("{modelo} ({}) · {}", if local { tr("local") } else { tr("cloud") }, if vezes == 1 { tr("1 reply").to_string() } else { trf("{n} replies", &[("n", vezes.to_string())]) }), "privacidade-sub");
        t.set_xalign(0.0);
        t.set_hexpand(true);
        t.set_ellipsize(gtk::pango::EllipsizeMode::End);
        t.set_max_width_chars(1);
        r.append(&t);
        r.append(&rotulo(&if local { tr("nothing left").to_string() } else { crate::central_uso::mil(tokens) }, "privacidade-mono"));
        bloco.append(&r);
    }
    // the rules
    section(&p.rules, tr("RULES"));
    let protegidas: Vec<String> = crate::servicos::kalangod_rpc("kalango/protegidas", json!({})).ok()
        .map(|v| v["pastas"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
    let casa = glib::home_dir();
    let curta = |x: &String| std::path::Path::new(x).strip_prefix(&casa).map(|r| format!("~/{}", r.display())).unwrap_or_else(|_| x.clone());
    let resumo_p = match protegidas.len() {
        0 => tr("none yet").to_string(),
        1 => trf("{a} — agents never read it", &[("a", curta(&protegidas[0]))]),
        2 => trf("{a} and {b} — agents never read them", &[("a", curta(&protegidas[0])), ("b", curta(&protegidas[1]))]),
        n => trf("{a}, {b} and {n} more — agents never read them", &[("a", curta(&protegidas[0])), ("b", curta(&protegidas[1])), ("n", (n - 2).to_string())]),
    };
    let r = row("cadeado", "#B9BAC4", tr("Private folders"), &resumo_p, None, "");
    let ir = gtk::Button::new();
    ir.add_css_class("privacidade-link");
    ir.set_child(Some(&ui_icon("chevron", "#8E8F99", 12)));
    ir.set_valign(Align::Center);
    ir.connect_clicked(|_| {
        hide_panel();
        crate::configuracoes::abrir_painel("privacidade");
    });
    r.append(&ir);
    p.rules.append(&r);
    let pergunta = crate::configuracoes_privacidade::pergunta_antes_da_nuvem();
    p.rules.append(&row("compartilhar", "#B9BAC4", tr("Ask before sending files to the cloud"), "", Some((pergunta, Box::new(crate::configuracoes_privacidade::definir_pergunta_antes_da_nuvem))), "privacidade chave nuvem"));
    let dias = crate::configuracoes_privacidade::dias_de_historico();
    p.rules.append(&row("lixeira", "#B9BAC4", &trf("Erase the agents' history after {n} days", &[("n", dias.to_string())]), "",
        Some((dias > 0, Box::new(|on| crate::configuracoes_privacidade::definir_historico(if on { 30 } else { 0 })))), "privacidade chave historico"));
    if debug() {
        log_place("privacidade pausar", p.pause.upcast_ref());
    }
}

/// For the session test: where a control of the panel is on the screen (the panel is anchored top right, below the
/// bar), once it is laid out.
fn log_place(name: &str, w: &gtk::Widget) {
    let (name, w) = (name.to_string(), w.downgrade());
    glib::timeout_add_local_once(Duration::from_millis(450), move || {
        let (Some(w), Some(p)) = (w.upgrade(), state().and_then(|s| s.panel.borrow().clone())) else { return };
        let (Some((largura, _)), Some(r)) = (crate::superficies::tamanho_da_saida(), w.compute_bounds(&p.window)) else { return };
        let x = largura - 8 - p.window.width() + (r.x() + r.width() / 2.) as i32;
        let y = crate::superficies::BARRA_ALTURA + 8 + (r.y() + r.height() / 2.) as i32;
        eprintln!("debug privacidade: {name} em {x} {y}");
    });
}

pub fn toggle_panel() {
    let Some(p) = panel() else { return };
    if p.window.is_visible() {
        hide_panel();
        return;
    }
    let Some(s) = state() else { return };
    fill(&p, &s);
    p.window.present();
    p.motion.entrar((0.0, -8.0), 0.97);
    if debug() {
        eprintln!("debug privacidade agora: painel aberto ({} acessos)", s.accesses.borrow().len());
    }
}

fn hide_panel() {
    let Some(p) = state().and_then(|s| s.panel.borrow().clone()) else { return };
    let w = p.window.downgrade();
    p.motion.sair((0.0, -8.0), 0.97, move || {
        if let Some(w) = w.upgrade() {
            w.set_visible(false);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_who_records_and_who_casts() {
        let v = json!([
            {"id": 70, "type": "PipeWire:Interface:Node", "info": {"props": {"media.class": "Stream/Input/Audio", "application.name": "Discord"}, "params": {"Props": [{"mute": false}]}}},
            {"id": 71, "type": "PipeWire:Interface:Node", "info": {"props": {"media.class": "Stream/Input/Audio", "application.name": "kalango-shell"}}},
            {"id": 80, "type": "PipeWire:Interface:Node", "info": {"props": {"media.class": "Video/Source", "node.name": "niri"}}},
            {"id": 81, "type": "PipeWire:Interface:Node", "info": {"props": {"media.class": "Video/Source", "node.name": "v4l2_input.webcam"}}},
            {"id": 82, "type": "PipeWire:Interface:Node", "info": {"props": {"media.class": "Stream/Input/Video", "application.name": "OBS"}}},
            {"id": 83, "type": "PipeWire:Interface:Node", "info": {"props": {"media.class": "Stream/Input/Video", "application.name": "Zoom"}}},
            {"id": 84, "type": "PipeWire:Interface:Link", "info": {"output-node-id": 80, "input-node-id": 82}},
            {"id": 85, "type": "PipeWire:Interface:Link", "info": {"output-node-id": 81, "input-node-id": 83}},
            {"id": 90, "type": "PipeWire:Interface:Node", "info": {"props": {"media.class": "Audio/Sink", "node.name": "alsa"}}}
        ]);
        let a = read_streams(&v);
        assert_eq!(a.len(), 3, "{a:?}");
        assert!(a.iter().any(|x| x.kind == Kind::Microphone && x.who == "Discord" && x.streams == vec![70] && !x.muted));
        assert!(a.iter().any(|x| x.kind == Kind::Screen && x.who == "OBS" && x.streams == vec![82]));
        assert!(a.iter().any(|x| x.kind == Kind::Camera && x.who == "Zoom" && x.streams == vec![83]));
    }
}
