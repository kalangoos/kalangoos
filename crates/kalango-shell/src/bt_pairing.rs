//! The Bluetooth pairing dialog (designs "Parear Bluetooth" and "Parear Bluetooth · confirmar código"), shown by the
//! shell's BlueZ agent (bluez_agent.rs):
//! - a keyboard types the code: its six digits, lighting up one by one as the keyboard reports each key (an elastic
//!   spring on each box), "aguardando o teclado · 2 de 6 dígitos", and "Cancelar";
//! - a phone or computer compares the code: the six digits lit, "Cancelar" and "Parear";
//! - an old device wants a PIN typed here: the boxes are the field;
//! - a device asks to pair or to use a service: "Recusar" and "Permitir".
//! When BlueZ says the device is paired, the boxes become a teal ✓ and the dialog closes.

use crate::conversa::ui_icon;
use crate::superficies::{caixa, camada, rotulo};
use gtk::prelude::*;
use gtk::{glib, Align, Orientation};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kalango_i18n::{tr, trf};
use kalango_movimento::{Animavel, Canal, ELASTICA};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

/// What the device wants.
#[derive(Clone, Debug, PartialEq)]
pub enum Mode {
    /// the person types this code on the device (a keyboard): `entered` digits so far (None: the device does not say)
    Type { code: String, entered: Option<usize> },
    /// the same code shows on the device: the person confirms
    Compare { code: String },
    /// the person types the device's PIN here
    Enter,
    /// the device asks to pair, or to use a service
    Allow,
}

/// The person's answer: Some(PIN) for Enter, Some("") for yes, None for no.
type Done = Box<dyn FnOnce(Option<String>)>;

struct Dialog {
    window: gtk::ApplicationWindow,
    card: Animavel,
    title: gtk::Label,
    subtitle: gtk::Label,
    code: gtk::Box,
    boxes: Vec<(gtk::Box, gtk::Label, Animavel)>,
    done_mark: Animavel,
    status: gtk::Box,
    status_text: gtk::Label,
    no: gtk::Button,
    yes: gtk::Button,
    pin: gtk::Text,
    device: RefCell<String>,
    name: RefCell<String>,
    keyboard: Cell<bool>,
    mode: RefCell<Option<Mode>>,
    lit: Cell<usize>,
    done: RefCell<Option<Done>>,
    /// "Cancelar" while a code is being typed on the device: BlueZ is told to stop
    on_cancel: RefCell<Option<Box<dyn Fn(&str)>>>,
}

thread_local! {
    static DIALOG: RefCell<Option<Rc<Dialog>>> = const { RefCell::new(None) };
}

