//! Lock screen (design "05 · Telas do sistema e apps", frame "Tela de bloqueio"), on the compositor's lock protocol
//! (ext-session-lock-v1, through gtk4-session-lock): while it is on, only these surfaces are drawn and get input, and
//! if the shell stops the screen stays locked. Wallpaper, the big clock, the date, three discreet cards (agents at
//! work and approvals waiting, what is playing, how many notifications and from which apps, never their content),
//! the avatar and "press any key". A key or a click turns the avatar into the password field; the password is
//! checked by PAM off the interface thread.
//!
//! Motion: on the first touch the clock goes up 24 px and fades (gentle spring), the cards come in the 24 ms
//! cascade and the avatar becomes the field.

use crate::conversa::ui_icon;
use crate::superficies::{caixa, rotulo};
use crate::Shell;
use gtk::glib::translate::{from_glib_full, ToGlibPtr};
use gtk::prelude::*;
use gtk::{gdk, glib, Align, Orientation};
use kalango_i18n::{tr, trf};
use kalango_movimento::{self as mv, Animavel, Canal};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::time::Duration;

extern "C" {
    fn gtk_session_lock_is_supported() -> glib::ffi::gboolean;
    fn gtk_session_lock_instance_new() -> *mut glib::gobject_ffi::GObject;
    fn gtk_session_lock_instance_lock(i: *mut glib::gobject_ffi::GObject) -> glib::ffi::gboolean;
    fn gtk_session_lock_instance_unlock(i: *mut glib::gobject_ffi::GObject);
    fn gtk_session_lock_instance_assign_window_to_monitor(i: *mut glib::gobject_ffi::GObject, w: *mut gtk::ffi::GtkWindow, m: *mut gdk::ffi::GdkMonitor);
}

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

/// A marker that this compositor's session is locked, so a shell that restarts (a crash) locks again at once: the
/// compositor keeps the screen locked when the lock client dies, and without this nobody could unlock it. It names
/// the compositor by its socket's time, so a marker left by an earlier session is ignored.
fn marker() -> Option<(PathBuf, String)> {
    let wd = std::env::var("WAYLAND_DISPLAY").ok().filter(|w| !w.is_empty())?;
    let rt = glib::user_runtime_dir();
    let t = std::fs::metadata(rt.join(&wd)).and_then(|m| m.modified()).ok()?;
    let id = t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos().to_string()).unwrap_or_default();
    Some((rt.join(format!("kalango/bloqueada-{}", wd.replace('/', "_"))), id))
}

/// One monitor's lock window.
struct Tela {
    janela: gtk::Window,
    relogio: Animavel,
    horas: [gtk::Label; 4],
    data: gtk::Label,
    cartoes: gtk::Box,
    /// the cards' springs (they step aside when the field opens)
    anims: RefCell<Vec<Animavel>>,
    avatar: gtk::Widget,
    dica: gtk::Box,
    /// the password state: the account's photo big in the middle, the name, the field (design 130156)
    centro: Animavel,
    /// the time, small, on top while the field is open
    topo: Animavel,
    topo_txt: gtk::Label,
    senha: Animavel,
    entrada: gtk::PasswordEntry,
    erro: gtk::Label,
    principal: bool,
}

pub struct Bloqueio {
    inst: RefCell<Option<glib::Object>>,
    telas: RefCell<Vec<Tela>>,
    shell: RefCell<Weak<Shell>>,
    /// waiting for the password (the field is showing)
    pedindo: Cell<bool>,
    conferindo: Cell<bool>,
    ultima_tecla: Cell<i64>,
    /// what the cards show now (they are redrawn only when it changes)
    resumo: RefCell<String>,
}

