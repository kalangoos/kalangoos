//! The permission request (design "Permissão de app (microfone)"): an app (or an agent) asks for the microphone, the
//! camera, the location or the speakers in a card over the dimmed screen — the app's icon › the sensor, what it wants
//! and why, and three choices stacked so the safest is never hidden: "Permitir enquanto o app estiver em uso",
//! "Só desta vez" and "Não permitir". The footer says the bar's dot will show it (privacy_now.rs) and where to change it.
//!
//! Apps reach it through xdg-desktop-portal: the shell's portal backend serves org.freedesktop.impl.portal.Access
//! (portal.rs), which the frontend calls for its Camera, Device (microphone, speakers) and Location portals and keeps
//! the answer in the permission store. "Só desta vez" answers yes and takes the stored permission back once the app
//! has no window left.

use crate::conversa::ui_icon;
use crate::superficies::{caixa, camada, rotulo};
use gtk::prelude::*;
use gtk::{gio, glib, Align, Orientation};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kalango_i18n::{tr, trf};
use kalango_movimento::Animavel;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

/// What is asked for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sensor {
    Microphone,
    Camera,
    Location,
    Speakers,
}

impl Sensor {
    /// From the portal's icon name (the frontend's own: audio-input-microphone, camera-web, find-location, audio-speakers).
    pub fn from_icon(icon: &str) -> Option<Sensor> {
        if icon.contains("microphone") || icon.contains("audio-input") {
            Some(Sensor::Microphone)
        } else if icon.contains("camera") {
            Some(Sensor::Camera)
        } else if icon.contains("location") {
            Some(Sensor::Location)
        } else if icon.contains("speaker") || icon.contains("audio-output") {
            Some(Sensor::Speakers)
        } else {
            None
        }
    }

    /// Where xdg-desktop-portal keeps the answer: (table, id) in the permission store.
    pub fn store_entry(self) -> (&'static str, &'static str) {
        match self {
            Sensor::Microphone => ("devices", "microphone"),
            Sensor::Camera => ("devices", "camera"),
            Sensor::Speakers => ("devices", "speakers"),
            Sensor::Location => ("location", "location"),
        }
    }

    fn glyph(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Sensor::Microphone => ("microfone", "#EFA2A6", "coral"),
            Sensor::Camera => ("camera", "#EFA2A6", "coral"),
            Sensor::Location => ("local", "#E5C98C", "ouro"),
            Sensor::Speakers => ("volume", "#7CC3DF", "azul"),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Sensor::Microphone => "microfone",
            Sensor::Camera => "câmera",
            Sensor::Location => "localização",
            Sensor::Speakers => "alto-falantes",
        }
    }
}

/// The person's answer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Choice {
    /// while the app is in use (the permission stays)
    Always,
    /// this time only (taken back when the app closes)
    Once,
    Deny,
}

pub struct Request {
    /// the app as the person knows it
    pub app: String,
    /// its desktop id ("com.discordapp.Discord"), for its icon and to take a "this time only" back
    pub app_id: String,
    /// None: a request the dialog has no words of its own for (the portal's title and body are shown)
    pub sensor: Option<Sensor>,
    pub title: String,
    /// why, in the app's (or the portal's) words; empty: the dialog says what the access means
    pub reason: String,
}

type Done = Box<dyn FnOnce(Choice)>;

struct Dialog {
    window: gtk::ApplicationWindow,
    card: Animavel,
    app_icon: gtk::Image,
    tile: gtk::Box,
    title: gtk::Label,
    body: gtk::Label,
    always: gtk::Button,
    footer: gtk::Label,
    done: RefCell<Option<Done>>,
    sensor_class: RefCell<&'static str>,
}

thread_local! {
    static DIALOG: RefCell<Option<Rc<Dialog>>> = const { RefCell::new(None) };
}

