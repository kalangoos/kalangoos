//! Focus mode (design "Modo foco"): the focus timer (the Today bar's ring, its desktop widget, `kalango-shell foco`)
//! becomes a session the whole desktop shows. While it runs the island is the timer ("Foco ▬ 24:10"), the desktop
//! behind the windows dims, notifications are held (a chip at the bottom left counts them) and a click on the island
//! opens "Sessão de foco": the ring, what the session is about (the window in focus when it began), its rules —
//! holding notifications, agents going on in the background (or paused until the end), a summary at the end — and
//! Pausar / Encerrar. At the end, the summary: how long, what was held, what the agents did.

use crate::conversa::ui_icon;
use crate::superficies::{caixa, camada, rotulo};
use gtk::prelude::*;
use gtk::{glib, Align, Orientation};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kalango_i18n::{tr, trf};
use kalango_movimento::{Animavel, Canal, ELASTICA, SUAVE};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

fn agora() -> i64 {
    glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0)
}

/// The session's rules (the panel's switches).
struct Rules {
    hold: Cell<bool>,
    agents_go_on: Cell<bool>,
    summary: Cell<bool>,
}

struct Panel {
    window: gtk::ApplicationWindow,
    motion: Animavel,
    ring: gtk::DrawingArea,
    time: gtk::Label,
    of: gtk::Label,
    name: gtk::Label,
    held: gtk::Label,
    pause: gtk::Button,
}

struct Session {
    active: Cell<bool>,
    started: Cell<i64>,
    name: RefCell<String>,
    rules: Rules,
    agents_paused: Cell<bool>,
    island: RefCell<Option<(glib::WeakRef<gtk::Stack>, gtk::Label, gtk::DrawingArea, gtk::Image, Animavel)>>,
    panel: RefCell<Option<Rc<Panel>>>,
    dim: RefCell<Option<(gtk::ApplicationWindow, Animavel)>>,
    chip: RefCell<Option<(gtk::ApplicationWindow, gtk::Label)>>,
    held_seen: Cell<usize>,
}

thread_local! {
    static SESSION: Rc<Session> = Rc::new(Session {
        active: Cell::new(false), started: Cell::new(0), name: RefCell::default(),
        rules: Rules { hold: Cell::new(true), agents_go_on: Cell::new(true), summary: Cell::new(true) },
        agents_paused: Cell::new(false), island: RefCell::default(), panel: RefCell::default(), dim: RefCell::default(), chip: RefCell::default(),
        held_seen: Cell::new(0),
    });
}

fn session() -> Rc<Session> {
    SESSION.with(Rc::clone)
}

/// The timer as the Today bar keeps it: seconds left, total, running.
fn timer() -> Option<(u32, u32, bool)> {
    let s = crate::shell_atual()?;
    let (r, t, run, _) = s.today_bar().foco_estado();
    Some((r, t, run))
}

/// Whether notifications are held now (a session with "Guardar notificações" on).
pub fn holding() -> bool {
    let s = session();
    s.active.get() && s.rules.hold.get()
}

/// The notices that arrived since the session began (held: not read yet).
fn held_count() -> usize {
    let s = session();
    let Some(sh) = crate::shell_atual() else { return 0 };
    let desde = s.started.get();
    let n = sh.notificacoes();
    let h = n.historico.borrow();
    h.iter().filter(|g| g.quando >= desde && !g.lida).count()
}