impl Bloqueio {
    pub fn new() -> Rc<Bloqueio> {
        let b = Rc::new(Bloqueio { inst: RefCell::default(), telas: RefCell::default(), shell: RefCell::default(), pedindo: Cell::new(false), conferindo: Cell::new(false), ultima_tecla: Cell::new(0), resumo: RefCell::default() });
        // the clock, every second while locked; back to "press any key" after 30 s without typing
        let w = Rc::downgrade(&b);
        glib::timeout_add_seconds_local(1, move || {
            let Some(b) = w.upgrade() else { return glib::ControlFlow::Break };
            if b.inst.borrow().is_some() {
                b.tick();
                b.preencher_cartoes();
                let agora = glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0);
                if b.pedindo.get() && !b.conferindo.get() && agora - b.ultima_tecla.get() > 30 {
                    b.voltar();
                }
            }
            glib::ControlFlow::Continue
        });
        b
    }

    pub fn ligar(self: &Rc<Self>, shell: &Rc<Shell>) {
        *self.shell.borrow_mut() = Rc::downgrade(shell);
        // a session nested in the real desktop shares its logind session: it must not lock when the desktop does
        if std::env::var_os("KALANGO_SHELL_DEV").is_none() {
            self.ouvir_logind();
        }
        // the shell stopped while the screen was locked: lock again so the person can unlock
        if let Some((m, id)) = marker() {
            if std::fs::read_to_string(&m).is_ok_and(|c| c == id) {
                if debug() {
                    eprintln!("debug bloqueio: retomando o bloqueio");
                }
                let w = Rc::downgrade(self);
                glib::idle_add_local_once(move || {
                    if let Some(b) = w.upgrade() {
                        b.bloquear();
                    }
                });
            } else {
                let _ = std::fs::remove_file(&m);
            }
        }
    }

    /// Ends the lock. The instance is taken out first: the library calls ::unlocked from inside this call.
    fn desbloquear(&self) {
        let inst = self.inst.borrow().clone();
        if let Some(i) = inst {
            unsafe { gtk_session_lock_instance_unlock(i.as_ptr()) };
        }
    }

    pub fn bloqueada(&self) -> bool {
        self.inst.borrow().is_some()
    }

    /// Locks the session (the compositor's lock).
    pub fn bloquear(self: &Rc<Self>) {
        if self.inst.borrow().is_some() {
            return;
        }
        if unsafe { gtk_session_lock_is_supported() } == 0 {
            eprintln!("kalango-shell: o compositor não oferece o protocolo de bloqueio (ext-session-lock)");
            return;
        }
        let ptr = unsafe { gtk_session_lock_instance_new() };
        let inst: glib::Object = unsafe { from_glib_full(ptr) };
        let w = Rc::downgrade(self);
        inst.connect_local("monitor", false, move |args| {
            let monitor = args.get(1).and_then(|v| v.get::<gdk::Monitor>().ok());
            if let (Some(b), Some(m)) = (w.upgrade(), monitor) {
                b.nova_tela(&m);
            }
            None
        });
        let w = Rc::downgrade(self);
        inst.connect_local("locked", false, move |_| {
            if debug() {
                eprintln!("debug bloqueio: tela bloqueada");
            }
            // the lock's sound, not when the lock comes back after the shell restarted (its marker is still there)
            let resumed = marker().is_some_and(|(m, id)| std::fs::read_to_string(m).is_ok_and(|c| c == id));
            if !resumed {
                crate::configuracoes_som::tocar("kalango-screen-lock");
            }
            if let Some((m, id)) = marker() {
                let _ = std::fs::create_dir_all(m.parent().unwrap_or(&m));
                let _ = std::fs::write(&m, id);
            }
            if let Some(b) = w.upgrade() {
                b.tick();
            }
            None
        });
        let w = Rc::downgrade(self);
        inst.connect_local("failed", false, move |_| {
            eprintln!("kalango-shell: o bloqueio da tela falhou (outro programa já bloqueou?)");
            // after the library is done with this emission
            let w = w.clone();
            glib::idle_add_local_once(move || {
                if let Some(b) = w.upgrade() {
                    b.limpar();
                }
            });
            None
        });
        let w = Rc::downgrade(self);
        inst.connect_local("unlocked", false, move |_| {
            if debug() {
                eprintln!("debug bloqueio: desbloqueado");
            }
            crate::configuracoes_som::tocar("kalango-screen-unlock");
            if let Some((m, _)) = marker() {
                let _ = std::fs::remove_file(m);
            }
            let w = w.clone();
            glib::idle_add_local_once(move || {
                if let Some(b) = w.upgrade() {
                    b.limpar();
                }
            });
            None
        });
        *self.inst.borrow_mut() = Some(inst.clone());
        self.pedindo.set(false);
        unsafe { gtk_session_lock_instance_lock(inst.as_ptr()) };
        // locked at night: the moment an update asked for "tonight" may install
        if let Some(s) = self.shell.borrow().upgrade() {
            glib::timeout_add_local_once(std::time::Duration::from_secs(2), move || crate::configuracoes::vigiar_atualizacao_noturna(&s));
        }
    }

    fn limpar(&self) {
        for t in self.telas.borrow_mut().drain(..) {
            t.janela.destroy();
        }
        self.inst.borrow_mut().take();
        self.resumo.borrow_mut().clear();
        self.pedindo.set(false);
        self.conferindo.set(false);
    }

    fn nova_tela(self: &Rc<Self>, monitor: &gdk::Monitor) {
        let principal = self.telas.borrow().is_empty();
        // the design is 1920 × 1080: positions and the clock follow the monitor's height
        let k = f64::from(monitor.geometry().height().max(480)) / 1080.0;
        let px = |v: f64| (v * k).round() as i32;
        let tamanho = |l: &gtk::Label, pt: f64| {
            let a = gtk::pango::AttrList::new();
            a.insert(gtk::pango::AttrSize::new_size_absolute((pt * k * f64::from(gtk::pango::SCALE)) as i32));
            l.set_attributes(Some(&a));
        };
        let janela = gtk::Window::new();
        janela.add_css_class("bloqueio");
        let raiz = gtk::Overlay::new();
        let fundo = caixa(Orientation::Vertical, 0, "bloqueio-fundo");
        // the output's wallpaper (the lock draws it itself: nothing else is shown while locked)
        if let Some(p) = crate::papel::para(monitor.geometry().width(), monitor.geometry().height()) {
            let pic = gtk::Picture::for_filename(p);
            pic.set_content_fit(gtk::ContentFit::Cover);
            pic.set_hexpand(true);
            pic.set_vexpand(true);
            fundo.append(&pic);
        }
        raiz.set_child(Some(&fundo));
        // "escurecer": the bottom of the wallpaper dims so the cards and the field read well
        let escurecer = caixa(Orientation::Vertical, 0, "bloqueio-escurecer");
        escurecer.set_can_target(false);
        raiz.add_overlay(&escurecer);
        // status, top right
        let status = caixa(Orientation::Horizontal, 12, "bloqueio-status");
        status.set_halign(Align::End);
        status.set_valign(Align::Start);
        for g in ["wifi", "volume", "desligar"] {
            status.append(&ui_icon(g, "#E2E2E6", 16));
        }
        raiz.add_overlay(&status);
        // clock and date
        let rc = caixa(Orientation::Vertical, 4, "");
        let linha = caixa(Orientation::Horizontal, 0, "");
        linha.set_halign(Align::Center);
        let mk = |c: &str| {
            let l = rotulo("0", "bloqueio-digito");
            l.add_css_class(c);
            tamanho(&l, 168.0);
            l.set_size_request(px(95.0), -1);
            l
        };
        let horas = [mk("d1"), mk("d2"), mk("d3"), mk("d4")];
        let dois_pontos = rotulo(":", "bloqueio-digito");
        dois_pontos.add_css_class("bloqueio-pontos");
        tamanho(&dois_pontos, 168.0);
        dois_pontos.set_size_request(px(48.0), -1);
        linha.append(&horas[0]);
        linha.append(&horas[1]);
        linha.append(&dois_pontos);
        linha.append(&horas[2]);
        linha.append(&horas[3]);
        rc.append(&linha);
        let data = rotulo("", "bloqueio-data");
        tamanho(&data, 26.0);
        rc.append(&data);
        let relogio = Animavel::new(&rc);
        relogio.set_halign(Align::Center);
        relogio.set_valign(Align::Start);
        relogio.set_margin_top(px(200.0));
        raiz.add_overlay(&relogio);
        // cards
        let cartoes = caixa(Orientation::Horizontal, 14, "");
        cartoes.set_halign(Align::Center);
        cartoes.set_valign(Align::Start);
        cartoes.set_margin_top(px(620.0));
        raiz.add_overlay(&cartoes);
        // avatar + hint, or the password field
        let baixo = caixa(Orientation::Vertical, 10, "");
        baixo.set_halign(Align::Center);
        baixo.set_valign(Align::End);
        baixo.set_margin_bottom(px(108.0));
        let nome = crate::energia::person_name();
        let avatar = crate::energia::avatar(&nome, 56, "bloqueio-avatar");
        baixo.append(&avatar);
        let dica = caixa(Orientation::Horizontal, 8, "");
        dica.set_halign(Align::Center);
        dica.append(&ui_icon("cadeado", "#8E8F99", 16));
        dica.append(&rotulo(tr("Press any key to unlock"), "bloqueio-dica"));
        baixo.append(&dica);
        // the field (as the login screen's): lock, the dots, and the arrow that confirms
        let caixa_senha = caixa(Orientation::Vertical, 8, "");
        let campo = caixa(Orientation::Horizontal, 8, "bloqueio-campo");
        campo.set_size_request(px(320.0) as i32, px(44.0) as i32);
        let cadeado = ui_icon("cadeado", "#8E8F99", 16);
        cadeado.set_valign(Align::Center);
        campo.append(&cadeado);
        let entrada = gtk::PasswordEntry::new();
        entrada.add_css_class("bloqueio-senha");
        entrada.set_hexpand(true);
        entrada.set_property("placeholder-text", tr("Password"));
        campo.append(&entrada);
        let ir = gtk::Button::new();
        ir.set_child(Some(&ui_icon("seta-direita", "#8E8F99", 16)));
        ir.add_css_class("bloqueio-ir");
        ir.set_tooltip_text(Some(tr("Unlock")));
        campo.append(&ir);
        caixa_senha.append(&campo);
        let erro = rotulo("", "bloqueio-erro");
        erro.set_halign(Align::Center);
        caixa_senha.append(&erro);
        caixa_senha.set_size_request(px(320.0) as i32, -1);
        let senha = Animavel::new(&caixa_senha);
        senha.set_visible(false);
        // the password state, as the login screens of other systems: the photo big in the middle, the name under it,
        // then the field and what it says
        let centro_box = caixa(Orientation::Vertical, 0, "");
        let grande = crate::energia::avatar(&nome, px(160.0) as i32, "bloqueio-avatar-grande");
        centro_box.append(&grande);
        let nome_l = rotulo(&nome, "bloqueio-nome");
        nome_l.set_halign(Align::Center);
        nome_l.set_margin_top(px(20.0) as i32);
        centro_box.append(&nome_l);
        senha.set_margin_top(px(24.0) as i32);
        centro_box.append(&senha);
        let centro = Animavel::new(&centro_box);
        centro.set_halign(Align::Center);
        centro.set_valign(Align::Center);
        centro.set_visible(false);
        let topo_txt = rotulo("", "bloqueio-topo");
        let topo = Animavel::new(&topo_txt);
        topo.set_halign(Align::Center);
        topo.set_valign(Align::Start);
        topo.set_margin_top(px(44.0) as i32);
        topo.set_visible(false);
        if principal {
            raiz.add_overlay(&baixo);
            raiz.add_overlay(&centro);
            raiz.add_overlay(&topo);
        }
        janela.set_child(Some(&raiz));
        let w = Rc::downgrade(self);
        entrada.connect_activate(move |e| {
            if let Some(b) = w.upgrade() {
                b.conferir(e.text().to_string());
            }
        });
        let (w, e) = (Rc::downgrade(self), entrada.clone());
        ir.connect_clicked(move |_| {
            if let Some(b) = w.upgrade() {
                b.conferir(e.text().to_string());
            }
        });
        // any key (or click) shows the field; typing goes straight into it
        let teclas = gtk::EventControllerKey::new();
        teclas.set_propagation_phase(gtk::PropagationPhase::Capture);
        let w = Rc::downgrade(self);
        teclas.connect_key_pressed(move |_, k, _, _| {
            let Some(b) = w.upgrade() else { return glib::Propagation::Proceed };
            b.ultima_tecla.set(glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0));
            if k == gdk::Key::Escape {
                b.voltar();
                return glib::Propagation::Stop;
            }
            if !b.pedindo.get() {
                b.pedir();
                // a printable key is the start of the password
                if k.to_unicode().is_some_and(|c| !c.is_control()) {
                    return glib::Propagation::Proceed;
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        janela.add_controller(teclas);
        let clique = gtk::GestureClick::new();
        let w = Rc::downgrade(self);
        clique.connect_pressed(move |_, _, _, _| {
            if let Some(b) = w.upgrade() {
                if !b.pedindo.get() {
                    b.pedir();
                }
            }
        });
        raiz.add_controller(clique);
        let t = Tela { janela: janela.clone(), relogio, horas, data, cartoes, anims: RefCell::default(), avatar, dica, centro, topo, topo_txt, senha, entrada, erro, principal };
        self.telas.borrow_mut().push(t);
        self.resumo.borrow_mut().clear();
        self.preencher_cartoes();
        self.tick();
        if let Some(inst) = self.inst.borrow().as_ref() {
            unsafe {
                gtk_session_lock_instance_assign_window_to_monitor(inst.as_ptr(), janela.to_glib_none().0, monitor.to_glib_none().0);
            }
        }
    }

    fn tick(&self) {
        let Ok(agora) = glib::DateTime::now_local() else { return };
        let hh = agora.format("%H").map(|s| s.to_string()).unwrap_or_default();
        let mm = agora.format("%M").map(|s| s.to_string()).unwrap_or_default();
        let dig = format!("{hh}{mm}");
        let data = agora.format("%A, %-d de %B").map(|s| s.to_string()).unwrap_or_default();
        for t in self.telas.borrow().iter() {
            for (i, l) in t.horas.iter().enumerate() {
                l.set_text(&dig.chars().nth(i).map(String::from).unwrap_or_default());
            }
            t.data.set_text(&data);
            t.topo_txt.set_text(&format!("{hh}:{mm} · {data}"));
        }
    }

    /// The three cards: counts and app names — what a notification says only if the person chose so (Settings ›
    /// Notifications › "Na tela de bloqueio"), and no notifications card if they chose nothing.
    fn preencher_cartoes(&self) {
        let Some(shell) = self.shell.borrow().upgrade() else { return };
        let ativos = crate::sistema::agent_pids().len();
        let pendentes = shell.estado.pendentes.get().unwrap_or(0);
        let lock_screen = crate::notification_policy::read().lock_screen;
        let unread: Vec<(String, String)> = if lock_screen == crate::notification_policy::LockScreen::Nothing { vec![] } else {
            shell.notificacoes().historico.borrow().iter().filter(|g| !g.lida).map(|g| (g.app.clone(), g.titulo.clone())).collect()
        };
        let nao_lidas: Vec<String> = unread.iter().map(|(a, _)| a.clone()).collect();
        let midia = shell.midia.atual();
        let resumo = format!("{ativos}|{pendentes}|{:?}|{lock_screen:?}|{:?}", unread, midia.as_ref().map(|m| (&m.titulo, &m.artista, m.tocando)));
        if *self.resumo.borrow() == resumo || self.pedindo.get() {
            return;
        }
        *self.resumo.borrow_mut() = resumo;
        for t in self.telas.borrow().iter() {
            while let Some(c) = t.cartoes.first_child() {
                t.cartoes.remove(&c);
            }
            let mut anims = Vec::new();
            let card = |glifo: &str, titulo: &str| {
                let c = caixa(Orientation::Vertical, 4, "bloqueio-cartao");
                c.set_size_request(340, 85);
                let cab = caixa(Orientation::Horizontal, 6, "");
                cab.append(&ui_icon(glifo, if glifo == "spark" { "#A5A1E2" } else { "#8E8F99" }, if glifo == "spark" { 16 } else { 13 }));
                cab.append(&rotulo(titulo, "caps"));
                c.append(&cab);
                c
            };
            let a = card("spark", tr("AGENTS"));
            let l = rotulo(&match ativos { 0 => tr("No task in progress").to_string(), 1 => tr("1 task in progress").to_string(), n => trf("{n} tasks in progress", &[("n", n.to_string())]) }, "bloqueio-cartao-titulo");
            l.set_xalign(0.0);
            a.append(&l);
            if pendentes > 0 {
                let p = rotulo(&if pendentes == 1 { tr("1 waits for your approval after unlocking").to_string() } else { trf("{n} wait for your approval after unlocking", &[("n", pendentes.to_string())]) }, "bloqueio-cartao-aviso");
                p.set_xalign(0.0);
                a.append(&p);
            }
            let aa = Animavel::new(&a);
            t.cartoes.append(&aa);
            anims.push(aa);
            if let Some(m) = &midia {
                let c = card("musica", tr("PLAYING"));
                let row = caixa(Orientation::Horizontal, 10, "");
                row.append(&crate::midia::capa(m, 36));
                let col = caixa(Orientation::Vertical, 0, "");
                col.set_valign(Align::Center);
                let ti = rotulo(&m.titulo, "bloqueio-midia-titulo");
                ti.set_xalign(0.0);
                ti.set_ellipsize(gtk::pango::EllipsizeMode::End);
                ti.set_max_width_chars(18);
                col.append(&ti);
                let ar = rotulo(&m.artista, "bloqueio-midia-artista");
                ar.set_xalign(0.0);
                col.append(&ar);
                row.append(&col);
                let controles = caixa(Orientation::Horizontal, 3, "");
                controles.set_valign(Align::Center);
                for (ic, cmd) in [("anterior", "Previous"), (if m.tocando { "pausa" } else { "tocar" }, "PlayPause"), ("proxima", "Next")] {
                    let b = gtk::Button::new();
                    b.set_child(Some(&ui_icon(ic, "#E2E2E6", 16)));
                    b.add_css_class("bloqueio-midia-botao");
                    b.set_widget_name(cmd);
                    let s = Rc::downgrade(&shell);
                    b.connect_clicked(move |_| {
                        if let Some(s) = s.upgrade() {
                            s.midia.comando(cmd);
                        }
                    });
                    controles.append(&b);
                }
                row.append(&controles);
                c.append(&row);
                let ca = Animavel::new(&c);
                t.cartoes.append(&ca);
                anims.push(ca);
            }
            if !nao_lidas.is_empty() {
                let c = card("sino", tr("NOTIFICATIONS"));
                let l = rotulo(&if nao_lidas.len() == 1 { tr("1 new").to_string() } else { trf("{n} new", &[("n", nao_lidas.len().to_string())]) }, "bloqueio-cartao-titulo");
                l.set_xalign(0.0);
                c.append(&l);
                let mut apps: Vec<String> = Vec::new();
                for a in &nao_lidas {
                    let a = if a.is_empty() { tr("Notice").to_string() } else { a.clone() };
                    if !apps.contains(&a) {
                        apps.push(a);
                    }
                }
                // what they say, when the person chose to see it here; only the apps otherwise
                let linha = if lock_screen == crate::notification_policy::LockScreen::All {
                    unread.iter().take(2).map(|(a, t)| if a.is_empty() { t.clone() } else { format!("{a}: {t}") }).collect::<Vec<_>>().join(" · ")
                } else {
                    apps.into_iter().take(3).collect::<Vec<_>>().join(" · ")
                };
                let s = rotulo(&linha, "bloqueio-cartao-sub");
                s.set_ellipsize(gtk::pango::EllipsizeMode::End);
                s.set_xalign(0.0);
                c.append(&s);
                let ca = Animavel::new(&c);
                t.cartoes.append(&ca);
                anims.push(ca);
            }
            mv::cascata(&anims, 8.0);
            *t.anims.borrow_mut() = anims;
        }
    }

    /// The cards step aside (down 24 px and out) in the 24 ms cascade, or come back.
    fn cartoes(&self, fora: bool) {
        for t in self.telas.borrow().iter() {
            t.cartoes.set_can_target(!fora);
            for (i, a) in t.anims.borrow().iter().enumerate() {
                let a = a.clone();
                glib::timeout_add_local_once(Duration::from_millis(24 * i as u64), move || {
                    a.ir(Canal::Y, if fora { 24.0 } else { 0.0 }, mv::SUAVE).ir(Canal::Opacidade, if fora { 0.0 } else { 1.0 }, mv::SUAVE);
                });
            }
        }
    }

    /// The first touch: the clock goes away, the avatar becomes the field.
    fn pedir(&self) {
        self.pedindo.set(true);
        self.ultima_tecla.set(glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0));
        for t in self.telas.borrow().iter() {
            t.relogio.ir(Canal::Y, -24.0, mv::SUAVE).ir(Canal::Opacidade, 0.0, mv::SUAVE);
            if t.principal {
                // the small avatar goes; the photo comes big to the middle, with the field (design 130156)
                t.dica.set_visible(false);
                t.avatar.set_visible(false);
                t.senha.set_visible(true);
                t.centro.set_visible(true);
                t.centro.entrar((0.0, 16.0), 0.96);
                t.topo.set_visible(true);
                t.topo.entrar((0.0, -8.0), 1.0);
                t.erro.set_text("");
                t.entrada.grab_focus();
            }
        }
        self.cartoes(true);
        if debug() {
            eprintln!("debug bloqueio: pedindo a senha");
        }
    }

    /// Back to "press any key".
    fn voltar(&self) {
        self.pedindo.set(false);
        for t in self.telas.borrow().iter() {
            t.relogio.ir(Canal::Y, 0.0, mv::SUAVE).ir(Canal::Opacidade, 1.0, mv::SUAVE);
            if t.principal {
                t.entrada.set_text("");
                t.senha.set_visible(false);
                t.centro.set_visible(false);
                t.topo.set_visible(false);
                t.avatar.set_visible(true);
                t.dica.set_visible(true);
            }
        }
        self.cartoes(false);
        // what changed while the field was open
        self.preencher_cartoes();
    }

    fn conferir(self: &Rc<Self>, senha: String) {
        if self.conferindo.get() || senha.is_empty() {
            return;
        }
        self.conferindo.set(true);
        for t in self.telas.borrow().iter().filter(|t| t.principal) {
            t.entrada.set_sensitive(false);
            t.erro.set_text(tr("Checking…"));
        }
        let usuario = glib::user_name().to_string_lossy().into_owned();
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(move || crate::pam::check(&usuario, &senha), move |ok| {
            let Some(b) = w.upgrade() else { return };
            b.conferindo.set(false);
            if ok {
                b.desbloquear();
                return;
            }
            if debug() {
                eprintln!("debug bloqueio: senha incorreta");
            }
            crate::configuracoes_som::tocar("kalango-auth-failed");
            for t in b.telas.borrow().iter().filter(|t| t.principal) {
                t.entrada.set_sensitive(true);
                t.entrada.set_text("");
                t.erro.set_text(tr("Wrong password. Try again."));
                t.entrada.grab_focus();
                // a short shake (quick spring)
                let s = t.senha.clone();
                s.add_css_class("tremendo");
                glib::timeout_add_local_once(Duration::from_millis(400), move || s.remove_css_class("tremendo"));
            }
        });
    }

    /// logind's Lock and Unlock for this session (`loginctl lock-session`, the power menu, idle): the real path.
    fn ouvir_logind(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        gtk::gio::bus_get(gtk::gio::BusType::System, gtk::gio::Cancellable::NONE, move |c| {
            let Ok(c) = c else { return };
            let pid = std::process::id();
            let c2 = c.clone();
            c.call(Some("org.freedesktop.login1"), "/org/freedesktop/login1", "org.freedesktop.login1.Manager", "GetSessionByPID",
                Some(&(pid,).to_variant()), None, gtk::gio::DBusCallFlags::NONE, 2000, gtk::gio::Cancellable::NONE, move |r| {
                    let Ok(v) = r else { return };
                    let Some(path) = v.child_value(0).str().map(str::to_string) else { return };
                    for (sinal, travar) in [("Lock", true), ("Unlock", false)] {
                        let w = w.clone();
                        c2.signal_subscribe(Some("org.freedesktop.login1"), Some("org.freedesktop.login1.Session"), Some(sinal), Some(&path), None,
                            gtk::gio::DBusSignalFlags::NONE, move |_, _, _, _, _, _| {
                                if let Some(b) = w.upgrade() {
                                    if travar {
                                        b.bloquear();
                                    } else {
                                        b.desbloquear();
                                    }
                                }
                            });
                    }
                });
        });
    }
}
