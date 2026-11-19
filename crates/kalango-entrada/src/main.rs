//! KalangoOS · tela de entrada (design "05 · Telas do sistema e apps", frame "Tela de entrada (login e troca de
//! usuário)"): the greeter greetd runs (GREETD_SOCK). The lock screen's clock in the middle, the people side by side
//! (the one chosen in the middle, bigger, with the password field under it), the guest when the system has one;
//! the session and the keyboard layout at the bottom left; large text, the network and power at the bottom right.
//!
//! - Choosing another person brings them to the middle (gentle spring) and the field is born under them.
//! - Wrong password: a horizontal shake, 3× in 240 ms. Right: the card shrinks and the screen dissolves (320 ms),
//!   then greetd starts the session (a person with a session already open goes back to it).
//!
//! `KALANGO_ENTRADA_TESTE=1`: the power buttons only say what they would do (tests). Without layer-shell (X11) it
//! opens as a full-screen window.

mod greetd;
mod pessoas;
mod sessoes;

use gtk::prelude::*;
use gtk::{gdk, gio, glib, Align, Orientation};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kalango_i18n::{tr, trf};
use kalango_movimento::{self as mv, Animavel, Canal};
use pessoas::Pessoa;
use sessoes::Sessao;
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

fn teste() -> bool {
    std::env::var_os("KALANGO_ENTRADA_TESTE").is_some()
}

fn debug() -> bool {
    std::env::var_os("KALANGO_ENTRADA_DEBUG").is_some() || teste()
}

fn caixa(o: Orientation, gap: i32, classe: &str) -> gtk::Box {
    let b = gtk::Box::new(o, gap);
    if !classe.is_empty() {
        b.add_css_class(classe);
    }
    b
}

fn rotulo(t: &str, classe: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(t));
    if !classe.is_empty() {
        l.add_css_class(classe);
    }
    l
}

/// The design's glyphs (the shell's), in one colour.
fn glifo(nome: &str, cor: &str, px: i32) -> gtk::Image {
    let svg = match nome {
        "cadeado" => include_str!("../../kalango-shell/assets/ui/cadeado.svg"),
        "seta-direita" => include_str!("../../kalango-shell/assets/ui/seta-direita.svg"),
        "monitor" => include_str!("../../kalango-shell/assets/ui/monitor.svg"),
        "teclado" => include_str!("../../kalango-shell/assets/ui/teclado.svg"),
        "olho" => include_str!("../../kalango-shell/assets/ui/olho.svg"),
        "wifi" => include_str!("../../kalango-shell/assets/ui/wifi.svg"),
        "desligar" => include_str!("../../kalango-shell/assets/ui/desligar.svg"),
        "usuario" => include_str!("../../kalango-shell/assets/ui/usuario.svg"),
        _ => "",
    };
    let mut out = String::new();
    let mut resto = svg;
    while let Some(i) = resto.find("=\"#") {
        out.push_str(&resto[..i + 2]);
        out.push_str(cor);
        resto = &resto[i + 2 + 7..];
    }
    out.push_str(resto);
    let img = gtk::Image::from_paintable(gdk::Texture::from_bytes(&glib::Bytes::from_owned(out.into_bytes())).ok().as_ref());
    img.set_pixel_size(px);
    img.set_size_request(px, px);
    // a paintable fills its allocation: centred, it stays at the glyph's size
    img.set_halign(Align::Center);
    img.set_valign(Align::Center);
    img
}

/// The wallpaper for this screen's proportion (the same versions as the session's).
fn papel(w: i32, h: i32) -> Option<PathBuf> {
    let v = |env: &str, f: &str| std::env::var_os(env).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/usr/share/kalango").join(f));
    let a = (f64::from(w.max(1)) / f64::from(h.max(1))).ln();
    [(16.0 / 9.0, v("KALANGO_PAPEL_DE_PAREDE", "papel-de-parede.png")), (9.0 / 16.0, v("KALANGO_PAPEL_DE_PAREDE_ALTO", "papel-de-parede-alto.png")),
        (3840.0 / 1648.0, v("KALANGO_PAPEL_DE_PAREDE_LARGO", "papel-de-parede-largo.png"))]
        .into_iter().filter(|(_, p)| p.is_file()).min_by(|x, y| (f64::ln(x.0) - a).abs().total_cmp(&(f64::ln(y.0) - a).abs())).map(|(_, p)| p)
}