/// The island while a session runs: the clock, its bar and the time left; a click opens the panel.
pub fn island(centro: &gtk::Stack) {
    let b = gtk::Button::new();
    b.add_css_class("ilha");
    b.add_css_class("foco");
    b.set_valign(Align::Center);
    let r = caixa(Orientation::Horizontal, 8, "");
    let icon = ui_icon("relogio", "#B9BAC4", 14);
    r.append(&icon);
    r.append(&rotulo(tr("Focus"), "data"));
    let bar = gtk::DrawingArea::new();
    bar.set_content_width(60);
    bar.set_content_height(4);
    bar.set_valign(Align::Center);
    bar.set_draw_func(|_, cr, w, h| {
        let (w, h) = (w as f64, h as f64);
        let f = timer().map(|(r, t, _)| if t == 0 { 0.0 } else { 1.0 - r as f64 / t as f64 }).unwrap_or(0.0);
        let (fr, fg, fb) = crate::tema::rgb("#FFFFFF");
        cr.set_source_rgba(fr, fg, fb, 0.12);
        rounded(cr, 0.0, w, h);
        let _ = cr.fill();
        let (r, g, bl) = crate::tema::rgb("#A5A1E2");
        cr.set_source_rgb(r, g, bl);
        rounded(cr, 0.0, (w * f).max(h), h);
        let _ = cr.fill();
    });
    r.append(&bar);
    let time = rotulo("", "hora");
    r.append(&time);
    let anim = Animavel::new(&r);
    b.set_child(Some(&anim));
    b.connect_clicked(|_| toggle_panel());
    centro.add_named(&b, Some("foco"));
    *session().island.borrow_mut() = Some((centro.downgrade(), time, bar, icon, anim));
}

fn rounded(cr: &gtk::cairo::Context, x: f64, w: f64, h: f64) {
    let r = h / 2.0;
    cr.new_sub_path();
    cr.arc(x + w - r, r, r, -std::f64::consts::FRAC_PI_2, std::f64::consts::FRAC_PI_2);
    cr.arc(x + r, r, r, std::f64::consts::FRAC_PI_2, 3.0 * std::f64::consts::FRAC_PI_2);
    cr.close_path();
}

/// What the island shows when nothing else needs it: the session, or the clock.
pub fn idle_island(c: &gtk::Stack) {
    c.set_visible_child_name(if session().active.get() { "foco" } else { "ilha" });
}

fn mmss(s: u32) -> String {
    format!("{:02}:{:02}", s / 60, s % 60)
}

/// The timer changed (started, paused, reset, ended, or a second went by): the session follows it.
pub fn changed() {
    let s = session();
    let Some((resta, total, rodando)) = timer() else { return };
    if !s.active.get() {
        if rodando {
            begin(&s);
        }
        return;
    }
    if !rodando && resta == 0 {
        end(&s, true);
        return;
    }
    if !rodando && resta == total {
        end(&s, false);
        return;
    }
    refresh(&s, resta, total, rodando);
}

fn begin(s: &Rc<Session>) {
    s.active.set(true);
    s.started.set(agora());
    s.held_seen.set(0);
    // what the session is about: the window in focus
    let nome = crate::shell_atual().and_then(|sh| sh.estado.janelas.borrow().values().find(|j| j.focada).map(|j| j.titulo.clone()))
        .filter(|t| !t.trim().is_empty()).unwrap_or_else(|| tr("Focus session").to_string());
    s.name.replace(nome.clone());
    if debug() {
        eprintln!("debug foco: começou «{nome}»");
    }
    if !s.rules.agents_go_on.get() {
        crate::sistema::pausar_agentes(true);
        s.agents_paused.set(true);
    }
    // the island becomes the timer, with the elastic spring
    if let Some((c, _, _, _, anim)) = s.island.borrow().as_ref() {
        if let Some(c) = c.upgrade() {
            if matches!(c.visible_child_name().as_deref(), Some("ilha") | None) {
                c.set_visible_child_name("foco");
            }
        }
        anim.definir(Canal::Escala, 0.9);
        anim.ir(Canal::Escala, 1.0, ELASTICA);
    }
    dim(true);
    if let Some((r, t, run)) = timer() {
        refresh(s, r, t, run);
    }
}

fn refresh(s: &Rc<Session>, resta: u32, total: u32, rodando: bool) {
    if let Some((_, time, bar, icon, _)) = s.island.borrow().as_ref() {
        time.set_text(&mmss(resta));
        bar.queue_draw();
        crate::conversa::set_icon(icon, if rodando { "relogio" } else { "pausa" }, "#B9BAC4");
    }
    let n = held_count();
    if n != s.held_seen.get() {
        s.held_seen.set(n);
        if debug() && n > 0 {
            eprintln!("debug foco: {n} guardadas");
        }
    }
    chip(s.rules.hold.get() && n > 0, n);
    if let Some(p) = s.panel.borrow().as_ref().filter(|p| p.window.is_visible()) {
        fill(p, s, resta, total, rodando, n);
    }
}