fn dialog() -> Option<Rc<Dialog>> {
    if let Some(d) = DIALOG.with(|d| d.borrow().clone()) {
        return Some(d);
    }
    let app = gio::Application::default().and_downcast::<gtk::Application>()?;
    let window = camada(&app, "kalango-permissao", Layer::Overlay, &[Edge::Top, Edge::Bottom, Edge::Left, Edge::Right]);
    window.add_css_class("wifi-fundo");
    crate::superficies::take_keyboard(&window);
    window.set_exclusive_zone(-1);
    let body_box = caixa(Orientation::Vertical, 12, "wifi-cartao");
    body_box.add_css_class("permissao-cartao");
    body_box.set_size_request(460, -1);
    // the app › the sensor
    let head = caixa(Orientation::Horizontal, 12, "");
    let app_icon = gtk::Image::new();
    app_icon.set_pixel_size(36);
    app_icon.set_valign(Align::Center);
    head.append(&app_icon);
    let seta = ui_icon("chevron", "#7A7B88", 12);
    seta.set_valign(Align::Center);
    head.append(&seta);
    let tile = caixa(Orientation::Horizontal, 0, "permissao-sensor");
    tile.set_size_request(44, 44);
    tile.set_hexpand(false);
    tile.set_valign(Align::Center);
    head.append(&tile);
    head.set_margin_bottom(4);
    body_box.append(&head);
    let title = rotulo("", "wifi-titulo");
    title.set_xalign(0.0);
    title.set_wrap(true);
    // (small natural widths: the labels wrap at the card's width, not the other way round)
    title.set_max_width_chars(24);
    body_box.append(&title);
    let body = rotulo("", "permissao-texto");
    body.set_xalign(0.0);
    body.set_wrap(true);
    body.set_max_width_chars(24);
    body_box.append(&body);
    // the choices, stacked: the safest one is a button like the others
    let choices = caixa(Orientation::Vertical, 8, "");
    choices.set_margin_top(6);
    let always = gtk::Button::with_label(tr("Allow while the app is in use"));
    always.add_css_class("wifi-botao");
    always.add_css_class("principal");
    let once = gtk::Button::with_label(tr("Just this time"));
    once.add_css_class("wifi-botao");
    let deny = gtk::Button::with_label(tr("Don't allow"));
    deny.add_css_class("permissao-negar");
    for b in [&always, &once, &deny] {
        b.set_hexpand(true);
        choices.append(b);
    }
    body_box.append(&choices);
    // where it shows and where it changes
    let foot = caixa(Orientation::Horizontal, 10, "");
    foot.set_margin_top(6);
    let escudo = ui_icon("escudo", "#8ED2C3", 16);
    escudo.set_valign(Align::Start);
    foot.append(&escudo);
    let footer = rotulo("", "permissao-rodape");
    footer.set_xalign(0.0);
    footer.set_wrap(true);
    footer.set_max_width_chars(24);
    footer.set_hexpand(true);
    foot.append(&footer);
    body_box.append(&foot);
    let card = Animavel::new(&body_box);
    card.origem(0.5, 0.5);
    card.set_halign(Align::Center);
    card.set_valign(Align::Center);
    window.set_child(Some(&card));
    let d = Rc::new(Dialog { window, card, app_icon, tile, title, body, always: always.clone(), footer, done: RefCell::default(), sensor_class: RefCell::new("") });
    for (b, c) in [(&always, Choice::Always), (&once, Choice::Once), (&deny, Choice::Deny)] {
        let w = Rc::downgrade(&d);
        b.connect_clicked(move |_| if let Some(d) = w.upgrade() { d.answer(c) });
        if debug() {
            let nome = match c { Choice::Always => "permissao sempre", Choice::Once => "permissao uma vez", Choice::Deny => "permissao negar" };
            crate::central::posicao_debug(nome, b.upcast_ref());
        }
    }
    // Esc and a click on the dimmed screen are "no"
    let keys = gtk::EventControllerKey::new();
    let w = Rc::downgrade(&d);
    keys.connect_key_pressed(move |_, k, _, _| {
        if k == gtk::gdk::Key::Escape {
            if let Some(d) = w.upgrade() {
                d.answer(Choice::Deny);
            }
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    d.window.add_controller(keys);
    let outside = gtk::GestureClick::new();
    let w = Rc::downgrade(&d);
    outside.connect_pressed(move |g, _, x, y| {
        let Some(d) = w.upgrade() else { return };
        let inside = d.card.compute_bounds(&d.window).is_some_and(|b| b.contains_point(&gtk::graphene::Point::new(x as f32, y as f32)));
        if !inside {
            g.set_state(gtk::EventSequenceState::Claimed);
            d.answer(Choice::Deny);
        }
    });
    d.window.add_controller(outside);
    DIALOG.with(|x| *x.borrow_mut() = Some(d.clone()));
    Some(d)
}

impl Dialog {
    fn answer(self: &Rc<Self>, c: Choice) {
        let Some(done) = self.done.borrow_mut().take() else { return };
        if debug() {
            eprintln!("debug permissao: respondido {}", match c { Choice::Always => "enquanto em uso", Choice::Once => "só desta vez", Choice::Deny => "não permitir" });
        }
        self.close();
        done(c);
    }

    fn close(self: &Rc<Self>) {
        self.window.set_keyboard_mode(KeyboardMode::None);
        self.window.remove_css_class("aberto");
        let w = Rc::downgrade(self);
        self.card.sair((0.0, 8.0), 0.98, move || {
            if let Some(d) = w.upgrade().filter(|d| d.done.borrow().is_none()) {
                d.window.set_visible(false);
            }
        });
    }
}

/// The app's icon, from its desktop file (a generic one when it has none).
fn app_icon(app_id: &str) -> gio::Icon {
    (!app_id.is_empty()).then(|| gio::DesktopAppInfo::new(&format!("{app_id}.desktop"))).flatten().and_then(|a| a.icon())
        .unwrap_or_else(|| gio::ThemedIcon::new("application-x-executable").upcast())
}

/// Asks; `done` gets the answer (one request at a time: an earlier one still open is answered "no").
pub fn ask(req: Request, done: impl FnOnce(Choice) + 'static) {
    let Some(d) = dialog() else {
        done(Choice::Deny);
        return;
    };
    if let Some(before) = d.done.borrow_mut().take() {
        before(Choice::Deny);
    }
    let app = if req.app.is_empty() { tr("An app").to_string() } else { req.app.clone() };
    if debug() {
        eprintln!("debug permissao: pedindo {} para «{app}»", req.sensor.map(Sensor::name).unwrap_or("acesso"));
    }
    d.app_icon.set_from_gicon(&app_icon(&req.app_id));
    while let Some(c) = d.tile.first_child() {
        d.tile.remove(&c);
    }
    let antes = d.sensor_class.replace("");
    if !antes.is_empty() {
        d.tile.remove_css_class(antes);
    }
    let (glifo, cor, classe) = req.sensor.map(Sensor::glyph).unwrap_or(("escudo", "#A5A1E2", "roxo"));
    let ic = ui_icon(glifo, cor, 20);
    ic.set_hexpand(true);
    ic.set_halign(Align::Center);
    ic.set_valign(Align::Center);
    d.tile.append(&ic);
    d.tile.add_css_class(classe);
    d.sensor_class.replace(classe);
    let (titulo, padrao, rodape) = match req.sensor {
        Some(Sensor::Microphone) => (trf("{app} wants to use the microphone", &[("app", app.clone())]),
            trf("{app} will hear the microphone while it is in use.", &[("app", app.clone())]),
            tr("A coral dot stays in the bar while the microphone is on. Change it later in Settings › Privacy.").to_string()),
        Some(Sensor::Camera) => (trf("{app} wants to use the camera", &[("app", app.clone())]),
            trf("{app} will see the camera while it is in use.", &[("app", app.clone())]),
            tr("A coral dot stays in the bar while the camera is on. Change it later in Settings › Privacy.").to_string()),
        Some(Sensor::Location) => (trf("{app} wants to know your location", &[("app", app.clone())]),
            trf("{app} will know where this computer is, roughly.", &[("app", app.clone())]),
            tr("Change it later in Settings › Privacy.").to_string()),
        Some(Sensor::Speakers) => (trf("{app} wants to use the speakers", &[("app", app.clone())]),
            trf("{app} will play sound on this computer.", &[("app", app.clone())]),
            tr("Change it later in Settings › Privacy.").to_string()),
        None => (if req.title.is_empty() { trf("{app} asks for access", &[("app", app.clone())]) } else { req.title.clone() }, String::new(),
            tr("Change it later in Settings › Privacy.").to_string()),
    };
    d.title.set_text(&titulo);
    let corpo = if req.reason.trim().is_empty() { padrao } else { req.reason.trim().to_string() };
    d.body.set_text(&corpo);
    d.body.set_visible(!corpo.is_empty());
    d.footer.set_text(&rodape);
    *d.done.borrow_mut() = Some(Box::new(done));
    d.window.present();
    crate::superficies::take_keyboard(&d.window);
    d.window.add_css_class("aberto");
    d.card.entrar((0.0, 10.0), 0.96);
    d.always.grab_focus();
}

/// Closes a request still open (the app gave up): answered "no".
pub fn withdraw() {
    if let Some(d) = DIALOG.with(|d| d.borrow().clone()) {
        d.answer(Choice::Deny);
    }
}

/// "Só desta vez": the permission xdg-desktop-portal stored for `app_id` is taken back once the app has no window
/// left (checked every 3 s, after a few seconds for the frontend to store it).
pub fn take_back_when_closed(app_id: String, sensor: Sensor) {
    if app_id.is_empty() {
        return;
    }
    let inicio = std::time::Instant::now();
    let fechada_antes = std::rc::Rc::new(std::cell::Cell::new(false));
    glib::timeout_add_local(Duration::from_secs(3), move || {
        if inicio.elapsed() < Duration::from_secs(4) {
            return glib::ControlFlow::Continue;
        }
        let aberta = crate::shell_atual().is_some_and(|s| s.estado.janelas.borrow().values().any(|j| j.app_id == app_id || j.app_id.eq_ignore_ascii_case(&app_id)));
        if aberta {
            fechada_antes.set(false);
            return glib::ControlFlow::Continue;
        }
        // two checks in a row without a window (a window can be replaced by another)
        if !fechada_antes.replace(true) {
            return glib::ControlFlow::Continue;
        }
        let (table, id) = sensor.store_entry();
        if let Ok(c) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) {
            let app = app_id.clone();
            c.call(Some("org.freedesktop.impl.portal.PermissionStore"), "/org/freedesktop/impl/portal/PermissionStore", "org.freedesktop.impl.portal.PermissionStore",
                "DeletePermission", Some(&(table, id, app_id.as_str()).to_variant()), None, gio::DBusCallFlags::NONE, 5000, gio::Cancellable::NONE, move |r| {
                    if debug() {
                        eprintln!("debug permissao: {} de «{app}» devolvida{}", sensor.name(), if r.is_ok() { "" } else { " (não estava guardada)" });
                    }
                });
        }
        glib::ControlFlow::Break
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_portal_icon_says_the_sensor() {
        assert_eq!(Sensor::from_icon("audio-input-microphone-symbolic"), Some(Sensor::Microphone));
        assert_eq!(Sensor::from_icon("camera-web-symbolic"), Some(Sensor::Camera));
        assert_eq!(Sensor::from_icon("find-location-symbolic"), Some(Sensor::Location));
        assert_eq!(Sensor::from_icon("audio-speakers-symbolic"), Some(Sensor::Speakers));
        assert_eq!(Sensor::from_icon("dialog-question"), None);
        assert_eq!(Sensor::Camera.store_entry(), ("devices", "camera"));
    }
}
