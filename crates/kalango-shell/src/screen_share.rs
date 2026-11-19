//! Sharing the screen (design "Compartilhar tela"): an app asks to see the screen (a call, a meeting) and the system
//! chooses, never the app — the whole screen or one window, each with its picture; "Esconder notificações enquanto
//! compartilha" holds the notices while it lasts. The privacy chip (blue) shows who receives it, and its switch stops it.
//!
//! Apps reach it through xdg-desktop-portal's ScreenCast, served by the shell's portal backend (portal.rs). The chosen
//! source is recorded by niri (its org.gnome.Mutter.ScreenCast: RecordMonitor or RecordWindow), and the PipeWire node
//! it gives goes back to the portal, which hands the app only that stream.

use crate::conversa::ui_icon;
use crate::superficies::{caixa, camada, rotulo};
use gtk::prelude::*;
use gtk::{gio, glib, Align, Orientation};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kalango_i18n::{tr, trf};
use kalango_movimento::Animavel;
use serde_json::json;
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

pub(crate) const MUTTER: &str = "org.gnome.Mutter.ScreenCast";
pub(crate) const MUTTER_PATH: &str = "/org/gnome/Mutter/ScreenCast";
/// Portal source types: monitor, window.
pub const SOURCE_MONITOR: u32 = 1;
pub const SOURCE_WINDOW: u32 = 2;

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

/// What was chosen.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    Monitor { connector: String, size: (i32, i32) },
    Window { id: u64, title: String },
}

/// A portal session: what the app asked for, and, once started, niri's session.
#[derive(Default)]
struct Session {
    app: String,
    types: u32,
    cursor: u32,
    mutter: Option<String>,
    hide_notices: bool,
    closed_sub: Option<gio::SignalSubscriptionId>,
}

thread_local! {
    static SESSIONS: RefCell<BTreeMap<String, Session>> = RefCell::default();
}

/// Whether a sharing that hides the notices is going on.
pub fn hides_notices() -> bool {
    SESSIONS.with(|s| s.borrow().values().any(|x| x.mutter.is_some() && x.hide_notices))
}

// ---------------------------------------------------------------------------------------------------- the chooser

pub struct Choice {
    pub source: Source,
    pub hide_notices: bool,
}

struct Chooser {
    window: gtk::ApplicationWindow,
    card: Animavel,
    icon: gtk::Image,
    title: gtk::Label,
    tabs: gtk::Box,
    grid: gtk::FlowBox,
    hide: gtk::Switch,
    share: gtk::Button,
    tab: Cell<u32>,
    types: Cell<u32>,
    chosen: RefCell<Option<Source>>,
    done: RefCell<Option<Box<dyn FnOnce(Option<Choice>)>>>,
}

thread_local! {
    static CHOOSER: RefCell<Option<Rc<Chooser>>> = const { RefCell::new(None) };
}

