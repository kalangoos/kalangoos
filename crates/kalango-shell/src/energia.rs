//! Menu de energia (página "12 · Janelas, busca e sistema", quadro "Menu de energia"): bloquear, suspender,
//! reiniciar, desligar e sair, pelo logind (D-Bus) e pelo niri. Com `KALANGO_SHELL_DEV=1` (sessão de teste numa
//! janela), as ações que afetariam o computador real só são registradas; "Sair" fecha a sessão de teste.
//!
//! Real data, as in the design: the person's photo (~/.face or AccountsService), how long the computer has been
//! on, the agents at work (with "wait for them to finish": restarting, shutting down or logging out waits until
//! the agent is done) and an update prepared by PackageKit, installed on the way out when ticked.
//! Shut down is the default (Enter); the footer names the action Enter will do.
//!
//! Movimento: o fundo escurece (mola rápida, ≈ 180 ms) e o painel cresce do ícone de energia (mola suave); Esc o
//! devolve ao ícone (mola rápida). Escolher uma ação afunda o bloco para 0,97 e dissolve a tela (≈ 320 ms, mola
//! lenta) antes de executar.

use crate::conversa::ui_icon;
use crate::servicos::niri_acao;
use crate::superficies::{caixa, camada, rotulo};
use crate::Shell;
use gtk::prelude::*;
use gtk::{gdk, gio, glib, Align, Orientation};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kalango_i18n::{n_, tr, trf};
use kalango_movimento::{self as mv, Animavel, Canal};
use serde_json::json;
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::time::Duration;

const TEXT: &str = "#E2E2E6";
const CORAL: &str = "#EFA2A6";

pub struct Energia {
    janela: gtk::ApplicationWindow,
    tudo: Animavel,
    fundo: Animavel,
    cartao: Animavel,
    botoes: RefCell<Vec<(&'static str, gtk::Button)>>,
    /// ponto da tela de onde o menu nasce (centro do ícone de energia)
    pub origem: Cell<(f64, f64)>,
    aberta: Cell<bool>,
    shell: RefCell<Weak<Shell>>,
    uptime: gtk::Label,
    agents: gtk::Box,
    agents_title: gtk::Label,
    agents_note: gtk::Label,
    wait: gtk::Switch,
    update: gtk::CheckButton,
    enter_label: gtk::Label,
    /// the action Enter does (the focused one)
    current: Cell<&'static str>,
    /// an action waiting for the agent to finish
    waiting: Cell<Option<&'static str>>,
}

/// (id, name, glyph)
const ACOES: &[(&str, &str, &str)] = &[
    ("bloquear", n_("Lock"), "cadeado"),
    ("suspender", n_("Suspend"), "lua"),
    ("reiniciar", n_("Restart"), "reiniciar"),
    ("desligar", n_("Shut down"), "desligar"),
    ("sair", n_("Log out"), "sair"),
];

pub(crate) fn dev() -> bool {
    std::env::var_os("KALANGO_SHELL_DEV").is_some()
}

fn ligado_ha() -> String {
    let s: f64 = std::fs::read_to_string("/proc/uptime").ok().and_then(|t| t.split_whitespace().next()?.parse().ok()).unwrap_or(0.0);
    let (d, h, m) = ((s / 86400.0) as u64, ((s % 86400.0) / 3600.0) as u64, ((s % 3600.0) / 60.0) as u64);
    match (d, h) {
        (0, 0) => trf("up for {m} min", &[("m", m.to_string())]),
        (0, _) => trf("up for {h} h {m} min", &[("h", h.to_string()), ("m", m.to_string())]),
        _ => trf("up for {d} d {h} h", &[("d", d.to_string()), ("h", h.to_string())]),
    }
}

pub(crate) fn person_name() -> String {
    let nome = glib::real_name().to_string_lossy().split(',').next().unwrap_or("").trim().to_string();
    if nome.is_empty() || nome == "Unknown" { glib::user_name().to_string_lossy().into_owned() } else { nome }
}

/// The person's photo: ~/.face (the desktop convention) or the one AccountsService keeps.
pub(crate) fn photo_path() -> Option<PathBuf> {
    let home = glib::home_dir();
    [home.join(".face"), home.join(".face.icon"), PathBuf::from("/var/lib/AccountsService/icons").join(glib::user_name())]
        .into_iter().find(|p| p.is_file())
}

pub(crate) fn avatar(name: &str, size: i32, class: &str) -> gtk::Widget {
    let b = caixa(Orientation::Vertical, 0, class);
    b.set_size_request(size, size);
    b.set_halign(Align::Center);
    b.set_overflow(gtk::Overflow::Hidden);
    match photo_path() {
        Some(p) => {
            let pic = gtk::Picture::for_filename(p);
            pic.set_content_fit(gtk::ContentFit::Cover);
            pic.set_size_request(size, size);
            b.append(&pic);
        }
        None => {
            let l = rotulo(&name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default(), "energia-inicial");
            l.set_vexpand(true);
            b.append(&l);
        }
    }
    b.upcast()
}

fn action_name(id: &str) -> &'static str {
    ACOES.iter().find(|(i, _, _)| *i == id).map(|(_, n, _)| tr(n)).unwrap_or("")
}

