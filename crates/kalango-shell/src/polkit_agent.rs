//! The shell as polkit's authentication agent (design "Autenticação (senha de administrador)"): when an app asks to do
//! something of the system's — install, change the date, mount a disk — polkitd asks here, and the dialog says who
//! asks, what changes and for how long, before the password. The password goes to polkit's own helper
//! (polkit-agent-helper-1, which runs PAM: a fingerprint reader works too when PAM has it), never through the shell's
//! memory longer than the moment it is written there. A wrong password: the field shakes in coral and asks again.
//!
//! Not in the development session (the system's polkit belongs to that desktop's own agent); the session test uses a
//! stand-in authority on the session bus (KALANGO_POLKIT_BUS=session) and a stand-in helper (KALANGO_POLKIT_HELPER).

use crate::conversa::ui_icon;
use crate::superficies::{caixa, camada, rotulo};
use gtk::prelude::*;
use gtk::{gio, glib, Align, Orientation};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kalango_i18n::tr;
use kalango_movimento::{Animavel, Canal, ELASTICA};
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::rc::Rc;

const PATH: &str = "/os/kalango/PolkitAgent";

const XML: &str = r#"<node>
  <interface name="org.freedesktop.PolicyKit1.AuthenticationAgent">
    <method name="BeginAuthentication">
      <arg type="s" name="action_id" direction="in"/><arg type="s" name="message" direction="in"/>
      <arg type="s" name="icon_name" direction="in"/><arg type="a{ss}" name="details" direction="in"/>
      <arg type="s" name="cookie" direction="in"/><arg type="a(sa{sv})" name="identities" direction="in"/>
    </method>
    <method name="CancelAuthentication"><arg type="s" name="cookie" direction="in"/></method>
  </interface>
</node>"#;

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

fn bus_type() -> Option<gio::BusType> {
    match std::env::var("KALANGO_POLKIT_BUS").as_deref() {
        Ok("session") => Some(gio::BusType::Session),
        _ if std::env::var_os("KALANGO_SHELL_DEV").is_some() => None,
        _ => Some(gio::BusType::System),
    }
}

fn helper() -> String {
    std::env::var("KALANGO_POLKIT_HELPER").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| {
        ["/usr/lib/polkit-1/polkit-agent-helper-1", "/usr/libexec/polkit-agent-helper-1"].into_iter().find(|p| std::path::Path::new(p).exists())
            .unwrap_or("/usr/lib/polkit-1/polkit-agent-helper-1").to_string()
    })
}

/// What the person reads: who asks, what changes, for how long.
struct Pedido {
    cookie: String,
    quem: String,
    oque: String,
    mensagem: String,
    manter: bool,
    usuario: String,
}

/// The app that asked: the caller's process (its desktop name when there is one).
fn quem_pede(details: &HashMap<String, String>) -> String {
    let pid = details.get("polkit.caller-pid").or_else(|| details.get("polkit.subject-pid")).cloned().unwrap_or_default();
    let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).map(|s| s.trim().to_string()).unwrap_or_default();
    let programa = details.get("program").and_then(|p| std::path::Path::new(p).file_name().map(|n| n.to_string_lossy().into_owned()));
    let nome = programa.unwrap_or(comm);
    gio::DesktopAppInfo::search(&nome).into_iter().flatten().next()
        .and_then(|id| gio::DesktopAppInfo::new(&id)).map(|a| a.name().to_string())
        .filter(|n| !n.is_empty()).unwrap_or(nome)
}

/// The account to authenticate: the person's own when it may administer, else the first one polkit offers.
fn usuario(ids: &[(String, HashMap<String, glib::Variant>)]) -> String {
    let meu = glib::user_name().to_string_lossy().into_owned();
    let nomes: Vec<String> = ids.iter().filter(|(k, _)| k == "unix-user").filter_map(|(_, d)| d.get("uid").and_then(|v| v.get::<u32>()))
        .filter_map(|uid| nome_de_uid(uid)).collect();
    if nomes.contains(&meu) { meu } else { nomes.into_iter().next().unwrap_or(meu) }
}

fn nome_de_uid(uid: u32) -> Option<String> {
    std::fs::read_to_string("/etc/passwd").ok()?.lines().find_map(|l| {
        let c: Vec<&str> = l.split(':').collect();
        (c.get(2)?.parse::<u32>().ok()? == uid).then(|| c[0].to_string())
    })
}