fn end(s: &Rc<Session>, finished: bool) {
    s.active.set(false);
    let minutos = ((agora() - s.started.get()).max(0) + 30) / 60;
    let guardadas = held_count();
    let nome = s.name.borrow().clone();
    if s.agents_paused.replace(false) {
        crate::sistema::pausar_agentes(false);
    }
    dim(false);
    chip(false, 0);
    hide_panel();
    if let Some((c, ..)) = s.island.borrow().as_ref() {
        if let Some(c) = c.upgrade().filter(|c| c.visible_child_name().as_deref() == Some("foco")) {
            c.set_visible_child_name("ilha");
        }
    }
    if debug() {
        eprintln!("debug foco: {} ({minutos} min, {guardadas} guardadas)", if finished { "terminou" } else { "encerrado" });
    }
    // the time is over (not ended by the person): the alarm
    if finished {
        crate::configuracoes_som::tocar("alarm-clock-elapsed");
    }
    if !s.rules.summary.get() {
        // no summary: only that the time is over
        if finished {
            if let Some(sh) = crate::shell_atual() {
                sh.notificar(tr("Focus"), tr("Focus time is over"), tr("Time for a break."));
            }
        }
        return;
    }
    // the summary: how long, what waited, what the agents did meanwhile (kalangod's log)
    let desde = s.started.get().max(0) as u64;
    let s2 = s.clone();
    crate::lateral::em_segundo_plano(move || {
        crate::servicos::kalangod_rpc("kalango/registros", serde_json::json!({"desde": desde, "limite": 1000})).ok()
            .map(|v| v["registros"].as_array().map(|a| a.iter().filter(|r| r["status"] == "ok").count()).unwrap_or(0)).unwrap_or(0)
    }, move |acoes| {
        let _ = &s2;
        // a session that lasted under a minute in which nothing waited and nothing was done: nothing to sum up
        if minutos == 0 && guardadas == 0 && acoes == 0 {
            if debug() {
                eprintln!("debug foco: nada para resumir");
            }
            return;
        }
        let Some(sh) = crate::shell_atual() else { return };
        let mut partes = vec![if minutos == 0 { trf("less than a minute on “{name}”", &[("name", nome.clone())]) } else { trf("{n} min on “{name}”", &[("n", minutos.to_string()), ("name", nome.clone())]) }];
        partes.push(match guardadas { 0 => tr("no notice waited").to_string(), 1 => tr("1 notice waited for you").to_string(), n => trf("{n} notices waited for you", &[("n", n.to_string())]) });
        partes.push(match acoes { 0 => tr("the agents did nothing meanwhile").to_string(), 1 => tr("the agents did 1 thing").to_string(), n => trf("the agents did {n} things", &[("n", n.to_string())]) });
        if debug() {
            eprintln!("debug foco: resumo ({})", partes.join(" · "));
        }
        let sw = Rc::downgrade(&sh);
        sh.notificacoes().notify_local_acoes(tr("Focus"), if finished { tr("Focus session finished") } else { tr("Focus session ended") }, &partes.join(" · "),
            &[("avisos", tr("See notices"))], move |k| {
                if k == "avisos" {
                    if let Some(s) = sw.upgrade() {
                        s.right_bar().abrir(crate::lateral::Modo::Avisos);
                    }
                }
            });
    });
}

