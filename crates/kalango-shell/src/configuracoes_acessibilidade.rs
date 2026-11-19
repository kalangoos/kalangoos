//! Settings › Accessibility (design "Configurações · Acessibilidade"): what the system really does for seeing, hearing
//! and typing, each one at once.
//!
//! - "Descrever a tela" (Super+Alt+D): the local vision model looks at the screen and the voice reads what is there.
//! - Seeing: high contrast and the text's size (the theme's own, as Appearance), reduced motion (springs become a
//!   90 ms fade: GTK's animations off for the shell and the apps, niri's off), the screen reader (Orca).
//! - Pointer: its size (niri's cursor, and the apps').
//! - Hearing: live captions of any sound (the local audio model), visual alerts (the screen flashes instead of the
//!   sound), mono audio (a PipeWire mono sink as the default).
//! - Typing and voice: the on-screen keyboard (wvkbd), dictation into any text field (Super+H), control by voice
//!   (holding Super: the same setting as Keyboard › Super key).
//!
//! What the compositor cannot do (a magnifier, colour filters, sticky keys, clicking by dwelling) left the design.
//! Kept in configuracoes.json["acessibilidade"]; niri's part in ~/.config/kalango/acessibilidade.kdl.

use crate::configuracoes::{deslizador, depurando, interruptor, rotulo_cartao};
use crate::configuracoes_moldura::Moldura;
use crate::superficies::{caixa, estrela, rotulo};
use crate::tema;
use gtk::prelude::*;
use gtk::{glib, Align, Orientation};
use kalango_i18n::tr;
use serde_json::{json, Value};
use std::cell::Cell;
use std::rc::{Rc, Weak};

/// The text's sizes of the design's segmented control.
const TEXTOS: &[f64] = &[1.0, 1.25, 1.5, 2.0];

fn left(t: &str, c: &str) -> gtk::Label {
    let l = rotulo(t, c);
    l.set_xalign(0.0);
    l
}

/// A row: the text (and its line under it) on the left, the control on the right.
fn row(title: &str, sub: Option<&str>, control: &impl IsA<gtk::Widget>) -> gtk::Box {
    let r = caixa(Orientation::Horizontal, 12, "cfg-tela-opcao");
    let t = caixa(Orientation::Vertical, 1, "");
    t.set_hexpand(true);
    t.set_valign(Align::Center);
    t.append(&left(title, "cfg-texto-13"));
    if let Some(s) = sub {
        let l = left(s, "cfg-sub");
        l.set_ellipsize(gtk::pango::EllipsizeMode::End);
        l.set_max_width_chars(1);
        l.set_hexpand(true);
        t.append(&l);
        r.add_css_class("alta");
    }
    r.append(&t);
    control.set_valign(Align::Center);
    r.append(control);
    r
}

/// The key caps of a combination.
fn caps(keys: &[&str]) -> gtk::Box {
    let b = caixa(Orientation::Horizontal, 4, "");
    b.set_valign(Align::Center);
    for k in keys {
        b.append(&rotulo(k, "cfg-tecla"));
    }
    b
}

/// The settings of this page.
pub fn ler() -> Value {
    let v = crate::configuracoes::ajustes()["acessibilidade"].clone();
    if v.is_object() { v } else { json!({}) }
}

fn salvar(chave: &str, v: Value) {
    let mut a = ler();
    a[chave] = v;
    crate::configuracoes::salvar_ajuste("acessibilidade", a);
}

pub fn ligado(chave: &str) -> bool {
    ler()[chave].as_bool().unwrap_or(false)
}

/// The development and test sessions never change the person's desktop (their GNOME settings, their screen reader).
fn sessao_de_teste() -> bool {
    std::env::var_os("KALANGO_SHELL_DEV").is_some()
}

