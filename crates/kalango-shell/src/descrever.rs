//! Accessibility's actions outside the settings page:
//!
//! - **Describe the screen** (Super+Alt+D): a picture of the screen, the local vision model (LFM3-VL) says what is
//!   there and what can be done, and the voice reads it (speech-dispatcher); the text also stays as a notice. The
//!   picture is the compositor's (it would go to the clipboard and say "Screenshot captured": neither happens, and the
//!   person's last copy is put back) and is deleted after.
//! - **Dictation** (Super+H): the microphone records until Super+H again, the local audio model writes it and the text
//!   goes into the field in use (the clipboard and the virtual keyboard's Ctrl+V, as the clipboard history pastes).
//! - The **on-screen keyboard** (wvkbd, or squeekboard where it is the one installed).
//! - **Visual alerts**: the screen flashes softly when a notification would play its sound.
//!
//! In the test session nothing is spoken out loud (KALANGO_SONS=nao): what would be said is logged.

use gtk::prelude::*;
use gtk::glib;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kalango_i18n::tr;
use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

thread_local! {
    /// until when a screen picture is ours (its notice and its clipboard copy are not the person's)
    static CAPTURA_ATE: Cell<Option<Instant>> = const { Cell::new(None) };
    static DESCREVENDO: Cell<bool> = const { Cell::new(false) };
    static DITADO: RefCell<Option<(crate::voz::Recording, u32)>> = const { RefCell::new(None) };
    static FLASH: RefCell<Option<gtk::Window>> = const { RefCell::new(None) };
}

/// The next seconds' screen picture is the shell's own (the screenshot tool's, as describing the screen's): its
/// notice is not shown and the person's clipboard comes back after it.
pub fn mark_own_capture() {
    CAPTURA_ATE.with(|c| c.set(Some(Instant::now() + Duration::from_secs(4))));
}

/// Whether a screen picture now is the one taken to describe the screen.
pub fn captura_propria() -> bool {
    CAPTURA_ATE.with(|c| c.get().is_some_and(|t| Instant::now() < t))
}

/// Says a text out loud (speech-dispatcher, in the system's language); in the test session only logged.
pub fn falar(texto: &str) {
    if std::env::var("KALANGO_SONS").as_deref() == Ok("nao") {
        if debug() {
            eprintln!("debug acessibilidade: falaria «{}»", texto.chars().take(80).collect::<String>().replace('\n', " "));
        }
        return;
    }
    let lang = kalango_i18n::system_locale().map(|l| l.split('.').next().unwrap_or("pt_BR").replace('_', "-")).unwrap_or_else(|| "pt-BR".into());
    let _ = std::process::Command::new("spd-say").args(["-l", &lang, "-C"]).status();
    let _ = std::process::Command::new("spd-say").args(["-l", &lang, texto]).stdin(std::process::Stdio::null()).spawn();
}

/// Describe the screen: picture → the vision model → the voice (and a notice with the text).
pub fn descrever_tela() {
    if DESCREVENDO.with(|d| d.replace(true)) {
        return;
    }
    let dir = crate::voz::private_dir();
    let png = dir.join(format!("tela-{}.png", std::process::id()));
    let _ = std::fs::remove_file(&png);
    CAPTURA_ATE.with(|c| c.set(Some(Instant::now() + Duration::from_secs(4))));
    let r = crate::servicos::niri_acao(serde_json::json!({"ScreenshotScreen": {"write_to_disk": true, "show_pointer": false, "path": png.display().to_string()}}));
    if let Err(e) = r {
        DESCREVENDO.with(|d| d.set(false));
        eprintln!("kalango-shell: descrever a tela: {e}");
        return;
    }
    if debug() {
        eprintln!("debug acessibilidade: descrevendo a tela");
    }
    falar(tr("Looking at the screen."));
    let bin = crate::voz::agent_binary();
    let pergunta = tr("Describe this screen for someone who cannot see it well: say in a few short sentences which window is in front, what it shows and the main things that can be done there. Answer in the language of these instructions, without lists or symbols.").to_string();
    crate::lateral::em_segundo_plano(move || {
        // the compositor writes the picture just after the request
        for _ in 0..40 {
            if png.metadata().is_ok_and(|m| m.len() > 0) {
                break;
            }
            std::thread::sleep(Duration::from_millis(75));
        }
        let o = std::process::Command::new(&bin).args(["--json", "ver"]).arg(&png).arg(&pergunta).stdin(std::process::Stdio::null()).output();
        let _ = std::fs::remove_file(&png);
        let o = o.map_err(|e| e.to_string())?;
        let v: serde_json::Value = String::from_utf8_lossy(&o.stdout).lines().filter_map(|l| serde_json::from_str(l).ok()).last().unwrap_or_default();
        match v["texto"].as_str() {
            Some(t) if v["tipo"] == "visao" => Ok(t.trim().to_string()),
            _ => Err(v["texto"].as_str().map(str::to_string).unwrap_or_else(|| String::from_utf8_lossy(&o.stderr).trim().to_string())),
        }
    }, |r: Result<String, String>| {
        DESCREVENDO.with(|d| d.set(false));
        crate::clipboard::repor_ultimo();
        match r {
            Ok(t) => {
                if debug() {
                    eprintln!("debug acessibilidade: tela descrita ({} caracteres)", t.chars().count());
                }
                falar(&t);
                if let Some(s) = crate::shell_atual() {
                    s.notificacoes().notify_local(tr("Accessibility"), tr("On the screen"), &t);
                }
            }
            Err(e) => {
                if debug() {
                    eprintln!("debug acessibilidade: descrever a tela falhou: {e}");
                }
                falar(tr("I could not describe the screen."));
            }
        }
    });
}

