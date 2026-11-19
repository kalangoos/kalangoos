//! The fixed dock as a taskbar (design "Dock fixa · barra de tarefas", for whoever prefers a fixed, Windows-like bar):
//! the same dock window in panel mode (not a second dock), and the top bar goes away. The bar is 52 px tall:
//! - **Left:** Iniciar (the start menu), "Buscar ou pedir ao agente" (the command bar) and Widgets (Today).
//! - **Middle:** the task view (the overview) and the apps, centred or on the left (`EstiloDock::alinhamento`).
//! - **Right:** the agents' chip, the hidden indicators, the tray (network, sound, Bluetooth: the controls), the time
//!   and date (Today), the notices with their count, and the thin "Mostrar área de trabalho" strip at the end.

use crate::conversa::{set_icon, ui_icon};
use crate::superficies::{caixa, rotulo};
use crate::Shell;
use gtk::prelude::*;
use gtk::{glib, Align, Orientation};
use kalango_i18n::{tr, trf};
use std::rc::Rc;

const ON: &str = "#E2E2E6";
const MUTED: &str = "#8E8F99";
const OFF: &str = "#7A7B88";
const ACCENT: &str = "#A5A1E2";

/// The bar's parts around the apps.
pub struct Parts {
    pub left: gtk::Box,
    pub tasks: gtk::Button,
    pub divider: gtk::Box,
    pub right: gtk::Box,
    pub start: gtk::Button,
}

fn flat(class: &str, tip: &str) -> gtk::Button {
    let b = gtk::Button::new();
    b.add_css_class("tb-botao");
    if !class.is_empty() {
        b.add_css_class(class);
    }
    b.set_tooltip_text(Some(tip));
    b.set_valign(Align::Center);
    b
}

