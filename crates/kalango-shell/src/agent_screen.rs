//! An agent controlling the screen (design page 29 "Agente controla a tela"): while kalangod's session lasts, the
//! banner under the top bar says who controls which window, how many actions and how much time are left, with
//! "Stop (Esc)"; the window the person allowed is outlined, and the agent's pointer carries its name. When the person
//! takes over (their own mouse or keyboard: niri tells kalangod) or the session ends, the banner says so for a moment.
//!
//! The state comes from kalangod (`kalango/tela` "estado", a long wait); the outline and the name sit on one
//! overlay surface that takes no input (clicks go through to the window under it).

use crate::superficies::rotulo;
use crate::Shell;
use gtk::prelude::*;
use gtk::{gdk, glib};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kalango_i18n::{tr, trf};
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

pub struct AgentScreen {
    shell: Weak<Shell>,
    overlay: gtk::ApplicationWindow,
    fixed: gtk::Fixed,
    outline: gtk::Box,
    name: gtk::Label,
    state: RefCell<Value>,
    /// when the last end shown happened (each end is told once)
    end_shown: Cell<u64>,
    /// the countdown's tick while a session lasts
    tick: RefCell<Option<glib::SourceId>>,
}

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

/// The person stopped the agent (the banner's button): kalangod ends the session at once.
fn stop() {
    std::thread::spawn(|| {
        let _ = crate::servicos::kalangod_rpc("kalango/tela", json!({"acao": "parar"}));
    });
}