/// Where the greeter remembers who and which session came last (its own state folder: $XDG_STATE_HOME or
/// ~/.local/state of the greeter's account).
fn ultimo_path() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).filter(|p| p.is_absolute())
        .unwrap_or_else(|| glib::home_dir().join(".local/state")).join("kalango-entrada/ultimo.json")
}

fn avatar(p: &Pessoa, px: i32, classe: &str) -> gtk::Widget {
    let b = caixa(Orientation::Vertical, 0, classe);
    b.set_size_request(px, px);
    b.set_halign(Align::Center);
    b.set_overflow(gtk::Overflow::Hidden);
    if p.convidado {
        let i = glifo("usuario", "#8E8F99", px / 4 + 4);
        i.set_vexpand(true);
        i.set_valign(Align::Center);
        b.append(&i);
    } else if let Some(f) = &p.foto {
        let pic = gtk::Picture::for_filename(f);
        pic.set_content_fit(gtk::ContentFit::Cover);
        pic.set_size_request(px, px);
        b.append(&pic);
    } else {
        let l = rotulo(&p.nome.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default(), "entrada-inicial");
        l.set_vexpand(true);
        b.append(&l);
    }
    b.upcast()
}

struct Entrada {
    janela: gtk::ApplicationWindow,
    tudo: Animavel,
    pessoas: Vec<Pessoa>,
    sel: Cell<usize>,
    linha: gtk::Box,
    /// the people's springs, by login (to slide them when the choice changes)
    animaveis: RefCell<Vec<(String, Animavel)>>,
    cartao: RefCell<Option<Animavel>>,
    senha: RefCell<Option<gtk::PasswordEntry>>,
    erro: RefCell<Option<gtk::Label>>,
    sessoes: Vec<Sessao>,
    sessao: Cell<usize>,
    sessao_rotulo: gtk::Label,
    ocupado: Cell<bool>,
}

impl Entrada {
    fn new(app: &gtk::Application) -> Rc<Entrada> {
        let janela = gtk::ApplicationWindow::new(app);
        janela.add_css_class("entrada");
        if gtk4_layer_shell::is_supported() {
            janela.init_layer_shell();
            janela.set_namespace(Some("kalango-entrada"));
            janela.set_layer(Layer::Overlay);
            for e in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
                janela.set_anchor(e, true);
            }
            janela.set_exclusive_zone(-1);
            janela.set_keyboard_mode(KeyboardMode::Exclusive);
        } else {
            janela.fullscreen();
        }
        let raiz = gtk::Overlay::new();
        let fundo = caixa(Orientation::Vertical, 0, "entrada-fundo");
        let monitor = gdk::Display::default().and_then(|d| d.monitors().item(0)).and_downcast::<gdk::Monitor>().map(|m| m.geometry());
        if let Some(p) = monitor.and_then(|g| papel(g.width(), g.height())).or_else(|| papel(16, 9)) {
            let pic = gtk::Picture::for_filename(p);
            pic.set_content_fit(gtk::ContentFit::Cover);
            pic.set_hexpand(true);
            pic.set_vexpand(true);
            fundo.append(&pic);
        }
        raiz.set_child(Some(&fundo));
        let escurecer = caixa(Orientation::Vertical, 0, "entrada-escurecer");
        escurecer.set_can_target(false);
        raiz.add_overlay(&escurecer);

        // the middle: clock, date, people
        let centro = caixa(Orientation::Vertical, 30, "");
        centro.set_halign(Align::Center);
        centro.set_valign(Align::Center);
        let relogio = caixa(Orientation::Vertical, 2, "");
        let hora = rotulo("", "entrada-hora");
        let data = rotulo("", "entrada-data");
        relogio.append(&hora);
        relogio.append(&data);
        centro.append(&relogio);
        let linha = caixa(Orientation::Horizontal, 34, "");
        linha.set_halign(Align::Center);
        centro.append(&linha);
        let tudo = Animavel::new(&centro);
        raiz.add_overlay(&tudo);
        let tick = {
            let (hora, data) = (hora.clone(), data.clone());
            move || {
                if let Ok(d) = glib::DateTime::now_local() {
                    hora.set_text(&d.format("%H:%M").map(|s| s.to_string()).unwrap_or_default());
                    data.set_text(&d.format("%A, %-d de %B").map(|s| s.to_string()).unwrap_or_default());
                }
            }
        };
        tick();
        glib::timeout_add_seconds_local(1, move || {
            tick();
            glib::ControlFlow::Continue
        });

