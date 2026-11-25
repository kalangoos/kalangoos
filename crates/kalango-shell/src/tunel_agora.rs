//! The internet tunnel as visible state (design "Gatilho pela internet · indicador na barra e menu rápido"): while a
//! tunnel is open a chip with a globe stays in the top bar with how many triggers it carries and for how long it has
//! been open. A click opens the quick menu: each trigger with its address and its last call, the red "Fechar todos
//! os túneis" and, before it closes, the confirmation (design "fechar todos (confirmação)"). Ctrl+Shift+F12 closes
//! everything from anywhere, with no question: it is the emergency.
//!
//! The chip reads the state file the service writes (`$XDG_RUNTIME_DIR/kalango/tunel.json`) once a second, so it is
//! there even when the agents center is closed; the menu asks the service for the last calls.

use crate::conversa::ui_icon;
use crate::internet_modelo::{self as modelo, Call, Tunnel};
use crate::superficies::{caixa, camada, rotulo};
use gtk::prelude::*;
use gtk::{glib, Align, Orientation};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kalango_i18n::tr;
use serde_json::json;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

struct Panel {
    window: gtk::ApplicationWindow,
    body: gtk::Box,
}

struct State {
    chip: gtk::Button,
    text: gtk::Label,
    time: gtk::Label,
    tunnel: RefCell<Tunnel>,
    calls: RefCell<Vec<Call>>,
    panel: RefCell<Option<Rc<Panel>>>,
}

thread_local! {
    static STATE: RefCell<Option<Rc<State>>> = const { RefCell::new(None) };
}

fn state() -> Option<Rc<State>> {
    STATE.with(|s| s.borrow().clone())
}

fn now() -> i64 {
    glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0)
}

/// The chip for the top bar (before the privacy chips). Hidden while no tunnel is open.
pub fn chips() -> gtk::Box {
    let holder = caixa(Orientation::Horizontal, 0, "");
    holder.set_valign(Align::Center);
    if let Some(s) = state() {
        holder.append(&s.chip);
        return holder;
    }
    let chip = gtk::Button::new();
    chip.add_css_class("tunel-chip");
    chip.set_valign(Align::Center);
    chip.set_visible(false);
    let row = caixa(Orientation::Horizontal, 6, "");
    let dot = caixa(Orientation::Horizontal, 0, "privacidade-ponto");
    dot.add_css_class("teal");
    dot.set_size_request(6, 6);
    dot.set_valign(Align::Center);
    row.append(&dot);
    let text = rotulo("", "tunel-chip-texto");
    row.append(&text);
    let time = rotulo("", "tunel-chip-tempo");
    row.append(&time);
    chip.set_child(Some(&row));
    chip.connect_clicked(|_| toggle_panel());
    holder.append(&chip);
    let s = Rc::new(State { chip, text, time, tunnel: RefCell::default(), calls: RefCell::default(), panel: RefCell::default() });
    STATE.with(|x| *x.borrow_mut() = Some(s));
    holder
}

/// Reads the state file every second: the chip appears and goes by itself, and the open menu follows it.
pub fn start() {
    glib::timeout_add_local(Duration::from_secs(1), || {
        let Some(s) = state() else { return glib::ControlFlow::Continue };
        let tunnel = std::fs::read_to_string(modelo::state_file()).ok().and_then(|t| modelo::parse_tunnel(&t)).unwrap_or_default();
        let t = now();
        match modelo::chip(&tunnel, t) {
            Some((text, time)) => {
                // (starting: amber, while it is not open yet)
                if tunnel.opening && !tunnel.open {
                    s.chip.add_css_class("abrindo");
                } else {
                    s.chip.remove_css_class("abrindo");
                }
                s.text.set_label(&text);
                s.time.set_label(&time);
                s.chip.set_tooltip_text(Some(modelo::host(&tunnel.url)));
                s.chip.set_visible(true);
            }
            None => {
                s.chip.set_visible(false);
                if s.tunnel.borrow().open || s.tunnel.borrow().opening {
                    hide_panel();
                }
            }
        }
        let changed = *s.tunnel.borrow() != tunnel;
        *s.tunnel.borrow_mut() = tunnel;
        if changed && s.panel.borrow().as_ref().is_some_and(|p| p.window.is_visible()) {
            refresh();
        }
        glib::ControlFlow::Continue
    });
}

