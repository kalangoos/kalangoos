//! Accessibility › live captions: what plays on the computer (the output's monitor: a video, a call, music), written
//! by the local audio model a few seconds at a time, in a bar above the dock. Silent pieces are skipped (no text is
//! made up from silence); the pieces are deleted as soon as they are read. Only while the person keeps it on.
//! KALANGO_GRAVADOR (tests): the recorder that stands for the sound (the session test has no sound).

use crate::superficies::caixa;
use gtk::prelude::*;
use gtk::{glib, Orientation};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const PEDACO: Duration = Duration::from_secs(4);


fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

struct Legendas {
    janela: gtk::Window,
    texto: gtk::Label,
    parar: Arc<AtomicBool>,
    ultima: RefCell<Instant>,
}

thread_local! {
    static ATIVAS: RefCell<Option<Legendas>> = const { RefCell::new(None) };
}

/// Turns the captions on or off (nothing changes when they already are).
pub fn seguir(on: bool) {
    let ativas = ATIVAS.with(|a| a.borrow().is_some());
    if on == ativas {
        return;
    }
    if !on {
        if let Some(l) = ATIVAS.with(|a| a.borrow_mut().take()) {
            l.parar.store(true, Ordering::SeqCst);
            l.janela.close();
        }
        return;
    }
    let Some(app) = gtk::gio::Application::default().and_downcast::<gtk::Application>() else { return };
    let janela = gtk::Window::new();
    janela.set_application(Some(&app));
    janela.init_layer_shell();
    janela.set_layer(Layer::Overlay);
    janela.set_namespace(Some("kalango-legendas"));
    janela.set_anchor(Edge::Bottom, true);
    // above the dock
    janela.set_margin(Edge::Bottom, 84);
    janela.set_keyboard_mode(KeyboardMode::None);
    janela.set_resizable(false);
    janela.add_css_class("legendas");
    let c = caixa(Orientation::Vertical, 0, "legendas-caixa");
    let texto = gtk::Label::new(None);
    texto.add_css_class("legendas-texto");
    texto.set_wrap(true);
    texto.set_max_width_chars(70);
    texto.set_lines(2);
    texto.set_ellipsize(gtk::pango::EllipsizeMode::Start);
    texto.set_justify(gtk::Justification::Center);
    c.append(&texto);
    janela.set_child(Some(&c));
    janela.connect_realize(|w| {
        if let Some(s) = w.surface() {
            s.set_input_region(&gtk::cairo::Region::create());
        }
    });
    let parar = Arc::new(AtomicBool::new(false));
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let p2 = parar.clone();
    std::thread::spawn(move || laco(p2, tx));
    glib::timeout_add_local(Duration::from_millis(200), move || {
        let vivo = ATIVAS.with(|a| {
            let a = a.borrow();
            let Some(l) = a.as_ref() else { return false };
            for t in rx.try_iter() {
                // the last two lines' worth: the new words at the end
                let junto = format!("{} {}", l.texto.text(), t);
                let palavras: Vec<&str> = junto.split_whitespace().collect();
                let fim = palavras[palavras.len().saturating_sub(24)..].join(" ");
                l.texto.set_text(&fim);
                *l.ultima.borrow_mut() = Instant::now();
                l.janela.present();
                if debug() {
                    eprintln!("debug legendas: «{t}»");
                }
            }
            // nothing new for a while: the bar goes (it comes back with the next words)
            if l.ultima.borrow().elapsed() > Duration::from_secs(8) && l.janela.is_visible() {
                l.janela.set_visible(false);
                l.texto.set_text("");
            }
            true
        });
        if vivo { glib::ControlFlow::Continue } else { glib::ControlFlow::Break }
    });
    if debug() {
        eprintln!("debug legendas: ligadas");
    }
    ATIVAS.with(|a| *a.borrow_mut() = Some(Legendas { janela, texto, parar, ultima: RefCell::new(Instant::now()) }));
}

/// Records the output a piece at a time and sends each piece's words.
fn laco(parar: Arc<AtomicBool>, tx: std::sync::mpsc::Sender<String>) {
    let dir = crate::voz::private_dir();
    let bin = crate::voz::agent_binary();
    let mut n = 0u32;
    while !parar.load(Ordering::SeqCst) {
        n = n.wrapping_add(1);
        let wav = dir.join(format!("legenda-{}-{n}.wav", std::process::id()));
        let mut cmd = match std::env::var_os("KALANGO_GRAVADOR") {
            Some(p) => std::process::Command::new(p),
            None => {
                let mut c = std::process::Command::new("pw-record");
                // what the computer plays (the default output's monitor), as the speech model wants it
                c.args(["--rate", "16000", "--channels", "1", "--format", "s16", "-P", "{ stream.capture.sink=true }"]);
                c
            }
        };
        crate::voz::as_recorder(&mut cmd);
        let Ok(mut filho) = cmd.arg(&wav).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn() else {
            std::thread::sleep(Duration::from_secs(2));
            continue;
        };
        let t0 = Instant::now();
        while t0.elapsed() < PEDACO && !parar.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(100));
        }
        // (SIGINT: the recorder closes the WAV properly)
        crate::voz::signal_child(filho.id(), crate::voz::SIGINT);
        // up to 2 s to close the file; then it is stopped for good
        let t1 = Instant::now();
        while filho.try_wait().ok().flatten().is_none() {
            if t1.elapsed() > Duration::from_secs(2) {
                let _ = filho.kill();
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = filho.wait();
        if parar.load(Ordering::SeqCst) {
            let _ = std::fs::remove_file(&wav);
            break;
        }
        if !silencio(&wav) {
            if let Ok(o) = std::process::Command::new(&bin).arg("transcrever").arg(&wav).stdin(std::process::Stdio::null()).output() {
                let t = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if o.status.success() && !t.is_empty() && tx.send(t).is_err() {
                    let _ = std::fs::remove_file(&wav);
                    break;
                }
            }
        }
        let _ = std::fs::remove_file(&wav);
    }
}

/// A piece with nothing to hear (its loudest part well under speech's level).
fn silencio(wav: &std::path::Path) -> bool {
    let Ok(b) = std::fs::read(wav) else { return true };
    if b.len() <= 44 {
        return true;
    }
    let pico = b[44..].chunks_exact(2).map(|s| i16::from_le_bytes([s[0], s[1]]).unsigned_abs()).max().unwrap_or(0);
    pico < 600
}