/// Restarting, shutting down and logging out end the agent's work; locking and suspending do not.
fn ends_agents(id: &str) -> bool {
    matches!(id, "reiniciar" | "desligar" | "sair")
}

impl Energia {
    pub fn new(app: &gtk::Application) -> Rc<Energia> {
        let janela = camada(app, "kalango-energia", Layer::Overlay, &[Edge::Top, Edge::Bottom, Edge::Left, Edge::Right]);
        janela.set_exclusive_zone(-1);
        let escuro = caixa(Orientation::Vertical, 0, "fundo");
        escuro.set_hexpand(true);
        escuro.set_vexpand(true);
        let cartao = caixa(Orientation::Vertical, 18, "energia");
        cartao.set_size_request(600, -1);
        let nome = person_name();
        cartao.append(&avatar(&nome, 56, "energia-avatar"));
        let quem = caixa(Orientation::Vertical, 2, "");
        quem.set_halign(Align::Center);
        quem.append(&rotulo(&nome, "energia-nome"));
        let uptime = rotulo(&ligado_ha(), "energia-ligado");
        quem.append(&uptime);
        cartao.append(&quem);
        let linha = caixa(Orientation::Horizontal, 10, "");
        linha.set_homogeneous(true);
        cartao.append(&linha);

        // the agents at work
        let agents = caixa(Orientation::Horizontal, 12, "energia-agentes");
        agents.append(&ui_icon("spark", "#A5A1E2", 16));
        let col = caixa(Orientation::Vertical, 2, "");
        col.set_hexpand(true);
        col.set_valign(Align::Center);
        let agents_title = rotulo("", "energia-agentes-titulo");
        agents_title.set_xalign(0.0);
        col.append(&agents_title);
        let agents_note = rotulo("", "energia-agentes-nota");
        agents_note.set_xalign(0.0);
        agents_note.set_wrap(true);
        col.append(&agents_note);
        agents.append(&col);
        let wait = gtk::Switch::new();
        wait.set_valign(Align::Center);
        wait.set_tooltip_text(Some(tr("Wait for the agents to finish before restarting, shutting down or logging out")));
        agents.append(&wait);
        cartao.append(&agents);

        // an update ready to install on the way out
        let update = gtk::CheckButton::new();
        update.add_css_class("energia-atualizar");
        update.set_active(true);
        update.set_halign(Align::Start);
        cartao.append(&update);

        let rodape = caixa(Orientation::Horizontal, 14, "");
        rodape.set_halign(Align::Center);
        let esc = caixa(Orientation::Horizontal, 4, "");
        esc.append(&rotulo("Esc", "tecla"));
        esc.append(&rotulo(tr("cancel"), "energia-rodape"));
        rodape.append(&esc);
        let ent = caixa(Orientation::Horizontal, 4, "");
        ent.append(&rotulo("↵", "tecla"));
        let enter_label = rotulo("", "energia-rodape");
        ent.append(&enter_label);
        rodape.append(&ent);
        cartao.append(&rodape);
        if dev() {
            let a = rotulo(tr("test session: lock, suspend, restart and shut down are only logged"), "aviso-dev");
            a.set_margin_top(-8);
            cartao.append(&a);
        }

        let raiz = gtk::Overlay::new();
        let fundo = Animavel::new(&escuro);
        raiz.set_child(Some(&fundo));
        let cartao_a = Animavel::new(&cartao);
        cartao_a.set_halign(Align::Center);
        cartao_a.set_valign(Align::Center);
        raiz.add_overlay(&cartao_a);
        let tudo = Animavel::new(&raiz);
        janela.set_child(Some(&tudo));
        let e = Rc::new(Energia {
            janela, tudo, fundo, cartao: cartao_a, botoes: RefCell::default(), origem: Cell::new((-1.0, -1.0)), aberta: Cell::new(false),
            shell: RefCell::default(), uptime, agents, agents_title, agents_note, wait, update, enter_label,
            current: Cell::new("desligar"), waiting: Cell::new(None),
        });
        for (id, r, glyph) in ACOES.iter() {
            let b = gtk::Button::new();
            b.add_css_class("acao");
            if *id == "desligar" {
                b.add_css_class("perigo");
            }
            b.set_focusable(true);
            b.set_size_request(102, 77);
            let col = caixa(Orientation::Vertical, 8, "");
            col.set_valign(Align::Start);
            col.append(&ui_icon(glyph, if *id == "desligar" { CORAL } else { TEXT }, 24));
            col.append(&rotulo(tr(r), "acao-nome"));
            b.set_child(Some(&col));
            let w = Rc::downgrade(&e);
            let id: &'static str = id;
            b.connect_clicked(move |_| {
                if let Some(e) = w.upgrade() {
                    e.escolher(id);
                }
            });
            // Enter does the focused action: the footer and the agents' note follow it
            let w = Rc::downgrade(&e);
            b.connect_has_focus_notify(move |b| {
                if let (Some(e), true) = (w.upgrade(), b.has_focus()) {
                    e.current.set(id);
                    e.refresh_labels();
                }
            });
            linha.append(&b);
            e.botoes.borrow_mut().push((id, b));
        }
        let teclas = gtk::EventControllerKey::new();
        {
            let w = Rc::downgrade(&e);
            teclas.connect_key_pressed(move |_, k, _, _| {
                if k == gdk::Key::Escape {
                    if let Some(e) = w.upgrade() {
                        e.fechar();
                    }
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
        }
        e.janela.add_controller(teclas);
        e
    }

    pub fn ligar(&self, shell: &Rc<Shell>) {
        *self.shell.borrow_mut() = Rc::downgrade(shell);
    }

    fn agent_busy(&self) -> Option<String> {
        let s = self.shell.borrow().upgrade()?;
        if !s.estado.agent_working.get() {
            return None;
        }
        let task = s.estado.agent_task.borrow().as_ref().map(|(t, _)| t.clone()).unwrap_or_default();
        Some(task)
    }

    fn refresh_labels(&self) {
        let id = self.current.get();
        self.enter_label.set_text(&action_name(id).to_lowercase());
        if let Some(waiting) = self.waiting.get() {
            self.agents_note.set_text(&trf("{action} as soon as the agent finishes · Esc cancels", &[("action", action_name(waiting).to_string())]));
            return;
        }
        self.agents_note.set_text(if ends_agents(id) {
            tr("It will be stopped. Wait for it to finish?")
        } else {
            tr("It keeps working (it pauses while the computer sleeps).")
        });
    }

    /// The agents block and the update line, read when the menu opens.
    fn refresh_state(self: &Rc<Self>) {
        self.uptime.set_text(&ligado_ha());
        let busy = self.agent_busy();
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug energia: {}", if busy.is_some() { "agente em execução" } else { "nenhum agente" });
        }
        match busy {
            Some(task) => {
                self.agents.set_visible(true);
                self.agents_title.set_text(&if task.is_empty() { tr("1 agent running").to_string() } else { trf("1 agent running: {task}", &[("task", task)]) });
                self.agents_title.set_ellipsize(gtk::pango::EllipsizeMode::End);
                self.agents_title.set_max_width_chars(60);
            }
            None => self.agents.set_visible(false),
        }
        self.update.set_visible(false);
        let w = Rc::downgrade(self);
        prepared_update(move |n| {
            let Some(e) = w.upgrade() else { return };
            if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() && n > 0 {
                eprintln!("debug energia: atualização preparada ({n})");
            }
            if n > 0 {
                e.update.set_label(Some(&if n == 1 { tr("Install 1 update when shutting down").to_string() } else { trf("Install {n} updates when shutting down", &[("n", n.to_string())]) }));
                e.update.set_visible(true);
            }
        });
    }

    /// Deslocamento do centro do painel até a origem (ícone de energia), em px.
    fn ate_origem(&self) -> (f64, f64) {
        let (ox, oy) = self.origem.get();
        if ox < 0.0 {
            return (0.0, -24.0);
        }
        let (w, h) = (self.janela.width() as f64, self.janela.height() as f64);
        (ox - w / 2.0, oy - h / 2.0)
    }

    pub fn abrir(self: &Rc<Self>) {
        self.aberta.set(true);
        self.waiting.set(None);
        self.wait.set_active(false);
        for (_, b) in self.botoes.borrow().iter() {
            b.set_sensitive(true);
        }
        self.refresh_state();
        self.current.set("desligar");
        self.refresh_labels();
        self.janela.present();
        let w = self.janela.downgrade();
        glib::timeout_add_local_once(Duration::from_millis(40), move || {
            if let Some(j) = w.upgrade() {
                crate::superficies::take_keyboard(&j);
            }
        });
        self.tudo.definir(Canal::Opacidade, 1.0).definir(Canal::Escala, 1.0);
        self.cartao.definir(Canal::Escala, 1.0);
        self.fundo.definir(Canal::Opacidade, 0.0);
        self.fundo.ir(Canal::Opacidade, 1.0, mv::RAPIDA);
        // o tamanho da janela só existe depois de mapear: começa no próximo quadro
        let c = self.cartao.downgrade();
        let alvo = self.ate_origem();
        self.cartao.definir(Canal::Opacidade, 0.0);
        let janela = self.janela.downgrade();
        let origem = self.origem.get();
        glib::idle_add_local_once(move || {
            let (Some(c), Some(j)) = (c.upgrade(), janela.upgrade()) else { return };
            let de = if origem.0 < 0.0 { alvo } else { (origem.0 - j.width() as f64 / 2.0, origem.1 - j.height() as f64 / 2.0) };
            c.entrar(de, 0.2);
        });
        // test aid: where the "wait" switch is (the window covers the whole output)
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            let (sw, j) = (self.wait.clone(), self.janela.clone());
            glib::timeout_add_local_once(Duration::from_millis(900), move || {
                if let Some(b) = sw.compute_bounds(&j).filter(|_| sw.is_mapped()) {
                    eprintln!("debug energia: interruptor {} {}", (b.x() + b.width() / 2.) as i32, (b.y() + b.height() / 2.) as i32);
                }
            });
        }
        // Enter shuts down, as in the design
        if let Some((_, b)) = self.botoes.borrow().iter().find(|(id, _)| *id == "desligar") {
            b.grab_focus();
        }
    }

    pub fn fechar(&self) {
        if !self.aberta.replace(false) {
            return;
        }
        self.waiting.set(None);
        self.janela.set_keyboard_mode(KeyboardMode::None);
        self.fundo.ir(Canal::Opacidade, 0.0, mv::RAPIDA);
        let j = self.janela.downgrade();
        self.cartao.sair(self.ate_origem(), 0.2, move || {
            if let Some(j) = j.upgrade() {
                j.set_visible(false);
            }
        });
    }

    pub fn alternar(self: &Rc<Self>) {
        if self.aberta.get() { self.fechar() } else { self.abrir() }
    }

    /// A choice in the menu: with "wait" on and the agent at work, an action that would end its work waits.
    fn escolher(self: &Rc<Self>, id: &'static str) {
        if self.wait.is_active() && ends_agents(id) && self.agent_busy().is_some() {
            self.waiting.set(Some(id));
            if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                eprintln!("debug energia: esperando o agente para {id}");
            }
            self.refresh_labels();
            for (_, b) in self.botoes.borrow().iter() {
                b.set_sensitive(false);
            }
            let w = Rc::downgrade(self);
            glib::timeout_add_local(Duration::from_millis(500), move || {
                let Some(e) = w.upgrade() else { return glib::ControlFlow::Break };
                if e.waiting.get() != Some(id) || !e.aberta.get() {
                    return glib::ControlFlow::Break;
                }
                if e.agent_busy().is_none() {
                    e.waiting.set(None);
                    e.executar(id);
                    return glib::ControlFlow::Break;
                }
                glib::ControlFlow::Continue
            });
            return;
        }
        self.executar(id);
    }

