//! KalangoOS's own on-screen keyboard (design "Teclado virtual", 63180; Settings › Accessibility › On-screen
//! keyboard, or `kalango-shell teclado`): it rises from the bottom when a text field gets the focus (the compositor
//! says so through input-method-v2) and the windows make room for it; the keys go to the focused app through the
//! compositor's virtual keyboard (a keymap that grows with each new character, so any letter or symbol types). A key
//! pressed sinks and shows its letter enlarged above it; held, it offers its accents; the bar above completes words
//! (from a list and from what the person typed) and takes dictation (the local audio model).

use crate::conversa::ui_icon;
use crate::superficies::{caixa, rotulo};
use gtk::prelude::*;
use gtk::{gdk, glib, Orientation};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kalango_i18n::tr;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io::Write;
use std::os::fd::AsFd;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols_misc::zwp_input_method_v2::client::{zwp_input_method_manager_v2 as im_manager, zwp_input_method_v2 as im};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{zwp_virtual_keyboard_manager_v1 as vk_manager, zwp_virtual_keyboard_v1 as vk};

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

// ----------------------------------------------------------------------------------------------- the keymap

/// The special keys: (name in the keymap, its keysym).
const SPECIAL: &[(&str, &str)] = &[("BKSP", "BackSpace"), ("RTRN", "Return"), ("TAB", "Tab"), ("LEFT", "Left"), ("RGHT", "Right"),
    ("SPCE", "space"), ("ESC", "Escape")];
const CTRL_CODE: u32 = 200;

/// The characters the keymap has, each with its own keycode (no levels: an "A" is not Shift+"a"), grown as needed.
#[derive(Default)]
pub struct Keymap {
    chars: Vec<char>,
}

fn keysym(c: char) -> String {
    match c {
        'a'..='z' | 'A'..='Z' | '0'..='9' => c.to_string(),
        _ => format!("U{:04X}", c as u32),
    }
}

impl Keymap {
    /// The keycode (evdev, the keymap's less 8) of a character, and whether the keymap grew for it.
    pub fn code(&mut self, c: char) -> (u32, bool) {
        if let Some(i) = self.chars.iter().position(|x| *x == c) {
            return (SPECIAL.len() as u32 + 1 + i as u32, false);
        }
        self.chars.push(c);
        (SPECIAL.len() as u32 + self.chars.len() as u32, true)
    }

    pub fn special(name: &str) -> u32 {
        SPECIAL.iter().position(|(n, _)| *n == name).map(|i| i as u32 + 1).unwrap_or(1)
    }

    pub fn text(&self) -> String {
        let mut codes = String::new();
        let mut syms = String::new();
        for (i, (n, s)) in SPECIAL.iter().enumerate() {
            codes.push_str(&format!("<{n}> = {}; ", i + 9));
            syms.push_str(&format!("key <{n}> {{ [ {s} ] }};\n"));
        }
        for (i, c) in self.chars.iter().enumerate() {
            let code = SPECIAL.len() + i + 9;
            codes.push_str(&format!("<C{code}> = {code}; "));
            syms.push_str(&format!("key <C{code}> {{ [ {} ] }};\n", keysym(*c)));
        }
        format!("xkb_keymap {{\nxkb_keycodes \"kalango\" {{ minimum = 8; maximum = 255; {codes}<LCTL> = {}; }};\n\
            xkb_types \"kalango\" {{ include \"complete\" }};\nxkb_compat \"kalango\" {{ include \"complete\" }};\n\
            xkb_symbols \"kalango\" {{\n{syms}key <LCTL> {{ [ Control_L ] }};\nmodifier_map Control {{ <LCTL> }};\n}};\n}};\n", CTRL_CODE + 8)
    }
}

// ----------------------------------------------------------------------------------------------- the wire

struct Wire {
    conn: Connection,
    vk: vk::ZwpVirtualKeyboardV1,
    keymap: Keymap,
    t0: std::time::Instant,
}

static WIRE: OnceLock<Mutex<Option<Wire>>> = OnceLock::new();
static SERIAL: AtomicU32 = AtomicU32::new(0);

