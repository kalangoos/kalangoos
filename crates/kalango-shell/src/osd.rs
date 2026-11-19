//! On-screen indicators (design "Indicadores na tela (volume, brilho, teclas)"): a card above the dock, centred, for
//! what a key changed — the volume and the brightness (a bar and the percentage), the microphone, the keyboard's
//! layout (niri's own switch, from its event stream), airplane mode and the power mode. It updates in place while the
//! key is held and fades (180 ms) 1.2 s after the last change. Caps Lock is not shown: the compositor does not tell
//! clients (it left the design).
//!
//! The media keys run `kalango-shell osd volume +5` (and so on): the change itself (wpctl, brightnessctl) and its card.

use crate::conversa::ui_icon;
use crate::superficies::{caixa, camada, rotulo};
use gtk::prelude::*;
use gtk::{glib, Align, Orientation};
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use kalango_i18n::{tr, trf};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

struct Osd {
    janela: gtk::ApplicationWindow,
    icone: gtk::Box,
    texto: gtk::Label,
    barra: gtk::DrawingArea,
    valor: Rc<Cell<f64>>,
    pct: gtk::Label,
    detalhe: gtk::Label,
    some: RefCell<Option<glib::SourceId>>,
}

thread_local! {
    static OSD: RefCell<Option<Rc<Osd>>> = const { RefCell::new(None) };
    /// niri's keyboard layouts (names) and the one in use, for "this → the next"
    static LAYOUTS: RefCell<(Vec<String>, usize)> = const { RefCell::new((Vec::new(), 0)) };
}

fn osd() -> Option<Rc<Osd>> {
    if let Some(o) = OSD.with(|o| o.borrow().clone()) {
        return Some(o);
    }
    let app = gtk::gio::Application::default().and_downcast::<gtk::Application>()?;
    let janela = camada(&app, "kalango-indicador", Layer::Overlay, &[Edge::Bottom]);
    janela.add_css_class("transparente");
    janela.set_resizable(false);
    // above the dock (its 54 px and the 12 px gap)
    janela.set_margin(Edge::Bottom, 84);
    janela.connect_realize(|w| {
        if let Some(s) = w.surface() {
            s.set_input_region(&gtk::cairo::Region::create());
        }
    });
    let card = caixa(Orientation::Horizontal, 12, "osd-cartao");
    card.set_size_request(440, -1);
    let icone = caixa(Orientation::Horizontal, 0, "");
    icone.set_valign(Align::Center);
    card.append(&icone);
    let texto = rotulo("", "osd-texto");
    texto.set_xalign(0.0);
    card.append(&texto);
    let valor: Rc<Cell<f64>> = Rc::default();
    let barra = gtk::DrawingArea::new();
    barra.set_content_height(6);
    barra.set_hexpand(true);
    barra.set_valign(Align::Center);
    let v2 = valor.clone();
    barra.set_draw_func(move |_, cr, w, h| {
        let (w, h) = (w as f64, h as f64);
        // a pill: the track (white at 12%) and the level in the accent
        let pilula = |cr: &gtk::cairo::Context, largura: f64| {
            let r = h / 2.0;
            cr.new_sub_path();
            cr.arc(largura - r, r, r, -std::f64::consts::FRAC_PI_2, std::f64::consts::FRAC_PI_2);
            cr.arc(r, r, r, std::f64::consts::FRAC_PI_2, 3.0 * std::f64::consts::FRAC_PI_2);
            cr.close_path();
        };
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.12);
        pilula(cr, w);
        let _ = cr.fill();
        let (r, g, b) = crate::tema::rgb(&crate::tema::cor("#A5A1E2"));
        cr.set_source_rgb(r, g, b);
        let nivel = (w * v2.get().clamp(0.0, 1.0)).max(h);
        pilula(cr, nivel);
        let _ = cr.fill();
    });
    card.append(&barra);
    let pct = rotulo("", "osd-pct");
    card.append(&pct);
    let detalhe = rotulo("", "osd-detalhe");
    detalhe.set_hexpand(true);
    detalhe.set_xalign(1.0);
    card.append(&detalhe);
    janela.set_child(Some(&card));
    let o = Rc::new(Osd { janela, icone, texto, barra, valor, pct, detalhe, some: RefCell::default() });
    OSD.with(|x| *x.borrow_mut() = Some(o.clone()));
    Some(o)
}