    /// Escolheu uma ação: o bloco afunda, a tela dissolve e só então a ação acontece.
    pub fn executar(&self, id: &str) {
        let id = id.to_string();
        // the test session locks only itself (logind would lock the real desktop around it)
        if id == "bloquear" && dev() {
            if self.aberta.get() {
                self.fechar();
            }
            if let Some(s) = self.shell.borrow().upgrade() {
                s.bloqueio().bloquear();
            }
            return;
        }
        let install = self.update.is_visible() && self.update.is_active() && matches!(id.as_str(), "reiniciar" | "desligar");
        if !self.aberta.get() {
            Self::fazer(&id, install);
            return;
        }
        self.aberta.set(false);
        self.janela.set_keyboard_mode(KeyboardMode::None);
        self.cartao.ir(Canal::Escala, 0.97, mv::RAPIDA);
        self.tudo.ir(Canal::Opacidade, 0.0, mv::LENTA);
        let j = self.janela.downgrade();
        self.tudo.ao_parar(move || {
            if let Some(j) = j.upgrade() {
                j.set_visible(false);
            }
            Self::fazer(&id, install);
        });
    }

    pub(crate) fn fazer(id: &str, install_update: bool) {
        // shutting down and restarting first wrap up the apps and the agents (design "Desligando")
        if matches!(id, "desligar" | "reiniciar") {
            crate::shutdown_screen::start(id, install_update);
            return;
        }
        Self::power(id, install_update);
    }

