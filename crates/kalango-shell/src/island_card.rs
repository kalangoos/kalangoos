//! The expanded island (design "Ilha expandida (atividades ao vivo)"): the island is the system's live surface. A
//! click on it — or the pointer resting on it, unless Settings › Appearance › Dock and multitasking turned that off —
//! grows it into a card with what is happening now: the agent's task on top (its progress, its last steps, Pausar,
//! Ver na linha do tempo, Parar) and "Também acontecendo" under it (the copy running, the music playing, the focus
//! session, a screen recording). Esc or a click outside folds it back. With nothing happening, a click on the island
//! opens the notices, as before.

use crate::conversa::ui_icon;
use crate::superficies::{caixa, camada, estrela, rotulo};
use gtk::prelude::*;
use gtk::{glib, Align, Orientation};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kalango_i18n::{tr, trf};
use kalango_movimento::{cascata, Animavel};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

/// Whether resting the pointer on the island expands it (Settings › Appearance › Dock and multitasking).
pub fn expands_on_hover() -> bool {
    crate::configuracoes::ajustes()["ilha_expandir_ao_passar"].as_bool().unwrap_or(true)
}

/// What is happening now, besides the agent: one line each.
enum Other {
    Copy(Rc<crate::operacao::Operacao>),
    Music(crate::midia::Midia),
    Focus(u32),
    Recording,
}

/// Whether anything is happening (the island then expands instead of opening the notices).
pub fn something_happening() -> bool {
    // (Settings › Appearance › "Atividades ao vivo na ilha" off: the island is only the clock)
    if !crate::notification_policy::read().in_island {
        return false;
    }
    let Some(s) = crate::shell_atual() else { return false };
    s.estado.agent_task.borrow().is_some() || !others(&s).is_empty()
}

fn others(s: &crate::Shell) -> Vec<Other> {
    let mut v = Vec::new();
    if let Some(op) = s.operacoes().and_then(|o| o.atual()) {
        v.push(Other::Copy(op));
    }
    if let Some(m) = s.midia.atual().filter(|m| !m.titulo.is_empty()) {
        v.push(Other::Music(m));
    }
    let (resta, _, rodando, _) = s.today_bar().foco_estado();
    if rodando {
        v.push(Other::Focus(resta));
    }
    if crate::screenshot_tool::recording() {
        v.push(Other::Recording);
    }
    v
}

/// What the card's structure depends on: the activities, the agent's steps and its pause, the song — not the
/// numbers that move every second (those are updated in place, so a button is never swapped under the pointer).
fn signature(s: &crate::Shell) -> String {
    let mut v = String::new();
    if let Some((p, _)) = s.estado.agent_task.borrow().as_ref() {
        let conv = s.right_bar().conversa.clone();
        v.push_str(&format!("A{p}|{}|", conv.work().is_some_and(|(_, _, p)| p)));
        for (m, t, _) in conv.passos(3) {
            v.push_str(&format!("{m}{t}|"));
        }
        v.push_str(&format!("{}|", s.usando().and_then(|u| u.progresso()).is_some()));
    }
    for o in others(s) {
        match o {
            Other::Copy(op) => v.push_str(&format!("C{}|", op.titulo())),
            Other::Music(m) => v.push_str(&format!("M{}{}{}|", m.titulo, m.artista, m.tocando)),
            Other::Focus(_) => v += "F|",
            Other::Recording => v += "R|",
        }
    }
    v
}

type Update = Box<dyn Fn()>;

struct Card {
    window: gtk::ApplicationWindow,
    motion: Animavel,
    body: gtk::Box,
    by_hover: Cell<bool>,
    inside: Cell<bool>,
    ticking: Cell<bool>,
    shown: RefCell<String>,
    updates: RefCell<Vec<Update>>,
}

thread_local! {
    static CARD: RefCell<Option<Rc<Card>>> = const { RefCell::new(None) };
}

