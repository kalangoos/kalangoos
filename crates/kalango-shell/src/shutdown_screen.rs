//! Shutting down (design "Desligando"): before the machine powers off (or restarts), the screen goes dark with the
//! symbol and what is being wrapped up, a step per line — the apps are asked to close (each one saves its work, or
//! asks to: "Ver" brings that app back and cancels), the agent's request and the running routines get some time to
//! finish; what does not finish is stopped and kept, and the next session offers to continue it. "Desligar mesmo
//! assim" stops waiting. Esc cancels. Unattended (the night's restart into an update, under the lock screen) it never
//! waits for a person: an app still waiting gives the restart up for another time.

use crate::conversa::ui_icon;
use crate::superficies::{caixa, camada, rotulo};
use gtk::prelude::*;
use gtk::{gio, glib, Align, Orientation};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kalango_i18n::{tr, trf};
use kalango_movimento::{Animavel, Canal, LENTA};
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// How long the apps get to close before the screen says which one is waiting.
const APPS_WAIT: Duration = Duration::from_secs(8);
/// How long the agent and the routines get to finish.
const AGENTS_WAIT: Duration = Duration::from_secs(30);

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

/// The requests a shutdown stopped, kept for the next session.
fn interrupted_path() -> std::path::PathBuf {
    std::env::var_os("XDG_STATE_HOME").map(std::path::PathBuf::from).filter(|p| p.is_absolute())
        .unwrap_or_else(|| glib::home_dir().join(".local/state")).join("kalango/interrompidos.json")
}

fn keep_interrupted(pedido: &str, passos: usize, rotina: Option<&str>) {
    let p = interrupted_path();
    let mut v: Vec<Value> = std::fs::read(&p).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    v.push(json!({"pedido": pedido, "passos": passos, "rotina": rotina, "t": glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0)}));
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(&p, serde_json::to_vec_pretty(&v).unwrap_or_default());
}

/// The request to give the agent to continue one that a shutdown stopped.
pub(crate) fn continue_request(pedido: &str, passos: usize) -> String {
    if passos == 0 {
        pedido.to_string()
    } else {
        trf("Continue this request, stopped when the computer was shut down after {n} steps (check what is already done): {request}",
            &[("n", passos.to_string()), ("request", pedido.to_string())])
    }
}