    /// The action itself (logind; PackageKit first when an update is prepared).
    pub(crate) fn power(id: &str, install_update: bool) {
        if id == "sair" {
            // the logout's sound first: the session ends when it does (no longer than it lasts)
            let wait = crate::configuracoes_som::tocar("desktop-logout").map(|d| d.min(crate::configuracoes_som::SESSION_END_WAIT)).unwrap_or_default();
            glib::timeout_add_local_once(wait, || {
                let _ = niri_acao(json!({"Quit": {"skip_confirmation": true}}));
            });
            return;
        }
        if dev() {
            let extra = if install_update { " (com a atualização preparada)" } else { "" };
            eprintln!("kalango-shell: [sessão de teste] '{id}'{extra} não foi executado de verdade");
            return;
        }
        // the live session of the install media: restart and power off skip stopping the units (a normal one stopped
        // forever on "Failed unmounting cdrom.mount" — the session runs from the media); nothing there is kept anyway
        if matches!(id, "reiniciar" | "desligar") && crate::configuracoes_energia::live_session() {
            let what = if id == "desligar" { "poweroff" } else { "reboot" };
            let _ = std::process::Command::new("sudo").args(["-n", "systemctl", what, "--force"]).spawn();
            return;
        }
        let (metodo, args) = match id {
            "bloquear" => ("LockSessions", None),
            "suspender" => ("Suspend", Some(true)),
            "reiniciar" => ("Reboot", Some(true)),
            // with an update, PackageKit installs it on the way and then powers off: the machine restarts into it
            "desligar" if install_update => ("Reboot", Some(true)),
            "desligar" => ("PowerOff", Some(true)),
            _ => return,
        };
        let params = args.map(|b| glib::Variant::tuple_from_iter([b.to_variant()]));
        let after = if id == "desligar" { "power-off" } else { "reboot" };
        gio::bus_get(gio::BusType::System, gio::Cancellable::NONE, move |c| {
            let Ok(c) = c else { return };
            let logind = move |c: &gio::DBusConnection| {
                c.call(Some("org.freedesktop.login1"), "/org/freedesktop/login1", "org.freedesktop.login1.Manager", metodo, params.as_ref(), None,
                    gio::DBusCallFlags::ALLOW_INTERACTIVE_AUTHORIZATION, -1, gio::Cancellable::NONE, |r| {
                        if let Err(e) = r {
                            eprintln!("kalango-shell: logind: {e}");
                        }
                    });
            };
            if !install_update {
                logind(&c);
                return;
            }
            let c2 = c.clone();
            c.call(Some("org.freedesktop.PackageKit"), "/org/freedesktop/PackageKit", "org.freedesktop.PackageKit.Offline", "Trigger",
                Some(&glib::Variant::tuple_from_iter([after.to_variant()])), None, gio::DBusCallFlags::ALLOW_INTERACTIVE_AUTHORIZATION, -1,
                gio::Cancellable::NONE, move |r| {
                    if let Err(e) = r {
                        // the update stays for next time; the chosen action still happens
                        eprintln!("kalango-shell: PackageKit: {e}");
                    }
                    logind(&c2);
                });
        });
    }
}

