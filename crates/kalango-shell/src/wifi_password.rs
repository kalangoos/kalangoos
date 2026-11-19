//! The Wi-Fi password dialog (design "Senha do Wi-Fi"): a protected network asks for its password in a card over the
//! dimmed screen — the network's name and security, the password (the eye shows it), "Conectar automaticamente" and
//! "Permitir que os agentes usem esta rede". Enter connects, Esc cancels; "Conectar" waits for a password the network
//! can take (WPA: 8 to 63 characters, or 64 hex digits; WEP: 5 or 13, or 10 or 26 hex digits). A wrong password
//! brings the card back with the field in coral, shaken once.
//!
//! Asked by Settings › Network and by NetworkManager itself (nm_agent.rs: the shell is the session's secret agent), so
//! a saved network whose password changed asks here too. Enterprise networks (802.1X) also ask for the user name.

use crate::conversa::ui_icon;
use crate::superficies::{caixa, camada, rotulo};
use gtk::prelude::*;
use gtk::{glib, Align, Orientation};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kalango_i18n::{tr, trf};
use kalango_movimento::{Animavel, Canal, ELASTICA};
use std::cell::RefCell;
use std::rc::Rc;

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

/// What a network asks for.
pub struct Request {
    pub ssid: String,
    /// "WPA3", "WPA2", "WEP"… as the person knows it
    pub security: String,
    /// 802.1X: a user name too
    pub enterprise: bool,
    /// the last password was refused
    pub wrong: bool,
}

/// The person's answer.
pub struct Answer {
    pub identity: Option<String>,
    pub password: String,
    pub autoconnect: bool,
    pub agents: bool,
}

type Done = Box<dyn FnOnce(Option<Answer>)>;

struct Dialog {
    window: gtk::ApplicationWindow,
    card: Animavel,
    field_motion: Animavel,
    field: gtk::Box,
    identity: gtk::Text,
    identity_row: gtk::Box,
    password: gtk::Text,
    eye: gtk::Button,
    error: gtk::Label,
    title: gtk::Label,
    subtitle: gtk::Label,
    auto: gtk::CheckButton,
    agents: gtk::CheckButton,
    connect: gtk::Button,
    request: RefCell<Option<Request>>,
    done: RefCell<Option<Done>>,
}

thread_local! {
    static DIALOG: RefCell<Option<Rc<Dialog>>> = const { RefCell::new(None) };
}

/// A password the network's security can take.
pub fn acceptable(security: &str, password: &str) -> bool {
    let hex = |n: usize| password.len() == n && password.chars().all(|c| c.is_ascii_hexdigit());
    if security.to_uppercase().contains("WEP") {
        return matches!(password.chars().count(), 5 | 13) || hex(10) || hex(26);
    }
    (8..=63).contains(&password.chars().count()) || hex(64)
}