fn chooser() -> Option<Rc<Chooser>> {
    if let Some(c) = CHOOSER.with(|c| c.borrow().clone()) {
        return Some(c);
    }
    let app = gio::Application::default().and_downcast::<gtk::Application>()?;
    let window = camada(&app, "kalango-compartilhar", Layer::Overlay, &[Edge::Top, Edge::Bottom, Edge::Left, Edge::Right]);
    window.add_css_class("wifi-fundo");
    crate::superficies::take_keyboard(&window);
    window.set_exclusive_zone(-1);
    let body = caixa(Orientation::Vertical, 14, "wifi-cartao");
    body.add_css_class("compartilhar-cartao");
    body.set_size_request(860, -1);
    let head = caixa(Orientation::Horizontal, 14, "");
    let icon = gtk::Image::new();
    icon.set_pixel_size(32);
    icon.set_valign(Align::Center);
    head.append(&icon);
    let tt = caixa(Orientation::Vertical, 2, "");
    tt.set_valign(Align::Center);
    let title = rotulo("", "wifi-titulo");
    title.set_xalign(0.0);
    let sub = rotulo(tr("choose what to show — the app sees only that"), "wifi-subtitulo");
    sub.set_xalign(0.0);
    tt.append(&title);
    tt.append(&sub);
    head.append(&tt);
    body.append(&head);
    let tabs = caixa(Orientation::Horizontal, 2, "compartilhar-abas");
    tabs.set_halign(Align::Start);
    body.append(&tabs);
    let grid = gtk::FlowBox::new();
    grid.set_selection_mode(gtk::SelectionMode::None);
    grid.set_max_children_per_line(3);
    grid.set_min_children_per_line(3);
    grid.set_column_spacing(12);
    grid.set_row_spacing(12);
    grid.set_homogeneous(true);
    let rolagem = gtk::ScrolledWindow::new();
    rolagem.set_hscrollbar_policy(gtk::PolicyType::Never);
    rolagem.set_propagate_natural_height(true);
    rolagem.set_max_content_height(440);
    rolagem.set_child(Some(&grid));
    body.append(&rolagem);
    // what goes with it
    let linha = caixa(Orientation::Horizontal, 12, "compartilhar-opcao");
    linha.append(&ui_icon("sino-mudo", "#8E8F99", 16));
    let l = rotulo(tr("Hide notifications while sharing"), "foco-regra-titulo");
    l.set_xalign(0.0);
    l.set_hexpand(true);
    linha.append(&l);
    let hide = gtk::Switch::new();
    hide.set_active(true);
    hide.set_valign(Align::Center);
    linha.append(&hide);
    body.append(&linha);
    let acoes = caixa(Orientation::Horizontal, 8, "");
    acoes.set_halign(Align::End);
    let cancel = gtk::Button::with_label(tr("Cancel"));
    cancel.add_css_class("wifi-botao");
    let share = gtk::Button::with_label(tr("Share"));
    share.add_css_class("wifi-botao");
    share.add_css_class("principal");
    share.set_sensitive(false);
    acoes.append(&cancel);
    acoes.append(&share);
    body.append(&acoes);
    let card = Animavel::new(&body);
    card.origem(0.5, 0.5);
    card.set_halign(Align::Center);
    card.set_valign(Align::Center);
    window.set_child(Some(&card));
    let c = Rc::new(Chooser {
        window, card, icon, title, tabs, grid, hide, share: share.clone(), tab: Cell::new(SOURCE_WINDOW), types: Cell::new(3),
        chosen: RefCell::default(), done: RefCell::default(),
    });
    let w = Rc::downgrade(&c);
    share.connect_clicked(move |_| if let Some(c) = w.upgrade() { c.answer(true) });
    let w = Rc::downgrade(&c);
    cancel.connect_clicked(move |_| if let Some(c) = w.upgrade() { c.answer(false) });
    if debug() {
        crate::central::posicao_debug("compartilhar botao", share.upcast_ref());
    }
    let keys = gtk::EventControllerKey::new();
    let w = Rc::downgrade(&c);
    keys.connect_key_pressed(move |_, k, _, _| {
        if k == gtk::gdk::Key::Escape {
            if let Some(c) = w.upgrade() {
                c.answer(false);
            }
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    c.window.add_controller(keys);
    CHOOSER.with(|x| *x.borrow_mut() = Some(c.clone()));
    Some(c)
}

/// The screens, from niri: (connector, logical size).
fn monitors() -> Vec<(String, (i32, i32))> {
    let Ok(v) = crate::servicos::niri_pedido(json!("Outputs")) else { return vec![] };
    v.get("Outputs").and_then(|o| o.as_object()).map(|o| {
        o.iter().filter_map(|(nome, s)| {
            let l = s.get("logical").filter(|l| !l.is_null())?;
            Some((nome.clone(), (l["width"].as_i64().unwrap_or(0) as i32, l["height"].as_i64().unwrap_or(0) as i32)))
        }).collect()
    }).unwrap_or_default()
}

fn app_label(app_id: &str, title: &str) -> String {
    gio::DesktopAppInfo::new(&format!("{app_id}.desktop")).map(|a| a.name().to_string()).unwrap_or_else(|| title.to_string())
}

fn app_gicon(app_id: &str) -> gio::Icon {
    gio::DesktopAppInfo::new(&format!("{app_id}.desktop")).and_then(|a| a.icon()).unwrap_or_else(|| gio::ThemedIcon::new("application-x-executable").upcast())
}

impl Chooser {
    fn render_tabs(self: &Rc<Self>) {
        while let Some(c) = self.tabs.first_child() {
            self.tabs.remove(&c);
        }
        for (t, nome) in [(SOURCE_MONITOR, tr("Whole screen")), (SOURCE_WINDOW, tr("One window"))] {
            if self.types.get() & t == 0 {
                continue;
            }
            let b = gtk::Button::with_label(nome);
            b.add_css_class("compartilhar-aba");
            if self.tab.get() == t {
                b.add_css_class("ativa");
            }
            let w = Rc::downgrade(self);
            b.connect_clicked(move |_| {
                if let Some(c) = w.upgrade() {
                    c.tab.set(t);
                    c.chosen.borrow_mut().take();
                    c.render_tabs();
                    c.render_sources();
                }
            });
            if debug() {
                crate::central::posicao_debug(if t == SOURCE_MONITOR { "compartilhar aba tela" } else { "compartilhar aba janela" }, b.upcast_ref());
            }
            self.tabs.append(&b);
        }
    }

    fn render_sources(self: &Rc<Self>) {
        while let Some(c) = self.grid.first_child() {
            self.grid.remove(&c);
        }
        self.share.set_sensitive(false);
        let mut fontes: Vec<(Source, String, gio::Icon)> = Vec::new();
        if self.tab.get() == SOURCE_MONITOR {
            for (nome, size) in monitors() {
                fontes.push((Source::Monitor { connector: nome.clone(), size }, trf("Screen {name} · {w}×{h}", &[("name", nome), ("w", size.0.to_string()), ("h", size.1.to_string())]),
                    gio::ThemedIcon::new("video-display").upcast()));
            }
        } else if let Some(sh) = crate::shell_atual() {
            let mut js: Vec<_> = sh.estado.janelas.borrow().values().cloned().collect();
            js.sort_by_key(|j| std::cmp::Reverse(j.foco_em));
            for j in js {
                let app = app_label(&j.app_id, &j.titulo);
                // the title without the app's name the app already puts at its end ("notas.md - Editor de Texto")
                let titulo = [" - ", " — ", " – "].iter().find_map(|sep| j.titulo.strip_suffix(&format!("{sep}{app}"))).unwrap_or(&j.titulo).trim().to_string();
                let rotulo = if titulo.is_empty() || titulo == app { app.clone() } else { format!("{titulo} — {app}") };
                fontes.push((Source::Window { id: j.id, title: rotulo.clone() }, rotulo, app_gicon(&j.app_id)));
            }
        }
        if debug() {
            eprintln!("debug compartilhar: {} {}", fontes.len(), if self.tab.get() == SOURCE_MONITOR { "telas" } else { "janelas" });
        }
        for (i, (fonte, rotulo_txt, gicon)) in fontes.into_iter().enumerate() {
            let tile = gtk::Button::new();
            tile.add_css_class("compartilhar-fonte");
            let col = caixa(Orientation::Vertical, 8, "");
            let quadro = gtk::Overlay::new();
            let fundo = caixa(Orientation::Vertical, 0, "compartilhar-quadro");
            fundo.set_size_request(250, 150);
            let ic = gtk::Image::from_gicon(&gicon);
            ic.set_pixel_size(40);
            ic.set_vexpand(true);
            ic.set_valign(Align::Center);
            fundo.append(&ic);
            quadro.set_child(Some(&fundo));
            if let Source::Window { id, .. } = &fonte {
                thumbnail(*id, &quadro);
            }
            col.append(&quadro);
            let r = caixa(Orientation::Horizontal, 8, "");
            let mini = gtk::Image::from_gicon(&gicon);
            mini.set_pixel_size(16);
            r.append(&mini);
            let l = rotulo(&rotulo_txt, "compartilhar-nome");
            l.set_xalign(0.0);
            l.set_ellipsize(gtk::pango::EllipsizeMode::End);
            l.set_max_width_chars(1);
            l.set_hexpand(true);
            r.append(&l);
            col.append(&r);
            tile.set_child(Some(&col));
            let w = Rc::downgrade(self);
            let tw = tile.downgrade();
            tile.connect_clicked(move |_| {
                let (Some(c), Some(t)) = (w.upgrade(), tw.upgrade()) else { return };
                let mut x = c.grid.first_child();
                while let Some(ch) = x {
                    if let Some(b) = ch.first_child() {
                        b.remove_css_class("escolhida");
                    }
                    x = ch.next_sibling();
                }
                t.add_css_class("escolhida");
                *c.chosen.borrow_mut() = Some(fonte.clone());
                c.share.set_sensitive(true);
                if debug() {
                    eprintln!("debug compartilhar: escolhida {:?}", fonte);
                }
            });
            if debug() {
                crate::central::posicao_debug(&format!("compartilhar fonte {i}"), tile.upcast_ref());
            }
            self.grid.append(&tile);
            // the first one (the window in focus, or the screen) comes chosen: Compartilhar is ready
            if i == 0 {
                tile.emit_clicked();
            }
        }
    }

    fn answer(self: &Rc<Self>, share: bool) {
        let Some(done) = self.done.borrow_mut().take() else { return };
        let escolha = share.then(|| self.chosen.borrow().clone()).flatten().map(|source| Choice { source, hide_notices: self.hide.is_active() });
        if share && escolha.is_none() {
            *self.done.borrow_mut() = Some(done);
            return;
        }
        if debug() {
            eprintln!("debug compartilhar: {}", if escolha.is_some() { "compartilhar" } else { "cancelado" });
        }
        self.window.set_keyboard_mode(KeyboardMode::None);
        self.window.remove_css_class("aberto");
        let w = self.window.downgrade();
        let me = Rc::downgrade(self);
        self.card.sair((0.0, 8.0), 0.98, move || {
            if let Some(w) = w.upgrade().filter(|_| me.upgrade().is_some_and(|c| c.done.borrow().is_none())) {
                w.set_visible(false);
            }
        });
        done(escolha);
    }
}

/// A window's picture, from niri (as the window switcher's), over its icon.
fn thumbnail(id: u64, quadro: &gtk::Overlay) {
    let path = crate::alternador::thumbs_dir().join(format!("compartilhar-{id}.png"));
    let _ = std::fs::remove_file(&path);
    if crate::servicos::niri_acao(json!({"ThumbnailWindow": {"id": id, "path": path.display().to_string(), "max_size": 500}})).is_err() {
        return;
    }
    let q = quadro.downgrade();
    let t0 = Instant::now();
    glib::timeout_add_local(Duration::from_millis(30), move || {
        let Some(q) = q.upgrade() else { return glib::ControlFlow::Break };
        if path.is_file() {
            let Some(pb) = crate::alternador::carregar_miniatura(&path) else { return glib::ControlFlow::Continue };
            #[allow(deprecated)]
            let tex = gtk::gdk::Texture::for_pixbuf(&pb);
            let pic = gtk::Picture::for_paintable(&tex);
            pic.set_content_fit(gtk::ContentFit::Cover);
            pic.add_css_class("compartilhar-imagem");
            q.add_overlay(&pic);
            let _ = std::fs::remove_file(&path);
            return glib::ControlFlow::Break;
        }
        if t0.elapsed() > Duration::from_secs(2) {
            return glib::ControlFlow::Break;
        }
        glib::ControlFlow::Continue
    });
}

/// Asks what to share; `done` gets the choice, or None (cancelled, or another request came first).
pub fn choose(app: &str, app_id: &str, types: u32, done: impl FnOnce(Option<Choice>) + 'static) {
    let Some(c) = chooser() else {
        done(None);
        return;
    };
    if let Some(before) = c.done.borrow_mut().take() {
        before(None);
    }
    let types = if types & (SOURCE_MONITOR | SOURCE_WINDOW) == 0 { SOURCE_MONITOR | SOURCE_WINDOW } else { types & (SOURCE_MONITOR | SOURCE_WINDOW) };
    c.types.set(types);
    c.tab.set(if types & SOURCE_WINDOW != 0 { SOURCE_WINDOW } else { SOURCE_MONITOR });
    c.chosen.borrow_mut().take();
    let nome = if app.is_empty() { tr("An app").to_string() } else { app.to_string() };
    c.title.set_text(&trf("{app} wants to see your screen", &[("app", nome.clone())]));
    c.icon.set_from_gicon(&app_gicon(app_id));
    if debug() {
        eprintln!("debug compartilhar: pedido de «{nome}»");
    }
    c.render_tabs();
    c.render_sources();
    *c.done.borrow_mut() = Some(Box::new(done));
    c.window.present();
    crate::superficies::take_keyboard(&c.window);
    c.window.add_css_class("aberto");
    c.card.entrar((0.0, 10.0), 0.96);
}

/// The portal gave up (Request.Close): the chooser closes.
pub fn withdraw() {
    if let Some(c) = CHOOSER.with(|c| c.borrow().clone()) {
        c.answer(false);
    }
}

// ---------------------------------------------------------------------------------------------------- the portal side

pub(crate) fn mutter_call(c: &gio::DBusConnection, path: &str, iface: &str, method: &str, args: Option<glib::Variant>, reply: Option<&str>,
    done: impl FnOnce(Result<glib::Variant, glib::Error>) + 'static) {
    let ty = reply.map(|r| glib::VariantType::new(r).expect("tipo"));
    c.call(Some(MUTTER), path, iface, method, args.as_ref(), ty.as_deref(), gio::DBusCallFlags::NONE, 8000, gio::Cancellable::NONE, done);
}

pub(crate) fn dict(entries: &[(&str, glib::Variant)]) -> glib::Variant {
    let d = glib::VariantDict::new(None);
    for (k, v) in entries {
        d.insert_value(k, v);
    }
    d.end()
}

/// CreateSession: the app's session (its object at `session_handle`, Close stops the sharing).
pub fn create_session(c: &gio::DBusConnection, session_handle: &str, app: &str) {
    SESSIONS.with(|s| s.borrow_mut().insert(session_handle.to_string(), Session { app: app.to_string(), types: SOURCE_MONITOR | SOURCE_WINDOW, ..Default::default() }));
    let info = gio::DBusNodeInfo::for_xml(r#"<node><interface name="org.freedesktop.impl.portal.Session"><method name="Close"/><signal name="Closed"/><property name="version" type="u" access="read"/></interface></node>"#).expect("Session XML");
    let iface = info.lookup_interface("org.freedesktop.impl.portal.Session").expect("Session");
    let (h, c2) = (session_handle.to_string(), c.clone());
    let r = c.register_object(session_handle, &iface).method_call(move |_, _, _, _, _, _, inv| {
        stop(&c2, &h, false);
        inv.return_value(None);
    }).property(|_, _, _, _, _| 1u32.to_variant()).build();
    if let Ok(id) = r {
        // the object goes with the session
        SESSION_OBJECTS.with(|o| o.borrow_mut().insert(session_handle.to_string(), id));
    }
    if debug() {
        eprintln!("debug compartilhar: sessão criada para «{app}»");
    }
}

thread_local! {
    static SESSION_OBJECTS: RefCell<BTreeMap<String, gio::RegistrationId>> = RefCell::default();
}

/// SelectSources: what the app accepts (monitor, window) and how the cursor goes.
pub fn select_sources(session_handle: &str, types: u32, cursor: u32) {
    SESSIONS.with(|s| {
        if let Some(x) = s.borrow_mut().get_mut(session_handle) {
            if types != 0 {
                x.types = types;
            }
            x.cursor = cursor;
        }
    });
}

/// Start: the chooser, then niri records the choice; `reply` gets (response, results).
pub fn start(c: &gio::DBusConnection, session_handle: &str, app_id: &str, reply: impl FnOnce(u32, glib::Variant) + 'static) {
    let Some((app, types, cursor)) = SESSIONS.with(|s| s.borrow().get(session_handle).map(|x| (x.app.clone(), x.types, x.cursor))) else {
        reply(2, dict(&[]));
        return;
    };
    let (c, h) = (c.clone(), session_handle.to_string());
    choose(&app, app_id, types, move |escolha| {
        let Some(escolha) = escolha else {
            reply(1, dict(&[]));
            return;
        };
        record(&c, &h, escolha, cursor, reply);
    });
}

/// The portal's cursor modes (1 hidden, 2 embedded, 4 metadata) as niri's (0, 1, 2).
pub(crate) fn mutter_cursor(portal: u32) -> u32 {
    match portal {
        2 => 1,
        4 => 2,
        _ => 0,
    }
}

fn record(c: &gio::DBusConnection, h: &str, escolha: Choice, cursor: u32, reply: impl FnOnce(u32, glib::Variant) + 'static) {
    let reply: Rc<RefCell<Option<Box<dyn FnOnce(u32, glib::Variant)>>>> = Rc::new(RefCell::new(Some(Box::new(reply))));
    let falhou = {
        let reply = reply.clone();
        move |motivo: String| {
            if debug() {
                eprintln!("debug compartilhar: não começou ({motivo})");
            }
            if let Some(f) = reply.borrow_mut().take() {
                f(2, dict(&[]));
            }
            if let Some(s) = crate::shell_atual() {
                s.notificar(tr("Screen sharing"), tr("The screen could not be shared"), &motivo);
            }
        }
    };
    let (c2, h2) = (c.clone(), h.to_string());
    mutter_call(c, MUTTER_PATH, MUTTER, "CreateSession", Some(glib::Variant::tuple_from_iter([dict(&[])])), Some("(o)"), move |r| {
        let sessao = match r.map(|v| v.child_value(0).get::<glib::variant::ObjectPath>().map(|p| p.as_str().to_string())) {
            Ok(Some(p)) => p,
            Ok(None) => return falhou("CreateSession".into()),
            Err(e) => return falhou(e.message().to_string()),
        };
        let (metodo, args, source_type) = match &escolha.source {
            Source::Monitor { connector, .. } => ("RecordMonitor", glib::Variant::tuple_from_iter([connector.to_variant(), dict(&[("cursor-mode", mutter_cursor(cursor).to_variant())])]), SOURCE_MONITOR),
            Source::Window { id, .. } => ("RecordWindow", glib::Variant::tuple_from_iter([dict(&[("window-id", id.to_variant()), ("cursor-mode", mutter_cursor(cursor).to_variant())])]), SOURCE_WINDOW),
        };
        let (c3, h3, s3) = (c2.clone(), h2.clone(), sessao.clone());
        mutter_call(&c2, &sessao, "org.gnome.Mutter.ScreenCast.Session", metodo, Some(args), Some("(o)"), move |r| {
            let stream = match r.map(|v| v.child_value(0).get::<glib::variant::ObjectPath>().map(|p| p.as_str().to_string())) {
                Ok(Some(p)) => p,
                Ok(None) => return falhou(metodo.into()),
                Err(e) => return falhou(e.message().to_string()),
            };
            // the node comes as a signal once niri's stream is ready
            let esperando: Rc<Cell<Option<gio::SignalSubscriptionId>>> = Rc::default();
            let falhou = Rc::new(RefCell::new(Some(falhou)));
            let (c4, h4, s4, st4, e4, f4) = (c3.clone(), h3.clone(), s3.clone(), stream.clone(), esperando.clone(), falhou.clone());
            let hide = escolha.hide_notices;
            let sub = c3.signal_subscribe(None, Some("org.gnome.Mutter.ScreenCast.Stream"), Some("PipeWireStreamAdded"), Some(&stream), None,
                gio::DBusSignalFlags::NONE, move |_, _, _, _, _, params| {
                    let Some(no) = params.child_value(0).get::<u32>() else { return };
                    if let Some(id) = e4.take() {
                        c4.signal_unsubscribe(id);
                    }
                    f4.borrow_mut().take();
                    let (c5, h5, s5) = (c4.clone(), h4.clone(), s4.clone());
                    let reply = reply.clone();
                    // its place and size (a monitor's; a window's are its own)
                    c4.call(Some(MUTTER), &st4, "org.freedesktop.DBus.Properties", "Get", Some(&("org.gnome.Mutter.ScreenCast.Stream", "Parameters").to_variant()),
                        None, gio::DBusCallFlags::NONE, 3000, gio::Cancellable::NONE, move |r| {
                            let params = r.ok().and_then(|v| v.child_value(0).as_variant());
                            let pd = params.map(|p| glib::VariantDict::new(Some(&p)));
                            let pos = pd.as_ref().and_then(|d| d.lookup_value("position", None)).and_then(|v| v.get::<(i32, i32)>()).unwrap_or((0, 0));
                            let size = pd.as_ref().and_then(|d| d.lookup_value("size", None)).and_then(|v| v.get::<(i32, i32)>()).unwrap_or((0, 0));
                            let props = dict(&[("position", pos.to_variant()), ("size", size.to_variant()), ("source_type", source_type.to_variant())]);
                            let item = glib::Variant::tuple_from_iter([no.to_variant(), props]);
                            let streams = glib::Variant::array_from_iter_with_type(glib::VariantTy::new("(ua{sv})").expect("tipo"), [item]);
                            started(&c5, &h5, &s5, hide);
                            if debug() {
                                eprintln!("debug compartilhar: transmitindo (nó {no})");
                            }
                            if let Some(f) = reply.borrow_mut().take() {
                                f(0, dict(&[("streams", streams), ("persist_mode", 0u32.to_variant())]));
                            }
                        });
                });
            esperando.set(Some(sub));
            let (c6, e6, f6) = (c3.clone(), esperando.clone(), falhou.clone());
            mutter_call(&c3, &s3, "org.gnome.Mutter.ScreenCast.Session", "Start", None, None, move |r| {
                if let Err(e) = r {
                    if let Some(id) = e6.take() {
                        c6.signal_unsubscribe(id);
                    }
                    if let Some(f) = f6.borrow_mut().take() {
                        f(e.message().to_string());
                    }
                }
            });
            // a stream that never comes (a compositor without casting): not started
            let (c7, e7, f7, s7) = (c3.clone(), esperando, falhou, s3.clone());
            glib::timeout_add_local_once(Duration::from_secs(6), move || {
                if let Some(id) = e7.take() {
                    c7.signal_unsubscribe(id);
                    mutter_call(&c7, &s7, "org.gnome.Mutter.ScreenCast.Session", "Stop", None, None, |_| {});
                    if let Some(f) = f7.borrow_mut().take() {
                        f(tr("The compositor did not give the stream.").to_string());
                    }
                }
            });
        });
    });
}

/// Sharing began: niri's session is kept (to stop it), and its end closes the app's session.
fn started(c: &gio::DBusConnection, h: &str, mutter: &str, hide: bool) {
    let (c2, h2) = (c.clone(), h.to_string());
    let sub = c.signal_subscribe(None, Some("org.gnome.Mutter.ScreenCast.Session"), Some("Closed"), Some(mutter), None, gio::DBusSignalFlags::NONE,
        move |_, _, _, _, _, _| stop(&c2, &h2, true));
    SESSIONS.with(|s| {
        if let Some(x) = s.borrow_mut().get_mut(h) {
            x.mutter = Some(mutter.to_string());
            x.hide_notices = hide;
            x.closed_sub = Some(sub);
        }
    });
}

/// The sharing ends: niri stops recording, the app's session is told it closed (unless it closed it itself).
fn stop(c: &gio::DBusConnection, h: &str, tell_app: bool) {
    let Some(sessao) = SESSIONS.with(|s| s.borrow_mut().remove(h)) else { return };
    if let Some(id) = sessao.closed_sub {
        c.signal_unsubscribe(id);
    }
    if let Some(m) = &sessao.mutter {
        mutter_call(c, m, "org.gnome.Mutter.ScreenCast.Session", "Stop", None, None, |_| {});
    }
    if tell_app || sessao.mutter.is_some() {
        let _ = c.emit_signal(None, h, "org.freedesktop.impl.portal.Session", "Closed", None);
    }
    if let Some(id) = SESSION_OBJECTS.with(|o| o.borrow_mut().remove(h)) {
        let _ = c.unregister_object(id);
    }
    if debug() {
        eprintln!("debug compartilhar: parou ({})", sessao.app);
    }
}

/// Stops every sharing (the privacy panel's switch, "Pausar tudo").
pub fn stop_all() {
    let Ok(c) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) else { return };
    let hs: Vec<String> = SESSIONS.with(|s| s.borrow().iter().filter(|(_, x)| x.mutter.is_some()).map(|(h, _)| h.clone()).collect());
    for h in hs {
        stop(&c, &h, true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_streams_reply_is_ua_sv() {
        let props = dict(&[("position", (0i32, 0i32).to_variant()), ("size", (1920i32, 1080i32).to_variant()), ("source_type", 1u32.to_variant())]);
        let item = glib::Variant::tuple_from_iter([7u32.to_variant(), props]);
        let streams = glib::Variant::array_from_iter_with_type(glib::VariantTy::new("(ua{sv})").unwrap(), [item]);
        assert_eq!(streams.type_().as_str(), "a(ua{sv})");
        assert_eq!(mutter_cursor(2), 1);
        assert_eq!(mutter_cursor(1), 0);
    }
}