struct State {
    seat: Option<wl_seat::WlSeat>,
    imm: Option<im_manager::ZwpInputMethodManagerV2>,
    vkm: Option<vk_manager::ZwpVirtualKeyboardManagerV1>,
    pending: Option<bool>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(s: &mut Self, r: &wl_registry::WlRegistry, ev: wl_registry::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>) {
        if let wl_registry::Event::Global { name, interface, version } = ev {
            match interface.as_str() {
                "wl_seat" if s.seat.is_none() => s.seat = Some(r.bind(name, version.min(7), qh, ())),
                "zwp_input_method_manager_v2" => s.imm = Some(r.bind(name, 1, qh, ())),
                "zwp_virtual_keyboard_manager_v1" => s.vkm = Some(r.bind(name, 1, qh, ())),
                _ => {}
            }
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(_: &mut Self, _: &wl_seat::WlSeat, _: wl_seat::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<im_manager::ZwpInputMethodManagerV2, ()> for State {
    fn event(_: &mut Self, _: &im_manager::ZwpInputMethodManagerV2, _: im_manager::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<vk_manager::ZwpVirtualKeyboardManagerV1, ()> for State {
    fn event(_: &mut Self, _: &vk_manager::ZwpVirtualKeyboardManagerV1, _: vk_manager::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<vk::ZwpVirtualKeyboardV1, ()> for State {
    fn event(_: &mut Self, _: &vk::ZwpVirtualKeyboardV1, _: vk::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

/// A text field got (or lost) the focus: the keyboard rises (or goes), on the main thread.
impl Dispatch<im::ZwpInputMethodV2, ()> for State {
    fn event(s: &mut Self, _: &im::ZwpInputMethodV2, ev: im::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match ev {
            im::Event::Activate => s.pending = Some(true),
            im::Event::Deactivate => s.pending = Some(false),
            im::Event::Done => {
                SERIAL.fetch_add(1, Ordering::Relaxed);
                if let Some(on) = s.pending.take() {
                    glib::MainContext::default().invoke(move || field_focus(on));
                }
            }
            im::Event::Unavailable => glib::MainContext::default().invoke(|| {
                if debug() {
                    eprintln!("debug teclado: outro método de entrada já está no ar");
                }
            }),
            _ => {}
        }
    }
}

fn upload(w: &mut Wire) -> Result<(), String> {
    let text = w.keymap.text();
    let path = glib::user_runtime_dir().join(format!("kalango-teclado-{}.xkb", std::process::id()));
    let mut f = std::fs::File::create(&path).map_err(|e| e.to_string())?;
    f.write_all(text.as_bytes()).and_then(|_| f.write_all(&[0])).map_err(|e| e.to_string())?;
    let f = std::fs::File::open(&path).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&path);
    w.vk.keymap(1, f.as_fd(), (text.len() + 1) as u32);
    Ok(())
}

/// The connection of its own (a thread reads the input method's news; the keys go from the main thread).
fn start_wire() {
    if WIRE.get().is_some() {
        return;
    }
    let _ = WIRE.set(Mutex::new(None));
    std::thread::spawn(|| {
        let r = (|| -> Result<(), String> {
            let conn = Connection::connect_to_env().map_err(|e| e.to_string())?;
            let mut queue = conn.new_event_queue();
            let qh = queue.handle();
            let _reg = conn.display().get_registry(&qh, ());
            let mut s = State { seat: None, imm: None, vkm: None, pending: None };
            queue.roundtrip(&mut s).map_err(|e| e.to_string())?;
            let (Some(seat), Some(vkm)) = (s.seat.clone(), s.vkm.clone()) else { return Err("the compositor has no virtual keyboard".into()) };
            let vkb = vkm.create_virtual_keyboard(&seat, &qh, ());
            let _im = s.imm.clone().map(|m| m.get_input_method(&seat, &qh, ()));
            let mut w = Wire { conn: conn.clone(), vk: vkb, keymap: Keymap::default(), t0: std::time::Instant::now() };
            upload(&mut w)?;
            let _ = conn.flush();
            if let Some(m) = WIRE.get() {
                *m.lock().map_err(|e| e.to_string())? = Some(w);
            }
            loop {
                queue.blocking_dispatch(&mut s).map_err(|e| e.to_string())?;
            }
        })();
        if let Err(e) = r {
            eprintln!("kalango-shell: teclado: {e}");
        }
    });
}

fn with_wire(f: impl FnOnce(&mut Wire)) {
    if let Some(m) = WIRE.get() {
        if let Ok(mut g) = m.lock() {
            if let Some(w) = g.as_mut() {
                f(w);
                let _ = w.conn.flush();
            }
        }
    }
}

fn tap(w: &mut Wire, code: u32) {
    let t = w.t0.elapsed().as_millis() as u32;
    w.vk.key(t, code, 1);
    w.vk.key(t + 1, code, 0);
}

/// Types text into the focused app.
pub fn type_text(text: &str) {
    with_wire(|w| {
        for c in text.chars() {
            if c == ' ' {
                tap(w, Keymap::special("SPCE"));
                continue;
            }
            if c == '\n' {
                tap(w, Keymap::special("RTRN"));
                continue;
            }
            let (code, grew) = w.keymap.code(c);
            if grew {
                let _ = upload(w);
            }
            tap(w, code);
        }
    });
}

/// A special key ("BKSP", "RTRN", "TAB", "LEFT", "RGHT"), with Ctrl when asked.
pub fn press(name: &str, ctrl: bool) {
    with_wire(|w| {
        if ctrl {
            w.vk.modifiers(4, 0, 0, 0);
        }
        tap(w, Keymap::special(name));
        if ctrl {
            w.vk.modifiers(0, 0, 0, 0);
        }
    });
}

/// Ctrl + a letter (Ctrl+C, Ctrl+V…).
pub fn ctrl_char(c: char) {
    with_wire(|w| {
        let (code, grew) = w.keymap.code(c);
        if grew {
            let _ = upload(w);
        }
        w.vk.modifiers(4, 0, 0, 0);
        tap(w, code);
        w.vk.modifiers(0, 0, 0, 0);
    });
}

// ----------------------------------------------------------------------------------------------- the words

const COMMON: &str = "a ao aos as até bem boa bom cada casa coisa com como da das de dele dela depois desde dia do dos e ela ele eles em \
    então entre era essa esse esta este estou eu fazer foi gente hoje isso isto já lá mais mas me mesmo meu minha muito na nas não nem \
    no nos nossa nosso o obrigado obrigada onde os ou para pela pelo pode por porque quando que quem se sem ser seu sim só sua também \
    tem tempo tenho ter todo toda tudo um uma você vocês vou amanhã agora ainda antes aqui assim bom-dia boa-noite boa-tarde certo \
    claro olá oi tchau reunião trabalho projeto arquivo arquivos pasta documento mensagem enviar receber computador sistema agente \
    agentes tela teclado senha internet rede música vídeo imagem foto fotos calendário evento lembrete amanhã semana mês ano hora horas";

fn words_file() -> std::path::PathBuf {
    glib::user_data_dir().join("kalango/teclado-palavras.json")
}

/// The words that start with what is typed: the person's own first (most used), then the common ones.
pub fn suggest(prefix: &str, learned: &HashMap<String, u32>) -> Vec<String> {
    if prefix.is_empty() {
        return Vec::new();
    }
    let p = prefix.to_lowercase();
    let mut own: Vec<(&String, &u32)> = learned.iter().filter(|(w, _)| w.starts_with(&p) && **w != p).collect();
    own.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    let mut out: Vec<String> = own.into_iter().map(|(w, _)| w.clone()).collect();
    for w in COMMON.split_whitespace() {
        if w.starts_with(&p) && w != p && !out.iter().any(|x| x == w) {
            out.push(w.to_string());
        }
    }
    out.truncate(3);
    // (the case typed is kept: "Lun" → "Lunar-shell")
    if prefix.chars().next().is_some_and(char::is_uppercase) {
        for w in &mut out {
            let mut c = w.chars();
            *w = c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default();
        }
    }
    out
}

// ----------------------------------------------------------------------------------------------- the keyboard

const ROWS_PT: [&str; 3] = ["qwertyuiop", "asdfghjklç", "zxcvbnm,."];
const ROWS_EN: [&str; 3] = ["qwertyuiop", "asdfghjkl'", "zxcvbnm,."];
const ROWS_NUM: [&str; 3] = ["1234567890", "@#$%&*-+()", "/;:!?\"_="];

fn accents(c: char) -> &'static [char] {
    match c {
        'a' => &['á', 'à', 'â', 'ã', 'ä'],
        'e' => &['é', 'ê', 'è', 'ë'],
        'i' => &['í', 'ì', 'î', 'ï'],
        'o' => &['ó', 'ô', 'õ', 'ò', 'ö'],
        'u' => &['ú', 'ü', 'ù', 'û'],
        'c' => &['ç'],
        'n' => &['ñ'],
        _ => &[],
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Letters,
    Numbers,
}

pub struct Osk {
    window: gtk::Window,
    rows: gtk::Box,
    sugg: gtk::Box,
    page: Cell<Page>,
    shift: Cell<u8>,
    english: Cell<bool>,
    ctrl: Cell<bool>,
    word: RefCell<String>,
    learned: RefCell<HashMap<String, u32>>,
    shown_by_person: Cell<bool>,
    put_away: Cell<bool>,
    recording: RefCell<Option<crate::voz::Recording>>,
    /// a long press showed the accents: the release that follows types nothing
    held: Cell<bool>,
    bubble: gtk::Popover,
    bubble_label: gtk::Label,
}

thread_local! {
    static OSK: RefCell<Option<Rc<Osk>>> = const { RefCell::new(None) };
    static ENABLED: Cell<bool> = const { Cell::new(false) };
}

/// The setting (Accessibility › On-screen keyboard): on, the keyboard listens for text fields.
pub fn enable(on: bool) {
    ENABLED.with(|e| e.set(on));
    if on {
        start_wire();
        osk();
    } else if let Some(k) = OSK.with(|o| o.borrow().clone()) {
        k.hide();
    }
    if debug() {
        eprintln!("debug teclado: ligado {on}");
    }
}

pub fn enabled() -> bool {
    ENABLED.with(Cell::get)
}

/// `kalango-shell teclado`: shows it (or puts it away) now, whatever the field.
pub fn toggle() {
    start_wire();
    let k = osk();
    if k.window.is_visible() {
        k.put_away.set(true);
        k.hide();
    } else {
        k.shown_by_person.set(true);
        k.put_away.set(false);
        k.show();
    }
}

fn field_focus(on: bool) {
    if debug() {
        eprintln!("debug teclado: campo de texto {}", if on { "em foco" } else { "sem foco" });
    }
    if !ENABLED.with(Cell::get) {
        return;
    }
    let k = osk();
    if on {
        k.put_away.set(false);
        k.word.borrow_mut().clear();
        k.refresh_suggestions();
        k.show();
    } else if !k.shown_by_person.get() {
        k.hide();
    }
}

fn osk() -> Rc<Osk> {
    if let Some(k) = OSK.with(|o| o.borrow().clone()) {
        return k;
    }
    let k = Osk::new();
    OSK.with(|o| *o.borrow_mut() = Some(k.clone()));
    k
}

impl Osk {
    fn new() -> Rc<Osk> {
        let app = gtk::gio::Application::default().and_downcast::<gtk::Application>().expect("app");
        let window = gtk::Window::new();
        window.set_application(Some(&app));
        window.init_layer_shell();
        window.set_layer(Layer::Top);
        window.set_namespace(Some("kalango-teclado"));
        window.set_anchor(Edge::Bottom, true);
        window.set_margin(Edge::Bottom, 86);
        // (it never takes the keyboard: the app keeps its focus and gets the keys)
        window.set_keyboard_mode(KeyboardMode::None);
        window.add_css_class("osk-janela");
        let col = caixa(Orientation::Vertical, 8, "osk");
        col.set_size_request(1180, -1);
        let sugg = caixa(Orientation::Horizontal, 8, "");
        col.append(&sugg);
        let rows = caixa(Orientation::Vertical, 8, "");
        col.append(&rows);
        window.set_child(Some(&col));
        let bubble = gtk::Popover::new();
        bubble.set_autohide(false);
        bubble.set_has_arrow(false);
        bubble.add_css_class("osk-bolha");
        let bubble_label = rotulo("", "osk-bolha-letra");
        bubble.set_child(Some(&bubble_label));
        bubble.set_parent(&col);
        let learned: HashMap<String, u32> = std::fs::read_to_string(words_file()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        let k = Rc::new(Osk {
            window, rows, sugg, page: Cell::new(Page::Letters), shift: Cell::new(0), english: Cell::new(false), ctrl: Cell::new(false),
            word: RefCell::default(), learned: RefCell::new(learned), shown_by_person: Cell::new(false), put_away: Cell::new(false),
            recording: RefCell::default(), held: Cell::new(false), bubble, bubble_label,
        });
        k.build_rows();
        k.refresh_suggestions();
        k
    }

    fn show(self: &Rc<Self>) {
        if self.window.is_visible() {
            return;
        }
        self.window.present();
        // (the windows make room: its height is kept free at the bottom)
        let w = self.window.clone();
        glib::timeout_add_local_once(Duration::from_millis(200), move || {
            w.set_exclusive_zone(w.height() + 86);
            // (for the session test: where it is — a layer at the bottom, centred, 86 px above the edge and above the
            // room a bottom dock keeps)
            if debug() {
                let (sw, sh) = crate::superficies::tamanho_da_saida().unwrap_or((1280, 800));
                let y = sh - 86 - w.height() - crate::superficies::zona_da_dock_embaixo();
                eprintln!("debug teclado: à vista ({}×{}) em {} {y}", w.width(), w.height(), (sw - w.width()) / 2);
            }
        });
    }

    fn hide(&self) {
        self.window.set_exclusive_zone(0);
        self.window.set_visible(false);
        self.shown_by_person.set(false);
        if debug() {
            eprintln!("debug teclado: guardado");
        }
    }

    fn key(self: &Rc<Self>, label: &str, class: &str, id: &str, f: impl Fn(&Rc<Osk>, &gtk::Button) + 'static) -> gtk::Button {
        let b = gtk::Button::new();
        b.add_css_class("osk-tecla");
        if !class.is_empty() {
            b.add_css_class(class);
        }
        b.set_child(Some(&rotulo(label, "")));
        b.set_hexpand(true);
        b.set_focusable(false);
        b.set_focus_on_click(false);
        crate::central::posicao_debug_sempre(&format!("teclado {id}"), b.upcast_ref());
        let w = Rc::downgrade(self);
        b.connect_clicked(move |b| {
            if let Some(k) = w.upgrade() {
                f(&k, b);
            }
        });
        b
    }

    fn glyph_key(self: &Rc<Self>, glyph: &str, class: &str, id: &str, f: impl Fn(&Rc<Osk>, &gtk::Button) + 'static) -> gtk::Button {
        let b = self.key("", class, id, f);
        b.set_child(Some(&ui_icon(glyph, if class == "osk-enter" { "#1C1D24" } else { "#C9CAD1" }, 16)));
        b
    }

    fn build_rows(self: &Rc<Self>) {
        while let Some(c) = self.rows.first_child() {
            self.rows.remove(&c);
        }
        let upper = self.shift.get() > 0;
        let letters = if self.english.get() { ROWS_EN } else { ROWS_PT };
        let rows = if self.page.get() == Page::Numbers { ROWS_NUM } else { letters };
        for (r, keys) in rows.iter().enumerate() {
            let line = caixa(Orientation::Horizontal, 8, "");
            if r == 1 {
                line.append(&self.glyph_key("seta-direita", "osk-tab", "tab", |_, _| press("TAB", false)));
            }
            if r == 2 {
                let sh = self.glyph_key("shift", if upper { "osk-mod-ativo" } else { "osk-mod" }, "shift", |k, _| k.shift_key());
                line.append(&sh);
            }
            for c in keys.chars() {
                let shown: String = if upper { c.to_uppercase().collect() } else { c.to_string() };
                let b = self.key(&shown, "", &format!("{}", c), move |k, b| k.letter(c, b));
                // held: its accents
                if !accents(c).is_empty() {
                    let lp = gtk::GestureLongPress::new();
                    lp.set_propagation_phase(gtk::PropagationPhase::Capture);
                    let (w, bb) = (Rc::downgrade(self), b.clone());
                    lp.connect_pressed(move |_, _, _| {
                        if let Some(k) = w.upgrade() {
                            k.held.set(true);
                            k.accents_for(c, &bb);
                        }
                    });
                    b.add_controller(lp);
                }
                line.append(&b);
            }
            match r {
                0 => line.append(&self.glyph_key("apagar-tecla", "osk-mod", "apagar", |k, _| k.backspace())),
                1 => {
                    let e = self.glyph_key("enter", "osk-enter", "enter", |k, _| {
                        k.end_word();
                        press("RTRN", false);
                    });
                    e.set_size_request(110, -1);
                    e.set_hexpand(false);
                    line.append(&e);
                }
                _ => {
                    let sh = self.glyph_key("shift", if upper { "osk-mod-ativo" } else { "osk-mod" }, "shift2", |k, _| k.shift_key());
                    line.append(&sh);
                }
            }
            self.rows.append(&line);
        }
        // the bottom row
        let line = caixa(Orientation::Horizontal, 8, "");
        let num = self.key(if self.page.get() == Page::Numbers { "abc" } else { "123" }, "osk-mod", "123", |k, _| {
            k.page.set(if k.page.get() == Page::Letters { Page::Numbers } else { Page::Letters });
            k.build_rows();
        });
        num.set_hexpand(false);
        num.set_size_request(96, -1);
        line.append(&num);
        let lang = self.key(if self.english.get() { "en" } else { "pt" }, "osk-mod", "idioma", |k, _| {
            k.english.set(!k.english.get());
            k.build_rows();
        });
        lang.set_hexpand(false);
        lang.set_size_request(70, -1);
        line.append(&lang);
        let ctrl = self.key("Ctrl", if self.ctrl.get() { "osk-mod-ativo" } else { "osk-mod" }, "ctrl", |k, _| {
            k.ctrl.set(!k.ctrl.get());
            k.build_rows();
        });
        ctrl.set_hexpand(false);
        ctrl.set_size_request(70, -1);
        line.append(&ctrl);
        let space = self.key(tr("space"), "osk-espaco", "espaco", |k, _| {
            k.end_word();
            type_text(" ");
        });
        space.set_size_request(660, -1);
        line.append(&space);
        for (g, id, key) in [("seta-esquerda", "esquerda", "LEFT"), ("seta-direita", "direita", "RGHT")] {
            let b = self.glyph_key(g, "osk-mod", id, move |_, _| press(key, false));
            b.set_hexpand(false);
            b.set_size_request(60, -1);
            line.append(&b);
        }
        let emoji = self.key("emoji", "osk-mod", "emoji", |_, _| {
            if let Some(app) = gtk::gio::Application::default().and_downcast::<gtk::Application>() {
                crate::emoji::alternar(&app);
            }
        });
        emoji.set_hexpand(false);
        emoji.set_size_request(96, -1);
        line.append(&emoji);
        self.rows.append(&line);
    }

    /// A letter: typed (Ctrl+letter when Ctrl is on), shown enlarged above the key for a moment.
    fn letter(self: &Rc<Self>, c: char, b: &gtk::Button) {
        // (the release after a long press: the accents are showing, nothing is typed)
        if self.held.replace(false) {
            return;
        }
        let upper = self.shift.get() > 0;
        let ch: char = if upper { c.to_uppercase().next().unwrap_or(c) } else { c };
        if self.ctrl.get() {
            ctrl_char(c);
            self.ctrl.set(false);
            self.build_rows();
            if debug() {
                eprintln!("debug teclado: Ctrl+{c}");
            }
            return;
        }
        type_text(&ch.to_string());
        if ch.is_alphanumeric() || ch == '-' {
            self.word.borrow_mut().push(ch);
        } else {
            self.end_word();
        }
        self.pop_letter(&ch.to_string(), b);
        // one-shot Shift goes back to small letters; a double tap keeps capitals
        if self.shift.get() == 1 {
            self.shift.set(0);
            self.build_rows();
        }
        self.refresh_suggestions();
        if debug() {
            eprintln!("debug teclado: tecla «{ch}»");
        }
    }

    fn pop_letter(&self, s: &str, b: &gtk::Button) {
        self.bubble_label.set_text(s);
        if let Some(r) = b.compute_bounds(&self.bubble.parent().unwrap_or_else(|| b.clone().upcast())) {
            self.bubble.set_pointing_to(Some(&gdk::Rectangle::new(r.x() as i32, r.y() as i32, r.width() as i32, r.height() as i32)));
            self.bubble.set_position(gtk::PositionType::Top);
            self.bubble.popup();
            let bb = self.bubble.clone();
            glib::timeout_add_local_once(Duration::from_millis(160), move || bb.popdown());
        }
    }

    fn accents_for(self: &Rc<Self>, c: char, b: &gtk::Button) {
        let pop = gtk::Popover::new();
        pop.add_css_class("menu");
        // (it stays until one is chosen: the long press's own release, outside it, must not close it)
        pop.set_autohide(false);
        let row = caixa(Orientation::Horizontal, 4, "");
        for a in accents(c) {
            let upper = self.shift.get() > 0;
            let ch: char = if upper { a.to_uppercase().next().unwrap_or(*a) } else { *a };
            let k = gtk::Button::with_label(&ch.to_string());
            k.add_css_class("osk-tecla");
            k.set_focusable(false);
            k.set_size_request(48, 48);
            crate::central::posicao_debug_sempre(&format!("teclado acento {ch}"), k.upcast_ref());
            let (w, p) = (Rc::downgrade(self), pop.clone());
            k.connect_clicked(move |_| {
                p.popdown();
                if let Some(o) = w.upgrade() {
                    type_text(&ch.to_string());
                    o.word.borrow_mut().push(ch);
                    o.refresh_suggestions();
                    if debug() {
                        eprintln!("debug teclado: tecla «{ch}»");
                    }
                }
            });
            row.append(&k);
        }
        pop.set_child(Some(&row));
        pop.set_parent(b);
        pop.set_position(gtk::PositionType::Top);
        pop.connect_closed(|p| p.unparent());
        pop.popup();
        // (left alone: it goes after 4 s)
        let p = pop.downgrade();
        glib::timeout_add_local_once(Duration::from_secs(4), move || {
            if let Some(p) = p.upgrade() {
                p.popdown();
            }
        });
    }

    fn shift_key(self: &Rc<Self>) {
        // off → once → kept (capitals) → off
        self.shift.set((self.shift.get() + 1) % 3);
        self.build_rows();
    }

    fn backspace(self: &Rc<Self>) {
        press("BKSP", false);
        self.word.borrow_mut().pop();
        self.refresh_suggestions();
    }

    /// The word ends (a space, Enter, punctuation): it is learned.
    fn end_word(self: &Rc<Self>) {
        let w = std::mem::take(&mut *self.word.borrow_mut()).to_lowercase();
        if w.chars().count() >= 3 {
            *self.learned.borrow_mut().entry(w).or_insert(0) += 1;
            if let Some(d) = words_file().parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let _ = std::fs::write(words_file(), serde_json::to_string(&*self.learned.borrow()).unwrap_or_default());
        }
        self.refresh_suggestions();
    }

    fn refresh_suggestions(self: &Rc<Self>) {
        while let Some(c) = self.sugg.first_child() {
            self.sugg.remove(&c);
        }
        let word = self.word.borrow().clone();
        let list = suggest(&word, &self.learned.borrow());
        for i in 0..3 {
            let s = list.get(i).cloned().unwrap_or_default();
            let b = gtk::Button::with_label(&s);
            b.add_css_class(if i == 0 && !s.is_empty() { "osk-sugestao-1" } else { "osk-sugestao" });
            b.set_hexpand(true);
            b.set_focusable(false);
            b.set_sensitive(!s.is_empty());
            crate::central::posicao_debug_sempre(&format!("teclado sugestao {}", i + 1), b.upcast_ref());
            let (w, typed) = (Rc::downgrade(self), word.clone());
            b.connect_clicked(move |_| {
                if let Some(k) = w.upgrade() {
                    // the rest of the word, and a space
                    let rest: String = s.chars().skip(typed.chars().count()).collect();
                    type_text(&format!("{rest} "));
                    *k.word.borrow_mut() = s.clone();
                    k.end_word();
                    if debug() {
                        eprintln!("debug teclado: sugestão «{s}»");
                    }
                }
            });
            self.sugg.append(&b);
        }
        let mic = gtk::Button::new();
        mic.add_css_class(if self.recording.borrow().is_some() { "osk-mic-ativo" } else { "osk-mod" });
        mic.set_child(Some(&ui_icon("microfone", "#C9CAD1", 14)));
        mic.set_focusable(false);
        mic.set_size_request(40, -1);
        crate::central::posicao_debug_sempre("teclado microfone", mic.upcast_ref());
        let w = Rc::downgrade(self);
        mic.connect_clicked(move |_| {
            if let Some(k) = w.upgrade() {
                k.dictate();
            }
        });
        self.sugg.append(&mic);
        let down = gtk::Button::new();
        down.add_css_class("osk-mod");
        down.set_child(Some(&ui_icon("chevron-baixo", "#C9CAD1", 14)));
        down.set_focusable(false);
        down.set_size_request(40, -1);
        crate::central::posicao_debug_sempre("teclado guardar", down.upcast_ref());
        let w = Rc::downgrade(self);
        down.connect_clicked(move |_| {
            if let Some(k) = w.upgrade() {
                k.put_away.set(true);
                k.hide();
            }
        });
        self.sugg.append(&down);
    }

    /// Dictation: a click listens, another writes what was said (the local audio model).
    fn dictate(self: &Rc<Self>) {
        let rec = self.recording.borrow_mut().take();
        match rec {
            None => {
                if let Ok(r) = crate::voz::start("teclado") {
                    *self.recording.borrow_mut() = Some(r);
                }
            }
            Some(r) => {
                let w = Rc::downgrade(self);
                crate::lateral::em_segundo_plano(move || crate::voz::finish(r), move |t: Result<String, String>| {
                    let Some(k) = w.upgrade() else { return };
                    if let Ok(t) = t {
                        let t = t.trim();
                        if !t.is_empty() {
                            type_text(&format!("{t} "));
                            if debug() {
                                eprintln!("debug teclado: ditado «{t}»");
                            }
                        }
                    }
                    k.refresh_suggestions();
                });
            }
        }
        self.refresh_suggestions();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keymap_grows_and_words() {
        let mut k = Keymap::default();
        let (a, grew) = k.code('a');
        assert!(grew);
        assert_eq!(k.code('a'), (a, false));
        let (c, _) = k.code('ç');
        assert_ne!(a, c);
        let t = k.text();
        assert!(t.contains("[ a ]") && t.contains("[ U00E7 ]") && t.contains("Control_L") && t.contains("BackSpace"));
        let mut learned = HashMap::new();
        learned.insert("lunar-shell".to_string(), 3);
        assert_eq!(suggest("lun", &learned).first().map(String::as_str), Some("lunar-shell"));
        assert!(suggest("reu", &HashMap::new()).contains(&"reunião".to_string()));
        assert_eq!(suggest("Lun", &learned).first().map(String::as_str), Some("Lunar-shell"));
    }
}