pub fn build(shell: &Rc<Shell>) -> Parts {
    // ---- left: start, search, widgets
    let left = caixa(Orientation::Horizontal, 6, "");
    left.set_valign(Align::Center);
    left.set_margin_start(8);
    let start = flat("tb-iniciar", tr("Start menu"));
    start.set_child(Some(&ui_icon("spark", ACCENT, 13)));
    {
        let s = shell.clone();
        start.connect_clicked(move |_| crate::start_menu::toggle(&s));
    }
    left.append(&start);
    let search = gtk::Button::new();
    search.add_css_class("tb-busca");
    search.set_valign(Align::Center);
    // (260 × 32 as in the design: its text does not widen it)
    search.set_size_request(260, 32);
    search.set_hexpand(false);
    let sb = caixa(Orientation::Horizontal, 8, "");
    sb.append(&ui_icon("spark", ACCENT, 13));
    let st = rotulo(tr("Search or ask the agent"), "tb-busca-texto");
    st.set_hexpand(true);
    st.set_xalign(0.0);
    st.set_ellipsize(gtk::pango::EllipsizeMode::End);
    st.set_width_chars(1);
    sb.append(&st);
    sb.append(&rotulo("Ctrl K", "tb-atalho"));
    search.set_child(Some(&sb));
    {
        let s = shell.clone();
        search.connect_clicked(move |_| s.comando().alternar());
    }
    left.append(&search);
    let widgets = flat("", tr("Widgets"));
    widgets.set_child(Some(&ui_icon("grade", MUTED, 16)));
    {
        let s = shell.clone();
        widgets.connect_clicked(move |_| s.today_bar().alternar());
    }
    left.append(&widgets);
    // (with the apps on the left: a divider before them)
    let divider = caixa(Orientation::Vertical, 0, "tb-divisor");
    divider.set_size_request(1, 24);
    divider.set_valign(Align::Center);
    left.append(&divider);

    // ---- the task view, before the apps
    let tasks = flat("", tr("Task view"));
    tasks.set_child(Some(&ui_icon("apps-pontos", MUTED, 16)));
    {
        let s = shell.clone();
        tasks.connect_clicked(move |_| s.visao().alternar());
    }

    // ---- right: agents, hidden indicators, tray, clock, notices, show the desktop
    let right = caixa(Orientation::Horizontal, 4, "");
    right.set_valign(Align::Center);
    right.set_halign(Align::End);
    right.set_margin_end(8);
    let agents = gtk::Button::new();
    agents.add_css_class("tb-agentes");
    agents.set_valign(Align::Center);
    let ab = caixa(Orientation::Horizontal, 6, "");
    ab.append(&ui_icon("spark", ACCENT, 13));
    let atxt = rotulo(tr("agents"), "tb-agentes-texto");
    ab.append(&atxt);
    let dots = caixa(Orientation::Horizontal, 3, "");
    dots.set_valign(Align::Center);
    ab.append(&dots);
    agents.set_child(Some(&ab));
    {
        let s = shell.clone();
        agents.connect_clicked(move |_| s.right_bar().alternar(crate::lateral::Modo::Agente));
    }
    right.append(&agents);
    let hidden = flat("tb-ocultos", tr("More indicators"));
    hidden.set_child(Some(&ui_icon("chevron-baixo", MUTED, 12)));
    {
        let s = shell.clone();
        hidden.connect_clicked(move |_| s.right_bar().alternar(crate::lateral::Modo::Controles));
    }
    right.append(&hidden);
    let tray = flat("tb-bandeja", tr("Network, sound and Bluetooth"));
    let tb = caixa(Orientation::Horizontal, 10, "");
    let net = ui_icon("wifi", ON, 14);
    let vol = ui_icon("volume", ON, 14);
    let bt = ui_icon("bluetooth", ON, 14);
    tb.append(&net);
    tb.append(&vol);
    tb.append(&bt);
    tray.set_child(Some(&tb));
    {
        let s = shell.clone();
        tray.connect_clicked(move |_| s.right_bar().alternar(crate::lateral::Modo::Controles));
    }
    {
        let (ni, vi, bi) = (net.clone(), vol.clone(), bt.clone());
        shell.on_system_state(move |st| {
            let (n, c) = if st.rede.aviao {
                ("aviao", ON)
            } else if st.rede.conectada.is_some() {
                (if st.rede.sinal <= 40 { "wifi-fraco" } else { "wifi" }, ON)
            } else if st.rede.cabo {
                ("cabo", ON)
            } else {
                ("wifi-sem", if st.rede.wifi_ligado { ON } else { OFF })
            };
            set_icon(&ni, n, c);
            set_icon(&vi, if st.som.mudo { "volume-mudo" } else if st.som.nivel > 0.4 { "volume" } else { "volume-baixo" }, ON);
            bi.set_visible(st.bt.existe);
            if st.bt.ligado { set_icon(&bi, "bluetooth", ON) } else { set_icon(&bi, "bluetooth-off", OFF) }
        });
    }
    right.append(&tray);
    let clock = flat("tb-relogio", tr("Today"));
    let cb = caixa(Orientation::Vertical, 0, "");
    cb.set_valign(Align::Center);
    let time = rotulo("", "tb-hora");
    time.set_halign(Align::End);
    let date = rotulo("", "tb-data");
    date.set_halign(Align::End);
    cb.append(&time);
    cb.append(&date);
    clock.set_child(Some(&cb));
    {
        let s = shell.clone();
        clock.connect_clicked(move |_| s.today_bar().alternar());
    }
    let tick = {
        let (time, date) = (time.clone(), date.clone());
        move || {
            let Ok(n) = glib::DateTime::now_local() else { return };
            let a = crate::configuracoes::ajustes();
            let f = if a["relogio_12h"].as_bool().unwrap_or(false) { "%I:%M %p" } else { "%H:%M" };
            time.set_text(&n.format(f).map(|s| s.to_string()).unwrap_or_default());
            date.set_text(&n.format("%d/%m/%Y").map(|s| s.to_string()).unwrap_or_default());
        }
    };
    tick();
    glib::timeout_add_seconds_local(5, move || {
        tick();
        glib::ControlFlow::Continue
    });
    right.append(&clock);
    let bell = flat("tb-avisos", tr("Notices"));
    let bo = gtk::Overlay::new();
    // (the glyph at its size in the middle of a 36 px square)
    let bell_box = caixa(Orientation::Vertical, 0, "");
    bell_box.set_size_request(36, 36);
    bell_box.set_valign(Align::Center);
    let bell_icon = ui_icon("sino", MUTED, 14);
    bell_icon.set_vexpand(true);
    bell_box.append(&bell_icon);
    bo.set_child(Some(&bell_box));
    let count = rotulo("", "tb-contador");
    count.set_halign(Align::End);
    count.set_valign(Align::Start);
    count.set_margin_top(4);
    count.set_visible(false);
    bo.add_overlay(&count);
    bell.set_child(Some(&bo));
    {
        let s = shell.clone();
        bell.connect_clicked(move |_| s.right_bar().alternar(crate::lateral::Modo::Avisos));
    }
    {
        let (s, count) = (Rc::downgrade(shell), count.clone());
        let update = move || {
            let Some(s) = s.upgrade() else { return };
            let n = s.notificacoes().historico.borrow().iter().filter(|g| !g.lida).count();
            count.set_text(&n.min(99).to_string());
            count.set_visible(n > 0);
        };
        update();
        shell.notificacoes().ouvir(update);
    }
    right.append(&bell);
    let desktop = gtk::Button::new();
    desktop.add_css_class("tb-area");
    desktop.set_size_request(6, 36);
    desktop.set_valign(Align::Center);
    desktop.set_tooltip_text(Some(tr("Show the desktop")));
    desktop.connect_clicked(|_| crate::edges::show_desktop());
    right.append(&desktop);

    // the agents' chip as the top bar's: what is waiting, or the agent at work
    {
        let est = shell.estado.clone();
        let render = move || {
            while let Some(c) = dots.first_child() {
                dots.remove(&c);
            }
            let dot = |op: f64, class: &str| {
                let p = caixa(Orientation::Horizontal, 0, "tb-ponto");
                p.add_css_class(class);
                p.set_size_request(4, 4);
                p.set_valign(Align::Center);
                p.set_opacity(op);
                p
            };
            match est.pendentes.get() {
                None => {
                    atxt.set_text(tr("agents"));
                    agents.set_tooltip_text(Some(tr("kalangod is not responding")));
                }
                Some(0) if est.agent_working.get() => {
                    atxt.set_text(tr("working"));
                    for op in [1.0, 0.6, 0.3] {
                        dots.append(&dot(op, "vivo"));
                    }
                    agents.set_tooltip_text(Some(tr("The agent is working on your request")));
                }
                Some(0) => {
                    atxt.set_text(tr("agents"));
                    dots.append(&dot(1.0, "vivo"));
                    agents.set_tooltip_text(Some(tr("No requests waiting for you")));
                }
                Some(n) => {
                    atxt.set_text(&if n == 1 { tr("1 request").to_string() } else { trf("{n} requests", &[("n", n.to_string())]) });
                    for _ in 0..n.min(3) {
                        dots.append(&dot(1.0, "pedido"));
                    }
                    agents.set_tooltip_text(Some(tr("Agent requests waiting for your approval")));
                }
            }
        };
        render();
        shell.estado.ouvir(render);
    }
    Parts { left, tasks, divider, right, start }
}

/// Debug: where the start button is on the output (the session test clicks it), once laid out.
pub fn log_places(p: &Parts) {
    let start = p.start.clone();
    let tasks = p.tasks.clone();
    glib::timeout_add_local_once(std::time::Duration::from_millis(600), move || {
        let Some(root) = start.root() else { return };
        let Some(b) = start.compute_bounds(&root) else { return };
        if !start.is_mapped() {
            return;
        }
        // (the bar is at the bottom of the output: its distance from the bottom edge)
        eprintln!("debug barra de tarefas: iniciar em {} a {} de baixo", (b.x() + b.width() / 2.) as i32, root.height() - (b.y() + b.height() / 2.) as i32);
        // and the first app after the task view
        if let Some(app) = tasks.next_sibling().and_then(|d| d.first_child()).and_then(|i| i.compute_bounds(&root)) {
            eprintln!("debug barra de tarefas: primeiro app em {} a {} de baixo", (app.x() + app.width() / 2.) as i32, root.height() - (app.y() + app.height() / 2.) as i32);
        }
    });
}