struct Dialogo {
    janela: gtk::ApplicationWindow,
    cartao: Animavel,
    campo_mov: Animavel,
    campo: gtk::Box,
    senha: gtk::Text,
    subtitulo: gtk::Label,
    quem: gtk::Label,
    oque: gtk::Label,
    vale: gtk::Label,
    erro: gtk::Label,
    info: gtk::Label,
    autorizar: gtk::Button,
    /// the helper's stdin while it waits for the password, and what to do with the answer
    pedido: RefCell<Option<Rc<Autenticacao>>>,
}

/// One authentication: polkit's helper running, the D-Bus call waiting.
struct Autenticacao {
    cookie: String,
    usuario: String,
    tentativas: std::cell::Cell<u32>,
    entrada: RefCell<Option<std::process::ChildStdin>>,
    filho: RefCell<Option<std::process::Child>>,
    invocacao: RefCell<Option<gio::DBusMethodInvocation>>,
}

impl Autenticacao {
    fn terminar(&self, ok: bool) {
        if let Some(mut f) = self.filho.borrow_mut().take() {
            let _ = f.kill();
            let _ = f.wait();
        }
        self.entrada.borrow_mut().take();
        if let Some(inv) = self.invocacao.borrow_mut().take() {
            if ok {
                inv.return_value(None);
            } else {
                inv.return_dbus_error("org.freedesktop.PolicyKit1.Error.Cancelled", "the person canceled");
            }
        }
    }
}

thread_local! {
    static DIALOGO: RefCell<Option<Rc<Dialogo>>> = const { RefCell::new(None) };
}