impl AgentScreen {
    pub fn new(app: &gtk::Application, shell: &Rc<Shell>) -> Rc<AgentScreen> {
        let overlay = gtk::ApplicationWindow::builder().application(app).build();
        overlay.init_layer_shell();
        overlay.set_namespace(Some("kalango-controle-tela"));
        overlay.set_layer(Layer::Overlay);
        for e in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
            overlay.set_anchor(e, true);
        }
        // (the whole output, over the bars: the outline is drawn where the window really is)
        overlay.set_exclusive_zone(-1);
        overlay.set_keyboard_mode(KeyboardMode::None);
        overlay.add_css_class("controle-tela");
        let fixed = gtk::Fixed::new();
        let outline = gtk::Box::new(gtk::Orientation::Vertical, 0);
        outline.add_css_class("controle-contorno");
        let name = rotulo("", "controle-cursor");
        fixed.put(&outline, 0.0, 0.0);
        fixed.put(&name, 0.0, 0.0);
        overlay.set_child(Some(&fixed));
        // no input at all: every click and key goes to the windows under it
        overlay.connect_realize(|w| {
            if let Some(s) = w.surface() {
                s.set_input_region(&gtk::cairo::Region::create());
            }
        });
        let a = Rc::new(AgentScreen { shell: Rc::downgrade(shell), overlay, fixed, outline, name, state: RefCell::new(Value::Null), end_shown: Cell::new(0), tick: RefCell::default() });
        a.listen();
        a
    }

    /// kalangod's state, waited for in a thread (a long wait: no polling while nothing changes).
    fn listen(self: &Rc<Self>) {
        let (tx, rx) = std::sync::mpsc::channel::<Value>();
        std::thread::spawn(move || {
            let mut since = u64::MAX;
            loop {
                match crate::servicos::kalangod_rpc_wait("kalango/tela", json!({"acao": "estado", "geracao": since, "max_s": 30}), std::time::Duration::from_secs(40)) {
                    Ok(v) => {
                        since = v["geracao"].as_u64().unwrap_or(u64::MAX);
                        if tx.send(v).is_err() {
                            return;
                        }
                    }
                    Err(_) => std::thread::sleep(std::time::Duration::from_secs(5)),
                }
            }
        });
        let w = Rc::downgrade(self);
        glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
            let Some(a) = w.upgrade() else { return glib::ControlFlow::Break };
            let mut last = None;
            while let Ok(v) = rx.try_recv() {
                last = Some(v);
            }
            if let Some(v) = last {
                a.apply(v);
            }
            glib::ControlFlow::Continue
        });
    }

    fn apply(self: &Rc<Self>, v: Value) {
        let active = v["ativa"] == true;
        *self.state.borrow_mut() = v.clone();
        let Some(usando) = self.shell.upgrade().and_then(|s| s.usando()) else { return };
        if active {
            self.banner();
            self.place();
            if self.tick.borrow().is_none() {
                let w = Rc::downgrade(self);
                let id = glib::timeout_add_seconds_local(1, move || {
                    let Some(a) = w.upgrade() else { return glib::ControlFlow::Break };
                    if a.state.borrow()["ativa"] != true {
                        a.tick.borrow_mut().take();
                        return glib::ControlFlow::Break;
                    }
                    // (the time left counts down here; kalangod's state only changes on actions)
                    let left = a.state.borrow()["restante_s"].as_u64().unwrap_or(0).saturating_sub(1);
                    a.state.borrow_mut()["restante_s"] = json!(left);
                    a.banner();
                    glib::ControlFlow::Continue
                });
                *self.tick.borrow_mut() = Some(id);
            }
            return;
        }
        if let Some(id) = self.tick.borrow_mut().take() {
            id.remove();
        }
        self.overlay.set_visible(false);
        let end = &v["fim"];
        let when = end["quando"].as_u64().unwrap_or(0);
        if when == 0 || when == self.end_shown.get() {
            if self.end_shown.get() == 0 {
                self.end_shown.set(when);
            }
            return;
        }
        self.end_shown.set(when);
        // (told only when it just happened: a shell started later does not repeat an old end)
        let now = glib::DateTime::now_utc().map(|d| d.to_unix() as u64).unwrap_or(0);
        if now.saturating_sub(when) > 10 {
            usando.esconder();
            return;
        }
        let who = end["agente"].as_str().filter(|a| !a.is_empty()).map(str::to_string).unwrap_or_else(|| tr("the agent").to_string());
        let (title, detail) = match end["motivo"].as_str().unwrap_or("") {
            "pessoa" => (tr("You took over the control").to_string(), trf("{who} stopped and waits for you", &[("who", who)])),
            _ => (tr("The screen control ended").to_string(), match end["acoes"].as_u64().unwrap_or(0) {
                1 => tr("1 action").to_string(),
                n => trf("{n} actions", &[("n", n.to_string())]),
            }),
        };
        usando.mostrar_aviso(&title, &detail);
        if debug() {
            eprintln!("debug controle da tela: terminou ({})", end["motivo"].as_str().unwrap_or(""));
        }
        let u = Rc::downgrade(&usando);
        let w = Rc::downgrade(self);
        glib::timeout_add_seconds_local_once(4, move || {
            // (unless a new session began meanwhile)
            if w.upgrade().is_some_and(|a| a.state.borrow()["ativa"] != true) {
                if let Some(u) = u.upgrade() {
                    u.esconder();
                }
            }
        });
    }

    fn banner(&self) {
        let Some(usando) = self.shell.upgrade().and_then(|s| s.usando()) else { return };
        let v = self.state.borrow().clone();
        let who = v["agente"].as_str().unwrap_or("").to_string();
        let window = if v["tela_inteira"] == true { tr("the screen").to_string() } else { short_title(v["janela"].as_str().unwrap_or("")) };
        usando.mostrar_controle(&who, &window, v["acoes"].as_u64().unwrap_or(0), v["max_acoes"].as_u64().unwrap_or(0), v["restante_s"].as_u64().unwrap_or(0), stop);
    }

    /// The outline on the allowed window and the agent's name by its pointer.
    fn place(&self) {
        let v = self.state.borrow().clone();
        let r = &v["retangulo"];
        let output = r["saida"].as_str().or(v["ponteiro"]["saida"].as_str()).unwrap_or("");
        // (on the output the session is on)
        if let Some(m) = gdk::Display::default().and_then(|d| {
            let ms = d.monitors();
            (0..ms.n_items()).filter_map(|i| ms.item(i).and_downcast::<gdk::Monitor>()).find(|m| m.connector().as_deref() == Some(output))
        }) {
            self.overlay.set_monitor(Some(&m));
        }
        let has_rect = r.is_object() && v["tela_inteira"] != true;
        self.outline.set_visible(has_rect);
        if has_rect {
            let (x, y, w, h) = (r["x"].as_f64().unwrap_or(0.0), r["y"].as_f64().unwrap_or(0.0), r["w"].as_f64().unwrap_or(0.0), r["h"].as_f64().unwrap_or(0.0));
            self.outline.set_size_request(w as i32, h as i32);
            self.fixed.move_(&self.outline, x, y);
        }
        let p = &v["ponteiro"];
        self.name.set_visible(p.is_object());
        if p.is_object() {
            self.name.set_text(v["agente"].as_str().unwrap_or(""));
            // (below and right of the pointer's tip, as in the design)
            self.fixed.move_(&self.name, p["x"].as_f64().unwrap_or(0.0) + 18.0, p["y"].as_f64().unwrap_or(0.0) + 25.0);
        }
        self.overlay.present();
        if debug() {
            eprintln!("debug controle da tela: {} · {}/{} ações · contorno {}", v["janela"].as_str().unwrap_or(""), v["acoes"], v["max_acoes"], r);
        }
    }
}

/// "notas.md — Text Editor" → "Text Editor" (the app, as the banner says it); a title without an app part stays.
pub fn short_title(t: &str) -> String {
    t.rsplit_once(" — ").map(|(_, app)| app.to_string()).unwrap_or_else(|| t.to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_banner_names_the_app() {
        assert_eq!(super::short_title("notas.md — Editor de Texto"), "Editor de Texto");
        assert_eq!(super::short_title("Calculadora"), "Calculadora");
    }
}