/// The desktop behind the windows dims 25 % (a layer under the windows, over the wallpaper and the widgets).
fn dim(on: bool) {
    let s = session();
    if s.dim.borrow().is_none() {
        let Some(app) = gtk::gio::Application::default().and_downcast::<gtk::Application>() else { return };
        let w = camada(&app, "kalango-foco-escurecer", Layer::Bottom, &[Edge::Top, Edge::Bottom, Edge::Left, Edge::Right]);
        w.add_css_class("transparente");
        w.set_exclusive_zone(-1);
        w.set_can_target(false);
        let veu = caixa(Orientation::Vertical, 0, "foco-veu");
        veu.set_hexpand(true);
        veu.set_vexpand(true);
        let anim = Animavel::new(&veu);
        w.set_child(Some(&anim));
        *s.dim.borrow_mut() = Some((w, anim));
    }
    let Some((w, anim)) = s.dim.borrow().clone() else { return };
    if on {
        w.present();
        // the input passes through: it only darkens
        if let Some(surf) = w.surface() {
            surf.set_input_region(&gtk::cairo::Region::create());
        }
        anim.definir(Canal::Opacidade, 0.0).ir(Canal::Opacidade, 1.0, SUAVE);
        if debug() {
            eprintln!("debug foco: a área de trabalho escurece");
        }
    } else {
        let wk = w.downgrade();
        anim.ir(Canal::Opacidade, 0.0, SUAVE);
        anim.ao_parar(move || {
            if let Some(w) = wk.upgrade().filter(|_| !session().active.get()) {
                w.set_visible(false);
            }
        });
    }
}

/// "N notificações guardadas · resumo ao final", at the bottom left.
fn chip(show: bool, n: usize) {
    let s = session();
    if s.chip.borrow().is_none() {
        if !show {
            return;
        }
        let Some(app) = gtk::gio::Application::default().and_downcast::<gtk::Application>() else { return };
        let w = camada(&app, "kalango-foco-guardadas", Layer::Top, &[Edge::Bottom, Edge::Left]);
        w.add_css_class("transparente");
        w.set_resizable(false);
        w.set_margin(Edge::Bottom, 12);
        w.set_margin(Edge::Left, 12);
        let r = caixa(Orientation::Horizontal, 8, "foco-guardadas");
        r.append(&ui_icon("sino-mudo", "#8E8F99", 14));
        let l = rotulo("", "foco-guardadas-texto");
        r.append(&l);
        w.set_child(Some(&r));
        *s.chip.borrow_mut() = Some((w, l));
    }
    let Some((w, l)) = s.chip.borrow().clone() else { return };
    if show {
        l.set_text(&if n == 1 { tr("1 notice held · summary at the end").to_string() } else { trf("{n} notices held · summary at the end", &[("n", n.to_string())]) });
        w.present();
    } else {
        w.set_visible(false);
    }
}

fn switch_row(icon: &str, title: &str, sub: &gtk::Label, on: bool, f: impl Fn(bool) + 'static, debug_name: &str) -> gtk::Box {
    let r = caixa(Orientation::Horizontal, 12, "foco-regra");
    let ic = ui_icon(icon, "#8E8F99", 16);
    ic.set_valign(Align::Center);
    r.append(&ic);
    let c = caixa(Orientation::Vertical, 1, "");
    c.set_hexpand(true);
    c.set_valign(Align::Center);
    let t = rotulo(title, "foco-regra-titulo");
    t.set_xalign(0.0);
    c.append(&t);
    sub.set_xalign(0.0);
    sub.add_css_class("foco-regra-sub");
    c.append(sub);
    r.append(&c);
    let sw = gtk::Switch::new();
    sw.set_active(on);
    sw.set_valign(Align::Center);
    sw.connect_state_set(move |_, v| {
        f(v);
        glib::Propagation::Proceed
    });
    r.append(&sw);
    log_place(debug_name, sw.upcast_ref());
    r
}

