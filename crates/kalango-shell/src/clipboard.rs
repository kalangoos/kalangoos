//! The clipboard history (design "Área de transferência (Super+V)"): every copy in the session — text, links, colours,
//! pictures — kept with the app it came from (passwords never: `clipboard_watch` leaves them out); Super+V opens the
//! panel over the window in use: pinned items, the recent ones, ↵ pastes the chosen one into that window (the
//! clipboard takes it and the compositor's virtual keyboard presses Ctrl+V), Ctrl+P pins, Del deletes, and "Colar
//! como…" turns it into clean text, a code block, a table, a translation or a summary with the local model first.

use crate::conversa::ui_icon;
use crate::superficies::{caixa, camada, estrela, rotulo};
use gtk::prelude::*;
use gtk::{gdk, glib, Align, Orientation};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kalango_i18n::{n_, tr, trf};
use kalango_movimento as mv;
use serde::{Deserialize, Serialize};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

const MAX_RECENT: usize = 30;
const SHOWN_RECENT: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Text,
    Link,
    Color,
    Image,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub id: u64,
    pub kind: Kind,
    /// the text, or the picture's file name in the history's folder
    pub content: String,
    pub app: String,
    pub at: i64,
    #[serde(default)]
    pub pinned: bool,
    /// a picture's size
    #[serde(default)]
    pub size: (u32, u32),
}

fn folder() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).filter(|p| p.is_absolute())
        .unwrap_or_else(|| glib::home_dir().join(".local/state")).join("kalango/area-de-transferencia")
}