fn dialog() -> Option<Rc<Dialog>> {
    if let Some(d) = DIALOG.with(|d| d.borrow().clone()) {
        return Some(d);
    }
    let app = gtk::gio::Application::default().and_downcast::<gtk::Application>()?;
    let window = camada(&app, "kalango-wifi-senha", Layer::Overlay, &[Edge::Top, Edge::Bottom, Edge::Left, Edge::Right]);
    window.add_css_class("wifi-fundo");
    crate::superficies::take_keyboard(&window);
    // over everything, the top bar and the dock too (the design dims the whole screen)
    window.set_exclusive_zone(-1);
    let body = caixa(Orientation::Vertical, 14, "wifi-cartao");
    body.set_size_request(440, -1);
    // header: the network's icon, its name and security
    let head = caixa(Orientation::Horizontal, 14, "");
    let tile = caixa(Orientation::Horizontal, 0, "wifi-icone");
    tile.set_size_request(44, 44);
    // (its own size: the centred icon's expansion must not widen it)
    tile.set_hexpand(false);
    tile.set_valign(Align::Center);
    let icon = ui_icon("wifi", "#7CC3DF", 18);
    icon.set_hexpand(true);
    icon.set_halign(Align::Center);
    icon.set_valign(Align::Center);
    tile.append(&icon);
    head.append(&tile);
    let texts = caixa(Orientation::Vertical, 4, "");
    texts.set_valign(Align::Center);
    let title = rotulo("", "wifi-titulo");
    title.set_xalign(0.0);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    title.set_max_width_chars(34);
    let subtitle = rotulo("", "wifi-subtitulo");
    subtitle.set_xalign(0.0);
    texts.append(&title);
    texts.append(&subtitle);
    head.append(&texts);
    body.append(&head);
    // enterprise networks: the user name
    let identity_row = caixa(Orientation::Horizontal, 8, "wifi-campo");
    identity_row.append(&ui_icon("usuario", "#8E8F99", 16));
    let identity = gtk::Text::new();
    identity.set_hexpand(true);
    identity.set_placeholder_text(Some(tr("User name")));
    identity_row.append(&identity);
    identity_row.set_visible(false);
    body.append(&identity_row);
    // the password, with the lock and the eye
    let field = caixa(Orientation::Horizontal, 8, "wifi-campo");
    field.append(&ui_icon("cadeado", "#8E8F99", 16));
    let password = gtk::Text::new();
    password.set_hexpand(true);
    password.set_visibility(false);
    // the design's dots are the small bullet
    password.set_invisible_char('•');
    password.set_input_purpose(gtk::InputPurpose::Password);
    password.set_placeholder_text(Some(tr("Network password")));
    field.append(&password);
    let eye = gtk::Button::new();
    eye.add_css_class("wifi-olho");
    eye.set_child(Some(&ui_icon("olho", "#8E8F99", 16)));
    eye.set_tooltip_text(Some(tr("Show password")));
    eye.set_focus_on_click(false);
    field.append(&eye);
    let field_motion = Animavel::new(&field);
    body.append(&field_motion);
    let error = rotulo("", "wifi-erro");
    error.set_xalign(0.0);
    error.set_visible(false);
    body.append(&error);
    // the options
    let options = caixa(Orientation::Vertical, 4, "");
    let auto = gtk::CheckButton::with_label(tr("Connect automatically"));
    auto.add_css_class("wifi-check");
    auto.set_active(true);
    let agents = gtk::CheckButton::with_label(tr("Allow agents to use this network"));
    agents.add_css_class("wifi-check");
    options.append(&auto);
    options.append(&agents);
    body.append(&options);
    // the actions, at the right
    let actions = caixa(Orientation::Horizontal, 8, "");
    actions.set_halign(Align::End);
    let cancel = gtk::Button::with_label(tr("Cancel"));
    cancel.add_css_class("wifi-botao");
    let connect = gtk::Button::with_label(tr("Connect"));
    connect.add_css_class("wifi-botao");
    connect.add_css_class("principal");
    connect.set_sensitive(false);
    actions.append(&cancel);
    actions.append(&connect);
    body.append(&actions);
    let card = Animavel::new(&body);
    card.origem(0.5, 0.5);
    card.set_halign(Align::Center);
    card.set_valign(Align::Center);
    window.set_child(Some(&card));
    let d = Rc::new(Dialog { window, card, field_motion, field, identity, identity_row, password, eye, error, title, subtitle, auto, agents, connect, request: RefCell::default(), done: RefCell::default() });
    // the eye shows and hides the password
    let w = Rc::downgrade(&d);
    d.eye.connect_clicked(move |b| {
        let Some(d) = w.upgrade() else { return };
        let show = !d.password.property::<bool>("visibility");
        d.password.set_visibility(show);
        b.set_child(Some(&ui_icon(if show { "olho-d" } else { "olho" }, "#8E8F99", 16)));
        b.set_tooltip_text(Some(if show { tr("Hide password") } else { tr("Show password") }));
    });
    // "Conectar" waits for a password the network can take
    let w = Rc::downgrade(&d);
    let check = move || {
        let Some(d) = w.upgrade() else { return };
        let ok = d.request.borrow().as_ref().is_some_and(|r| {
            if r.enterprise {
                !d.identity.text().trim().is_empty() && !d.password.text().is_empty()
            } else {
                acceptable(&r.security, &d.password.text())
            }
        });
        d.connect.set_sensitive(ok);
        // typing again clears the coral
        d.field.remove_css_class("erro");
    };
    let c2 = check.clone();
    d.password.connect_changed(move |_| c2());
    d.identity.connect_changed(move |_| check());
    let w = Rc::downgrade(&d);
    d.password.connect_activate(move |_| if let Some(d) = w.upgrade() { d.answer(true) });
    let w = Rc::downgrade(&d);
    d.identity.connect_activate(move |_| if let Some(d) = w.upgrade() { d.password.grab_focus(); });
    let w = Rc::downgrade(&d);
    d.connect.connect_clicked(move |_| if let Some(d) = w.upgrade() { d.answer(true) });
    let w = Rc::downgrade(&d);
    cancel.connect_clicked(move |_| if let Some(d) = w.upgrade() { d.answer(false) });
    // Esc cancels; a click on the dimmed screen too
    let keys = gtk::EventControllerKey::new();
    let w = Rc::downgrade(&d);
    keys.connect_key_pressed(move |_, k, _, _| {
        if k == gtk::gdk::Key::Escape {
            if let Some(d) = w.upgrade() {
                d.answer(false);
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
            d.answer(false);
        }
    });
    d.window.add_controller(outside);
    DIALOG.with(|x| *x.borrow_mut() = Some(d.clone()));
    Some(d)
}

impl Dialog {
    fn answer(self: &Rc<Self>, connect: bool) {
        if connect && !self.connect.is_sensitive() {
            if !self.password.text().is_empty() {
                self.shake();
            }
            return;
        }
        let Some(done) = self.done.borrow_mut().take() else { return };
        let ssid = self.request.borrow().as_ref().map(|r| r.ssid.clone()).unwrap_or_default();
        let answer = connect.then(|| Answer {
            identity: self.identity_row.is_visible().then(|| self.identity.text().trim().to_string()),
            password: self.password.text().to_string(),
            autoconnect: self.auto.is_active(),
            agents: self.agents.is_active(),
        });
        if debug() {
            match &answer {
                Some(a) => eprintln!("debug wifi: respondido {ssid} (automática {}, agentes {})", a.autoconnect, a.agents),
                None => eprintln!("debug wifi: cancelado {ssid}"),
            }
        }
        self.close();
        done(answer);
    }

    fn close(self: &Rc<Self>) {
        self.window.set_keyboard_mode(KeyboardMode::None);
        self.password.set_text("");
        self.identity.set_text("");
        self.window.remove_css_class("aberto");
        let w = Rc::downgrade(self);
        self.card.sair((0.0, 8.0), 0.98, move || {
            if let Some(d) = w.upgrade().filter(|d| d.done.borrow().is_none()) {
                d.window.set_visible(false);
            }
        });
    }

    /// Once, side to side, with the elastic spring (no animations: the coral alone says it).
    fn shake(&self) {
        if kalango_movimento::reduzido() {
            return;
        }
        self.field_motion.definir(Canal::X, -10.0);
        self.field_motion.ir(Canal::X, 0.0, ELASTICA);
    }
}

/// Asks for a network's password; `done` gets the answer, or None when the person cancels (or another network asks
/// first: one dialog at a time).
pub fn ask(req: Request, done: impl FnOnce(Option<Answer>) + 'static) {
    let Some(d) = dialog() else {
        done(None);
        return;
    };
    if let Some(before) = d.done.borrow_mut().take() {
        before(None);
    }
    if debug() {
        eprintln!("debug wifi: pedindo {} ({}{})", req.ssid, req.security, if req.wrong { ", senha recusada" } else { "" });
    }
    d.title.set_text(&trf("Connect to “{name}”", &[("name", req.ssid.clone())]));
    d.subtitle.set_text(&if req.security.is_empty() { tr("Protected network").to_string() } else { trf("Protected network ({security})", &[("security", req.security.clone())]) });
    d.identity_row.set_visible(req.enterprise);
    d.password.set_visibility(false);
    d.eye.set_child(Some(&ui_icon("olho", "#8E8F99", 16)));
    d.error.set_text(if req.wrong { tr("Wrong password, or the network refused it.") } else { "" });
    d.error.set_visible(req.wrong);
    if req.wrong {
        d.field.add_css_class("erro");
    } else {
        d.field.remove_css_class("erro");
    }
    d.connect.set_sensitive(false);
    let wrong = req.wrong;
    *d.request.borrow_mut() = Some(req);
    *d.done.borrow_mut() = Some(Box::new(done));
    crate::superficies::take_keyboard(&d.window);
    d.window.add_css_class("aberto");
    d.window.present();
    d.card.entrar((0.0, 12.0), 0.96);
    if wrong {
        d.shake();
    }
    if d.identity_row.is_visible() {
        d.identity.grab_focus();
    } else {
        d.password.grab_focus();
    }
    // (for the session test: where its controls are, on the screen)
    if debug() {
        for (n, w) in [("wifi senha", d.password.upcast_ref::<gtk::Widget>()), ("wifi conectar", d.connect.upcast_ref()), ("wifi automatica", d.auto.upcast_ref()), ("wifi agentes", d.agents.upcast_ref())] {
            crate::central::posicao_debug(n, w);
        }
    }
}

/// Closes the dialog without an answer (NetworkManager stopped asking).
pub fn withdraw(ssid: &str) {
    let Some(d) = DIALOG.with(|d| d.borrow().clone()) else { return };
    if d.request.borrow().as_ref().is_some_and(|r| r.ssid == ssid) && d.done.borrow().is_some() {
        d.answer(false);
    }
}

#[cfg(test)]
mod tests {
    use super::acceptable;

    #[test]
    fn passwords_the_network_can_take() {
        assert!(!acceptable("WPA2", "curta"));
        assert!(acceptable("WPA2", "senha-certa"));
        assert!(acceptable("WPA3", &"a".repeat(63)));
        assert!(!acceptable("WPA3", &"z".repeat(64)));
        assert!(acceptable("WPA2", &"ab".repeat(32)));
        assert!(acceptable("WEP", "12345"));
        assert!(acceptable("WEP", "0123456789"));
        assert!(!acceptable("WEP", "123456"));
    }
}
