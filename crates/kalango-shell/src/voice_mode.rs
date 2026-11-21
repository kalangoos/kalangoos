//! Voice mode (design "Modo voz (segure Super e fale)"): holding Super (the compositor tells the shell: `voz iniciar`)
//! records the microphone — the panel rises at the bottom with the voice as bars —; letting go (`voz parar`) has the
//! local voice model write what was said, and the local model lists what it understood ("Entendi:"). Nothing runs
//! by itself: ↵ "Executar" hands the request to the agent (whose approvals stand), "Corrigir" opens it in the command
//! bar to edit, Esc "Cancelar" drops it. Another key with Super (`voz cancelar`: it was a shortcut) throws the
//! recording away.

use crate::conversa::ui_icon;
use crate::superficies::{caixa, camada, rotulo};
use gtk::prelude::*;
use gtk::{gdk, glib, Align, Orientation};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kalango_i18n::tr;
use kalango_movimento as mv;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::{Rc, Weak};
use std::time::Duration;

const BARS: usize = 60;

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    Closed,
    Listening,
    Transcribing,
    Ready,
}

pub struct VoiceMode {
    shell: RefCell<Weak<crate::Shell>>,
    window: gtk::ApplicationWindow,
    card: mv::Animavel,
    wave: gtk::DrawingArea,
    transcript: gtk::Label,
    understood: gtk::Box,
    understood_row: gtk::Box,
    actions: gtk::Box,
    hint: gtk::Label,
    levels: RefCell<VecDeque<f32>>,
    recording: RefCell<Option<crate::voz::Recording>>,
    phase: Cell<Phase>,
    text: RefCell<String>,
    timer: RefCell<Option<glib::SourceId>>,
}

fn key_cap(t: &str) -> gtk::Label {
    let l = rotulo(t, "clip-tecla");
    l.set_valign(Align::Center);
    l
}