fn card() -> Option<Rc<Card>> {
    if let Some(c) = CARD.with(|c| c.borrow().clone()) {
        return Some(c);
    }
    let app = gtk::gio::Application::default().and_downcast::<gtk::Application>()?;
    // over the whole screen (to see a click outside), the card at the top in the middle, over the bar: it grows
    // from the island itself
    let window = camada(&app, "kalango-ilha-expandida", Layer::Overlay, &[Edge::Top, Edge::Bottom, Edge::Left, Edge::Right]);
    window.add_css_class("transparente");
    window.set_exclusive_zone(-1);
    window.set_keyboard_mode(KeyboardMode::OnDemand);
    let body = caixa(Orientation::Vertical, 10, "ilha-cartao");
    body.set_size_request(560, -1);
    let motion = Animavel::new(&body);
    motion.origem(0.5, 0.0);
    motion.set_halign(Align::Center);
    motion.set_valign(Align::Start);
    motion.set_margin_top(4);
    window.set_child(Some(&motion));
    let c = Rc::new(Card { window, motion, body, by_hover: Cell::new(false), inside: Cell::new(false), ticking: Cell::new(false),
        shown: RefCell::default(), updates: RefCell::default() });
    let outside = gtk::GestureClick::new();
    let w = Rc::downgrade(&c);
    outside.connect_pressed(move |g, _, x, y| {
        let Some(c) = w.upgrade() else { return };
        let dentro = c.motion.compute_bounds(&c.window).is_some_and(|b| b.contains_point(&gtk::graphene::Point::new(x as f32, y as f32)));
        if !dentro {
            g.set_state(gtk::EventSequenceState::Claimed);
            fold();
        }
    });
    c.window.add_controller(outside);
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(|_, k, _, _| {
        if k == gtk::gdk::Key::Escape {
            fold();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    c.window.add_controller(keys);
    // opened by the pointer: it folds once the pointer has left the card for a moment
    let hover = gtk::EventControllerMotion::new();
    let w = Rc::downgrade(&c);
    hover.connect_enter(move |_, _, _| {
        if let Some(c) = w.upgrade() {
            c.inside.set(true);
        }
    });
    let w = Rc::downgrade(&c);
    hover.connect_leave(move |_| {
        let Some(c) = w.upgrade() else { return };
        c.inside.set(false);
        if c.by_hover.get() {
            let w2 = Rc::downgrade(&c);
            glib::timeout_add_local_once(Duration::from_millis(450), move || {
                if let Some(c) = w2.upgrade().filter(|c| !c.inside.get() && c.by_hover.get()) {
                    let _ = c;
                    fold();
                }
            });
        }
    });
    c.motion.add_controller(hover);
    CARD.with(|x| *x.borrow_mut() = Some(c.clone()));
    Some(c)
}

fn button(glifo: &str, cor: &str, texto: &str, class: &str, f: impl Fn() + 'static) -> gtk::Button {
    let b = gtk::Button::new();
    b.add_css_class(class);
    let r = caixa(Orientation::Horizontal, 7, "");
    if !glifo.is_empty() {
        r.append(&ui_icon(glifo, cor, 14));
    }
    if !texto.is_empty() {
        r.append(&gtk::Label::new(Some(texto)));
    }
    b.set_child(Some(&r));
    b.connect_clicked(move |_| f());
    b
}

fn clear(b: &gtk::Box) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
}

/// The card's content now: the agent's task, then the rest.
fn fill(c: &Card) -> Vec<Animavel> {
    clear(&c.body);
    let mut itens = Vec::new();
    let mut updates: Vec<Update> = Vec::new();
    let Some(s) = crate::shell_atual() else { return itens };
    c.shown.replace(signature(&s));
    let tarefa = s.estado.agent_task.borrow().clone();
    if let Some((pedido, n_passos)) = tarefa {
        let conv = s.right_bar().conversa.clone();
        let sec = caixa(Orientation::Vertical, 10, "");
        let cab = caixa(Orientation::Horizontal, 12, "");
        let tile = caixa(Orientation::Horizontal, 0, "ilha-cartao-icone");
        tile.set_size_request(36, 36);
        // (its own size: the centred icon's expansion must not widen it)
        tile.set_hexpand(false);
        tile.set_valign(Align::Center);
        let e = estrela(16, "#A5A1E2");
        e.set_hexpand(true);
        e.set_halign(Align::Center);
        e.set_valign(Align::Center);
        tile.append(&e);
        cab.append(&tile);
        let tt = caixa(Orientation::Vertical, 2, "");
        tt.set_hexpand(true);
        tt.set_valign(Align::Center);
        let progresso = s.usando().and_then(|u| u.progresso());
        let nome = progresso.as_ref().map(|(q, _, _)| q.clone()).unwrap_or_else(|| tr("Agent").to_string());
        let t = rotulo(&nome, "ilha-cartao-titulo");
        t.set_xalign(0.0);
        tt.append(&t);
        let sub = rotulo(&format!("{} · {}", pedido.trim(), tr("local model")), "ilha-cartao-sub");
        sub.set_xalign(0.0);
        sub.set_ellipsize(gtk::pango::EllipsizeMode::End);
        sub.set_max_width_chars(1);
        sub.set_hexpand(true);
        tt.append(&sub);
        cab.append(&tt);
        let conta = match &progresso {
            Some((_, feitos, total)) => format!("{feitos} / {total}"),
            None => match n_passos {
                0 => tr("thinking…").to_string(),
                1 => tr("1 step").to_string(),
                n => trf("{n} steps", &[("n", n.to_string())]),
            },
        };
        let cl = rotulo(&conta, "ilha-cartao-conta");
        cl.set_valign(Align::Center);
        cab.append(&cl);
        let clw = cl.downgrade();
        updates.push(Box::new(move || {
            let (Some(l), Some(s)) = (clw.upgrade(), crate::shell_atual()) else { return };
            if let Some((_, f, t)) = s.usando().and_then(|u| u.progresso()) {
                l.set_text(&format!("{f} / {t}"));
            }
        }));
        sec.append(&cab);
        // the bar only with a known size (no made-up share)
        if let Some((_, f, t)) = progresso.as_ref().filter(|(_, _, t)| *t > 0) {
            let barra = gtk::LevelBar::for_interval(0.0, 1.0);
            barra.add_css_class("ilha-cartao-barra");
            barra.set_value((*f as f64 / *t as f64).min(1.0));
            sec.append(&barra);
            let bw = barra.downgrade();
            updates.push(Box::new(move || {
                let (Some(b), Some(s)) = (bw.upgrade(), crate::shell_atual()) else { return };
                if let Some((_, f, t)) = s.usando().and_then(|u| u.progresso()).filter(|(_, _, t)| *t > 0) {
                    b.set_value((f as f64 / t as f64).min(1.0));
                }
            }));
        }
        // its last steps
        let passos = caixa(Orientation::Vertical, 4, "");
        for (marca, texto, _) in conv.passos(3) {
            let r = caixa(Orientation::Horizontal, 8, "");
            let ic: gtk::Widget = match marca {
                '✓' => ui_icon("check", "#8ED2C3", 14).upcast(),
                '✗' => ui_icon("fechar", "#EFA2A6", 14).upcast(),
                '!' => ui_icon("alerta", "#E5C98C", 14).upcast(),
                _ => {
                    let sp = gtk::Spinner::new();
                    sp.add_css_class("desligando-anel");
                    sp.set_size_request(12, 12);
                    sp.start();
                    sp.upcast()
                }
            };
            r.append(&ic);
            let l = rotulo(&texto, "ilha-cartao-passo");
            l.set_xalign(0.0);
            l.set_ellipsize(gtk::pango::EllipsizeMode::End);
            l.set_max_width_chars(1);
            l.set_hexpand(true);
            r.append(&l);
            passos.append(&r);
        }
        sec.append(&passos);
        let acoes = caixa(Orientation::Horizontal, 8, "");
        let pausado = conv.work().is_some_and(|(_, _, p)| p);
        let cv = conv.clone();
        let pausar = button(if pausado { "tocar" } else { "pausa" }, "#E2E2E6", if pausado { tr("Resume") } else { tr("Pause") }, "ilha-cartao-botao", move || {
            let p = cv.work().is_some_and(|(_, _, p)| p);
            cv.pause(!p);
            if debug() {
                eprintln!("debug ilha: agente {}", if p { "retomado" } else { "pausado" });
            }
            refresh();
        });
        acoes.append(&pausar);
        let ver = button("relogio", "#A5A1E2", tr("See in the timeline"), "ilha-cartao-linha", || {
            fold();
            if let Some(s) = crate::shell_atual() {
                s.abrir_central(Some("linha"));
            }
        });
        acoes.append(&ver);
        let esp = gtk::Box::new(Orientation::Horizontal, 0);
        esp.set_hexpand(true);
        acoes.append(&esp);
        let cv = conv.clone();
        let parar = button("parar", "#EFA2A6", tr("Stop"), "ilha-cartao-parar", move || {
            cv.stop(true);
            if debug() {
                eprintln!("debug ilha: agente parado");
            }
            fold();
        });
        acoes.append(&parar);
        if debug() {
            crate::central::posicao_debug("ilha cartao pausar", pausar.upcast_ref());
            crate::central::posicao_debug("ilha cartao parar", parar.upcast_ref());
        }
        sec.append(&acoes);
        let a = Animavel::new(&sec);
        c.body.append(&a);
        itens.push(a);
    }
    let resto = others(&s);
    if !resto.is_empty() {
        let cab = rotulo(if itens.is_empty() { tr("HAPPENING NOW") } else { tr("ALSO HAPPENING") }, "grade-agente-secao");
        cab.set_xalign(0.0);
        if !itens.is_empty() {
            let sep = caixa(Orientation::Horizontal, 0, "ilha-cartao-separador");
            c.body.append(&sep);
        }
        let a = Animavel::new(&cab);
        c.body.append(&a);
        itens.push(a);
    }
    for o in resto.into_iter().take(3) {
        let r = caixa(Orientation::Horizontal, 12, "ilha-cartao-linha-atividade");
        match o {
            Other::Copy(op) => {
                let tile = caixa(Orientation::Horizontal, 0, "ilha-cartao-mini");
                tile.set_size_request(32, 32);
                tile.set_hexpand(false);
                tile.set_valign(Align::Center);
                let ic = ui_icon("download", "#8ED2C3", 16);
                ic.set_hexpand(true);
                tile.append(&ic);
                r.append(&tile);
                let col = caixa(Orientation::Vertical, 6, "");
                col.set_hexpand(true);
                col.set_valign(Align::Center);
                let (v, falta) = op.ritmo();
                let texto = |op: &crate::operacao::Operacao, v: f64| format!("{} · {:.0}% · {}/s", op.titulo(), op.fracao() * 100.0, crate::arquivos::tamanho(v as u64));
                let t = rotulo(&texto(&op, v), "ilha-cartao-item");
                t.set_xalign(0.0);
                t.set_ellipsize(gtk::pango::EllipsizeMode::End);
                t.set_max_width_chars(1);
                col.append(&t);
                let b = gtk::LevelBar::for_interval(0.0, 1.0);
                b.add_css_class("ilha-cartao-barra");
                b.set_value(op.fracao());
                col.append(&b);
                r.append(&col);
                let l = rotulo(&falta.map(crate::operacao::tempo).unwrap_or_default(), "ilha-cartao-sub");
                l.set_valign(Align::Center);
                r.append(&l);
                let (tw, bw, lw, opw) = (t.downgrade(), b.downgrade(), l.downgrade(), Rc::downgrade(&op));
                updates.push(Box::new(move || {
                    let (Some(t), Some(b), Some(l), Some(op)) = (tw.upgrade(), bw.upgrade(), lw.upgrade(), opw.upgrade()) else { return };
                    let (v, falta) = op.ritmo();
                    t.set_text(&texto(&op, v));
                    b.set_value(op.fracao());
                    l.set_text(&falta.map(crate::operacao::tempo).unwrap_or_default());
                }));
                let g = gtk::GestureClick::new();
                g.connect_released(|_, _, _, _| {
                    fold();
                    if let Some(o) = crate::shell_atual().and_then(|s| s.operacoes()) {
                        o.expandir();
                    }
                });
                r.add_controller(g);
            }
            Other::Music(m) => {
                let capa = crate::midia::capa(&m, 32);
                capa.set_valign(Align::Center);
                r.append(&capa);
                let col = caixa(Orientation::Vertical, 1, "");
                col.set_hexpand(true);
                col.set_valign(Align::Center);
                let t = rotulo(&m.titulo, "ilha-cartao-item");
                t.set_xalign(0.0);
                t.set_ellipsize(gtk::pango::EllipsizeMode::End);
                t.set_max_width_chars(1);
                col.append(&t);
                let a = rotulo(&m.artista, "ilha-cartao-sub");
                a.set_xalign(0.0);
                col.append(&a);
                r.append(&col);
                for (g, metodo) in [("anterior", "Previous"), (if m.tocando { "pausa" } else { "tocar" }, "PlayPause"), ("proxima", "Next")] {
                    let b = button(g, "#E2E2E6", "", "ilha-cartao-midia", move || {
                        if let Some(s) = crate::shell_atual() {
                            s.midia.comando(metodo);
                        }
                        glib::timeout_add_local_once(Duration::from_millis(300), refresh);
                    });
                    b.set_valign(Align::Center);
                    if debug() {
                        crate::central::posicao_debug(&format!("ilha cartao {metodo}"), b.upcast_ref());
                    }
                    r.append(&b);
                }
            }
            Other::Focus(resta) => {
                let tile = caixa(Orientation::Horizontal, 0, "ilha-cartao-mini");
                tile.set_size_request(32, 32);
                tile.set_hexpand(false);
                tile.set_valign(Align::Center);
                let ic = ui_icon("relogio", "#A5A1E2", 16);
                ic.set_hexpand(true);
                tile.append(&ic);
                r.append(&tile);
                let t = rotulo(tr("Focus session"), "ilha-cartao-item");
                t.set_xalign(0.0);
                t.set_hexpand(true);
                r.append(&t);
                let tl = rotulo(&format!("{:02}:{:02}", resta / 60, resta % 60), "ilha-cartao-conta");
                r.append(&tl);
                let tw = tl.downgrade();
                updates.push(Box::new(move || {
                    let (Some(l), Some(s)) = (tw.upgrade(), crate::shell_atual()) else { return };
                    let r = s.today_bar().foco_estado().0;
                    l.set_text(&format!("{:02}:{:02}", r / 60, r % 60));
                }));
                let g = gtk::GestureClick::new();
                g.connect_released(|_, _, _, _| {
                    fold();
                    crate::focus_session::toggle_panel();
                });
                r.add_controller(g);
            }
            Other::Recording => {
                let tile = caixa(Orientation::Horizontal, 0, "ilha-cartao-mini");
                tile.set_size_request(32, 32);
                tile.set_hexpand(false);
                tile.set_valign(Align::Center);
                let ponto = caixa(Orientation::Horizontal, 0, "gravando-ponto");
                ponto.set_size_request(8, 8);
                ponto.set_hexpand(true);
                ponto.set_halign(Align::Center);
                ponto.set_valign(Align::Center);
                tile.append(&ponto);
                r.append(&tile);
                let t = rotulo(tr("Recording the screen"), "ilha-cartao-item");
                t.set_xalign(0.0);
                t.set_hexpand(true);
                r.append(&t);
                r.append(&button("parar", "#EFA2A6", tr("Stop"), "ilha-cartao-parar", || {
                    fold();
                    crate::screenshot_tool::stop_recording();
                }));
            }
        }
        let a = Animavel::new(&r);
        c.body.append(&a);
        itens.push(a);
    }
    *c.updates.borrow_mut() = updates;
    itens
}

/// Keeps the card current: rebuilt only when what it shows changed, its numbers updated in place otherwise.
fn keep_current(c: &Card) {
    let Some(s) = crate::shell_atual() else { return };
    if *c.shown.borrow() != signature(&s) {
        fill(c);
    } else {
        for u in c.updates.borrow().iter() {
            u();
        }
    }
}

fn refresh() {
    if let Some(c) = CARD.with(|c| c.borrow().clone()).filter(|c| c.window.is_visible()) {
        keep_current(&c);
    }
}

/// Grows the island into the card (`by_hover`: opened by the pointer resting on it, it folds when the pointer goes).
pub fn expand(by_hover: bool) -> bool {
    if !something_happening() {
        return false;
    }
    let Some(c) = card() else { return false };
    if c.window.is_visible() {
        if !by_hover {
            c.by_hover.set(false);
        }
        return true;
    }
    c.by_hover.set(by_hover);
    let itens = fill(&c);
    c.window.present();
    // from the island's own shape: small and close to it, with the elastic spring; the content in a cascade
    c.motion.entrar((0.0, -10.0), 0.55);
    cascata(&itens, 6.0);
    if debug() {
        eprintln!("debug ilha: expandida ({} atividades{})", itens.len(), if by_hover { ", pelo ponteiro" } else { "" });
    }
    // what it shows keeps up while it is open
    if !c.ticking.replace(true) {
        glib::timeout_add_local(Duration::from_millis(1000), || {
            let Some(c) = CARD.with(|c| c.borrow().clone()) else { return glib::ControlFlow::Break };
            if !c.window.is_visible() {
                c.ticking.set(false);
                return glib::ControlFlow::Break;
            }
            if !something_happening() {
                fold();
            } else {
                keep_current(&c);
            }
            glib::ControlFlow::Continue
        });
    }
    true
}

/// Back into the island.
pub fn fold() {
    let Some(c) = CARD.with(|c| c.borrow().clone()) else { return };
    if !c.window.is_visible() {
        return;
    }
    c.by_hover.set(false);
    let w = c.window.downgrade();
    c.motion.sair((0.0, -10.0), 0.55, move || {
        if let Some(w) = w.upgrade() {
            w.set_visible(false);
        }
    });
    if debug() {
        eprintln!("debug ilha: recolhida");
    }
}

/// The island's own behaviour: a click expands it (or opens the notices with nothing happening); the pointer
/// resting on it a moment expands it too.
pub fn attach(ilha: &gtk::Button, shell: &Rc<crate::Shell>) {
    let s = Rc::downgrade(shell);
    ilha.connect_clicked(move |_| {
        if !expand(false) {
            if let Some(s) = s.upgrade() {
                s.right_bar().alternar(crate::lateral::Modo::Avisos);
            }
        }
    });
    let hover = gtk::EventControllerMotion::new();
    let espera: Rc<RefCell<Option<glib::SourceId>>> = Rc::default();
    let e2 = espera.clone();
    hover.connect_enter(move |_, _, _| {
        if !expands_on_hover() {
            return;
        }
        let e3 = e2.clone();
        *e2.borrow_mut() = Some(glib::timeout_add_local_once(Duration::from_millis(400), move || {
            e3.borrow_mut().take();
            expand(true);
        }));
    });
    hover.connect_leave(move |_| {
        if let Some(id) = espera.borrow_mut().take() {
            id.remove();
        }
    });
    ilha.add_controller(hover);
    if debug() {
        crate::central::posicao_debug("ilha", ilha.upcast_ref());
    }
}