/// PackageKit lives on the system bus; tests use a stand-in on the session bus (KALANGO_PACKAGEKIT_BUS=session).
fn packagekit_bus() -> gio::BusType {
    if std::env::var("KALANGO_PACKAGEKIT_BUS").as_deref() == Ok("session") { gio::BusType::Session } else { gio::BusType::System }
}

/// How many packages PackageKit has downloaded and prepared to install offline (0 when none or no PackageKit).
/// PackageKit starts on demand: a prepared update shows even if the daemon was not running.
pub(crate) fn prepared_update(done: impl FnOnce(usize) + 'static) {
    gio::bus_get(packagekit_bus(), gio::Cancellable::NONE, move |c| {
        let Ok(c) = c else { return done(0) };
        let c2 = c.clone();
        c.call(Some("org.freedesktop.PackageKit"), "/org/freedesktop/PackageKit", "org.freedesktop.DBus.Properties", "Get",
            Some(&("org.freedesktop.PackageKit.Offline", "UpdatePrepared").to_variant()), None, gio::DBusCallFlags::NONE, 8000,
            gio::Cancellable::NONE, move |r| {
                let prepared = r.ok().and_then(|v| v.child_value(0).as_variant()).and_then(|v| v.get::<bool>()).unwrap_or(false);
                if !prepared {
                    return done(0);
                }
                c2.call(Some("org.freedesktop.PackageKit"), "/org/freedesktop/PackageKit", "org.freedesktop.PackageKit.Offline", "GetPrepared",
                    None, None, gio::DBusCallFlags::NONE, 8000, gio::Cancellable::NONE, move |r| {
                        let n = r.ok().and_then(|v| v.child_value(0).get::<Vec<String>>()).map(|v| v.len()).unwrap_or(0);
                        done(n);
                    });
            });
    });
}