        // bottom left: session and keyboard
        let chips = caixa(Orientation::Horizontal, 8, "");
        chips.set_halign(Align::Start);
        chips.set_valign(Align::End);
        chips.set_margin_start(28);
        chips.set_margin_bottom(36);
        let sessao_b = gtk::MenuButton::new();
        sessao_b.add_css_class("entrada-chip");
        let sl = caixa(Orientation::Horizontal, 5, "");
        sl.append(&glifo("monitor", "#8E8F99", 16));
        let sessao_rotulo = rotulo("", "entrada-chip-texto");
        sl.append(&sessao_rotulo);
        sessao_b.set_child(Some(&sl));
        chips.append(&sessao_b);
        if let Some(t) = sessoes::teclado() {
            let k = caixa(Orientation::Horizontal, 5, "entrada-chip");
            k.append(&glifo("teclado", "#8E8F99", 16));
            k.append(&rotulo(&t, "entrada-chip-texto"));
            k.set_tooltip_text(Some(tr("Keyboard layout")));
            chips.append(&k);
        }
        raiz.add_overlay(&chips);
        // bottom right: large text, network, power
        let sistema = caixa(Orientation::Horizontal, 8, "");
        sistema.set_halign(Align::End);
        sistema.set_valign(Align::End);
        sistema.set_margin_end(36);
        sistema.set_margin_bottom(28);
        raiz.add_overlay(&sistema);
        janela.set_child(Some(&raiz));

        let mut pessoas = pessoas::listar();
        if pessoas.is_empty() {
            pessoas.push(Pessoa { usuario: String::new(), nome: tr("Someone").to_string(), foto: None, ultimo: None, sessao: None, convidado: false });
        }
        let ultimo: serde_json::Value = std::fs::read(ultimo_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let sel = pessoas.iter().position(|p| Some(p.usuario.as_str()) == ultimo["usuario"].as_str())
            // else whoever logged in last (on a tie, the first)
            .or_else(|| pessoas.iter().enumerate().filter(|(_, p)| !p.convidado).rev().max_by_key(|(_, p)| p.ultimo.unwrap_or(0)).map(|(i, _)| i))
            .unwrap_or(0);
        let lista_sessoes = sessoes::listar();
        let sessao = lista_sessoes.iter().position(|s| Some(s.id.as_str()) == ultimo["sessao"].as_str()).unwrap_or(0);
        let e = Rc::new(Entrada {
            janela, tudo, pessoas, sel: Cell::new(sel), linha, animaveis: RefCell::default(), cartao: RefCell::default(), senha: RefCell::default(),
            erro: RefCell::default(), sessoes: lista_sessoes, sessao: Cell::new(sessao), sessao_rotulo, ocupado: Cell::new(false),
        });
        e.menu_sessoes(&sessao_b);
        e.botoes_sistema(&sistema);
        e.mostrar_sessao();
        e.desenhar(None);
        e.tudo.entrar((0.0, 12.0), 0.98);
        e
    }

    fn mostrar_sessao(&self) {
        match self.sessoes.get(self.sessao.get()) {
            Some(s) => self.sessao_rotulo.set_text(&format!("{} · {}", s.nome, if s.wayland { "Wayland" } else { "X11" })),
            None => self.sessao_rotulo.set_text(tr("No session installed")),
        }
    }