/// Shows a card: its glyph, a text or a bar with the percentage, and a detail at the right.
fn mostrar(glifo: &str, cor: &str, texto: &str, nivel: Option<f64>, detalhe: &str) {
    let Some(o) = osd() else { return };
    while let Some(c) = o.icone.first_child() {
        o.icone.remove(&c);
    }
    o.icone.append(&ui_icon(glifo, cor, 16));
    o.texto.set_text(texto);
    o.texto.set_visible(nivel.is_none());
    o.barra.set_visible(nivel.is_some());
    o.pct.set_visible(nivel.is_some());
    if let Some(n) = nivel {
        o.valor.set(n);
        o.barra.queue_draw();
        o.pct.set_text(&format!("{:.0}%", n * 100.0));
    }
    o.detalhe.set_text(detalhe);
    o.detalhe.set_visible(!detalhe.is_empty());
    o.janela.set_opacity(1.0);
    o.janela.present();
    if let Some(t) = o.some.borrow_mut().take() {
        t.remove();
    }
    let ow = Rc::downgrade(&o);
    *o.some.borrow_mut() = Some(glib::timeout_add_local_once(Duration::from_millis(1200), move || {
        let Some(o) = ow.upgrade() else { return };
        o.some.borrow_mut().take();
        // a 180 ms fade
        let t0 = std::time::Instant::now();
        let ow = Rc::downgrade(&o);
        glib::timeout_add_local(Duration::from_millis(16), move || {
            let Some(o) = ow.upgrade() else { return glib::ControlFlow::Break };
            if o.some.borrow().is_some() {
                o.janela.set_opacity(1.0);
                return glib::ControlFlow::Break;
            }
            let k = t0.elapsed().as_secs_f64() / 0.18;
            if k >= 1.0 {
                o.janela.set_visible(false);
                return glib::ControlFlow::Break;
            }
            o.janela.set_opacity(1.0 - k);
            glib::ControlFlow::Continue
        });
    }));
    if debug() {
        eprintln!("debug osd: {texto}{}{}", nivel.map(|n| format!(" {:.0}%", n * 100.0)).unwrap_or_default(), if detalhe.is_empty() { String::new() } else { format!(" ({detalhe})") });
    }
}

/// wpctl's "Volume: 0.62 [MUTED]" of a node.
fn ler_volume(no: &str) -> Option<(f64, bool)> {
    let o = std::process::Command::new(crate::configuracoes_som::comando("wpctl")).args(["get-volume", no]).output().ok()?;
    let t = String::from_utf8_lossy(&o.stdout);
    let v: f64 = t.split_whitespace().nth(1)?.parse().ok()?;
    Some((v, t.contains("MUTED")))
}

fn wpctl(args: &[&str]) {
    let _ = std::process::Command::new(crate::configuracoes_som::comando("wpctl")).args(args).status();
}