impl VoiceMode {
    pub fn new(app: &gtk::Application) -> Rc<VoiceMode> {
        let window = camada(app, "kalango-modo-voz", Layer::Overlay, &[Edge::Bottom]);
        window.add_css_class("transparente");
        window.set_margin(Edge::Bottom, 80);
        let panel = caixa(Orientation::Vertical, 14, "voz-painel");
        panel.set_size_request(760, -1);
        let wave = gtk::DrawingArea::new();
        wave.set_content_height(64);
        wave.set_hexpand(true);
        panel.append(&wave);
        let transcript = rotulo("", "voz-transcricao");
        transcript.set_wrap(true);
        transcript.set_justify(gtk::Justification::Center);
        transcript.set_max_width_chars(60);
        panel.append(&transcript);
        let div = caixa(Orientation::Horizontal, 0, "clip-divisor");
        div.set_size_request(-1, 1);
        panel.append(&div);
        let understood_row = caixa(Orientation::Horizontal, 8, "");
        understood_row.append(&rotulo(tr("Understood:"), "voz-dica"));
        let understood = caixa(Orientation::Horizontal, 8, "");
        understood.set_hexpand(true);
        understood_row.append(&understood);
        understood_row.append(&ui_icon("escudo", "#8ED2C3", 16));
        understood_row.append(&rotulo(tr("transcribed by the local model"), "voz-dica"));
        panel.append(&understood_row);
        let actions = caixa(Orientation::Horizontal, 8, "");
        let run = gtk::Button::new();
        run.add_css_class("voz-executar");
        let rb = caixa(Orientation::Horizontal, 6, "");
        rb.append(&ui_icon("tocar", "#1C1D24", 16));
        rb.append(&rotulo(tr("Run"), ""));
        run.set_child(Some(&rb));
        actions.append(&run);
        actions.append(&key_cap("↵"));
        let fix = gtk::Button::with_label(tr("Correct"));
        fix.add_css_class("voz-corrigir");
        actions.append(&fix);
        let cancel = gtk::Button::with_label(tr("Cancel"));
        cancel.add_css_class("voz-cancelar");
        actions.append(&cancel);
        actions.append(&key_cap("Esc"));
        let gap = caixa(Orientation::Horizontal, 0, "");
        gap.set_hexpand(true);
        actions.append(&gap);
        actions.append(&ui_icon("microfone", "#A5A1E2", 16));
        let hint = rotulo("", "voz-dica");
        actions.append(&hint);
        panel.append(&actions);
        let card = mv::Animavel::new(&panel);
        window.set_child(Some(&card));
        let v = Rc::new(VoiceMode { shell: RefCell::default(), window, card, wave, transcript, understood, understood_row, actions, hint,
            levels: RefCell::new(VecDeque::from(vec![0.0; BARS])), recording: RefCell::default(), phase: Cell::new(Phase::Closed),
            text: RefCell::default(), timer: RefCell::default() });
        let w = Rc::downgrade(&v);
        v.wave.set_draw_func(move |_, cr, width, height| {
            if let Some(v) = w.upgrade() {
                v.draw_wave(cr, f64::from(width), f64::from(height));
            }
        });
        let w = Rc::downgrade(&v);
        run.connect_clicked(move |_| if let Some(v) = w.upgrade() { v.run() });
        let w = Rc::downgrade(&v);
        fix.connect_clicked(move |_| if let Some(v) = w.upgrade() { v.correct() });
        let w = Rc::downgrade(&v);
        cancel.connect_clicked(move |_| if let Some(v) = w.upgrade() { v.close() });
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let w = Rc::downgrade(&v);
        keys.connect_key_pressed(move |_, k, _, _| {
            let Some(v) = w.upgrade() else { return glib::Propagation::Proceed };
            match k {
                gdk::Key::Escape => v.close(),
                gdk::Key::Return | gdk::Key::KP_Enter if v.phase.get() == Phase::Ready => v.run(),
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        v.window.add_controller(keys);
        v
    }

    pub fn connect(&self, shell: &Rc<crate::Shell>) {
        *self.shell.borrow_mut() = Rc::downgrade(shell);
    }

    /// Super held: the panel rises and the microphone records (the person is holding the key: they chose to speak).
    pub fn start(self: &Rc<Self>) {
        if self.phase.get() == Phase::Listening {
            return;
        }
        self.drop_recording();
        match crate::voz::start("modo-voz") {
            Ok(r) => *self.recording.borrow_mut() = Some(r),
            Err(e) => {
                eprintln!("kalango-shell: modo voz: {e}");
                return;
            }
        }
        self.phase.set(Phase::Listening);
        crate::configuracoes_som::tocar("kalango-voice-start");
        self.text.borrow_mut().clear();
        self.levels.borrow_mut().iter_mut().for_each(|l| *l = 0.0);
        self.transcript.set_text(tr("Listening…"));
        self.transcript.add_css_class("incerta");
        self.understood_row.set_visible(false);
        self.actions.set_sensitive(false);
        self.hint.set_text(tr("release Super to finish"));
        self.window.set_keyboard_mode(KeyboardMode::None);
        self.window.present();
        self.card.entrar((0.0, 16.0), 1.0);
        // the bars follow the voice (20 a second)
        let w = Rc::downgrade(self);
        *self.timer.borrow_mut() = Some(glib::timeout_add_local(Duration::from_millis(50), move || {
            let Some(v) = w.upgrade().filter(|v| v.phase.get() == Phase::Listening) else { return glib::ControlFlow::Break };
            let level = v.recording.borrow().as_ref().map(|r| r.level()).unwrap_or(0.0);
            {
                let mut l = v.levels.borrow_mut();
                l.pop_front();
                l.push_back(level);
            }
            v.wave.queue_draw();
            glib::ControlFlow::Continue
        }));
        if debug() {
            eprintln!("debug voz: ouvindo");
        }
    }

    /// Super let go: what was said is written by the local voice model, then understood.
    pub fn finish(self: &Rc<Self>) {
        if self.phase.get() != Phase::Listening {
            return;
        }
        self.stop_timer();
        let Some(rec) = self.recording.borrow_mut().take() else { return };
        self.phase.set(Phase::Transcribing);
        crate::configuracoes_som::tocar("kalango-voice-stop");
        self.transcript.set_text(tr("Writing what you said…"));
        self.hint.set_text("");
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(move || crate::voz::finish(rec), move |r| {
            let Some(v) = w.upgrade() else { return };
            if v.phase.get() != Phase::Transcribing {
                return;
            }
            match r {
                Ok(t) if !t.trim().is_empty() => v.show_transcript(t.trim().to_string()),
                Ok(_) => v.fail(tr("Nothing was heard.")),
                Err(e) => v.fail(&e),
            }
        });
        if debug() {
            eprintln!("debug voz: transcrevendo");
        }
    }

    /// Another key joined Super: it was a shortcut, the recording goes away.
    pub fn cancel(self: &Rc<Self>) {
        if self.phase.get() == Phase::Listening {
            if debug() {
                eprintln!("debug voz: cancelado (era um atalho)");
            }
            self.close();
        }
    }

    fn fail(&self, why: &str) {
        self.phase.set(Phase::Ready);
        self.transcript.set_text(why);
        self.transcript.add_css_class("incerta");
        self.actions.set_sensitive(true);
        self.take_keyboard();
    }

    fn show_transcript(self: &Rc<Self>, text: String) {
        *self.text.borrow_mut() = text.clone();
        self.transcript.set_text(&format!("“{text}”"));
        self.transcript.remove_css_class("incerta");
        self.phase.set(Phase::Ready);
        self.actions.set_sensitive(true);
        self.take_keyboard();
        if debug() {
            eprintln!("debug voz: transcrito «{text}»");
        }
        // what the local model understood
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(move || {
            let mut c = std::process::Command::new(crate::voz::agent_binary()).arg("intencoes").stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn().ok()?;
            if let Some(mut i) = c.stdin.take() {
                let _ = std::io::Write::write_all(&mut i, text.as_bytes());
            }
            let o = c.wait_with_output().ok()?;
            let v: serde_json::Value = serde_json::from_slice(&o.stdout).ok()?;
            Some(v["acoes"].as_array()?.iter().filter_map(|a| a.as_str().map(str::to_string)).collect::<Vec<_>>())
        }, move |acts: Option<Vec<String>>| {
            let Some(v) = w.upgrade() else { return };
            while let Some(x) = v.understood.first_child() {
                v.understood.remove(&x);
            }
            let acts = acts.unwrap_or_default();
            for a in &acts {
                let chip = caixa(Orientation::Horizontal, 6, "voz-entendi");
                chip.append(&ui_icon("check", "#8ED2C3", 16));
                chip.append(&rotulo(a, "voz-entendi-texto"));
                v.understood.append(&chip);
            }
            v.understood_row.set_visible(!acts.is_empty());
            if debug() {
                eprintln!("debug voz: entendi {}", acts.join(" | "));
            }
        });
    }

    fn take_keyboard(&self) {
        let w = self.window.downgrade();
        glib::timeout_add_local_once(Duration::from_millis(40), move || {
            if let Some(j) = w.upgrade() {
                crate::superficies::take_keyboard(&j);
            }
        });
    }

    /// ↵ "Executar": the agent gets the request (its approvals stand for anything risky).
    fn run(self: &Rc<Self>) {
        let text = self.text.borrow().clone();
        if text.is_empty() {
            return;
        }
        self.close();
        if debug() {
            eprintln!("debug voz: executar «{text}»");
        }
        if let Some(s) = self.shell.borrow().upgrade() {
            s.right_bar().ask_agent(&text);
        }
    }

    /// "Corrigir": the request in the command bar, to edit.
    fn correct(self: &Rc<Self>) {
        let text = self.text.borrow().clone();
        self.close();
        if let Some(s) = self.shell.borrow().upgrade() {
            s.comando().abrir(&text);
        }
    }

    fn stop_timer(&self) {
        if let Some(t) = self.timer.borrow_mut().take() {
            t.remove();
        }
    }

    fn drop_recording(&self) {
        if let Some(r) = self.recording.borrow_mut().take() {
            std::thread::spawn(move || crate::voz::discard(r));
        }
    }

    pub fn close(&self) {
        self.stop_timer();
        self.drop_recording();
        if self.phase.replace(Phase::Closed) == Phase::Closed {
            return;
        }
        self.window.set_keyboard_mode(KeyboardMode::None);
        let w = self.window.downgrade();
        self.card.sair((0.0, 16.0), 1.0, move || {
            if let Some(j) = w.upgrade() {
                j.set_visible(false);
            }
        });
    }

    /// The voice as bars: the newest at the right, brighter towards the middle (as the design's).
    fn draw_wave(&self, cr: &gtk::cairo::Context, w: f64, h: f64) {
        let levels = self.levels.borrow();
        let (r, g, b) = crate::tema::rgb(crate::tema::ACENTO);
        let total = BARS as f64 * 8.0 - 4.0;
        let x0 = (w - total) / 2.0;
        for (i, l) in levels.iter().enumerate() {
            let d = (i as f64 - (BARS as f64 - 1.0) / 2.0).abs() / ((BARS as f64 - 1.0) / 2.0);
            let alpha = 0.33 + 0.67 * (1.0 - d * d);
            let bh = (4.0 + f64::from(*l) * (h - 8.0)).min(h);
            let (x, y) = (x0 + i as f64 * 8.0, (h - bh) / 2.0);
            cr.set_source_rgba(r, g, b, alpha);
            cr.new_sub_path();
            cr.arc(x + 2.0, y + 2.0, 2.0, std::f64::consts::PI, 1.5 * std::f64::consts::PI);
            cr.arc(x + 2.0, y + 2.0, 2.0, 1.5 * std::f64::consts::PI, 2.0 * std::f64::consts::PI);
            cr.rectangle(x, y + 2.0, 4.0, (bh - 4.0).max(0.0));
            cr.arc(x + 2.0, y + bh - 2.0, 2.0, 0.0, std::f64::consts::PI);
            let _ = cr.fill();
        }
    }
}