fn panel() -> Option<Rc<Panel>> {
    let s = session();
    if let Some(p) = s.panel.borrow().clone() {
        return Some(p);
    }
    let app = gtk::gio::Application::default().and_downcast::<gtk::Application>()?;
    let window = camada(&app, "kalango-foco", Layer::Top, &[Edge::Top]);
    window.add_css_class("transparente");
    window.set_resizable(false);
    window.set_margin(Edge::Top, 8);
    window.set_keyboard_mode(KeyboardMode::OnDemand);
    let body = caixa(Orientation::Vertical, 12, "foco-cartao");
    body.set_size_request(380, -1);
    let cab = caixa(Orientation::Horizontal, 6, "");
    cab.set_halign(Align::Center);
    cab.append(&ui_icon("relogio", "#A5A1E2", 14));
    cab.append(&rotulo(tr("FOCUS SESSION"), "grade-agente-secao"));
    body.append(&cab);
    // the ring empties as the time goes
    let ring = gtk::DrawingArea::new();
    ring.set_content_width(150);
    ring.set_content_height(150);
    ring.set_halign(Align::Center);
    ring.set_draw_func(|_, cr, w, h| {
        let (cx, cy, r) = (w as f64 / 2.0, h as f64 / 2.0, (w.min(h) as f64) / 2.0 - 6.0);
        cr.set_line_width(8.0);
        cr.set_line_cap(gtk::cairo::LineCap::Round);
        let (fr, fg, fb) = crate::tema::rgb("#FFFFFF");
        cr.set_source_rgba(fr, fg, fb, 0.08);
        cr.arc(cx, cy, r, 0.0, std::f64::consts::TAU);
        let _ = cr.stroke();
        let v = timer().map(|(r, t, _)| if t == 0 { 0.0 } else { r as f64 / t as f64 }).unwrap_or(0.0).clamp(0.0, 1.0);
        if v > 0.0 {
            let (r2, g, b) = crate::tema::rgb("#A5A1E2");
            cr.set_source_rgb(r2, g, b);
            let ini = -std::f64::consts::FRAC_PI_2;
            cr.arc(cx, cy, r, ini, ini + v * std::f64::consts::TAU);
            let _ = cr.stroke();
        }
    });
    let sobre = gtk::Overlay::new();
    sobre.set_child(Some(&ring));
    sobre.set_halign(Align::Center);
    let meio = caixa(Orientation::Vertical, 0, "");
    meio.set_halign(Align::Center);
    meio.set_valign(Align::Center);
    let time = rotulo("", "foco-tempo");
    let of = rotulo("", "foco-regra-sub");
    meio.append(&time);
    meio.append(&of);
    sobre.add_overlay(&meio);
    body.append(&sobre);
    let name = rotulo("", "foco-nome");
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    name.set_max_width_chars(36);
    body.append(&name);
    let regras = caixa(Orientation::Vertical, 0, "");
    let held = gtk::Label::new(None);
    regras.append(&switch_row("sino-mudo", tr("Hold notifications"), &held, s.rules.hold.get(), |v| {
        session().rules.hold.set(v);
        if debug() {
            eprintln!("debug foco: guardar notificações {v}");
        }
        changed();
    }, "foco chave guardar"));
    let ag = gtk::Label::new(Some(tr("they only call you to approve something")));
    let agw = ag.downgrade();
    regras.append(&switch_row("spark", tr("Agents go on in the background"), &ag, s.rules.agents_go_on.get(), move |v| {
        let s = session();
        s.rules.agents_go_on.set(v);
        // off: the agents wait until the session ends
        if s.active.get() && v == s.agents_paused.get() {
            crate::sistema::pausar_agentes(!v);
            s.agents_paused.set(!v);
        }
        if let Some(l) = agw.upgrade() {
            l.set_text(if v { tr("they only call you to approve something") } else { tr("paused until the end") });
        }
        if debug() {
            eprintln!("debug foco: agentes continuam {v}");
        }
    }, "foco chave agentes"));
    let res = gtk::Label::new(Some(tr("what arrived and what the agents did")));
    regras.append(&switch_row("doc", tr("Summary at the end"), &res, s.rules.summary.get(), |v| session().rules.summary.set(v), "foco chave resumo"));
    body.append(&regras);
    let acoes = caixa(Orientation::Horizontal, 8, "");
    acoes.set_homogeneous(true);
    let pause = gtk::Button::new();
    pause.add_css_class("foco-pausar");
    pause.connect_clicked(|_| {
        if let Some(sh) = crate::shell_atual() {
            sh.today_bar().foco_alternar();
        }
    });
    let encerrar = gtk::Button::with_label(tr("End"));
    encerrar.add_css_class("foco-encerrar");
    encerrar.connect_clicked(|_| {
        if let Some(sh) = crate::shell_atual() {
            sh.today_bar().foco_zerar();
        }
    });
    acoes.append(&pause);
    acoes.append(&encerrar);
    body.append(&acoes);
    log_place("foco pausar", pause.upcast_ref());
    log_place("foco encerrar", encerrar.upcast_ref());
    let motion = Animavel::new(&body);
    motion.origem(0.5, 0.0);
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
    let p = Rc::new(Panel { window, motion, ring, time, of, name, held, pause });
    *s.panel.borrow_mut() = Some(p.clone());
    Some(p)
}