fn panel() -> Option<Rc<Panel>> {
    let s = state()?;
    if let Some(p) = s.panel.borrow().clone() {
        return Some(p);
    }
    let app = gtk::gio::Application::default().and_downcast::<gtk::Application>()?;
    let window = camada(&app, "kalango-tunel", Layer::Top, &[Edge::Top, Edge::Right]);
    window.add_css_class("transparente");
    window.set_resizable(false);
    window.set_margin(Edge::Top, 8);
    window.set_margin(Edge::Right, 8);
    window.set_keyboard_mode(KeyboardMode::OnDemand);
    let body = caixa(Orientation::Vertical, 10, "privacidade-cartao");
    body.add_css_class("tunel-cartao");
    // (the window gives way: never wider than the screen, never fixed to a number of its own)
    body.set_size_request(380, -1);
    window.set_child(Some(&body));
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(|_, k, _, _| {
        if k == gtk::gdk::Key::Escape {
            hide_panel();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    window.add_controller(keys);
    let p = Rc::new(Panel { window, body });
    *s.panel.borrow_mut() = Some(p.clone());
    Some(p)
}

pub fn toggle_panel() {
    let Some(p) = panel() else { return };
    if p.window.is_visible() {
        hide_panel();
        return;
    }
    refresh();
    p.window.present();
}

fn hide_panel() {
    if let Some(p) = state().and_then(|s| s.panel.borrow().clone()) {
        p.window.set_visible(false);
    }
}

/// Asks the service for the latest calls, then draws the menu.
fn refresh() {
    crate::fluxos::call("internet.calls", json!({"limit": 100}), |r| {
        let Some(s) = state() else { return };
        *s.calls.borrow_mut() = r.map(|v| modelo::parse_calls(&v)).unwrap_or_default();
        draw();
    });
    draw();
}

fn draw() {
    let (Some(s), Some(p)) = (state(), panel()) else { return };
    while let Some(c) = p.body.first_child() {
        p.body.remove(&c);
    }
    let tunnel = s.tunnel.borrow().clone();
    let head = caixa(Orientation::Horizontal, 8, "");
    head.append(&ui_icon("globo", "#8ED2C3", 14));
    let title = rotulo(&if tunnel.open { modelo::menu_title(&tunnel) } else { tr("Opening the tunnel").to_string() }, "privacidade-titulo");
    title.set_xalign(0.0);
    head.append(&title);
    p.body.append(&head);
    let t = now();
    // starting (design “Abrindo o túnel”): a spinner, the seconds, what it is doing
    if tunnel.opening && !tunnel.open {
        let r = caixa(Orientation::Horizontal, 12, "tunel-linha");
        let sp = gtk::Spinner::new();
        sp.set_spinning(true);
        sp.set_size_request(28, 28);
        r.append(&sp);
        let c = caixa(Orientation::Vertical, 2, "");
        c.append(&rotulo(&kalango_i18n::trf("Opening quick tunnel… {s} s", &[("s", (t - tunnel.opening_since).max(0).to_string())]), "privacidade-linha-titulo"));
        let sub = rotulo(tr("Connecting to the tunnel service · checking the program · registering the address"), "privacidade-sub");
        sub.set_xalign(0.0);
        sub.set_wrap(true);
        sub.set_max_width_chars(1);
        c.append(&sub);
        r.append(&c);
        p.body.append(&r);
        let note = rotulo(tr("It can take about 45 s the first time. The trigger only receives calls when it turns green."), "privacidade-sub");
        note.set_xalign(0.0);
        note.set_wrap(true);
        note.set_max_width_chars(1);
        p.body.append(&note);
        return;
    }
    // one tunnel carries all the triggers: its address once, then each trigger with its last call
    let addr = rotulo(modelo::host(&tunnel.url), "privacidade-mono");
    addr.set_xalign(0.0);
    addr.set_selectable(true);
    p.body.append(&addr);
    for trg in &tunnel.triggers {
        let calls = s.calls.borrow();
        let last = calls.iter().find(|c| c.trigger_id == trg.id);
        let card = caixa(Orientation::Vertical, 3, "tunel-linha");
        let top = caixa(Orientation::Horizontal, 8, "");
        let dot = caixa(Orientation::Horizontal, 0, "privacidade-ponto");
        dot.add_css_class(if tunnel.healthy { "teal" } else { "ouro" });
        dot.set_size_request(6, 6);
        dot.set_valign(Align::Center);
        top.append(&dot);
        let name = rotulo(&trg.flow, "privacidade-linha-titulo");
        name.set_xalign(0.0);
        name.set_hexpand(true);
        name.set_ellipsize(gtk::pango::EllipsizeMode::End);
        name.set_max_width_chars(1);
        top.append(&name);
        card.append(&top);
        let line = rotulo(&modelo::last_call(last, t), "privacidade-sub");
        if last.is_some_and(|c| c.status >= 400) {
            line.add_css_class("tunel-aviso");
        }
        line.set_xalign(0.0);
        card.append(&line);
        p.body.append(&card);
    }
    let kill = gtk::Button::new();
    kill.add_css_class("tunel-fechar-todos");
    let kr = caixa(Orientation::Horizontal, 6, "");
    kr.set_halign(Align::Center);
    kr.append(&rotulo("✕", "tunel-fechar-x"));
    kr.append(&rotulo(&modelo::kill_title(2), "tunel-fechar-texto"));
    kill.set_child(Some(&kr));
    kill.connect_clicked(|_| confirm());
    p.body.append(&kill);
    let foot = rotulo(tr("Ctrl Shift F12 also closes everything, from anywhere."), "privacidade-sub");
    foot.set_xalign(0.0);
    foot.set_wrap(true);
    p.body.append(&foot);
}

thread_local! {
    static CONFIRM: RefCell<Option<gtk::ApplicationWindow>> = const { RefCell::new(None) };
}

/// The confirmation (design "fechar todos (confirmação)"): the centred card of the approval screen's look, with what
/// closes, what stays and how many calls are still going (asked of the service).
fn confirm() {
    let Some(s) = state() else { return };
    let Some(app) = gtk::gio::Application::default().and_downcast::<gtk::Application>() else { return };
    hide_panel();
    CONFIRM.with(|c| {
        if let Some(w) = c.borrow_mut().take() {
            w.close();
        }
    });
    let tunnel = s.tunnel.borrow().clone();
    let win = camada(&app, "kalango-tunel-confirmar", Layer::Overlay, &[]);
    win.add_css_class("transparente");
    win.set_keyboard_mode(KeyboardMode::OnDemand);
    let body = caixa(Orientation::Vertical, 10, "privacidade-cartao");
    body.add_css_class("tunel-confirmar");
    body.set_size_request(460, -1);
    let head = caixa(Orientation::Horizontal, 8, "");
    head.append(&ui_icon("escudo", "#EFA2A6", 14));
    let title = rotulo(tr("Permission needed"), "privacidade-titulo");
    title.set_hexpand(true);
    title.set_xalign(0.0);
    head.append(&title);
    head.append(&rotulo(tr("Turns off now"), "tunel-selo"));
    body.append(&head);
    let names: Vec<String> = tunnel.triggers.iter().map(|t| format!("“{}”", t.flow)).collect();
    let q = rotulo(&kalango_i18n::trf("Close the open tunnel? The triggers {names} stop receiving calls at once.", &[("names", names.join(", "))]), "privacidade-sub");
    q.set_xalign(0.0);
    q.set_wrap(true);
    q.set_max_width_chars(1);
    q.set_hexpand(true);
    body.append(&q);
    let warn = caixa(Orientation::Vertical, 2, "tunel-aviso-caixa");
    let wt = rotulo(tr("Whoever calls will get an error"), "tunel-aviso-titulo");
    wt.set_xalign(0.0);
    warn.append(&wt);
    let w = rotulo(tr("The addresses of the quick tunnel change on the next opening; your own domain stays the same. Opening again asks for approval."), "privacidade-sub");
    w.set_xalign(0.0);
    w.set_wrap(true);
    w.set_max_width_chars(1);
    w.set_hexpand(true);
    warn.append(&w);
    body.append(&warn);
    let rows = caixa(Orientation::Vertical, 3, "tunel-quadro");
    let value_labels: Vec<gtk::Label> = modelo::kill_confirm_rows(&tunnel, 0)
        .into_iter()
        .map(|(k, v)| {
            let r = caixa(Orientation::Horizontal, 12, "");
            let kl = rotulo(&k, "privacidade-sub");
            kl.set_xalign(0.0);
            kl.set_size_request(120, -1);
            r.append(&kl);
            let vl = rotulo(&v, "privacidade-mono");
            vl.set_xalign(0.0);
            r.append(&vl);
            rows.append(&r);
            vl
        })
        .collect();
    body.append(&rows);
    // (the calls in progress come from the service: the row is filled when it answers)
    let t2 = tunnel.clone();
    crate::fluxos::call("internet.inflight", json!({}), move |r| {
        if let (Ok(v), Some(l)) = (r, value_labels.last()) {
            l.set_label(&modelo::kill_confirm_rows(&t2, v["runs"].as_i64().unwrap_or(0))[2].1);
        }
    });
    let note = rotulo(tr("Closing does not delete triggers, keys or the token of the domain."), "privacidade-sub");
    note.set_xalign(0.0);
    note.set_wrap(true);
    note.set_max_width_chars(1);
    body.append(&note);
    let buttons = caixa(Orientation::Horizontal, 8, "");
    buttons.set_homogeneous(true);
    let cancel = gtk::Button::with_label(tr("Cancel"));
    cancel.add_css_class("tunel-botao");
    let go = gtk::Button::with_label(tr("Close all"));
    go.add_css_class("tunel-fechar-todos");
    buttons.append(&cancel);
    buttons.append(&go);
    body.append(&buttons);
    win.set_child(Some(&body));
    let w = win.clone();
    cancel.connect_clicked(move |_| w.close());
    let w = win.clone();
    go.connect_clicked(move |_| {
        close_all(false);
        w.close();
    });
    let keys = gtk::EventControllerKey::new();
    let w = win.clone();
    keys.connect_key_pressed(move |_, k, _, _| {
        if k == gtk::gdk::Key::Escape {
            w.close();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    win.add_controller(keys);
    win.present();
    CONFIRM.with(|c| *c.borrow_mut() = Some(win));
}

/// The menu, already at the question "close all?" (the Túneis page's button).
pub fn ask_to_close() {
    confirm();
}

/// Closes every tunnel. `ask`: through the confirmation first (a button in the menu); the shortcut never asks.
pub fn close_all(ask: bool) {
    if ask {
        confirm();
        return;
    }
    crate::fluxos::call("internet.tunnel_close_all", json!({}), |_| {
        hide_panel();
        // (the chip goes away at the next look at the state file, which the service has just rewritten)
    });
}