/// niri's part: animations and the cursor's size (an optional include of the session's configuration).
fn escrever_niri() {
    let a = ler();
    let mut kdl = String::from("// KalangoOS · Configurações › Acessibilidade (feito pelo shell; não editar à mão)\n");
    if a["reduzir_animacoes"].as_bool().unwrap_or(false) {
        kdl += "animations {\n    off\n}\n";
    }
    if let Some(n) = a["ponteiro"].as_u64() {
        let theme = std::env::var("XCURSOR_THEME").ok().filter(|t| !t.is_empty()).unwrap_or_else(|| "Adwaita".into());
        kdl += &format!("cursor {{\n    xcursor-theme \"{theme}\"\n    xcursor-size {n}\n}}\n");
    }
    let p = glib::user_config_dir().join("kalango/acessibilidade.kdl");
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(&p, kdl);
}

/// The apps' side (GNOME's interface settings), outside the development sessions.
fn gnome(chave: &str, v: glib::Variant) {
    if sessao_de_teste() {
        return;
    }
    let Some(src) = gtk::gio::SettingsSchemaSource::default() else { return };
    for schema in ["org.gnome.desktop.interface", "org.gnome.desktop.a11y.applications", "org.gnome.desktop.a11y.interface"] {
        if src.lookup(schema, true).is_some_and(|s| s.has_key(chave)) {
            let _ = gtk::gio::Settings::new(schema).set_value(chave, &v);
            return;
        }
    }
}

/// Applies what is kept (at the shell's start, and after each change).
pub fn aplicar() {
    let a = ler();
    let reduzir = a["reduzir_animacoes"].as_bool().unwrap_or(false);
    if let Some(s) = gtk::Settings::default() {
        if s.is_gtk_enable_animations() == reduzir {
            s.set_gtk_enable_animations(!reduzir);
        }
    }
    escrever_niri();
    crate::legendas::seguir(a["legendas"].as_bool().unwrap_or(false));
    if a["teclado"].as_bool().unwrap_or(false) {
        crate::osk::enable(true);
    }
}

/// The screen reader (Orca): started or stopped (in the test session only said).
fn leitor(on: bool) {
    if sessao_de_teste() {
        if depurando() {
            eprintln!("debug acessibilidade: leitor de tela {} (orca, não executado na sessão de teste)", if on { "ligado" } else { "desligado" });
        }
        return;
    }
    if on {
        let _ = std::process::Command::new("orca").arg("--replace").stdin(std::process::Stdio::null()).spawn();
    } else {
        let _ = std::process::Command::new("pkill").args(["-x", "orca"]).status();
    }
    gnome("screen-reader-enabled", on.to_variant());
}