pub fn read() -> Vec<Item> {
    std::fs::read_to_string(folder().join("historico.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn save(items: &[Item]) {
    let d = folder();
    let _ = std::fs::create_dir_all(&d);
    let _ = std::fs::write(d.join("historico.json"), serde_json::to_string_pretty(items).unwrap_or_default());
}

/// What a copied text is: a colour (#RRGGBB), a link, or text.
pub fn kind_of(text: &str) -> Kind {
    let t = text.trim();
    let hex = t.len() == 7 && t.starts_with('#') && t[1..].chars().all(|c| c.is_ascii_hexdigit());
    if hex {
        Kind::Color
    } else if (t.starts_with("https://") || t.starts_with("http://")) && !t.contains(char::is_whitespace) {
        Kind::Link
    } else {
        Kind::Text
    }
}

/// A PNG's size from its header.
pub fn png_size(data: &[u8]) -> Option<(u32, u32)> {
    if data.len() < 24 || &data[..8] != b"\x89PNG\r\n\x1a\n" {
        return None;
    }
    Some((u32::from_be_bytes(data[16..20].try_into().ok()?), u32::from_be_bytes(data[20..24].try_into().ok()?)))
}

/// Adds a copy at the top (the same text again only moves up); the recent ones beyond the limit go (their pictures
/// too), the pinned ones stay.
pub fn add(items: &mut Vec<Item>, kind: Kind, content: String, app: &str, at: i64, size: (u32, u32)) -> u64 {
    if kind != Kind::Image {
        if let Some(i) = items.iter().position(|x| x.kind != Kind::Image && x.content == content) {
            let mut x = items.remove(i);
            x.at = at;
            if !app.is_empty() {
                x.app = app.to_string();
            }
            let id = x.id;
            items.insert(0, x);
            return id;
        }
    }
    let id = items.iter().map(|x| x.id).max().unwrap_or(0) + 1;
    items.insert(0, Item { id, kind, content, app: app.to_string(), at, pinned: false, size });
    let mut recent = 0;
    let mut gone = Vec::new();
    items.retain(|x| {
        if x.pinned {
            return true;
        }
        recent += 1;
        if recent > MAX_RECENT {
            gone.push(x.clone());
            return false;
        }
        true
    });
    for g in gone.iter().filter(|g| g.kind == Kind::Image) {
        let _ = std::fs::remove_file(folder().join(&g.content));
    }
    id
}

fn now() -> i64 {
    glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0)
}

/// The app of the window in use (where a copy came from, where a paste goes).
fn focused_app() -> (String, String) {
    let v = crate::servicos::niri_pedido(serde_json::json!("FocusedWindow")).ok().map(|v| v["FocusedWindow"].clone()).unwrap_or_default();
    let id = v["app_id"].as_str().unwrap_or("").to_string();
    let all = crate::apps::todos();
    let name = crate::apps::da_janela(&id, &all).map(|a| a.nome.clone()).unwrap_or_else(|| id.clone());
    (id, name)
}

/// A terminal pastes with Ctrl+Shift+V.
fn is_terminal(app_id: &str) -> bool {
    let a = app_id.to_lowercase();
    ["terminal", "console", "kitty", "alacritty", "foot", "ptyxis", "wezterm", "konsole", "tilix"].iter().any(|t| a.contains(t))
}

/// Puts this on the clipboard (served by a thread of its own until something else is copied).
pub fn set_clipboard(item: &Item) -> Result<(), String> {
    use wl_clipboard_rs::copy::{MimeType, Options, Source};
    let (bytes, mime) = match item.kind {
        Kind::Image => (std::fs::read(folder().join(&item.content)).map_err(|e| e.to_string())?, MimeType::Specific("image/png".into())),
        _ => (item.content.clone().into_bytes(), MimeType::Text),
    };
    Options::new().copy(Source::Bytes(bytes.into_boxed_slice()), mime).map_err(|e| e.to_string())
}

thread_local! {
    /// until when a copy is the shell putting the person's last item back (not a new copy for the history)
    static REPONDO_ATE: Cell<Option<std::time::Instant>> = const { Cell::new(None) };
}

/// The person's last item back on the clipboard (after the shell used it for a screen picture); the watcher will see
/// it come in again, and that is not a new copy.
pub fn repor_ultimo() {
    if let Some(it) = read().into_iter().next() {
        REPONDO_ATE.with(|r| r.set(Some(std::time::Instant::now() + Duration::from_secs(2))));
        let _ = set_clipboard(&it);
    }
}

/// A text on the clipboard, from scripts (`kalango-shell transferencia copiar TEXTO`); with `secret`, marked as a
/// password manager marks one (it must stay out of the history).
pub fn copy_text(text: &str, secret: bool) -> Result<(), String> {
    use wl_clipboard_rs::copy::{MimeSource, MimeType, Options, Source};
    let mut sources = vec![MimeSource { source: Source::Bytes(text.as_bytes().into()), mime_type: MimeType::Text }];
    if secret {
        sources.push(MimeSource { source: Source::Bytes(b"secret".to_vec().into_boxed_slice()), mime_type: MimeType::Specific("x-kde-passwordManagerHint".into()) });
    }
    Options::new().copy_multi(sources).map_err(|e| e.to_string())
}

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

const FORMS: &[(&str, &str)] = &[
    ("limpo", n_("Clean text")),
    ("codigo", n_("Code block")),
    ("tabela", n_("Table")),
    ("traduzir", n_("Translate")),
    ("resumir", n_("Summarize")),
];

pub struct Clipboard {
    window: gtk::ApplicationWindow,
    card: mv::Animavel,
    list: gtk::Box,
    forms: gtk::Box,
    status: gtk::Label,
    items: RefCell<Vec<Item>>,
    /// the shown items' ids, in order, and which one is chosen
    shown: RefCell<Vec<u64>>,
    chosen: Cell<usize>,
    open: Cell<bool>,
    /// the window the paste goes to: its app id
    target: RefCell<String>,
    busy: Cell<bool>,
}

fn key_cap(t: &str) -> gtk::Label {
    let l = rotulo(t, "clip-tecla");
    l.set_valign(Align::Center);
    l
}

impl Clipboard {
    pub fn new(app: &gtk::Application) -> Rc<Clipboard> {
        let window = camada(app, "kalango-area-de-transferencia", Layer::Overlay, &[Edge::Top, Edge::Left]);
        window.add_css_class("transparente");
        let panel = caixa(Orientation::Vertical, 8, "clip-painel");
        panel.set_size_request(380, -1);
        let head = caixa(Orientation::Horizontal, 8, "");
        let title = rotulo(tr("Clipboard"), "clip-titulo");
        title.set_xalign(0.0);
        title.set_hexpand(true);
        head.append(&title);
        head.append(&key_cap("Super"));
        head.append(&key_cap("V"));
        panel.append(&head);
        let list = caixa(Orientation::Vertical, 8, "");
        panel.append(&list);
        let div = caixa(Orientation::Horizontal, 0, "clip-divisor");
        div.set_size_request(-1, 1);
        panel.append(&div);
        let pc = caixa(Orientation::Vertical, 8, "");
        let pl = caixa(Orientation::Horizontal, 6, "");
        let sp = estrela(13, "#A5A1E2");
        sp.set_margin_start(1);
        pl.append(&sp);
        pl.append(&rotulo(tr("PASTE AS…"), "clip-rotulo"));
        pc.append(&pl);
        // the chips in rows of four, as the design's
        let forms_box = caixa(Orientation::Vertical, 6, "");
        pc.append(&forms_box);
        panel.append(&pc);
        let status = rotulo("", "clip-estado");
        status.set_xalign(0.0);
        status.set_visible(false);
        panel.append(&status);
        let foot = caixa(Orientation::Horizontal, 14, "");
        for (keys, what) in [(vec!["↵"], tr("paste")), (vec!["Ctrl", "P"], tr("pin")), (vec!["Del"], tr("delete"))] {
            let g = caixa(Orientation::Horizontal, 4, "");
            for k in keys {
                g.append(&key_cap(k));
            }
            g.append(&rotulo(what, "clip-dica"));
            foot.append(&g);
        }
        panel.append(&foot);
        let card = mv::Animavel::new(&panel);
        window.set_child(Some(&card));
        let c = Rc::new(Clipboard { window, card, list, forms: forms_box, status, items: RefCell::new(read()), shown: RefCell::default(), chosen: Cell::new(0),
            open: Cell::new(false), target: RefCell::default(), busy: Cell::new(false) });
        let mut row = caixa(Orientation::Horizontal, 6, "");
        for (i, (mode, label)) in FORMS.iter().enumerate() {
            if i > 0 && i % 4 == 0 {
                c.forms.append(&row);
                row = caixa(Orientation::Horizontal, 6, "");
            }
            let b = gtk::Button::with_label(tr(label));
            b.set_widget_name(mode);
            b.add_css_class("clip-forma");
            let (w, mode) = (Rc::downgrade(&c), *mode);
            b.connect_clicked(move |_| {
                if let Some(c) = w.upgrade() {
                    c.paste_as(mode);
                }
            });
            row.append(&b);
        }
        c.forms.append(&row);
        let keys = gtk::EventControllerKey::new();
        // before the buttons' own arrow-key focus moves
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let w = Rc::downgrade(&c);
        keys.connect_key_pressed(move |_, k, _, m| {
            let Some(c) = w.upgrade() else { return glib::Propagation::Proceed };
            let ctrl = m.contains(gdk::ModifierType::CONTROL_MASK);
            match k {
                gdk::Key::Escape => c.close(),
                gdk::Key::Down => c.choose(c.chosen.get() + 1),
                gdk::Key::Up => c.choose(c.chosen.get().saturating_sub(1)),
                gdk::Key::Return | gdk::Key::KP_Enter => c.paste_chosen(),
                gdk::Key::Delete => c.delete_chosen(),
                gdk::Key::p | gdk::Key::P if ctrl => c.pin_chosen(),
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        c.window.add_controller(keys);
        // (for the session test: the keys go only once the panel has the keyboard)
        if debug() {
            c.window.connect_is_active_notify(|j| {
                if j.is_active() {
                    eprintln!("debug transferencia: com o teclado");
                }
            });
        }
        c.start_watching();
        c
    }

    /// Each copy in the session comes into the history.
    fn start_watching(self: &Rc<Self>) {
        let (tx, rx) = std::sync::mpsc::channel();
        if let Err(e) = crate::clipboard_watch::watch(tx) {
            eprintln!("kalango-shell: área de transferência: {e}");
            return;
        }
        let w = Rc::downgrade(self);
        glib::timeout_add_local(Duration::from_millis(250), move || {
            let Some(c) = w.upgrade() else { return glib::ControlFlow::Break };
            for copied in rx.try_iter() {
                c.copied(copied);
            }
            glib::ControlFlow::Continue
        });
    }

    fn copied(self: &Rc<Self>, copied: crate::clipboard_watch::Copied) {
        if REPONDO_ATE.with(|r| r.take().is_some_and(|t| std::time::Instant::now() < t)) {
            if debug() {
                eprintln!("debug transferencia: o último item de volta (não é cópia nova)");
            }
            return;
        }
        let (_, app) = focused_app();
        let mut items = read();
        let (kind, label) = match copied {
            crate::clipboard_watch::Copied::Text(t) => {
                if t.trim().is_empty() {
                    return;
                }
                let k = kind_of(&t);
                if debug() {
                    eprintln!("debug transferencia: conteúdo «{}»", t.chars().take(60).collect::<String>().replace('\n', " "));
                }
                add(&mut items, k, t, &app, now(), (0, 0));
                (k, "texto")
            }
            crate::clipboard_watch::Copied::Image(data) => {
                // (the screen picture taken to describe the screen: the compositor puts it on the clipboard too)
                if crate::descrever::captura_propria() {
                    return;
                }
                let size = png_size(&data).unwrap_or((0, 0));
                let name = format!("imagem-{}-{}.png", now(), data.len());
                let _ = std::fs::create_dir_all(folder());
                if std::fs::write(folder().join(&name), &data).is_err() {
                    return;
                }
                add(&mut items, Kind::Image, name, &app, now(), size);
                (Kind::Image, "imagem")
            }
        };
        save(&items);
        if debug() {
            eprintln!("debug transferencia: copiado {label} ({kind:?}) de {app}");
        }
        let keep = self.chosen_item().map(|x| x.id);
        *self.items.borrow_mut() = items;
        if self.open.get() {
            self.fill();
            // the same item stays chosen (the list moved down under it)
            if let Some(i) = keep.and_then(|id| self.shown.borrow().iter().position(|x| *x == id)) {
                if i != self.chosen.get() {
                    self.chosen.set(i);
                    self.fill();
                }
            }
        }
    }

    pub fn toggle(self: &Rc<Self>) {
        if self.open.get() {
            self.close();
        } else {
            self.open();
        }
    }

    /// Over the window in use, a little under its top (the text being written is usually there).
    pub fn open(self: &Rc<Self>) {
        let v = crate::servicos::niri_pedido(serde_json::json!("FocusedWindow")).ok().map(|v| v["FocusedWindow"].clone()).unwrap_or_default();
        *self.target.borrow_mut() = v["app_id"].as_str().unwrap_or("").to_string();
        let screen = crate::configuracoes_tela::monitor_principal().map(|m| (m.geometry().width(), m.geometry().height())).unwrap_or((1920, 1080));
        let l = &v["layout"];
        let (x, y) = match (l["tile_pos_in_workspace_view"][0].as_f64(), l["tile_pos_in_workspace_view"][1].as_f64(), l["window_size"][0].as_f64()) {
            (Some(px), Some(py), Some(w)) => ((px + w / 2.0 - 190.0) as i32, (py + 140.0) as i32),
            _ => (screen.0 / 2 - 190, 140),
        };
        // (the top bar's zone already pushes the layer down)
        self.window.set_margin(Edge::Left, x.clamp(12, (screen.0 - 392).max(12)));
        self.window.set_margin(Edge::Top, (y - 32).clamp(8, (screen.1 - 560).max(8)));
        *self.items.borrow_mut() = read();
        self.chosen.set(0);
        self.status.set_visible(false);
        self.fill();
        self.open.set(true);
        self.window.present();
        let w = self.window.downgrade();
        glib::timeout_add_local_once(Duration::from_millis(40), move || {
            if let Some(j) = w.upgrade() {
                crate::superficies::take_keyboard(&j);
            }
        });
        self.card.entrar((0.0, 0.0), 0.96);
        if debug() {
            let mut row = self.forms.first_child();
            while let Some(r) = row {
                let mut b = r.first_child();
                while let Some(x) = b {
                    self.log_pos(&format!("transferencia forma {}", x.widget_name()), &x);
                    b = x.next_sibling();
                }
                row = r.next_sibling();
            }
            eprintln!("debug transferencia: aberta ({} itens) sobre {}", self.shown.borrow().len(), self.target.borrow());
        }
    }

    pub fn close(&self) {
        if !self.open.replace(false) {
            return;
        }
        self.window.set_keyboard_mode(KeyboardMode::None);
        let w = self.window.downgrade();
        self.card.sair((0.0, 0.0), 0.98, move || {
            if let Some(j) = w.upgrade() {
                j.set_visible(false);
            }
        });
    }

    fn fill(self: &Rc<Self>) {
        while let Some(x) = self.list.first_child() {
            self.list.remove(&x);
        }
        let items = self.items.borrow().clone();
        let pinned: Vec<&Item> = items.iter().filter(|x| x.pinned).collect();
        let recent: Vec<&Item> = items.iter().filter(|x| !x.pinned).take(SHOWN_RECENT).collect();
        let mut shown = Vec::new();
        let section = |t: &str| {
            let l = rotulo(t, "clip-rotulo");
            l.set_xalign(0.0);
            l
        };
        if !pinned.is_empty() {
            self.list.append(&section(tr("PINNED CLIPS")));
            for it in &pinned {
                self.list.append(&self.row(it, shown.len()));
                shown.push(it.id);
            }
        }
        self.list.append(&section(tr("RECENT")));
        if recent.is_empty() {
            let e = rotulo(tr("What you copy shows up here."), "clip-vazio");
            e.set_xalign(0.0);
            self.list.append(&e);
        }
        for it in &recent {
            self.list.append(&self.row(it, shown.len()));
            shown.push(it.id);
        }
        // "Colar como…" only for text
        let chosen = shown.get(self.chosen.get().min(shown.len().saturating_sub(1))).and_then(|id| items.iter().find(|x| x.id == *id));
        self.forms.set_visible(chosen.is_some_and(|x| x.kind != Kind::Image));
        *self.shown.borrow_mut() = shown;
    }

    fn when(at: i64) -> String {
        let s = (now() - at).max(0);
        if s < 60 { tr("now").to_string() } else { crate::configuracoes_rede::ha(s) }
    }

    fn row(self: &Rc<Self>, it: &Item, index: usize) -> gtk::Widget {
        let b = gtk::Button::new();
        b.add_css_class("clip-item");
        if index == self.chosen.get() {
            b.add_css_class("escolhido");
        }
        let from = if it.app.is_empty() { Self::when(it.at) } else { format!("{} · {}", it.app, Self::when(it.at)) };
        match it.kind {
            Kind::Color => {
                b.add_css_class("cor");
                let r = caixa(Orientation::Horizontal, 10, "");
                let sw = gtk::DrawingArea::new();
                sw.set_content_width(20);
                sw.set_content_height(20);
                sw.set_valign(Align::Center);
                let hex = it.content.trim().to_string();
                sw.set_draw_func(move |_, cr, w, h| {
                    let (r, g, b) = crate::tema::rgb(&hex);
                    cr.set_source_rgb(r, g, b);
                    let (w, h) = (f64::from(w), f64::from(h));
                    cr.new_sub_path();
                    cr.arc(w - 4.0, 4.0, 4.0, -std::f64::consts::FRAC_PI_2, 0.0);
                    cr.arc(w - 4.0, h - 4.0, 4.0, 0.0, std::f64::consts::FRAC_PI_2);
                    cr.arc(4.0, h - 4.0, 4.0, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
                    cr.arc(4.0, 4.0, 4.0, std::f64::consts::PI, 1.5 * std::f64::consts::PI);
                    cr.close_path();
                    let _ = cr.fill();
                });
                r.append(&sw);
                let t = rotulo(it.content.trim(), "clip-cor");
                t.set_xalign(0.0);
                t.set_hexpand(true);
                r.append(&t);
                if it.pinned {
                    r.append(&ui_icon("estrela-n", "#E5C98C", 16));
                }
                b.set_child(Some(&r));
            }
            Kind::Image => {
                b.add_css_class("imagem");
                let r = caixa(Orientation::Horizontal, 10, "");
                let pic = gtk::Picture::new();
                if let Ok(t) = gdk::Texture::from_filename(folder().join(&it.content)) {
                    pic.set_paintable(Some(&t));
                }
                pic.set_content_fit(gtk::ContentFit::Cover);
                pic.set_size_request(96, 54);
                pic.add_css_class("clip-miniatura");
                r.append(&pic);
                let t = caixa(Orientation::Vertical, 2, "");
                t.set_valign(Align::Center);
                let name = trf("Picture · {w} × {h}", &[("w", it.size.0.to_string()), ("h", it.size.1.to_string())]);
                let n = rotulo(&name, "clip-nome");
                n.set_xalign(0.0);
                t.append(&n);
                let d = rotulo(&from, "clip-de");
                d.set_xalign(0.0);
                t.append(&d);
                r.append(&t);
                b.set_child(Some(&r));
            }
            Kind::Text | Kind::Link => {
                let r = caixa(Orientation::Vertical, 4, "");
                let first = it.content.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
                let t = rotulo(first, if it.kind == Kind::Link { "clip-link" } else { "clip-texto" });
                t.set_xalign(0.0);
                t.set_ellipsize(gtk::pango::EllipsizeMode::End);
                t.set_max_width_chars(1);
                t.set_hexpand(true);
                r.append(&t);
                let d = rotulo(&from, "clip-de");
                d.set_xalign(0.0);
                r.append(&d);
                b.set_child(Some(&r));
            }
        }
        let w = Rc::downgrade(self);
        b.connect_clicked(move |_| {
            if let Some(c) = w.upgrade() {
                c.chosen.set(index);
                c.paste_chosen();
            }
        });
        if debug() {
            self.log_pos(&format!("transferencia item {index}"), b.upcast_ref());
        }
        b.upcast()
    }

    /// For the session test: where a control is on the screen (the panel sits by its margins, under the top bar).
    fn log_pos(&self, name: &str, w: &gtk::Widget) {
        let (name, w, win) = (name.to_string(), w.downgrade(), self.window.downgrade());
        glib::timeout_add_local_once(Duration::from_millis(400), move || {
            let (Some(w), Some(win)) = (w.upgrade(), win.upgrade()) else { return };
            if let Some(b) = w.compute_bounds(&win) {
                let x = win.margin(Edge::Left) as f32 + b.x() + b.width() / 2.0;
                let y = crate::superficies::BARRA_ALTURA as f32 + win.margin(Edge::Top) as f32 + b.y() + b.height() / 2.0;
                eprintln!("debug central: {name} em {x:.0} {y:.0}");
            }
        });
    }

    fn choose(self: &Rc<Self>, i: usize) {
        let n = self.shown.borrow().len();
        if n == 0 {
            return;
        }
        self.chosen.set(i.min(n - 1));
        self.fill();
        if debug() {
            eprintln!("debug transferencia: escolhido {}", self.chosen.get());
        }
    }

    fn chosen_item(&self) -> Option<Item> {
        let id = *self.shown.borrow().get(self.chosen.get())?;
        self.items.borrow().iter().find(|x| x.id == id).cloned()
    }

    fn pin_chosen(self: &Rc<Self>) {
        let Some(it) = self.chosen_item() else { return };
        let mut items = read();
        if let Some(x) = items.iter_mut().find(|x| x.id == it.id) {
            x.pinned = !x.pinned;
        }
        save(&items);
        if debug() {
            eprintln!("debug transferencia: {} {:?}", if it.pinned { "solto" } else { "fixado" }, it.kind);
        }
        *self.items.borrow_mut() = items;
        self.chosen.set(0);
        self.fill();
    }

    fn delete_chosen(self: &Rc<Self>) {
        let Some(it) = self.chosen_item() else { return };
        let mut items = read();
        items.retain(|x| x.id != it.id);
        if it.kind == Kind::Image {
            let _ = std::fs::remove_file(folder().join(&it.content));
        }
        save(&items);
        if debug() {
            eprintln!("debug transferencia: apagado {:?}", it.kind);
        }
        *self.items.borrow_mut() = items;
        self.choose(self.chosen.get());
        self.fill();
    }

    /// The chosen item goes to the clipboard and into the window the panel opened over.
    fn paste_chosen(self: &Rc<Self>) {
        if let Some(it) = self.chosen_item() {
            self.paste(it);
        }
    }

    fn paste(self: &Rc<Self>, it: Item) {
        if let Err(e) = set_clipboard(&it) {
            self.show_status(&e);
            return;
        }
        let shift = is_terminal(&self.target.borrow());
        self.close();
        if debug() {
            eprintln!("debug transferencia: colado {:?}", it.kind);
        }
        // once the window has the keyboard back
        glib::timeout_add_local_once(Duration::from_millis(180), move || {
            std::thread::spawn(move || {
                if let Err(e) = crate::paste_keys::paste(shift) {
                    eprintln!("kalango-shell: colar: {e}");
                }
            });
        });
    }

    fn show_status(&self, t: &str) {
        self.status.set_text(t);
        self.status.set_visible(true);
    }

    /// "Colar como…": the local model turns the chosen text into another form, then it is pasted.
    fn paste_as(self: &Rc<Self>, mode: &'static str) {
        let Some(it) = self.chosen_item().filter(|x| x.kind != Kind::Image) else { return };
        if self.busy.replace(true) {
            return;
        }
        self.show_status(tr("The local model is preparing it…"));
        if debug() {
            eprintln!("debug transferencia: colar como {mode}");
        }
        let w = Rc::downgrade(self);
        let text = it.content.clone();
        crate::lateral::em_segundo_plano(move || {
            let bin = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("kalango-agente"))).filter(|p| p.is_file())
                .unwrap_or_else(|| "kalango-agente".into());
            let mut c = std::process::Command::new(bin).args(["colar-como", mode]).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null()).spawn().map_err(|e| e.to_string())?;
            if let Some(mut i) = c.stdin.take() {
                std::io::Write::write_all(&mut i, text.as_bytes()).map_err(|e| e.to_string())?;
            }
            let o = c.wait_with_output().map_err(|e| e.to_string())?;
            let out = String::from_utf8_lossy(&o.stdout).to_string();
            if o.status.success() && !out.trim().is_empty() { Ok(out) } else { Err(tr("The local model could not do it now.").to_string()) }
        }, move |r: Result<String, String>| {
            let Some(c) = w.upgrade() else { return };
            c.busy.set(false);
            match r {
                Ok(text) => {
                    if debug() {
                        eprintln!("debug transferencia: colar como {mode} pronto ({} caracteres)", text.chars().count());
                    }
                    let it = Item { id: 0, kind: kind_of(&text), content: text, app: String::new(), at: now(), pinned: false, size: (0, 0) };
                    c.paste(it);
                }
                Err(e) => c.show_status(&e),
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_copy_is_text_a_link_or_a_colour() {
        assert_eq!(kind_of("#A5A1E2"), Kind::Color);
        assert_eq!(kind_of(" https://docs.rs/gdkx11/0.18.2 "), Kind::Link);
        assert_eq!(kind_of("veja https://x.y"), Kind::Text);
        assert_eq!(kind_of("#A5A1E"), Kind::Text);
    }

    #[test]
    fn the_same_text_moves_up_and_old_ones_go_but_pinned_stay() {
        let mut v = Vec::new();
        let a = add(&mut v, Kind::Text, "um".into(), "Terminal", 1, (0, 0));
        add(&mut v, Kind::Text, "dois".into(), "Chrome", 2, (0, 0));
        assert_eq!(add(&mut v, Kind::Text, "um".into(), "", 3, (0, 0)), a);
        assert_eq!(v.iter().map(|x| x.content.as_str()).collect::<Vec<_>>(), vec!["um", "dois"]);
        assert_eq!(v[0].app, "Terminal");
        v[1].pinned = true;
        for i in 0..40 {
            add(&mut v, Kind::Text, format!("t{i}"), "", 10 + i, (0, 0));
        }
        assert_eq!(v.iter().filter(|x| !x.pinned).count(), MAX_RECENT);
        assert!(v.iter().any(|x| x.pinned && x.content == "dois"));
    }

    #[test]
    fn a_pictures_size_from_its_png_header() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&1280u32.to_be_bytes());
        png.extend_from_slice(&720u32.to_be_bytes());
        assert_eq!(png_size(&png), Some((1280, 720)));
        assert_eq!(png_size(b"nope"), None);
    }
}