fn fill(p: &Panel, s: &Session, resta: u32, total: u32, rodando: bool, n: usize) {
    p.time.set_text(&mmss(resta));
    p.of.set_text(&trf("of {n} min", &[("n", (total / 60).to_string())]));
    p.name.set_text(&s.name.borrow());
    p.held.set_text(&match n { 0 => tr("none held yet").to_string(), 1 => tr("1 held so far").to_string(), n => trf("{n} held so far", &[("n", n.to_string())]) });
    let r = caixa(Orientation::Horizontal, 6, "");
    r.set_halign(Align::Center);
    r.append(&ui_icon(if rodando { "pausa" } else { "tocar" }, "#E2E2E6", 12));
    r.append(&gtk::Label::new(Some(if rodando { tr("Pause") } else { tr("Resume") })));
    p.pause.set_child(Some(&r));
    p.ring.queue_draw();
}

thread_local! {
    /// The panel's controls the session test clicks (measured each time the panel opens).
    static PLACES: RefCell<Vec<(String, glib::WeakRef<gtk::Widget>)>> = const { RefCell::new(Vec::new()) };
}

/// For the session test: where a control of the panel is on the screen (centred under the bar).
fn log_place(name: &str, w: &gtk::Widget) {
    if debug() {
        PLACES.with(|p| p.borrow_mut().push((name.to_string(), w.downgrade())));
    }
}

fn log_places() {
    if !debug() {
        return;
    }
    glib::timeout_add_local_once(Duration::from_millis(450), || {
        let Some(p) = session().panel.borrow().clone() else { return };
        let Some((largura, _)) = crate::superficies::tamanho_da_saida() else { return };
        PLACES.with(|l| {
            for (name, w) in l.borrow().iter() {
                if let Some(r) = w.upgrade().and_then(|w| w.compute_bounds(&p.window)) {
                    let x = (largura - p.window.width()) / 2 + (r.x() + r.width() / 2.) as i32;
                    let y = crate::superficies::BARRA_ALTURA + 8 + (r.y() + r.height() / 2.) as i32;
                    eprintln!("debug foco: {name} em {x} {y}");
                }
            }
        });
    });
}

pub fn toggle_panel() {
    let Some(p) = panel() else { return };
    if p.window.is_visible() {
        hide_panel();
        return;
    }
    let s = session();
    let Some((r, t, run)) = timer() else { return };
    fill(&p, &s, r, t, run, held_count());
    p.window.present();
    p.motion.entrar((0.0, -8.0), 0.97);
    log_places();
    if debug() {
        eprintln!("debug foco: painel aberto");
    }
}

fn hide_panel() {
    let Some(p) = session().panel.borrow().clone() else { return };
    if !p.window.is_visible() {
        return;
    }
    let w = p.window.downgrade();
    p.motion.sair((0.0, -8.0), 0.97, move || {
        if let Some(w) = w.upgrade() {
            w.set_visible(false);
        }
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_time_reads_as_minutes_and_seconds() {
        assert_eq!(super::mmss(24 * 60 + 10), "24:10");
        assert_eq!(super::mmss(5), "00:05");
    }
}