    fn menu_sessoes(self: &Rc<Self>, b: &gtk::MenuButton) {
        let pop = gtk::Popover::new();
        pop.add_css_class("entrada-menu");
        let col = caixa(Orientation::Vertical, 2, "");
        for (i, s) in self.sessoes.iter().enumerate() {
            let it = gtk::Button::with_label(&format!("{} · {}", s.nome, if s.wayland { "Wayland" } else { "X11" }));
            it.add_css_class("entrada-menu-item");
            let (w, p) = (Rc::downgrade(self), pop.clone());
            it.connect_clicked(move |_| {
                if let Some(e) = w.upgrade() {
                    e.sessao.set(i);
                    e.mostrar_sessao();
                }
                p.popdown();
            });
            col.append(&it);
        }
        pop.set_child(Some(&col));
        b.set_popover(Some(&pop));
    }

    fn botoes_sistema(self: &Rc<Self>, caixa_s: &gtk::Box) {
        // large text (accessibility)
        let olho = gtk::ToggleButton::new();
        olho.add_css_class("entrada-icone");
        olho.set_child(Some(&glifo("olho", "#E2E2E6", 14)));
        olho.set_tooltip_text(Some(tr("Large text")));
        let j = self.janela.clone();
        olho.connect_toggled(move |b| if b.is_active() { j.add_css_class("texto-grande") } else { j.remove_css_class("texto-grande") });
        caixa_s.append(&olho);
        // the network (what is connected)
        let rede = gtk::Button::new();
        rede.add_css_class("entrada-icone");
        rede.set_child(Some(&glifo("wifi", "#E2E2E6", 14)));
        rede.set_tooltip_text(Some(tr("No network")));
        if let Ok(p) = gio::Subprocess::newv(&[std::ffi::OsStr::new("nmcli"), "-t".as_ref(), "-f".as_ref(), "NAME".as_ref(), "connection".as_ref(), "show".as_ref(), "--active".as_ref()],
            gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_SILENCE) {
            let r = rede.clone();
            p.communicate_utf8_async(None, gio::Cancellable::NONE, move |res| {
                if let Ok((Some(out), _)) = res {
                    if let Some(n) = out.lines().find(|l| !l.is_empty() && *l != "lo") {
                        r.set_tooltip_text(Some(&trf("Connected to {name}", &[("name", n.to_string())])));
                    }
                }
            });
        }
        caixa_s.append(&rede);
        // power (logind)
        let energia = gtk::MenuButton::new();
        energia.add_css_class("entrada-icone");
        energia.set_child(Some(&glifo("desligar", "#E2E2E6", 14)));
        energia.set_tooltip_text(Some(tr("Power")));
        let pop = gtk::Popover::new();
        pop.add_css_class("entrada-menu");
        let col = caixa(Orientation::Vertical, 2, "");
        for (metodo, nome) in [("Suspend", tr("Suspend")), ("Reboot", tr("Restart")), ("PowerOff", tr("Shut down"))] {
            let it = gtk::Button::with_label(nome);
            it.add_css_class("entrada-menu-item");
            let p = pop.clone();
            it.connect_clicked(move |_| {
                p.popdown();
                if teste() {
                    eprintln!("debug entrada: energia {metodo} (teste: nada acontece)");
                    return;
                }
                if let Ok(bus) = gio::bus_get_sync(gio::BusType::System, gio::Cancellable::NONE) {
                    bus.call(Some("org.freedesktop.login1"), "/org/freedesktop/login1", "org.freedesktop.login1.Manager", metodo,
                        Some(&(true,).to_variant()), None, gio::DBusCallFlags::NONE, 5000, gio::Cancellable::NONE, |_| {});
                }
            });
            col.append(&it);
        }
        pop.set_child(Some(&col));
        energia.set_popover(Some(&pop));
        caixa_s.append(&energia);
    }