/// At the session's start: what a shutdown stopped becomes a notice with "Continuar" (nothing runs by itself).
pub fn offer_interrupted(shell: &Rc<crate::Shell>) {
    let p = interrupted_path();
    let Some(lista) = std::fs::read(&p).ok().and_then(|b| serde_json::from_slice::<Vec<Value>>(&b).ok()) else { return };
    let _ = std::fs::remove_file(&p);
    for item in lista {
        let pedido = item["pedido"].as_str().unwrap_or("").to_string();
        if pedido.is_empty() {
            continue;
        }
        let passos = item["passos"].as_u64().unwrap_or(0) as usize;
        let nome = item["rotina"].as_str().map(str::to_string).unwrap_or_else(|| pedido.clone());
        if debug() {
            eprintln!("debug desligando: oferecendo continuar «{nome}»");
        }
        let s = Rc::downgrade(shell);
        shell.notificacoes().notify_local_acoes(tr("Agents"), tr("Stopped when the computer was shut down"), &trf("“{name}”, after {n} steps", &[("name", nome), ("n", passos.to_string())]),
            &[("continuar", tr("Continue"))], move |k| {
                if k == "continuar" {
                    if let Some(s) = s.upgrade() {
                        s.right_bar().ask_agent(&continue_request(&pedido, passos));
                    }
                }
            });
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mark {
    Running,
    Done,
    Waiting,
}

struct Step {
    row: gtk::Box,
    icon: gtk::Box,
    label: gtk::Label,
    extra: gtk::Box,
}

impl Step {
    fn new(list: &gtk::Box) -> Step {
        let row = caixa(Orientation::Horizontal, 10, "desligando-passo");
        let icon = caixa(Orientation::Horizontal, 0, "");
        icon.set_size_request(16, 16);
        icon.set_valign(Align::Center);
        row.append(&icon);
        let label = rotulo("", "desligando-texto");
        label.set_xalign(0.0);
        row.append(&label);
        let extra = caixa(Orientation::Horizontal, 6, "");
        row.append(&extra);
        list.append(&row);
        Step { row, icon, label, extra }
    }

    fn set(&self, mark: Mark, text: &str) {
        while let Some(c) = self.icon.first_child() {
            self.icon.remove(&c);
        }
        let w: gtk::Widget = match mark {
            Mark::Done => ui_icon("check", "#8ED2C3", 16).upcast(),
            Mark::Waiting => ui_icon("alerta", "#EFA2A6", 16).upcast(),
            Mark::Running => {
                let s = gtk::Spinner::new();
                s.add_css_class("desligando-anel");
                s.set_size_request(14, 14);
                s.start();
                s.upcast()
            }
        };
        self.icon.append(&w);
        self.label.set_text(text);
        if mark == Mark::Running { self.row.add_css_class("ativo") } else { self.row.remove_css_class("ativo") }
    }
}

/// The work being wrapped up.
struct Agent {
    pedido: String,
    rotina: Option<u64>,
    step: Step,
    done: Cell<bool>,
}

struct Screen {
    window: gtk::ApplicationWindow,
    all: Animavel,
    title: gtk::Label,
    list: gtk::Box,
    note: gtk::Label,
    action: RefCell<String>,
    install: Cell<bool>,
    started: Cell<Option<Instant>>,
    windows: RefCell<BTreeMap<u64, String>>,
    closed_apps: RefCell<Vec<String>>,
    apps: RefCell<Option<Step>>,
    agents: RefCell<Vec<Agent>>,
    finished: Cell<bool>,
    /// nobody is there: what to do when an app waits (the restart is given up)
    unattended: RefCell<Option<Box<dyn FnOnce()>>>,
}

thread_local! {
    static SCREEN: RefCell<Option<Rc<Screen>>> = const { RefCell::new(None) };
}

fn screen() -> Option<Rc<Screen>> {
    if let Some(s) = SCREEN.with(|s| s.borrow().clone()) {
        return Some(s);
    }
    let app = gio::Application::default().and_downcast::<gtk::Application>()?;
    let window = camada(&app, "kalango-desligando", Layer::Overlay, &[Edge::Top, Edge::Bottom, Edge::Left, Edge::Right]);
    window.add_css_class("desligando-fundo");
    crate::superficies::take_keyboard(&window);
    window.set_exclusive_zone(-1);
    let col = caixa(Orientation::Vertical, 0, "");
    col.set_halign(Align::Center);
    col.set_valign(Align::Center);
    // (an image of a fixed size: a picture would grow with the column)
    let simbolo = gtk::Image::from_paintable(gtk::gdk::Texture::from_bytes(&glib::Bytes::from_static(include_bytes!("../assets/simbolo.png"))).ok().as_ref());
    simbolo.set_pixel_size(110);
    simbolo.set_halign(Align::Center);
    col.append(&simbolo);
    let title = rotulo("", "desligando-titulo");
    title.set_margin_top(18);
    col.append(&title);
    let list = caixa(Orientation::Vertical, 8, "");
    list.set_halign(Align::Center);
    list.set_margin_top(18);
    col.append(&list);
    let note = rotulo("", "desligando-nota");
    note.set_margin_top(22);
    note.set_wrap(true);
    note.set_max_width_chars(60);
    note.set_justify(gtk::Justification::Center);
    col.append(&note);
    let force = gtk::Button::with_label(tr("Shut down anyway"));
    force.add_css_class("desligando-forcar");
    force.set_halign(Align::Center);
    force.set_margin_top(18);
    col.append(&force);
    let all = Animavel::new(&col);
    window.set_child(Some(&all));
    let s = Rc::new(Screen {
        window, all, title, list, note, action: RefCell::default(), install: Cell::new(false), started: Cell::new(None), windows: RefCell::default(),
        closed_apps: RefCell::default(), apps: RefCell::default(), agents: RefCell::default(), finished: Cell::new(false),
        unattended: RefCell::default(),
    });
    let w = Rc::downgrade(&s);
    force.connect_clicked(move |_| if let Some(s) = w.upgrade() { s.finish(true) });
    if debug() {
        crate::central::posicao_debug_sempre("desligando mesmo assim", force.upcast_ref());
    }
    let keys = gtk::EventControllerKey::new();
    let w = Rc::downgrade(&s);
    keys.connect_key_pressed(move |_, k, _, _| {
        if k == gtk::gdk::Key::Escape {
            if let Some(s) = w.upgrade() {
                s.cancel(None);
            }
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    s.window.add_controller(keys);
    SCREEN.with(|x| *x.borrow_mut() = Some(s.clone()));
    Some(s)
}

/// The app as the person knows it.
fn app_label(app_id: &str, title: &str) -> String {
    gio::DesktopAppInfo::new(&format!("{app_id}.desktop")).map(|a| a.name().to_string()).filter(|n| !n.is_empty())
        .unwrap_or_else(|| if title.is_empty() { app_id.to_string() } else { title.to_string() })
}

/// Unattended: as `start`, but an app that waits gives the action up (`gave_up` is called).
pub fn start_unattended(action: &str, install: bool, gave_up: impl FnOnce() + 'static) {
    start(action, install);
    if let Some(s) = SCREEN.with(|s| s.borrow().clone()).filter(|s| s.started.get().is_some()) {
        *s.unattended.borrow_mut() = Some(Box::new(gave_up));
    }
}

/// An app's dialog can come with another id ("gnome-text-editor" for org.gnome.TextEditor's "Salvar alterações?"):
/// the id of the app it belongs to, when one of the open windows' ids is the same once punctuation and case go.
fn same_app(app_id: &str, ids: &[String]) -> String {
    let norm = |s: &str| s.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_lowercase();
    let n = norm(app_id);
    if n.is_empty() || gio::DesktopAppInfo::new(&format!("{app_id}.desktop")).is_some() {
        return app_id.to_string();
    }
    ids.iter().find(|o| o.as_str() != app_id && norm(o).ends_with(&n)).cloned().unwrap_or_else(|| app_id.to_string())
}

/// Shuts down ("desligar") or restarts ("reiniciar") after wrapping up; `install` as the power menu chose.
pub fn start(action: &str, install: bool) {
    let Some(sh) = crate::shell_atual() else {
        crate::energia::Energia::power(action, install);
        return;
    };
    let Some(s) = screen() else {
        crate::energia::Energia::power(action, install);
        return;
    };
    if s.started.get().is_some() {
        return;
    }
    s.action.replace(action.to_string());
    s.install.set(install);
    s.finished.set(false);
    s.started.set(Some(Instant::now()));
    s.title.set_text(if action == "reiniciar" { tr("Restarting…") } else { tr("Shutting down…") });
    while let Some(c) = s.list.first_child() {
        s.list.remove(&c);
    }
    // the apps: every window asked to close
    let janelas: BTreeMap<u64, String> = {
        let todas = sh.estado.janelas.borrow();
        let ids: Vec<String> = todas.values().map(|j| j.app_id.clone()).collect();
        todas.values().map(|j| (j.id, app_label(&same_app(&j.app_id, &ids), &j.titulo))).collect()
    };
    if debug() {
        let ids: Vec<String> = sh.estado.janelas.borrow().values().map(|j| format!("{} {}", j.id, j.app_id)).collect();
        eprintln!("debug desligando: {} janelas para fechar ({})", janelas.len(), ids.join(", "));
    }
    let apps = Step::new(&s.list);
    apps.set(Mark::Running, &tr("Closing the apps…"));
    *s.apps.borrow_mut() = Some(apps);
    // the shell's own windows (Settings, Files, Agents…) are closed here — niri takes no action while the screen is
    // locked, and the night's restart into an update runs locked; the other apps are asked by niri
    let proprias: Vec<u64> = sh.estado.janelas.borrow().values().filter(|j| j.app_id.starts_with("os.kalango.")).map(|j| j.id).collect();
    if let Some(app) = gio::Application::default().and_downcast::<gtk::Application>() {
        for w in app.windows() {
            if w.is_visible() && !w.is_layer_window() {
                w.close();
            }
        }
    }
    for id in janelas.keys().filter(|id| !proprias.contains(id)) {
        let _ = crate::servicos::niri_acao(json!({"CloseWindow": {"id": id}}));
    }
    *s.windows.borrow_mut() = janelas;
    s.closed_apps.borrow_mut().clear();
    // the agent's request and the routines at work
    let mut agents = Vec::new();
    if let Some((pedido, passos)) = sh.estado.agent_task.borrow().clone() {
        let step = Step::new(&s.list);
        step.set(Mark::Running, &trf("Agent finishing “{task}” (step {n})…", &[("task", short(&pedido)), ("n", passos.max(1).to_string())]));
        agents.push(Agent { pedido, rotina: None, step, done: Cell::new(false) });
    }
    if let Some(ag) = sh.agendador() {
        let nomes: BTreeMap<u64, String> = crate::rotinas::Rotinas::carregar().lista.into_iter().map(|r| (r.id, r.nome)).collect();
        for id in ag.rodando.borrow().keys() {
            let nome = nomes.get(id).cloned().unwrap_or_default();
            let step = Step::new(&s.list);
            step.set(Mark::Running, &trf("{routine} finishing…", &[("routine", nome.clone())]));
            agents.push(Agent { pedido: nome, rotina: Some(*id), step, done: Cell::new(false) });
        }
    }
    s.note.set_text(if agents.is_empty() { "" } else { tr("What the agents do not finish stays for you to continue when you turn it on again.") });
    s.note.set_visible(!agents.is_empty());
    *s.agents.borrow_mut() = agents;
    s.window.present();
    crate::superficies::take_keyboard(&s.window);
    s.all.definir(Canal::Opacidade, 0.0).ir(Canal::Opacidade, 1.0, LENTA);
    let w = Rc::downgrade(&s);
    glib::timeout_add_local(Duration::from_millis(300), move || {
        let Some(s) = w.upgrade() else { return glib::ControlFlow::Break };
        if s.started.get().is_none() || s.finished.get() {
            return glib::ControlFlow::Break;
        }
        s.tick();
        glib::ControlFlow::Continue
    });
}

fn short(t: &str) -> String {
    let t = t.trim();
    if t.chars().count() > 48 { format!("{}…", t.chars().take(46).collect::<String>().trim_end()) } else { t.to_string() }
}

impl Screen {
    fn tick(self: &Rc<Self>) {
        let Some(sh) = crate::shell_atual() else { return };
        let Some(inicio) = self.started.get() else { return };
        // the apps that closed
        let abertas: Vec<u64> = sh.estado.janelas.borrow().keys().copied().collect();
        let fechadas: Vec<(u64, String)> = self.windows.borrow().iter().filter(|(id, _)| !abertas.contains(id)).map(|(i, n)| (*i, n.clone())).collect();
        for (id, nome) in fechadas {
            self.windows.borrow_mut().remove(&id);
            if !self.closed_apps.borrow().contains(&nome) {
                self.closed_apps.borrow_mut().push(nome.clone());
            }
            if debug() {
                eprintln!("debug desligando: {nome} fechou");
            }
        }
        let restantes = self.windows.borrow().clone();
        let n = self.closed_apps.borrow().len();
        if let Some(apps) = self.apps.borrow().as_ref() {
            if restantes.is_empty() {
                if !apps.row.has_css_class("feito") {
                    apps.row.add_css_class("feito");
                    while let Some(c) = apps.extra.first_child() {
                        apps.extra.remove(&c);
                    }
                    apps.set(Mark::Done, &match n { 0 => tr("No app was open").to_string(), 1 => tr("Work saved in 1 app").to_string(), n => trf("Work saved in {n} apps", &[("n", n.to_string())]) });
                }
            } else if inicio.elapsed() >= APPS_WAIT && self.unattended.borrow().is_some() {
                // nobody to answer the app: not now
                if debug() {
                    eprintln!("debug desligando: um app esperando, sem ninguém; fica para outra vez");
                }
                let gave_up = self.unattended.borrow_mut().take();
                self.cancel(None);
                if let Some(f) = gave_up {
                    f();
                }
                return;
            } else if inicio.elapsed() >= APPS_WAIT && apps.extra.first_child().is_none() {
                // an app that did not close is asking something (to save): it can be seen, which cancels
                let (id, nome) = restantes.iter().next().map(|(i, n)| (*i, n.clone())).unwrap_or_default();
                apps.set(Mark::Waiting, &if restantes.len() == 1 { trf("{app} is waiting for you (unsaved work?)", &[("app", nome.clone())]) } else {
                    trf("{app} and {n} more are waiting for you", &[("app", nome.clone()), ("n", (restantes.len() - 1).to_string())])
                });
                let ver = gtk::Button::with_label(tr("See"));
                ver.add_css_class("desligando-ver");
                let w = Rc::downgrade(self);
                ver.connect_clicked(move |_| if let Some(s) = w.upgrade() { s.cancel(Some(id)) });
                apps.extra.append(&ver);
                if debug() {
                    eprintln!("debug desligando: esperando «{nome}»");
                    crate::central::posicao_debug("desligando ver", ver.upcast_ref());
                }
            }
        }
        // the agent and the routines
        let trabalhando = sh.estado.agent_task.borrow().is_some();
        let rodando: Vec<u64> = sh.agendador().map(|a| a.rodando.borrow().keys().copied().collect()).unwrap_or_default();
        for a in self.agents.borrow().iter().filter(|a| !a.done.get()) {
            let acabou = match a.rotina { None => !trabalhando, Some(id) => !rodando.contains(&id) };
            if acabou {
                a.done.set(true);
                a.step.set(Mark::Done, &trf("{task} finished", &[("task", short(&a.pedido))]));
            }
        }
        let agentes_pendentes = self.agents.borrow().iter().any(|a| !a.done.get());
        if agentes_pendentes && inicio.elapsed() >= AGENTS_WAIT {
            self.stop_agents();
        }
        if restantes.is_empty() && !self.agents.borrow().iter().any(|a| !a.done.get()) {
            self.finish(false);
        }
    }

    /// The agent's request and the routines that did not finish: stopped and kept for the next session.
    fn stop_agents(&self) {
        let sh = crate::shell_atual();
        for a in self.agents.borrow().iter().filter(|a| !a.done.get()) {
            a.done.set(true);
            match a.rotina {
                None => {
                    let passos = sh.as_ref().and_then(|s| s.estado.agent_task.borrow().as_ref().map(|(_, n)| *n)).unwrap_or(0);
                    keep_interrupted(&a.pedido, passos, None);
                    if let Some(s) = &sh {
                        s.right_bar().conversa.stop(false);
                    }
                    a.step.set(Mark::Done, &trf("Agent stopped at step {n}, kept to continue", &[("n", passos.max(1).to_string())]));
                    if debug() {
                        eprintln!("debug desligando: agente parado no passo {passos}");
                    }
                }
                Some(id) => {
                    keep_interrupted(&a.pedido, 0, Some(&a.pedido));
                    if let Some(ag) = sh.as_ref().and_then(|s| s.agendador()) {
                        ag.parar(id);
                    }
                    a.step.set(Mark::Done, &trf("{routine} stopped, kept to continue", &[("routine", a.pedido.clone())]));
                    if debug() {
                        eprintln!("debug desligando: rotina {} parada", a.pedido);
                    }
                }
            }
        }
    }

    /// Everything wrapped up (or "Desligar mesmo assim"): the power action.
    fn finish(self: &Rc<Self>, forced: bool) {
        if self.finished.replace(true) {
            return;
        }
        if forced {
            self.stop_agents();
        }
        if debug() {
            eprintln!("debug desligando: {}", if forced { "desligando mesmo assim" } else { "tudo encerrado" });
        }
        let action = self.action.borrow().clone();
        let install = self.install.get();
        // the shutdown's sound (not unattended: the night's restart under the lock screen is quiet), and a moment to
        // read the last line — as long as the sound, never longer than SESSION_END_WAIT — then the action (the
        // session that is only a test stays, its screen goes)
        let sound = if self.unattended.borrow().is_none() { crate::configuracoes_som::tocar("system-shutdown") } else { None };
        let wait = sound.map(|d| d.min(crate::configuracoes_som::SESSION_END_WAIT)).unwrap_or_default().max(Duration::from_millis(700));
        let w = Rc::downgrade(self);
        glib::timeout_add_local_once(wait, move || {
            crate::energia::Energia::power(&action, install);
            if let Some(s) = w.upgrade() {
                if crate::energia::dev() {
                    s.close();
                }
            }
        });
    }

    /// Cancelled (Esc, or "Ver" on an app that is waiting): the screen goes and the app comes forward.
    fn cancel(self: &Rc<Self>, focus: Option<u64>) {
        if self.finished.get() {
            return;
        }
        self.finished.set(true);
        if debug() {
            eprintln!("debug desligando: cancelado");
        }
        self.close();
        if let Some(id) = focus {
            let _ = crate::servicos::niri_acao(json!({"FocusWindow": {"id": id}}));
        }
    }

    fn close(self: &Rc<Self>) {
        self.started.set(None);
        self.unattended.borrow_mut().take();
        self.window.set_keyboard_mode(KeyboardMode::None);
        let w = self.window.downgrade();
        self.all.ir(Canal::Opacidade, 0.0, LENTA);
        self.all.ao_parar(move || {
            if let Some(w) = w.upgrade() {
                w.set_visible(false);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stopped_request_comes_back_with_its_steps() {
        assert_eq!(continue_request("organizar Downloads", 0), "organizar Downloads");
        let c = continue_request("organizar Downloads", 3);
        assert!(c.contains("organizar Downloads") && c.contains('3'));
        assert_eq!(short("curto"), "curto");
        let ids = vec!["org.gnome.TextEditor".to_string(), "gnome-text-editor".to_string()];
        assert_eq!(same_app("gnome-text-editor", &ids), "org.gnome.TextEditor");
        assert!(short(&"x".repeat(80)).ends_with('…'));
    }
}