/// `kalango-shell osd …`: the key's change, then its card.
pub fn comando(args: &[String]) {
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    match a.as_slice() {
        ["volume", passo] => {
            match *passo {
                "mudo" => wpctl(&["set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"]),
                p => {
                    let (sinal, n) = p.split_at(1);
                    wpctl(&["set-volume", "-l", "1.0", "@DEFAULT_AUDIO_SINK@", &format!("{}%{}", n, if sinal == "-" { "-" } else { "+" })]);
                }
            }
            if let Some((v, mudo)) = ler_volume("@DEFAULT_AUDIO_SINK@") {
                if mudo {
                    mostrar("volume-mudo", "#8E8F99", tr("Sound muted"), None, "");
                } else {
                    mostrar(if v < 0.34 { "volume-baixo" } else { "volume" }, "#B9BAC4", tr("Volume"), Some(v), "");
                }
            }
        }
        ["microfone"] => {
            wpctl(&["set-mute", "@DEFAULT_AUDIO_SOURCE@", "toggle"]);
            let mudo = ler_volume("@DEFAULT_AUDIO_SOURCE@").is_none_or(|(_, m)| m);
            mostrar(if mudo { "microfone-d" } else { "microfone" }, if mudo { "#EFA2A6" } else { "#8ED2C3" }, if mudo { tr("Microphone off") } else { tr("Microphone on") }, None, "");
        }
        ["brilho", passo] => {
            let (sinal, n) = passo.split_at(1);
            let ok = std::process::Command::new("brightnessctl").args(["-q", "set", &format!("{n}%{sinal}")]).status().is_ok_and(|s| s.success());
            let atual = std::process::Command::new("brightnessctl").args(["-m", "info"]).output().ok()
                .and_then(|o| String::from_utf8_lossy(&o.stdout).split(',').nth(3).and_then(|p| p.trim_end_matches('%').parse::<f64>().ok()));
            match (ok, atual) {
                (true, Some(p)) => mostrar("sol", "#E5C98C", tr("Brightness"), Some(p / 100.0), ""),
                _ => mostrar("sol", "#E5C98C", tr("This screen's brightness is set on the screen itself"), None, ""),
            }
        }
        _ => eprintln!("kalango-shell: osd: volume +5|-5|mudo, brilho +5|-5, microfone"),
    }
}

/// niri's layouts (its event "KeyboardLayoutsChanged").
pub fn layouts(nomes: Vec<String>, atual: usize) {
    LAYOUTS.with(|l| *l.borrow_mut() = (nomes, atual));
}

/// niri switched the layout: "Português (Brasil)", and which one comes next.
pub fn layout_trocado(idx: usize) {
    let (nomes, antes) = LAYOUTS.with(|l| l.borrow().clone());
    LAYOUTS.with(|l| l.borrow_mut().1 = idx);
    if idx == antes || nomes.len() < 2 {
        return;
    }
    let Some(nome) = nomes.get(idx) else { return };
    let proximo = nomes.get((idx + 1) % nomes.len()).cloned().unwrap_or_default();
    mostrar("teclado", "#B9BAC4", nome, None, &format!("→ {proximo}"));
}

/// The power mode chosen by a key or by the agent.
pub fn modo_de_energia(nome: &str) {
    mostrar("raio", "#EFA2A6", nome, None, "");
}

/// A Bluetooth device connected or gone (design "Indicadores na tela", DISPOSITIVO): its kind's glyph (BlueZ's
/// icon name), its name and, when it says, its battery.
pub fn dispositivo(nome: &str, icone: &str, conectado: bool, bateria: Option<u8>) {
    let glifo = match icone {
        "audio-headphones" | "audio-headset" | "audio-card" => "fone",
        "input-mouse" | "input-tablet" => "mouse",
        "input-keyboard" => "teclado",
        "phone" => "celular",
        _ => "bluetooth",
    };
    let texto = if conectado { trf("{name} connected", &[("name", nome.to_string())]) } else { trf("{name} disconnected", &[("name", nome.to_string())]) };
    mostrar(glifo, "#7CC3DF", &texto, None, &bateria.filter(|_| conectado).map(|b| format!("{b}%")).unwrap_or_default());
}

/// Airplane mode on or off.
pub fn modo_aviao(on: bool) {
    mostrar("aviao", "#A5A1E2", if on { tr("Airplane mode on") } else { tr("Airplane mode off") }, None, "");
}