    /// The people: the chosen one in the middle. `antes`: where each one was (to slide them from there).
    fn desenhar(self: &Rc<Self>, antes: Option<Vec<(String, f32)>>) {
        while let Some(c) = self.linha.first_child() {
            self.linha.remove(&c);
        }
        self.animaveis.borrow_mut().clear();
        let sel = self.sel.get();
        // the others around the chosen one: half on each side, the guest at the far right
        let outros: Vec<usize> = (0..self.pessoas.len()).filter(|i| *i != sel).collect();
        let (comuns, convidado): (Vec<usize>, Vec<usize>) = outros.iter().partition(|i| !self.pessoas[**i].convidado);
        // (with one other, it goes on the left: the chosen one stays in the middle)
        let metade = comuns.len().div_ceil(2);
        let ordem: Vec<usize> = comuns[..metade].iter().copied().chain([sel]).chain(comuns[metade..].iter().copied()).chain(convidado).collect();
        let agora = glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0);
        for i in ordem {
            let p = &self.pessoas[i];
            let w: gtk::Widget = if i == sel { self.escolhido(p, agora) } else { self.outro(i, p, agora) };
            let a = Animavel::new(&w);
            a.set_valign(Align::Start);
            self.linha.append(&a);
            self.animaveis.borrow_mut().push((p.usuario.clone(), a));
        }
        // smart animate: each one slides from where it was
        if let Some(antes) = antes {
            let w = Rc::downgrade(self);
            glib::idle_add_local_once(move || {
                let Some(e) = w.upgrade() else { return };
                for (u, a) in e.animaveis.borrow().iter() {
                    if let (Some((_, x0)), Some(b)) = (antes.iter().find(|(v, _)| v == u), a.compute_bounds(&e.janela)) {
                        let dx = f64::from(*x0 - (b.x() + b.width() / 2.0));
                        a.definir(Canal::X, dx);
                        a.ir(Canal::X, 0.0, mv::SUAVE);
                    }
                }
            });
        }
        if debug() {
            // (the cards' places for the session test, once they have a size: measured again while the first layout
            // is not done)
            let w = Rc::downgrade(self);
            let tentativas = std::rc::Rc::new(std::cell::Cell::new(0));
            glib::timeout_add_local(Duration::from_millis(300), move || {
                let Some(e) = w.upgrade() else { return glib::ControlFlow::Break };
                let lugares: Vec<(String, gtk::graphene::Rect)> = e.animaveis.borrow().iter()
                    .filter_map(|(u, a)| a.compute_bounds(&e.janela).map(|b| (u.clone(), b))).collect();
                tentativas.set(tentativas.get() + 1);
                if lugares.iter().any(|(_, b)| b.width() < 1.0) && tentativas.get() < 20 {
                    return glib::ControlFlow::Continue;
                }
                for (u, b) in lugares {
                    eprintln!("debug entrada: {u} em {} {}", (b.x() + b.width() / 2.0) as i32, (b.y() + 40.0) as i32);
                }
                glib::ControlFlow::Break
            });
        }
    }

    fn escolhido(self: &Rc<Self>, p: &Pessoa, agora: i64) -> gtk::Widget {
        let col = caixa(Orientation::Vertical, 10, "");
        col.set_size_request(260, -1);
        col.append(&avatar(p, 112, "entrada-avatar-escolhido"));
        let n = rotulo(&p.nome, "entrada-nome-escolhido");
        col.append(&n);
        let d = pessoas::detalhe(p, agora);
        if !d.is_empty() && !p.convidado {
            col.append(&rotulo(&d, "entrada-detalhe-escolhido"));
        }
        let campo = caixa(Orientation::Horizontal, 8, "entrada-campo");
        campo.set_size_request(260, 40);
        let entrada = gtk::PasswordEntry::new();
        entrada.add_css_class("entrada-senha");
        entrada.set_hexpand(true);
        let ir = gtk::Button::new();
        ir.add_css_class("entrada-ir");
        ir.set_child(Some(&glifo("seta-direita", "#8E8F99", 16)));
        ir.set_tooltip_text(Some(tr("Log in")));
        if p.convidado {
            // the guest has no password: the button is the way in
            let l = rotulo(tr("Log in as guest"), "entrada-convidado");
            l.set_hexpand(true);
            l.set_xalign(0.0);
            campo.append(&glifo("usuario", "#8E8F99", 16));
            campo.append(&l);
            entrada.set_visible(false);
            campo.append(&entrada);
        } else {
            campo.append(&glifo("cadeado", "#8E8F99", 16));
            entrada.set_property("placeholder-text", tr("Password"));
            campo.append(&entrada);
        }
        campo.append(&ir);
        let erro = rotulo("", "entrada-erro");
        let cartao = caixa(Orientation::Vertical, 8, "");
        cartao.append(&campo);
        cartao.append(&erro);
        let ca = Animavel::new(&cartao);
        col.append(&ca);
        ca.entrar((0.0, -8.0), 0.97);
        let w = Rc::downgrade(self);
        entrada.connect_activate(move |_| {
            if let Some(e) = w.upgrade() {
                e.entrar();
            }
        });
        let w = Rc::downgrade(self);
        ir.connect_clicked(move |_| {
            if let Some(e) = w.upgrade() {
                e.entrar();
            }
        });
        let foco = entrada.clone();
        glib::idle_add_local_once(move || {
            foco.grab_focus();
        });
        *self.senha.borrow_mut() = Some(entrada);
        *self.erro.borrow_mut() = Some(erro);
        *self.cartao.borrow_mut() = Some(ca);
        col.upcast()
    }

    fn outro(self: &Rc<Self>, i: usize, p: &Pessoa, agora: i64) -> gtk::Widget {
        let b = gtk::Button::new();
        b.add_css_class("entrada-pessoa");
        b.set_size_request(260, -1);
        let col = caixa(Orientation::Vertical, 10, "");
        col.append(&avatar(p, 80, if p.convidado { "entrada-avatar-convidado" } else { "entrada-avatar" }));
        col.append(&rotulo(&p.nome, "entrada-nome"));
        let d = pessoas::detalhe(p, agora);
        if !d.is_empty() {
            let l = rotulo(&d, "entrada-detalhe");
            if p.sessao.is_some() {
                l.add_css_class("aberta");
            }
            col.append(&l);
        }
        b.set_child(Some(&col));
        let w = Rc::downgrade(self);
        b.connect_clicked(move |_| {
            if let Some(e) = w.upgrade() {
                e.escolher(i);
            }
        });
        b.upcast()
    }

    fn escolher(self: &Rc<Self>, i: usize) {
        if self.ocupado.get() || i == self.sel.get() {
            return;
        }
        let antes: Vec<(String, f32)> = self.animaveis.borrow().iter()
            .filter_map(|(u, a)| a.compute_bounds(&self.janela).map(|b| (u.clone(), b.x() + b.width() / 2.0))).collect();
        self.sel.set(i);
        self.desenhar(Some(antes));
        if debug() {
            eprintln!("debug entrada: escolhida {}", self.pessoas[i].usuario);
        }
    }

    /// The password through greetd (off the interface thread), then the session.
    fn entrar(self: &Rc<Self>) {
        if self.ocupado.replace(true) {
            return;
        }
        let p = self.pessoas[self.sel.get()].clone();
        let senha = self.senha.borrow().as_ref().map(|e| e.text().to_string()).unwrap_or_default();
        let sessao = self.sessoes.get(self.sessao.get()).cloned();
        if let Some(e) = self.erro.borrow().as_ref() {
            e.set_text(tr("Checking…"));
            e.remove_css_class("falhou");
        }
        let (tx, rx) = std::sync::mpsc::channel::<Result<(), (bool, String)>>();
        let sock = std::env::var_os("GREETD_SOCK").map(PathBuf::from);
        std::thread::spawn(move || {
            let r = (|| {
                let sock = sock.ok_or((false, tr("The login service (greetd) is not running.").to_string()))?;
                let mut g = greetd::Greetd::conectar(&sock).map_err(|e| (false, e))?;
                greetd::autenticar(&mut g, &p.usuario, &senha)?;
                // a session already open: back to it (logind), this login is not needed
                if let Some(id) = &p.sessao {
                    let ok = gio::bus_get_sync(gio::BusType::System, gio::Cancellable::NONE).ok().and_then(|b| b.call_sync(Some("org.freedesktop.login1"),
                        "/org/freedesktop/login1", "org.freedesktop.login1.Manager", "ActivateSession", Some(&(id.as_str(),).to_variant()), None,
                        gio::DBusCallFlags::NONE, 5000, gio::Cancellable::NONE).ok()).is_some();
                    if ok {
                        let _ = g.cancelar();
                        return Ok(());
                    }
                }
                let s = sessao.ok_or((false, tr("No session installed").to_string()))?;
                let env = vec![
                    format!("XDG_SESSION_TYPE={}", if s.wayland { "wayland" } else { "x11" }),
                    format!("XDG_SESSION_DESKTOP={}", s.id),
                    format!("XDG_CURRENT_DESKTOP={}", s.nome),
                ];
                match g.iniciar(&s.cmd, &env).map_err(|e| (false, e))? {
                    greetd::Resposta::Sucesso => {
                        let _ = std::fs::create_dir_all(ultimo_path().parent().unwrap_or(&ultimo_path()));
                        let _ = std::fs::write(ultimo_path(), serde_json::json!({"usuario": p.usuario, "sessao": s.id}).to_string());
                        Ok(())
                    }
                    greetd::Resposta::Erro { texto, .. } => Err((false, texto)),
                    _ => Err((false, tr("The session did not start.").to_string())),
                }
            })();
            let _ = tx.send(r);
        });
        let w = Rc::downgrade(self);
        glib::timeout_add_local(Duration::from_millis(50), move || {
            let Ok(r) = rx.try_recv() else { return glib::ControlFlow::Continue };
            let Some(e) = w.upgrade() else { return glib::ControlFlow::Break };
            e.ocupado.set(false);
            match r {
                Ok(()) => e.saiu(),
                Err((senha_errada, msg)) => e.falhou(senha_errada, &msg),
            }
            glib::ControlFlow::Break
        });
    }

    fn falhou(&self, senha_errada: bool, msg: &str) {
        if debug() {
            eprintln!("debug entrada: {} ({msg})", if senha_errada { "senha incorreta" } else { "erro" });
        }
        if let Some(e) = self.erro.borrow().as_ref() {
            e.set_text(if senha_errada { tr("Wrong password. Try again.") } else { msg });
            e.add_css_class("falhou");
        }
        if let Some(s) = self.senha.borrow().as_ref() {
            s.set_text("");
            s.grab_focus();
        }
        // a horizontal shake, 3× in 240 ms
        if let Some(c) = self.cartao.borrow().clone() {
            let inicio = std::time::Instant::now();
            c.add_tick_callback(move |w, _| {
                let t = inicio.elapsed().as_secs_f64();
                if t >= 0.24 {
                    if let Some(a) = w.downcast_ref::<Animavel>() {
                        a.definir(Canal::X, 0.0);
                    }
                    return glib::ControlFlow::Break;
                }
                if let Some(a) = w.downcast_ref::<Animavel>() {
                    a.definir(Canal::X, 10.0 * (1.0 - t / 0.24) * (t / 0.24 * 3.0 * std::f64::consts::TAU).sin());
                }
                glib::ControlFlow::Continue
            });
        }
    }

    /// In: the card shrinks, the screen dissolves (320 ms), then the greeter ends and greetd starts the session.
    fn saiu(&self) {
        if debug() {
            eprintln!("debug entrada: entrou");
        }
        if let Some(c) = self.cartao.borrow().as_ref() {
            c.ir(Canal::Escala, 0.92, mv::RAPIDA);
        }
        self.tudo.ir(Canal::Opacidade, 0.0, mv::SUAVE);
        let j = self.janela.clone();
        glib::timeout_add_local_once(Duration::from_millis(320), move || {
            if let Some(app) = j.application() {
                app.quit();
            }
        });
    }
}

fn main() -> glib::ExitCode {
    let app = gtk::Application::builder().application_id("os.kalango.Entrada").build();
    app.connect_startup(|_| {
        let css = gtk::CssProvider::new();
        css.load_from_string(&format!("{}\n{}", include_str!("../assets/tokens-dark.css"), include_str!("../assets/style.css")));
        if let Some(d) = gdk::Display::default() {
            // above the person's own ~/.config/gtk-4.0/gtk.css (USER priority): a desktop theme from elsewhere ("button { border-radius: 12px }")
            // must not redraw the system's interface
            gtk::style_context_add_provider_for_display(&d, &css, gtk::STYLE_PROVIDER_PRIORITY_USER + 1);
        }
    });
    let ent: Rc<RefCell<Option<Rc<Entrada>>>> = Rc::default();
    app.connect_activate(move |app| {
        if ent.borrow().is_some() {
            return;
        }
        let e = Entrada::new(app);
        e.janela.present();
        *ent.borrow_mut() = Some(e);
    });
    app.run_with_args::<&str>(&[])
}