/// Mono audio: a PipeWire mono sink (through pipewire-pulse) in front of the output, as the default; off, the output
/// is the default again and the sink goes.
fn audio_mono(on: bool) -> Result<(), String> {
    let pactl = std::env::var("KALANGO_PACTL").unwrap_or_else(|_| "pactl".into());
    let run = |args: &[&str]| -> Result<String, String> {
        let o = std::process::Command::new(&pactl).args(args).output().map_err(|e| format!("{pactl}: {e}"))?;
        if !o.status.success() {
            return Err(String::from_utf8_lossy(&o.stderr).trim().to_string());
        }
        Ok(String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    if on {
        let master = run(&["get-default-sink"])?;
        if master == "kalango_mono" {
            return Ok(());
        }
        let id = run(&["load-module", "module-remap-sink", "sink_name=kalango_mono", &format!("master={master}"), "channels=1", "channel_map=mono",
            "sink_properties=device.description=Mono"])?;
        run(&["set-default-sink", "kalango_mono"])?;
        salvar("audio_mono_modulo", json!({"id": id, "saida": master}));
    } else {
        let m = ler()["audio_mono_modulo"].clone();
        if let Some(s) = m["saida"].as_str() {
            let _ = run(&["set-default-sink", s]);
        }
        if let Some(id) = m["id"].as_str() {
            run(&["unload-module", id])?;
        }
        salvar("audio_mono_modulo", Value::Null);
    }
    Ok(())
}

pub struct AccessibilityPage {
    pub frame: Rc<Moldura>,
    loading: Cell<bool>,
    contraste: gtk::Switch,
    textos: Vec<gtk::ToggleButton>,
    reduzir: gtk::Switch,
    leitor: gtk::Switch,
    ponteiro: gtk::Scale,
    legendas: gtk::Switch,
    alertas: gtk::Switch,
    mono: gtk::Switch,
    teclado: gtk::Switch,
    voz: gtk::Switch,
}

impl AccessibilityPage {
    pub fn new(c: &Rc<crate::configuracoes::Configuracoes>) -> Rc<AccessibilityPage> {
        let m = Moldura::new(tr("Accessibility"), &[], tr("Hearing and voice"));
        m.sub.set_text(tr("Make the system fit how you see, hear and use it"));
        // "describe the screen": the agent reads aloud what is visible and what can be done
        let descrever = gtk::Button::new();
        descrever.add_css_class("cfg-faixa-agente");
        let dc = caixa(Orientation::Horizontal, 10, "");
        dc.append(&estrela(14, "#A5A1E2"));
        let dl = left(tr("Describe the screen: the agent reads aloud what is visible and what can be done."), "cfg-texto-13");
        dl.set_hexpand(true);
        dl.set_ellipsize(gtk::pango::EllipsizeMode::End);
        dl.set_max_width_chars(1);
        dc.append(&dl);
        dc.append(&caps(&["Super", "Alt", "D"]));
        descrever.set_child(Some(&dc));
        descrever.connect_clicked(|_| crate::descrever::descrever_tela());
        m.corpo.append(&descrever);
        // seeing
        let visao = caixa(Orientation::Vertical, 2, "cfg-cartao");
        visao.append(&rotulo_cartao("olho", tr("SEEING")));
        let contraste = interruptor();
        visao.append(&row(tr("High contrast"), Some(tr("pure black, white text and strong outlines")), &contraste));
        let seg = caixa(Orientation::Horizontal, 2, "cfg-segmentado");
        seg.set_size_request(256, -1);
        seg.set_homogeneous(true);
        let mut textos = Vec::new();
        let mut grupo: Option<gtk::ToggleButton> = None;
        for t in TEXTOS {
            let b = gtk::ToggleButton::with_label(&format!("{:.0}%", t * 100.0));
            b.add_css_class("cfg-seg");
            if let Some(g) = &grupo {
                b.set_group(Some(g));
            } else {
                grupo = Some(b.clone());
            }
            seg.append(&b);
            textos.push(b);
        }
        visao.append(&row(tr("Text size"), None, &seg));
        let reduzir = interruptor();
        visao.append(&row(tr("Reduce motion"), Some(tr("springs become a 90 ms fade")), &reduzir));
        let leitor_s = interruptor();
        let lr = row(tr("Screen reader"), None, &leitor_s);
        lr.set_visible(glib::find_program_in_path("orca").is_some());
        visao.append(&lr);
        m.corpo.append(&visao);
        // the pointer
        let pc = caixa(Orientation::Vertical, 2, "cfg-cartao");
        pc.append(&rotulo_cartao("mouse", tr("POINTER")));
        let ponteiro = deslizador();
        ponteiro.set_range(16.0, 64.0);
        ponteiro.set_increments(8.0, 8.0);
        ponteiro.set_hexpand(false);
        ponteiro.set_size_request(200, -1);
        pc.append(&row(tr("Pointer size"), None, &ponteiro));
        m.corpo.append(&pc);
        // the panel: hearing, typing and voice
        let ouvir = caixa(Orientation::Vertical, 2, "cfg-cartao");
        ouvir.append(&rotulo_cartao("volume", tr("HEARING")));
        let legendas = interruptor();
        ouvir.append(&row(tr("Live captions"), Some(tr("for any sound · written by the local model")), &legendas));
        let alertas = interruptor();
        ouvir.append(&row(tr("Visual alerts"), Some(tr("the screen flashes softly instead of the sound")), &alertas));
        let mono = interruptor();
        ouvir.append(&row(tr("Mono audio"), None, &mono));
        m.painel.append(&ouvir);
        let digitar = caixa(Orientation::Vertical, 2, "cfg-cartao");
        digitar.append(&rotulo_cartao("teclado-d", tr("TYPING AND VOICE")));
        let teclado = interruptor();
        let tr_ = row(tr("On-screen keyboard"), None, &teclado);
        tr_.set_visible(["wvkbd-mobintl", "squeekboard"].iter().any(|p| glib::find_program_in_path(p).is_some()));
        digitar.append(&tr_);
        digitar.append(&row(tr("Dictation"), Some(tr("speak into any text field")), &caps(&["Super", "H"])));
        let voz = interruptor();
        digitar.append(&row(tr("Control by voice"), Some(tr("hold Super and speak: the agent does what you ask")), &voz));
        m.painel.append(&digitar);
        let page = Rc::new(AccessibilityPage { frame: m, loading: Cell::new(false), contraste, textos, reduzir, leitor: leitor_s, ponteiro, legendas, alertas, mono, teclado, voz });
        for (nome, w) in [("descrever", descrever.upcast_ref::<gtk::Widget>()), ("contraste", page.contraste.upcast_ref()), ("reduzir", page.reduzir.upcast_ref()),
            ("leitor", page.leitor.upcast_ref()), ("ponteiro", page.ponteiro.upcast_ref()), ("legendas", page.legendas.upcast_ref()), ("alertas", page.alertas.upcast_ref()),
            ("mono", page.mono.upcast_ref()), ("teclado", page.teclado.upcast_ref()), ("voz", page.voz.upcast_ref())] {
            crate::configuracoes::registrar_posicao(c, &format!("cfg acess {nome}"), w);
        }
        for (k, b) in page.textos.iter().enumerate() {
            crate::configuracoes::registrar_posicao(c, &format!("cfg acess texto {}", (TEXTOS[k] * 100.0) as u32), b.upcast_ref());
        }
        page.ligar(Rc::downgrade(c));
        page
    }

    fn ligar(self: &Rc<Self>, _c: Weak<crate::configuracoes::Configuracoes>) {
        let chave = |s: &gtk::Switch, f: fn(&AccessibilityPage, bool)| {
            let w = Rc::downgrade(self);
            s.connect_state_set(move |_, on| {
                if let Some(p) = w.upgrade().filter(|p| !p.loading.get()) {
                    f(&p, on);
                }
                glib::Propagation::Proceed
            });
        };
        chave(&self.contraste, |_, on| {
            let mut t = tema::ler();
            t.contraste = on;
            tema::salvar(&t);
            tema::aplicar(&t);
            log(&format!("contraste {on}"));
        });
        chave(&self.reduzir, |_, on| {
            salvar("reduzir_animacoes", json!(on));
            gnome("enable-animations", (!on).to_variant());
            aplicar();
            log(&format!("reduzir animações {on}"));
        });
        chave(&self.leitor, |_, on| {
            salvar("leitor_de_tela", json!(on));
            leitor(on);
        });
        chave(&self.legendas, |_, on| {
            salvar("legendas", json!(on));
            crate::legendas::seguir(on);
            log(&format!("legendas {on}"));
        });
        chave(&self.alertas, |_, on| {
            salvar("alertas_visuais", json!(on));
            log(&format!("alertas visuais {on}"));
        });
        chave(&self.mono, |p, on| {
            let r = audio_mono(on);
            log(&format!("áudio mono {on} ({})", r.as_ref().map(|_| "ok".to_string()).unwrap_or_else(|e| e.clone())));
            if r.is_err() {
                p.loading.set(true);
                p.mono.set_active(!on);
                p.mono.set_state(!on);
                p.loading.set(false);
            } else {
                salvar("audio_mono", json!(on));
            }
        });
        chave(&self.teclado, |_, on| {
            crate::descrever::teclado_na_tela(on);
            log(&format!("teclado virtual {on}"));
        });
        // control by voice: holding Super talks to the agent (Keyboard › Super key › Held, the same setting)
        chave(&self.voz, |_, on| {
            crate::configuracoes::salvar_ajuste("super_segurar", json!(if on { "voz" } else { "nada" }));
            log(&format!("controle por voz {on}"));
        });
        for (k, b) in self.textos.iter().enumerate() {
            let w = Rc::downgrade(self);
            let v = TEXTOS[k];
            b.connect_toggled(move |b| {
                if let Some(_p) = w.upgrade().filter(|p| b.is_active() && !p.loading.get()) {
                    let mut t = tema::ler();
                    t.texto = v;
                    tema::salvar(&t);
                    tema::aplicar(&t);
                    log(&format!("texto {:.0}%", v * 100.0));
                }
            });
        }
        let w = Rc::downgrade(self);
        let pendente: Rc<Cell<Option<glib::SourceId>>> = Rc::default();
        self.ponteiro.connect_value_changed(move |s| {
            let Some(p) = w.upgrade().filter(|p| !p.loading.get()) else { return };
            let n = ((s.value() / 8.0).round() * 8.0) as u64;
            // written once the slider rests (niri reloads its configuration on each write)
            if let Some(t) = pendente.take() {
                t.remove();
            }
            let pend = pendente.clone();
            pendente.set(Some(glib::timeout_add_local_once(std::time::Duration::from_millis(300), move || {
                pend.take();
                salvar("ponteiro", json!(n));
                escrever_niri();
                gnome("cursor-size", (n as i32).to_variant());
                log(&format!("ponteiro {n}"));
            })));
            let _ = p;
        });
    }

    /// The controls as things are now.
    pub fn mostrar(self: &Rc<Self>) {
        self.loading.set(true);
        let a = ler();
        let t = tema::ler();
        self.contraste.set_active(t.contraste);
        let k = TEXTOS.iter().position(|v| (v - t.texto).abs() < 0.01).unwrap_or(0);
        for (i, b) in self.textos.iter().enumerate() {
            b.set_active(i == k);
        }
        let b = |k: &str| a[k].as_bool().unwrap_or(false);
        self.reduzir.set_active(b("reduzir_animacoes"));
        self.leitor.set_active(b("leitor_de_tela"));
        self.ponteiro.set_value(a["ponteiro"].as_u64().unwrap_or(24) as f64);
        self.legendas.set_active(b("legendas"));
        self.alertas.set_active(b("alertas_visuais"));
        self.mono.set_active(b("audio_mono"));
        self.teclado.set_active(crate::descrever::teclado_ligado());
        self.voz.set_active(crate::configuracoes::ajustes()["super_segurar"].as_str().unwrap_or("voz") == "voz");
        for s in [&self.contraste, &self.reduzir, &self.leitor, &self.legendas, &self.alertas, &self.mono, &self.teclado, &self.voz] {
            s.set_state(s.is_active());
        }
        self.loading.set(false);
        if depurando() {
            eprintln!("debug acessibilidade: lido (contraste {}, texto {:.0}%, reduzir {}, ponteiro {})", t.contraste, t.texto * 100.0, b("reduzir_animacoes"), a["ponteiro"].as_u64().unwrap_or(24));
        }
    }

    pub fn compactar(&self, c: bool) {
        self.frame.compactar(c);
    }
}

fn log(s: &str) {
    if depurando() {
        eprintln!("debug acessibilidade: {s}");
    }
}