fn dialog() -> Option<Rc<Dialog>> {
    if let Some(d) = DIALOG.with(|d| d.borrow().clone()) {
        return Some(d);
    }
    let app = gtk::gio::Application::default().and_downcast::<gtk::Application>()?;
    let window = camada(&app, "kalango-bluetooth-parear", Layer::Overlay, &[Edge::Top, Edge::Bottom, Edge::Left, Edge::Right]);
    window.add_css_class("wifi-fundo");
    window.set_exclusive_zone(-1);
    let body = caixa(Orientation::Vertical, 14, "wifi-cartao");
    body.add_css_class("bt-cartao");
    body.set_size_request(440, -1);
    let tile = caixa(Orientation::Horizontal, 0, "wifi-icone");
    tile.set_size_request(52, 52);
    tile.set_hexpand(false);
    tile.set_halign(Align::Center);
    let icon = ui_icon("bluetooth", "#7CC3DF", 20);
    icon.set_hexpand(true);
    icon.set_halign(Align::Center);
    icon.set_valign(Align::Center);
    tile.append(&icon);
    body.append(&tile);
    let texts = caixa(Orientation::Vertical, 14, "");
    let title = rotulo("", "wifi-titulo");
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    title.set_max_width_chars(36);
    let subtitle = rotulo("", "wifi-subtitulo");
    subtitle.set_wrap(true);
    subtitle.set_justify(gtk::Justification::Center);
    subtitle.set_max_width_chars(48);
    texts.append(&title);
    texts.append(&subtitle);
    body.append(&texts);
    // the six boxes, in two groups of three; the PIN field hides behind them (typing fills the boxes)
    let code = caixa(Orientation::Horizontal, 8, "");
    code.set_halign(Align::Center);
    let mut boxes = Vec::new();
    for i in 0..6 {
        if i == 3 {
            let gap = gtk::Box::new(Orientation::Horizontal, 0);
            gap.set_size_request(8, -1);
            code.append(&gap);
        }
        let b = caixa(Orientation::Horizontal, 0, "bt-digito");
        b.set_size_request(44, 54);
        let l = rotulo("", "bt-digito-texto");
        l.set_hexpand(true);
        l.set_halign(Align::Center);
        l.set_valign(Align::Center);
        b.append(&l);
        let a = Animavel::new(&b);
        a.origem(0.5, 0.5);
        code.append(&a);
        boxes.push((b, l, a));
    }
    let mark_box = caixa(Orientation::Horizontal, 0, "bt-pareado");
    mark_box.set_size_request(54, 54);
    let mark = ui_icon("check", "#8ED2C3", 26);
    mark.set_hexpand(true);
    mark.set_halign(Align::Center);
    mark.set_valign(Align::Center);
    mark_box.append(&mark);
    let done_mark = Animavel::new(&mark_box);
    done_mark.origem(0.5, 0.5);
    done_mark.set_halign(Align::Center);
    done_mark.set_visible(false);
    let pin = gtk::Text::new();
    pin.set_input_purpose(gtk::InputPurpose::Pin);
    pin.set_max_length(6);
    pin.add_css_class("bt-pin");
    let stage = gtk::Overlay::new();
    stage.set_child(Some(&code));
    // (the field takes the keys; the boxes show them)
    pin.set_opacity(0.0);
    pin.set_can_target(false);
    stage.add_overlay(&pin);
    stage.add_overlay(&done_mark);
    body.append(&stage);
    let status = caixa(Orientation::Horizontal, 8, "");
    status.set_halign(Align::Center);
    let spinner = gtk::Spinner::new();
    spinner.set_spinning(true);
    spinner.add_css_class("bt-espera");
    status.append(&spinner);
    let status_text = rotulo("", "bt-espera-texto");
    status.append(&status_text);
    body.append(&status);
    let actions = caixa(Orientation::Horizontal, 8, "");
    actions.set_homogeneous(true);
    let no = gtk::Button::with_label(tr("Cancel"));
    no.add_css_class("wifi-botao");
    let yes = gtk::Button::with_label(tr("Pair"));
    yes.add_css_class("wifi-botao");
    yes.add_css_class("principal");
    actions.append(&no);
    actions.append(&yes);
    body.append(&actions);
    let card = Animavel::new(&body);
    card.origem(0.5, 0.5);
    card.set_halign(Align::Center);
    card.set_valign(Align::Center);
    window.set_child(Some(&card));
    let d = Rc::new(Dialog {
        window, card, title, subtitle, code, boxes, done_mark, status, status_text, no, yes, pin,
        device: RefCell::default(), name: RefCell::default(), keyboard: Cell::new(false), mode: RefCell::default(), lit: Cell::new(0),
        done: RefCell::default(), on_cancel: RefCell::default(),
    });
    let w = Rc::downgrade(&d);
    d.no.connect_clicked(move |_| if let Some(d) = w.upgrade() { d.answer(false) });
    let w = Rc::downgrade(&d);
    d.yes.connect_clicked(move |_| if let Some(d) = w.upgrade() { d.answer(true) });
    // the PIN typed here fills the boxes; Enter pairs
    let w = Rc::downgrade(&d);
    d.pin.connect_changed(move |t| {
        let Some(d) = w.upgrade() else { return };
        let digits: String = t.text().chars().filter(char::is_ascii_digit).collect();
        if digits != t.text().as_str() {
            t.set_text(&digits);
            return;
        }
        d.show_digits(&format!("{digits:<6}"), digits.len());
        d.yes.set_sensitive(digits.len() >= 4);
    });
    let w = Rc::downgrade(&d);
    d.pin.connect_activate(move |_| if let Some(d) = w.upgrade() { d.answer(true) });
    let keys = gtk::EventControllerKey::new();
    let w = Rc::downgrade(&d);
    keys.connect_key_pressed(move |_, k, _, _| {
        let Some(d) = w.upgrade() else { return glib::Propagation::Proceed };
        match k {
            gtk::gdk::Key::Escape => {
                d.answer(false);
                glib::Propagation::Stop
            }
            gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter if matches!(*d.mode.borrow(), Some(Mode::Compare { .. }) | Some(Mode::Allow)) => {
                d.answer(true);
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    });
    d.window.add_controller(keys);
    DIALOG.with(|x| *x.borrow_mut() = Some(d.clone()));
    Some(d)
}

impl Dialog {
    /// The boxes: `text`'s characters, the first `lit` lit (an elastic pop for each one that lights now).
    fn show_digits(&self, text: &str, lit: usize) {
        let before = self.lit.replace(lit);
        for (i, (b, l, a)) in self.boxes.iter().enumerate() {
            l.set_text(text.chars().nth(i).filter(|c| *c != ' ').map(String::from).as_deref().unwrap_or(""));
            if i < lit {
                b.add_css_class("aceso");
                if i >= before && !kalango_movimento::reduzido() {
                    a.definir(Canal::Escala, 0.86);
                    a.ir(Canal::Escala, 1.0, ELASTICA);
                }
            } else {
                b.remove_css_class("aceso");
            }
        }
    }

    fn answer(self: &Rc<Self>, yes: bool) {
        let mode = self.mode.borrow().clone();
        let device = self.device.borrow().clone();
        if debug() {
            eprintln!("debug bluetooth: {} {}", if yes { "aceito" } else { "recusado" }, self.name.borrow());
        }
        match mode {
            // a code typed on the device: there is nothing to answer, only to stop
            Some(Mode::Type { .. }) => {
                if let Some(f) = self.on_cancel.borrow().as_ref() {
                    f(&device);
                }
                self.close();
            }
            _ => {
                let pin = self.pin.text().to_string();
                if yes && matches!(mode, Some(Mode::Enter)) && pin.len() < 4 {
                    return;
                }
                let done = self.done.borrow_mut().take();
                // after "Parear" the dialog waits for BlueZ's word (the ✓, or it closes when it fails)
                if yes {
                    self.yes.set_sensitive(false);
                    self.no.set_sensitive(false);
                    self.status_text.set_text(tr("pairing…"));
                    self.status.set_visible(true);
                    // (BlueZ may never say: it closes by itself after a while)
                    let w = Rc::downgrade(self);
                    let dev = device.clone();
                    glib::timeout_add_local_once(Duration::from_secs(30), move || {
                        if let Some(d) = w.upgrade().filter(|d| *d.device.borrow() == dev && d.window.is_visible()) {
                            d.close();
                        }
                    });
                } else {
                    self.close();
                }
                if let Some(f) = done {
                    f(yes.then(|| if matches!(mode, Some(Mode::Enter)) { pin } else { String::new() }));
                }
            }
        }
    }

    fn close(self: &Rc<Self>) {
        self.window.set_keyboard_mode(KeyboardMode::None);
        self.window.remove_css_class("aberto");
        self.mode.replace(None);
        if let Some(f) = self.done.borrow_mut().take() {
            f(None);
        }
        let w = Rc::downgrade(self);
        self.card.sair((0.0, 8.0), 0.98, move || {
            if let Some(d) = w.upgrade().filter(|d| d.mode.borrow().is_none()) {
                d.window.set_visible(false);
            }
        });
    }
}

/// The device's word for the status line ("o teclado", "o aparelho").
fn device_word(keyboard: bool) -> &'static str {
    if keyboard {
        tr("waiting for the keyboard")
    } else {
        tr("waiting for the device")
    }
}

/// Shows (or updates) the dialog for a device. `done` gets the answer for Compare, Enter and Allow.
pub fn show(device: &str, name: &str, icon: &str, mode: Mode, done: Option<Box<dyn FnOnce(Option<String>)>>, on_cancel: impl Fn(&str) + 'static) {
    let Some(d) = dialog() else {
        if let Some(f) = done {
            f(None);
        }
        return;
    };
    let same = *d.device.borrow() == device && d.window.is_visible() && d.mode.borrow().is_some();
    // a code that goes on: only the count changes
    if let (true, Mode::Type { code, entered }) = (same, &mode) {
        if matches!(*d.mode.borrow(), Some(Mode::Type { .. })) {
            let n = entered.unwrap_or(0).min(6);
            d.show_digits(code, n);
            if let Some(e) = entered {
                d.status_text.set_text(&format!("{} · {}", device_word(d.keyboard.get()), trf("{n} of {total} digits", &[("n", e.to_string()), ("total", code.chars().count().to_string())])));
            }
            if debug() {
                eprintln!("debug bluetooth: {name} digitou {n} de 6");
            }
            *d.mode.borrow_mut() = Some(mode);
            return;
        }
    }
    if let Some(f) = d.done.borrow_mut().take() {
        f(None);
    }
    let keyboard = icon.starts_with("input-keyboard");
    d.keyboard.set(keyboard);
    *d.device.borrow_mut() = device.to_string();
    *d.name.borrow_mut() = name.to_string();
    *d.on_cancel.borrow_mut() = Some(Box::new(on_cancel));
    *d.done.borrow_mut() = done;
    d.title.set_text(&trf("Pair with “{name}”", &[("name", name.to_string())]));
    d.done_mark.set_visible(false);
    d.code.set_opacity(1.0);
    d.pin.set_text("");
    d.yes.set_sensitive(true);
    d.no.set_sensitive(true);
    d.no.set_visible(true);
    match &mode {
        Mode::Type { code, entered } => {
            d.subtitle.set_text(&if keyboard { tr("Type this code on the keyboard and press Enter.").to_string() } else { trf("Type this code on “{name}”.", &[("name", name.to_string())]) });
            d.show_digits(code, entered.unwrap_or(0).min(6));
            d.code.set_visible(true);
            d.status.set_visible(true);
            d.status_text.set_text(&match entered {
                Some(e) => format!("{} · {}", device_word(keyboard), trf("{n} of {total} digits", &[("n", e.to_string()), ("total", code.chars().count().to_string())])),
                None => device_word(keyboard).to_string(),
            });
            d.no.set_label(tr("Cancel"));
            d.yes.set_visible(false);
        }
        Mode::Compare { code } => {
            d.subtitle.set_text(&if icon.starts_with("phone") { tr("Check that the same code shows on the phone.").to_string() } else { trf("Check that the same code shows on “{name}”.", &[("name", name.to_string())]) });
            d.show_digits(code, 6);
            d.code.set_visible(true);
            d.status.set_visible(false);
            d.no.set_label(tr("Cancel"));
            d.yes.set_label(tr("Pair"));
            d.yes.set_visible(true);
        }
        Mode::Enter => {
            d.subtitle.set_text(&trf("Type the PIN of “{name}” (often 0000 or in its manual).", &[("name", name.to_string())]));
            d.show_digits("      ", 0);
            d.code.set_visible(true);
            d.status.set_visible(false);
            d.no.set_label(tr("Cancel"));
            d.yes.set_label(tr("Pair"));
            d.yes.set_visible(true);
            d.yes.set_sensitive(false);
        }
        Mode::Allow => {
            d.subtitle.set_text(&trf("“{name}” wants to pair with this computer.", &[("name", name.to_string())]));
            d.code.set_visible(false);
            d.status.set_visible(false);
            d.no.set_label(tr("Refuse"));
            d.yes.set_label(tr("Allow"));
            d.yes.set_visible(true);
        }
    }
    if debug() {
        eprintln!("debug bluetooth: pedido de {name} ({mode:?})");
    }
    *d.mode.borrow_mut() = Some(mode.clone());
    crate::superficies::take_keyboard(&d.window);
    d.window.add_css_class("aberto");
    d.window.present();
    d.card.entrar((0.0, 12.0), 0.96);
    if matches!(mode, Mode::Enter) {
        d.pin.grab_focus();
    } else if d.yes.is_visible() {
        d.yes.grab_focus();
    } else {
        d.no.grab_focus();
    }
    if debug() {
        for (n, w) in [("bt dialogo sim", d.yes.upcast_ref::<gtk::Widget>()), ("bt dialogo nao", d.no.upcast_ref())] {
            crate::central::posicao_debug(n, w);
        }
    }
}

/// BlueZ says the device is paired: the boxes become a teal ✓, and the dialog closes.
pub fn paired(device: &str) {
    let Some(d) = DIALOG.with(|d| d.borrow().clone()) else { return };
    if *d.device.borrow() != device || !d.window.is_visible() {
        return;
    }
    if debug() {
        eprintln!("debug bluetooth: pareado pelo diálogo {}", d.name.borrow());
    }
    // (a question still open is moot now: answered yes, never left without a reply)
    if let Some(f) = d.done.borrow_mut().take() {
        f(Some(String::new()));
    }
    d.mode.replace(Some(Mode::Allow));
    d.code.set_opacity(0.0);
    d.code.set_visible(true);
    d.status.set_visible(false);
    d.subtitle.set_text(tr("Paired."));
    d.no.set_visible(false);
    d.yes.set_visible(false);
    d.done_mark.set_visible(true);
    if !kalango_movimento::reduzido() {
        d.done_mark.definir(Canal::Escala, 0.4).definir(Canal::Opacidade, 0.0);
        d.done_mark.ir(Canal::Escala, 1.0, ELASTICA).ir(Canal::Opacidade, 1.0, ELASTICA);
    }
    let w = Rc::downgrade(&d);
    glib::timeout_add_local_once(Duration::from_millis(900), move || {
        if let Some(d) = w.upgrade() {
            d.close();
        }
    });
}

/// BlueZ gave up (the device went away, or it took too long): the dialog closes.
pub fn withdraw() {
    let Some(d) = DIALOG.with(|d| d.borrow().clone()) else { return };
    if d.window.is_visible() && d.mode.borrow().is_some() {
        if debug() {
            eprintln!("debug bluetooth: o BlueZ desistiu de {}", d.name.borrow());
        }
        d.close();
    }
}