/// Dictation: the first Super+H starts recording, the second writes what was said into the field in use.
pub fn ditar() {
    let gravando = DITADO.with(|d| d.borrow_mut().take());
    match gravando {
        None => match crate::voz::start("ditado") {
            Ok(rec) => {
                let id = crate::shell_atual().map(|s| s.notificacoes().notify_local_acoes(tr("Dictation"), tr("Listening…"), tr("Super+H again writes it in the field."), &[], |_| {})).unwrap_or(0);
                DITADO.with(|d| *d.borrow_mut() = Some((rec, id)));
                if debug() {
                    eprintln!("debug acessibilidade: ditado gravando");
                }
            }
            Err(e) => eprintln!("kalango-shell: ditado: {e}"),
        },
        Some((rec, id)) => {
            if let Some(s) = crate::shell_atual() {
                s.notificacoes().dispensar(id);
            }
            crate::lateral::em_segundo_plano(move || crate::voz::finish(rec), |r| match r {
                Ok(t) if !t.trim().is_empty() => {
                    let t = t.trim().to_string();
                    let colado = crate::clipboard::copy_text(&t, false).and_then(|_| {
                        std::thread::sleep(Duration::from_millis(120));
                        crate::paste_keys::paste(false)
                    });
                    if debug() {
                        eprintln!("debug acessibilidade: ditado «{}» ({})", t.chars().take(60).collect::<String>(), if colado.is_ok() { "colado" } else { "não colado" });
                    }
                }
                Ok(_) => {
                    if debug() {
                        eprintln!("debug acessibilidade: ditado sem fala");
                    }
                }
                Err(e) => eprintln!("kalango-shell: ditado: {e}"),
            });
        }
    }
}

/// The on-screen keyboard is on (it rises with each text field).
pub fn teclado_ligado() -> bool {
    crate::osk::enabled()
}

/// The on-screen keyboard: KalangoOS's own (osk.rs), not another system's.
pub fn teclado_na_tela(on: bool) {
    crate::osk::enable(on);
}

/// Visual alerts: a soft white flash over every screen (it takes no clicks), fading in 300 ms.
pub fn piscar() {
    if !crate::configuracoes_acessibilidade::ligado("alertas_visuais") {
        return;
    }
    if debug() {
        eprintln!("debug acessibilidade: alerta visual");
    }
    let Some(app) = gtk::gio::Application::default().and_downcast::<gtk::Application>() else { return };
    let w = FLASH.with(|f| f.borrow().clone()).unwrap_or_else(|| {
        let w = gtk::Window::new();
        w.set_application(Some(&app));
        w.init_layer_shell();
        w.set_layer(Layer::Overlay);
        w.set_namespace(Some("kalango-alerta-visual"));
        for e in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
            w.set_anchor(e, true);
        }
        w.set_exclusive_zone(-1);
        w.set_keyboard_mode(KeyboardMode::None);
        w.add_css_class("alerta-visual");
        w.set_can_target(false);
        let caixa = gtk::Box::new(gtk::Orientation::Vertical, 0);
        caixa.add_css_class("alerta-visual-luz");
        w.set_child(Some(&caixa));
        // no input at all: the flash never takes a click meant for what is under it
        w.connect_realize(|w| {
            if let Some(s) = w.surface() {
                s.set_input_region(&gtk::cairo::Region::create());
            }
        });
        FLASH.with(|f| *f.borrow_mut() = Some(w.clone()));
        w
    });
    w.set_opacity(1.0);
    w.present();
    let inicio = Instant::now();
    let ww = w.downgrade();
    glib::timeout_add_local(Duration::from_millis(16), move || {
        let Some(w) = ww.upgrade() else { return glib::ControlFlow::Break };
        let t = inicio.elapsed().as_secs_f64() / 0.3;
        if t >= 1.0 {
            w.set_visible(false);
            return glib::ControlFlow::Break;
        }
        w.set_opacity(1.0 - t);
        glib::ControlFlow::Continue
    });
}