fn dialogo() -> Option<Rc<Dialogo>> {
    if let Some(d) = DIALOGO.with(|d| d.borrow().clone()) {
        return Some(d);
    }
    let app = gio::Application::default().and_downcast::<gtk::Application>()?;
    let janela = camada(&app, "kalango-autenticacao", Layer::Overlay, &[Edge::Top, Edge::Bottom, Edge::Left, Edge::Right]);
    janela.add_css_class("wifi-fundo");
    janela.add_css_class("autenticacao");
    janela.set_exclusive_zone(-1);
    let corpo = caixa(Orientation::Vertical, 14, "wifi-cartao");
    corpo.set_size_request(460, -1);
    let cab = caixa(Orientation::Horizontal, 14, "");
    let tile = caixa(Orientation::Horizontal, 0, "aut-icone");
    tile.set_size_request(44, 44);
    tile.set_hexpand(false);
    tile.set_valign(Align::Center);
    let ic = ui_icon("cadeado", "#E5C98C", 18);
    ic.set_hexpand(true);
    ic.set_halign(Align::Center);
    ic.set_valign(Align::Center);
    tile.append(&ic);
    cab.append(&tile);
    let t = caixa(Orientation::Vertical, 4, "");
    t.set_valign(Align::Center);
    let titulo = rotulo(tr("Your password is needed"), "wifi-titulo");
    titulo.set_xalign(0.0);
    t.append(&titulo);
    let subtitulo = rotulo("", "wifi-subtitulo");
    subtitulo.set_xalign(0.0);
    subtitulo.set_wrap(true);
    subtitulo.set_max_width_chars(40);
    t.append(&subtitulo);
    cab.append(&t);
    corpo.append(&cab);
    // who asks, what changes, for how long
    let quadro = caixa(Orientation::Vertical, 8, "aut-quadro");
    let linha = |nome: &str| {
        let l = caixa(Orientation::Horizontal, 8, "");
        let a = rotulo(nome, "aut-chave");
        a.set_xalign(0.0);
        a.set_hexpand(true);
        l.append(&a);
        let v = rotulo("", "aut-valor");
        v.set_xalign(1.0);
        v.set_wrap(true);
        v.set_max_width_chars(34);
        v.set_justify(gtk::Justification::Right);
        l.append(&v);
        quadro.append(&l);
        v
    };
    let quem = linha(tr("Who asks"));
    let oque = linha(tr("What changes"));
    let vale = linha(tr("Valid for"));
    corpo.append(&quadro);
    let campo = caixa(Orientation::Horizontal, 8, "wifi-campo");
    campo.append(&ui_icon("cadeado", "#8E8F99", 16));
    let senha = gtk::Text::new();
    senha.set_hexpand(true);
    senha.set_visibility(false);
    senha.set_invisible_char('•');
    senha.set_input_purpose(gtk::InputPurpose::Password);
    campo.append(&senha);
    let olho = gtk::Button::new();
    olho.add_css_class("wifi-olho");
    olho.set_child(Some(&ui_icon("olho", "#8E8F99", 16)));
    olho.set_focus_on_click(false);
    campo.append(&olho);
    let campo_mov = Animavel::new(&campo);
    corpo.append(&campo_mov);
    let erro = rotulo("", "wifi-erro");
    erro.set_xalign(0.0);
    erro.set_visible(false);
    corpo.append(&erro);
    // what PAM says besides the password (a fingerprint reader: "touch the reader")
    let info = rotulo("", "wifi-subtitulo");
    info.set_xalign(0.0);
    info.set_wrap(true);
    info.set_visible(false);
    corpo.append(&info);
    let acoes = caixa(Orientation::Horizontal, 8, "");
    acoes.set_halign(Align::End);
    let cancelar = gtk::Button::with_label(tr("Cancel"));
    cancelar.add_css_class("wifi-botao");
    let autorizar = gtk::Button::with_label(tr("Authorize"));
    autorizar.add_css_class("wifi-botao");
    autorizar.add_css_class("principal");
    acoes.append(&cancelar);
    acoes.append(&autorizar);
    corpo.append(&acoes);
    let cartao = Animavel::new(&corpo);
    cartao.origem(0.5, 0.5);
    cartao.set_halign(Align::Center);
    cartao.set_valign(Align::Center);
    janela.set_child(Some(&cartao));
    let d = Rc::new(Dialogo { janela, cartao, campo_mov, campo, senha, subtitulo, quem, oque, vale, erro, info, autorizar, pedido: RefCell::default() });
    let w = Rc::downgrade(&d);
    olho.connect_clicked(move |b| {
        let Some(d) = w.upgrade() else { return };
        let mostrar = !d.senha.property::<bool>("visibility");
        d.senha.set_visibility(mostrar);
        b.set_child(Some(&ui_icon(if mostrar { "olho-d" } else { "olho" }, "#8E8F99", 16)));
    });
    let w = Rc::downgrade(&d);
    d.senha.connect_activate(move |_| if let Some(d) = w.upgrade() { d.enviar() });
    let w = Rc::downgrade(&d);
    d.autorizar.connect_clicked(move |_| if let Some(d) = w.upgrade() { d.enviar() });
    let w = Rc::downgrade(&d);
    cancelar.connect_clicked(move |_| if let Some(d) = w.upgrade() { d.cancelar() });
    let w = Rc::downgrade(&d);
    d.senha.connect_changed(move |_| if let Some(d) = w.upgrade() { d.campo.remove_css_class("erro") });
    let teclas = gtk::EventControllerKey::new();
    let w = Rc::downgrade(&d);
    teclas.connect_key_pressed(move |_, k, _, _| {
        if k == gtk::gdk::Key::Escape {
            if let Some(d) = w.upgrade() {
                d.cancelar();
            }
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    d.janela.add_controller(teclas);
    DIALOGO.with(|x| *x.borrow_mut() = Some(d.clone()));
    Some(d)
}

impl Dialogo {
    fn mostrar(self: &Rc<Self>, p: &Pedido, a: Rc<Autenticacao>) {
        // polkit's own sentence for the action (translated by polkit), and the details below
        self.subtitulo.set_text(&p.mensagem);
        self.quem.set_text(&p.quem);
        self.oque.set_text(&p.oque);
        self.vale.set_text(if p.manter { tr("this action, for a few minutes") } else { tr("only this action") });
        self.erro.set_visible(false);
        self.info.set_visible(false);
        self.campo.remove_css_class("erro");
        self.senha.set_text("");
        *self.pedido.borrow_mut() = Some(a);
        crate::superficies::take_keyboard(&self.janela);
        self.janela.add_css_class("aberto");
        self.janela.present();
        self.cartao.entrar((0.0, 12.0), 0.96);
        self.senha.grab_focus();
        if debug() {
            eprintln!("debug polkit: pedindo a senha de {} ({} · {})", p.usuario, p.quem, p.oque);
        }
    }

    fn enviar(self: &Rc<Self>) {
        let Some(a) = self.pedido.borrow().clone() else { return };
        let s = self.senha.text().to_string();
        if s.is_empty() {
            return;
        }
        if let Some(e) = a.entrada.borrow_mut().as_mut() {
            let _ = writeln!(e, "{s}");
            let _ = e.flush();
        }
        // the field forgets it at once
        self.senha.set_text("");
        self.autorizar.set_sensitive(false);
        self.autorizar.set_label(tr("Checking…"));
    }

    /// The helper said no: the field shakes (3 times in 240 ms) and turns coral.
    fn recusada(&self, msg: &str) {
        self.autorizar.set_sensitive(true);
        self.autorizar.set_label(tr("Authorize"));
        self.campo.add_css_class("erro");
        self.erro.set_text(if msg.is_empty() { tr("Wrong password.") } else { msg });
        self.erro.set_visible(true);
        if !kalango_movimento::reduzido() {
            self.campo_mov.definir(Canal::X, -12.0);
            self.campo_mov.ir(Canal::X, 0.0, ELASTICA);
        }
        self.senha.grab_focus();
        if debug() {
            eprintln!("debug polkit: senha recusada");
        }
    }

    fn fechar(self: &Rc<Self>) {
        self.janela.set_keyboard_mode(KeyboardMode::None);
        self.janela.remove_css_class("aberto");
        self.autorizar.set_sensitive(true);
        self.autorizar.set_label(tr("Authorize"));
        let w = Rc::downgrade(self);
        self.cartao.sair((0.0, 8.0), 0.98, move || {
            if let Some(d) = w.upgrade().filter(|d| d.pedido.borrow().is_none()) {
                d.janela.set_visible(false);
            }
        });
    }

    fn cancelar(self: &Rc<Self>) {
        if let Some(a) = self.pedido.borrow_mut().take() {
            if debug() {
                eprintln!("debug polkit: cancelado");
            }
            a.terminar(false);
        }
        self.fechar();
    }
}

/// Shows the dialog and runs polkit's helper for the account and the cookie.
fn autenticar(p: Pedido, inv: gio::DBusMethodInvocation) {
    let Some(d) = dialogo() else {
        inv.return_dbus_error("org.freedesktop.PolicyKit1.Error.Failed", "no dialog");
        return;
    };
    let a = Rc::new(Autenticacao { cookie: p.cookie.clone(), usuario: p.usuario.clone(), tentativas: std::cell::Cell::new(0), entrada: RefCell::default(),
        filho: RefCell::default(), invocacao: RefCell::new(Some(inv)) });
    d.mostrar(&p, a.clone());
    if !conversar(&d, &a) {
        d.pedido.borrow_mut().take();
        a.terminar(false);
        d.fechar();
    }
}

/// One run of polkit's helper (one PAM conversation): the cookie on its stdin, then its prompts and the answer. A
/// wrong password ends it with FAILURE: the dialog stays, shakes in coral, and a new run starts (three tries, as
/// GNOME's agent does).
fn conversar(d: &Rc<Dialogo>, a: &Rc<Autenticacao>) -> bool {
    let Ok(mut filho) = Command::new(helper()).arg(&a.usuario).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn() else {
        return false;
    };
    let mut entrada = filho.stdin.take();
    if let Some(e) = entrada.as_mut() {
        let _ = writeln!(e, "{}", a.cookie);
        let _ = e.flush();
    }
    let saida = filho.stdout.take();
    *a.entrada.borrow_mut() = entrada;
    *a.filho.borrow_mut() = Some(filho);
    // its lines, read aside and handed to the main loop (polled, as the command bar's engine is)
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    if let Some(s) = saida {
        std::thread::spawn(move || {
            for l in BufReader::new(s).lines().map_while(Result::ok) {
                if tx.send(l).is_err() {
                    break;
                }
            }
        });
    }
    let (dw, aw) = (Rc::downgrade(d), Rc::downgrade(a));
    glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
        let (Some(d), Some(a)) = (dw.upgrade(), aw.upgrade()) else { return glib::ControlFlow::Break };
        if d.pedido.borrow().as_ref().is_none_or(|x| !Rc::ptr_eq(x, &a)) {
            return glib::ControlFlow::Break;
        }
        loop {
            let l = match rx.try_recv() {
                Ok(l) => l,
                Err(std::sync::mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                // the helper ended without a word: as a failure
                Err(std::sync::mpsc::TryRecvError::Disconnected) => "FAILURE".to_string(),
            };
            let (tipo, texto) = l.split_once(' ').unwrap_or((l.as_str(), ""));
            match tipo {
                "PAM_PROMPT_ECHO_OFF" | "PAM_PROMPT_ECHO_ON" => {
                    d.autorizar.set_sensitive(true);
                    d.autorizar.set_label(tr("Authorize"));
                }
                "PAM_ERROR_MSG" => {
                    d.erro.set_text(texto.trim());
                    d.erro.set_visible(true);
                }
                "PAM_TEXT_INFO" => {
                    d.info.set_text(texto.trim());
                    d.info.set_visible(true);
                }
                "SUCCESS" => {
                    if debug() {
                        eprintln!("debug polkit: autorizado");
                    }
                    d.pedido.borrow_mut().take();
                    a.terminar(true);
                    d.fechar();
                    return glib::ControlFlow::Break;
                }
                "FAILURE" => {
                    if let Some(mut f) = a.filho.borrow_mut().take() {
                        let _ = f.wait();
                    }
                    a.tentativas.set(a.tentativas.get() + 1);
                    if a.tentativas.get() < 3 && conversar(&d, &a) {
                        d.recusada("");
                    } else {
                        if debug() {
                            eprintln!("debug polkit: o PAM recusou");
                        }
                        d.pedido.borrow_mut().take();
                        a.terminar(false);
                        d.fechar();
                    }
                    return glib::ControlFlow::Break;
                }
                _ => {}
            }
        }
    });
    true
}

fn metodo(nome: &str, params: &glib::Variant, inv: gio::DBusMethodInvocation) {
    match nome {
        "BeginAuthentication" => {
            let action: String = params.child_value(0).get().unwrap_or_default();
            let mensagem: String = params.child_value(1).get().unwrap_or_default();
            let details: HashMap<String, String> = params.child_value(3).get().unwrap_or_default();
            let cookie: String = params.child_value(4).get().unwrap_or_default();
            let ids: Vec<(String, HashMap<String, glib::Variant>)> = params.child_value(5).get().unwrap_or_default();
            let quem = quem_pede(&details);
            // what changes: pkexec's command, or the action's own words
            let oque = details.get("command_line").cloned().unwrap_or_else(|| mensagem.clone());
            let manter = details.get("polkit.retains_authorization_after_challenge").is_some_and(|v| v == "1");
            if debug() {
                eprintln!("debug polkit: {action} pedido por {quem}");
            }
            autenticar(Pedido { cookie, quem, oque, mensagem, manter, usuario: usuario(&ids) }, inv);
        }
        "CancelAuthentication" => {
            let cookie: String = params.child_value(0).get().unwrap_or_default();
            if let Some(d) = DIALOGO.with(|d| d.borrow().clone()) {
                let atual = d.pedido.borrow().as_ref().is_some_and(|a| a.cookie == cookie);
                if atual {
                    d.cancelar();
                }
            }
            inv.return_value(None);
        }
        _ => inv.return_dbus_error("org.freedesktop.DBus.Error.UnknownMethod", nome),
    }
}

/// Registers the agent for this session with polkitd.
pub fn start() {
    let Some(bus) = bus_type() else { return };
    let Ok(c) = gio::bus_get_sync(bus, gio::Cancellable::NONE) else { return };
    let info = gio::DBusNodeInfo::for_xml(XML).expect("AuthenticationAgent XML");
    let iface = info.lookup_interface("org.freedesktop.PolicyKit1.AuthenticationAgent").expect("AuthenticationAgent interface");
    if let Err(e) = c.register_object(PATH, &iface).method_call(|_, _, _, _, nome, params, inv| metodo(nome, &params, inv)).build() {
        eprintln!("kalango-shell: agente do polkit: {e}");
        return;
    }
    // the subject: this login session (logind's id), else this process
    let sujeito: (String, HashMap<String, glib::Variant>) = match std::env::var("XDG_SESSION_ID") {
        Ok(id) if !id.is_empty() => ("unix-session".into(), HashMap::from([("session-id".to_string(), id.to_variant())])),
        _ => ("unix-process".into(), HashMap::from([("pid".to_string(), std::process::id().to_variant()), ("start-time".to_string(), 0u64.to_variant())])),
    };
    let local = std::env::var("LANG").unwrap_or_else(|_| "pt_BR.UTF-8".into());
    let caminho = glib::variant::ObjectPath::try_from(PATH).expect("agent path");
    let watch = gio::bus_watch_name_on_connection(&c, "org.freedesktop.PolicyKit1", gio::BusNameWatcherFlags::NONE, move |c, _, _| {
        let args = glib::Variant::tuple_from_iter([sujeito.to_variant(), local.to_variant(), caminho.as_str().to_variant()]);
        c.call(Some("org.freedesktop.PolicyKit1"), "/org/freedesktop/PolicyKit1/Authority", "org.freedesktop.PolicyKit1.Authority", "RegisterAuthenticationAgent",
            Some(&args), None, gio::DBusCallFlags::NONE, 5000, gio::Cancellable::NONE, |r| match r {
                Ok(_) => {
                    if debug() {
                        eprintln!("debug polkit: agente registrado");
                    }
                }
                Err(e) => eprintln!("kalango-shell: agente do polkit: {e}"),
            });
    }, |_, _| {});
    std::mem::forget(watch);
}
