//! A drive connected (design "Dispositivo conectado (pendrive)"). The shell mounts removable drives as they arrive
//! (as GNOME Shell's automount does; an encrypted one asks for its passphrase) and shows a card under the status area
//! for 12 s: its name and free space, the photos on it that Pictures does not have yet, and
//! - "Abrir no Arquivos" (↵);
//! - "Importar N fotos": into Imagens/<year>, skipping the ones already there (same name and size), with the island's
//!   copy progress;
//! - "Pedir ao agente para organizar": the same import, then the agent (the local model) sorts them into folders by
//!   date.
//! "Sempre fazer isto" remembers the choice for this drive (its UUID) and does it next time without the card.
//! "Ejetar" unmounts and powers the drive off, and says it can be removed only once that is safe; a drive in use
//! says so.
//!
//! The session test has no USB: KALANGO_MIDIA_TESTE names a folder whose subfolders are "drives" connected.

use crate::conversa::ui_icon;
use crate::superficies::{caixa, camada, rotulo};
use gtk::prelude::*;
use gtk::{gio, glib, Align, Orientation};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kalango_i18n::{tr, trf};
use kalango_movimento::Animavel;
use serde_json::json;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

const PHOTO_EXT: &[&str] = &["jpg", "jpeg", "png", "heic", "heif", "webp", "gif", "tif", "tiff", "dng", "cr2", "cr3", "nef", "arw", "orf", "rw2", "raf"];

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

/// A drive as the card knows it.
#[derive(Clone)]
struct Drive {
    name: String,
    root: PathBuf,
    /// what "Sempre fazer isto" remembers it by
    id: String,
    mount: Option<gio::Mount>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Action {
    Open,
    Import,
    Organize,
}

impl Action {
    fn key(self) -> &'static str {
        match self {
            Action::Open => "abrir",
            Action::Import => "importar",
            Action::Organize => "organizar",
        }
    }

    fn from_key(k: &str) -> Option<Action> {
        match k {
            "abrir" => Some(Action::Open),
            "importar" => Some(Action::Import),
            "organizar" => Some(Action::Organize),
            _ => None,
        }
    }
}

struct Card {
    window: gtk::ApplicationWindow,
    motion: Animavel,
    title: gtk::Label,
    subtitle: gtk::Label,
    bar: gtk::DrawingArea,
    used: Rc<Cell<f64>>,
    rows: gtk::Box,
    always: gtk::CheckButton,
    eject: gtk::Button,
    drive: RefCell<Option<Drive>>,
    photos: RefCell<Vec<PathBuf>>,
    selected: Cell<usize>,
    buttons: RefCell<Vec<(Action, gtk::Button)>>,
    hovered: Cell<bool>,
    closing: RefCell<Option<glib::SourceId>>,
}

thread_local! {
    static CARD: RefCell<Option<Rc<Card>>> = const { RefCell::new(None) };
    static MONITOR: RefCell<Option<gio::VolumeMonitor>> = const { RefCell::new(None) };
}

/// Where the photos go: Pictures/<this year>. (The session test's Pictures: KALANGO_MIDIA_DESTINO — its HOME is the
/// real one, so without it a test drive imports nothing.)
fn destination() -> Option<PathBuf> {
    let pics = match std::env::var_os("KALANGO_MIDIA_DESTINO") {
        Some(d) => PathBuf::from(d),
        None if std::env::var_os("KALANGO_MIDIA_TESTE").is_some() => return None,
        None => glib::user_special_dir(glib::UserDirectory::Pictures).unwrap_or_else(|| glib::home_dir().join("Imagens")),
    };
    Some(pics.join(glib::DateTime::now_local().map(|d| d.year()).unwrap_or(2026).to_string()))
}

/// The photos on the drive that the destination does not have (same name and size).
fn new_photos(root: &Path, dest: &Path) -> Vec<PathBuf> {
    let have: HashSet<(String, u64)> = std::fs::read_dir(dest).into_iter().flatten().flatten()
        .filter_map(|e| Some((e.file_name().to_string_lossy().into_owned(), e.metadata().ok()?.len()))).collect();
    let mut out = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0)];
    let mut seen = 0usize;
    while let Some((dir, depth)) = stack.pop() {
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            seen += 1;
            if seen > 50_000 {
                return out;
            }
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let Ok(m) = e.metadata() else { continue };
            if m.is_dir() {
                if depth < 6 {
                    stack.push((p, depth + 1));
                }
            } else if p.extension().and_then(|x| x.to_str()).is_some_and(|x| PHOTO_EXT.contains(&x.to_lowercase().as_str())) && !have.contains(&(name, m.len())) {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

fn human(b: u64) -> String {
    crate::configuracoes_sistema::human(b)
}

fn remembered(id: &str) -> Option<Action> {
    crate::configuracoes::ajustes()["midia_sempre"][id].as_str().and_then(Action::from_key)
}

fn remember(id: &str, a: Action) {
    let mut m = crate::configuracoes::ajustes()["midia_sempre"].clone();
    if !m.is_object() {
        m = json!({});
    }
    m[id] = json!(a.key());
    crate::configuracoes::salvar_ajuste("midia_sempre", m);
}

fn card() -> Option<Rc<Card>> {
    if let Some(c) = CARD.with(|c| c.borrow().clone()) {
        return Some(c);
    }
    let app = gio::Application::default().and_downcast::<gtk::Application>()?;
    let window = camada(&app, "kalango-dispositivo", Layer::Top, &[Edge::Top, Edge::Right]);
    window.add_css_class("transparente");
    window.set_resizable(false);
    // under the status area: 12 px below the bar, 16 from the edge
    window.set_margin(Edge::Top, 12);
    window.set_margin(Edge::Right, 16);
    window.set_keyboard_mode(KeyboardMode::OnDemand);
    let body = caixa(Orientation::Vertical, 10, "midia-cartao");
    body.set_size_request(430, -1);
    let head = caixa(Orientation::Horizontal, 12, "");
    let tile = caixa(Orientation::Horizontal, 0, "midia-icone");
    tile.set_size_request(36, 36);
    tile.set_hexpand(false);
    tile.set_valign(Align::Center);
    let ic = ui_icon("baixar", "#8ED2C3", 16);
    ic.set_hexpand(true);
    ic.set_halign(Align::Center);
    ic.set_valign(Align::Center);
    tile.append(&ic);
    head.append(&tile);
    let texts = caixa(Orientation::Vertical, 2, "");
    texts.set_hexpand(true);
    texts.set_valign(Align::Center);
    let title = rotulo("", "midia-titulo");
    title.set_xalign(0.0);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let subtitle = rotulo("", "midia-sub");
    subtitle.set_xalign(0.0);
    subtitle.set_wrap(true);
    texts.append(&title);
    texts.append(&subtitle);
    head.append(&texts);
    let close = gtk::Button::new();
    close.add_css_class("midia-fechar");
    close.set_child(Some(&ui_icon("fechar", "#8E8F99", 14)));
    close.set_valign(Align::Center);
    close.set_tooltip_text(Some(tr("Close")));
    head.append(&close);
    body.append(&head);
    let used: Rc<Cell<f64>> = Rc::default();
    let bar = gtk::DrawingArea::new();
    bar.set_content_height(4);
    let u2 = used.clone();
    bar.set_draw_func(move |_, cr, w, h| {
        let (w, h) = (w as f64, h as f64);
        let pill = |cr: &gtk::cairo::Context, len: f64| {
            let r = h / 2.0;
            cr.new_sub_path();
            cr.arc(len - r, r, r, -std::f64::consts::FRAC_PI_2, std::f64::consts::FRAC_PI_2);
            cr.arc(r, r, r, std::f64::consts::FRAC_PI_2, 3.0 * std::f64::consts::FRAC_PI_2);
            cr.close_path();
        };
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.1);
        pill(cr, w);
        let _ = cr.fill();
        cr.set_source_rgb(0x8E as f64 / 255.0, 0xD2 as f64 / 255.0, 0xC3 as f64 / 255.0);
        pill(cr, (w * u2.get().clamp(0.0, 1.0)).max(h));
        let _ = cr.fill();
    });
    body.append(&bar);
    let rows = caixa(Orientation::Vertical, 2, "");
    body.append(&rows);
    let div = caixa(Orientation::Horizontal, 0, "midia-div");
    div.set_size_request(-1, 1);
    body.append(&div);
    let foot = caixa(Orientation::Horizontal, 8, "");
    let always = gtk::CheckButton::with_label(tr("Always do this"));
    always.add_css_class("wifi-check");
    always.set_hexpand(true);
    always.set_halign(Align::Start);
    foot.append(&always);
    let eject = gtk::Button::new();
    eject.add_css_class("midia-ejetar");
    let eb = caixa(Orientation::Horizontal, 6, "");
    eb.append(&ui_icon("ejetar", "#E2E2E6", 14));
    eb.append(&rotulo(tr("Eject"), ""));
    eject.set_child(Some(&eb));
    foot.append(&eject);
    body.append(&foot);
    let motion = Animavel::new(&body);
    motion.origem(1.0, 0.0);
    window.set_child(Some(&motion));
    let c = Rc::new(Card { window, motion, title, subtitle, bar, used, rows, always, eject, drive: RefCell::default(), photos: RefCell::default(),
        selected: Cell::new(0), buttons: RefCell::default(), hovered: Cell::new(false), closing: RefCell::default() });
    let w = Rc::downgrade(&c);
    close.connect_clicked(move |_| if let Some(c) = w.upgrade() { c.hide() });
    let w = Rc::downgrade(&c);
    c.eject.connect_clicked(move |_| if let Some(c) = w.upgrade() { c.do_eject() });
    // while the pointer is on it, it stays
    let hover = gtk::EventControllerMotion::new();
    let w = Rc::downgrade(&c);
    hover.connect_enter(move |_, _, _| if let Some(c) = w.upgrade() { c.hovered.set(true) });
    let w = Rc::downgrade(&c);
    hover.connect_leave(move |_| {
        if let Some(c) = w.upgrade() {
            c.hovered.set(false);
            c.close_later(4);
        }
    });
    c.window.add_controller(hover);
    // the keyboard: ↑ ↓ choose, ↵ does it, Esc closes
    let keys = gtk::EventControllerKey::new();
    let w = Rc::downgrade(&c);
    keys.connect_key_pressed(move |_, k, _, _| {
        let Some(c) = w.upgrade() else { return glib::Propagation::Proceed };
        let n = c.buttons.borrow().len();
        match k {
            gtk::gdk::Key::Escape => c.hide(),
            gtk::gdk::Key::Down if n > 0 => c.select((c.selected.get() + 1) % n),
            gtk::gdk::Key::Up if n > 0 => c.select((c.selected.get() + n - 1) % n),
            gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter => {
                let a = c.buttons.borrow().get(c.selected.get()).map(|(a, _)| *a);
                if let Some(a) = a {
                    c.run(a);
                }
            }
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    });
    c.window.add_controller(keys);
    CARD.with(|x| *x.borrow_mut() = Some(c.clone()));
    Some(c)
}

impl Card {
    fn select(&self, i: usize) {
        self.selected.set(i);
        for (k, (_, b)) in self.buttons.borrow().iter().enumerate() {
            if k == i {
                b.add_css_class("escolhida");
            } else {
                b.remove_css_class("escolhida");
            }
        }
    }

    fn row(self: &Rc<Self>, a: Action, glyph: &str, glyph_color: &str, title: &str, sub: &str) {
        let b = gtk::Button::new();
        b.add_css_class("midia-acao");
        let r = caixa(Orientation::Horizontal, 12, "");
        let ic = ui_icon(glyph, glyph_color, 16);
        ic.set_valign(Align::Center);
        r.append(&ic);
        let t = caixa(Orientation::Vertical, 1, "");
        t.set_hexpand(true);
        t.set_valign(Align::Center);
        let tl = rotulo(title, "midia-acao-titulo");
        tl.set_xalign(0.0);
        t.append(&tl);
        if !sub.is_empty() {
            let sl = rotulo(sub, "midia-sub");
            sl.set_xalign(0.0);
            t.append(&sl);
        }
        r.append(&t);
        let enter = rotulo("↵", "midia-sub");
        enter.add_css_class("midia-enter");
        r.append(&enter);
        b.set_child(Some(&r));
        let w = Rc::downgrade(self);
        b.connect_clicked(move |_| if let Some(c) = w.upgrade() { c.run(a) });
        self.rows.append(&b);
        self.buttons.borrow_mut().push((a, b));
    }

    fn fill(self: &Rc<Self>) {
        while let Some(x) = self.rows.first_child() {
            self.rows.remove(&x);
        }
        self.buttons.borrow_mut().clear();
        let n = self.photos.borrow().len();
        let year = destination().and_then(|d| d.file_name().map(|x| x.to_string_lossy().into_owned())).unwrap_or_default();
        self.row(Action::Open, "pasta", "#B9BAC4", tr("Open in the Files app"), "");
        if n > 0 {
            let title = if n == 1 { tr("Import 1 photo").to_string() } else { trf("Import {n} photos", &[("n", n.to_string())]) };
            self.row(Action::Import, "imagem", "#B9BAC4", &title,
                &trf("to Pictures/{year}, without duplicating the ones already there", &[("year", year)]));
            self.row(Action::Organize, "spark", "#A5A1E2", tr("Ask the agent to organize them"), tr("imports the photos and sorts them by date · local model"));
        }
        self.select(0);
        if debug() {
            let names: Vec<&str> = self.buttons.borrow().iter().map(|(a, _)| a.key()).collect();
            eprintln!("debug midia: ações {}", names.join(", "));
            // (for the session test: where they are on the screen, once laid out — the card is at the top right)
            let w = Rc::downgrade(self);
            glib::timeout_add_local_once(Duration::from_millis(700), move || {
                let Some(c) = w.upgrade() else { return };
                let mut all: Vec<(String, gtk::Widget)> = c.buttons.borrow().iter().map(|(a, b)| (a.key().to_string(), b.clone().upcast())).collect();
                all.push(("ejetar".into(), c.eject.clone().upcast()));
                all.push(("sempre".into(), c.always.clone().upcast()));
                for (n, wd) in all {
                    if let Some((x, y)) = c.on_screen(&wd) {
                        eprintln!("debug midia: {n} em {x} {y}");
                    }
                }
            });
        }
    }

    fn on_screen(&self, wd: &gtk::Widget) -> Option<(i32, i32)> {
        // the output's size from niri (GDK's monitor can keep an old size after a scale change)
        let (largura, _) = crate::superficies::tamanho_da_saida()?;
        let r = wd.compute_bounds(&self.window)?;
        let x = largura - 16 - self.window.width() + (r.x() + r.width() / 2.) as i32;
        let y = crate::superficies::BARRA_ALTURA + 12 + (r.y() + r.height() / 2.) as i32;
        Some((x, y))
    }

    fn summary(&self, free: Option<u64>, photos: Option<usize>) -> String {
        let mut parts = Vec::new();
        if let Some(f) = free {
            parts.push(trf("{size} free", &[("size", human(f))]));
        }
        parts.push(match photos {
            None => tr("counting photos…").to_string(),
            Some(0) => tr("no new photos").to_string(),
            Some(1) => tr("1 new photo").to_string(),
            Some(n) => trf("{n} new photos", &[("n", n.to_string())]),
        });
        parts.join(" · ")
    }

    fn show(self: &Rc<Self>, d: Drive) {
        if debug() {
            eprintln!("debug midia: conectado {} em {}", d.name, d.root.display());
        }
        self.title.set_text(&trf("{name} connected", &[("name", d.name.clone())]));
        let fs = gio::File::for_path(&d.root).query_filesystem_info("filesystem::free,filesystem::size", gio::Cancellable::NONE).ok();
        let free = fs.as_ref().map(|i| i.attribute_uint64("filesystem::free"));
        let size = fs.as_ref().map(|i| i.attribute_uint64("filesystem::size")).unwrap_or(0);
        self.used.set(if size > 0 { 1.0 - free.unwrap_or(size) as f64 / size as f64 } else { 0.0 });
        self.bar.queue_draw();
        self.subtitle.remove_css_class("erro");
        self.subtitle.set_text(&self.summary(free, None));
        self.photos.borrow_mut().clear();
        *self.drive.borrow_mut() = Some(d.clone());
        self.always.set_active(false);
        self.always.set_visible(true);
        self.eject.set_visible(true);
        self.eject.set_sensitive(true);
        self.fill();
        self.window.present();
        self.motion.entrar((0.0, -8.0), 0.97);
        self.close_later(12);
        // the photos it has that Pictures does not: counted aside
        let w = Rc::downgrade(self);
        let root = d.root.clone();
        crate::lateral::em_segundo_plano(move || destination().map(|d| new_photos(&root, &d)).unwrap_or_default(), move |found| {
            let Some(c) = w.upgrade() else { return };
            if c.drive.borrow().as_ref().map(|x| x.root.clone()) != Some(d.root.clone()) {
                return;
            }
            if debug() {
                eprintln!("debug midia: {} fotos novas em {}", found.len(), d.name);
            }
            c.subtitle.set_text(&c.summary(free, Some(found.len())));
            *c.photos.borrow_mut() = found;
            c.fill();
        });
    }

    /// Closes after `secs`, unless the pointer is on it then.
    fn close_later(self: &Rc<Self>, secs: u32) {
        if let Some(t) = self.closing.borrow_mut().take() {
            t.remove();
        }
        let w = Rc::downgrade(self);
        *self.closing.borrow_mut() = Some(glib::timeout_add_seconds_local_once(secs, move || {
            let Some(c) = w.upgrade() else { return };
            c.closing.borrow_mut().take();
            if !c.hovered.get() {
                c.hide();
            }
        }));
    }

    fn hide(self: &Rc<Self>) {
        if let Some(t) = self.closing.borrow_mut().take() {
            t.remove();
        }
        if !self.window.is_visible() {
            return;
        }
        let w = Rc::downgrade(self);
        self.motion.sair((0.0, -8.0), 0.97, move || {
            if let Some(c) = w.upgrade() {
                c.window.set_visible(false);
            }
        });
    }

    fn run(self: &Rc<Self>, a: Action) {
        let Some(d) = self.drive.borrow().clone() else { return };
        if self.always.is_active() {
            remember(&d.id, a);
            if debug() {
                eprintln!("debug midia: sempre {} para {}", a.key(), d.name);
            }
        }
        let photos = self.photos.borrow().clone();
        self.hide();
        act(&d, a, photos);
    }

    fn do_eject(self: &Rc<Self>) {
        let Some(d) = self.drive.borrow().clone() else { return };
        self.eject.set_sensitive(false);
        self.subtitle.set_text(tr("Ejecting…"));
        if let Some(t) = self.closing.borrow_mut().take() {
            t.remove();
        }
        let w = Rc::downgrade(self);
        let drive = d.clone();
        eject(&drive, move |res| {
            let Some(c) = w.upgrade() else { return };
            match res {
                Ok(()) => {
                    if debug() {
                        eprintln!("debug midia: pode remover {}", d.name);
                    }
                    // only now it is safe: the card says so, and goes
                    c.title.set_text(&trf("You can remove {name}", &[("name", d.name.clone())]));
                    c.subtitle.set_text(tr("Everything was written; it is safe to unplug it."));
                    while let Some(x) = c.rows.first_child() {
                        c.rows.remove(&x);
                    }
                    c.buttons.borrow_mut().clear();
                    c.always.set_visible(false);
                    c.eject.set_visible(false);
                    c.used.set(0.0);
                    c.bar.queue_draw();
                    c.close_later(4);
                }
                Err(e) => {
                    if debug() {
                        eprintln!("debug midia: não ejetou {}: {e}", d.name);
                    }
                    c.subtitle.add_css_class("erro");
                    c.subtitle.set_text(tr("In use: close what is open on it and try again."));
                    c.eject.set_sensitive(true);
                    c.close_later(12);
                }
            }
        });
    }
}

/// What the person chose for a drive (from the card, or remembered).
fn act(d: &Drive, a: Action, photos: Vec<PathBuf>) {
    let Some(s) = crate::shell_atual() else { return };
    if debug() {
        eprintln!("debug midia: {} {}", a.key(), d.name);
    }
    match a {
        Action::Open => s.abrir_arquivos(Some(&d.root)),
        Action::Import | Action::Organize => {
            let Some(dest) = destination().filter(|_| !photos.is_empty()) else { return };
            let _ = std::fs::create_dir_all(&dest);
            let n = photos.len();
            match s.operacoes() {
                Some(ops) => {
                    ops.iniciar(photos, dest.clone(), false);
                    if a == Action::Organize {
                        // once the copy is done, the agent sorts them
                        let name = d.name.clone();
                        glib::timeout_add_local(Duration::from_millis(500), move || {
                            if ops.ativa() {
                                return glib::ControlFlow::Continue;
                            }
                            ask_agent(&dest, &name, n);
                            glib::ControlFlow::Break
                        });
                    }
                }
                None => eprintln!("kalango-shell: mídia: sem as operações de arquivos"),
            }
        }
    }
}

fn ask_agent(dest: &Path, name: &str, n: usize) {
    let Some(s) = crate::shell_atual() else { return };
    let pedido = trf("{n} photos just came from “{name}” to {path}. Sort them into folders by date (year-month), by when each was taken, without deleting anything.",
        &[("n", n.to_string()), ("name", name.to_string()), ("path", dest.display().to_string())]);
    if debug() {
        eprintln!("debug midia: pedido ao agente: {pedido}");
    }
    s.right_bar().ask_agent(&pedido);
}

fn eject(d: &Drive, done: impl FnOnce(Result<(), String>) + 'static) {
    // the test's drives: a mark beside the folder
    if let Some(m) = d.mount.clone() {
        let op = gtk::MountOperation::new(gtk::Window::NONE);
        let finish = move |r: Result<(), glib::Error>| done(r.map_err(|e| e.message().to_string()));
        if m.can_eject() {
            m.eject_with_operation(gio::MountUnmountFlags::NONE, Some(&op), gio::Cancellable::NONE, finish);
        } else {
            m.unmount_with_operation(gio::MountUnmountFlags::NONE, Some(&op), gio::Cancellable::NONE, finish);
        }
        return;
    }
    let mark = d.root.with_extension("ejetado");
    // (a drive "in use": the test leaves a file named "em-uso" in it)
    if d.root.join("em-uso").exists() {
        done(Err("busy".into()));
        return;
    }
    let r = std::fs::write(&mark, b"").map_err(|e| e.to_string());
    TEST_SEEN.with(|s| s.borrow_mut().remove(&d.root));
    // (a real drive ejected powers off: the volume monitor's "disconnected" plays it)
    if r.is_ok() {
        removed(&d.name);
    }
    done(r);
}

thread_local! {
    static TEST_SEEN: RefCell<BTreeSet<PathBuf>> = const { RefCell::new(BTreeSet::new()) };
}

/// A drive arrived: what is remembered for it, else the card.
fn arrived(d: Drive) {
    crate::configuracoes_som::tocar("device-added");
    if let Some(a) = remembered(&d.id) {
        if debug() {
            eprintln!("debug midia: conectado {} (sempre {})", d.name, a.key());
        }
        let root = d.root.clone();
        crate::lateral::em_segundo_plano(move || destination().map(|x| new_photos(&root, &x)).unwrap_or_default(), move |photos| act(&d, a, photos));
        return;
    }
    if let Some(c) = card() {
        c.show(d);
    }
}

/// A drive was unplugged (or ejected and powered off).
fn removed(name: &str) {
    if debug() {
        eprintln!("debug midia: removido {name}");
    }
    crate::configuracoes_som::tocar("device-removed");
}

fn is_removable(m: &gio::Mount) -> bool {
    m.drive().is_some_and(|d| d.is_removable() || d.is_media_removable()) || m.can_eject()
}

/// Mounts removable drives as they come, and follows them.
pub fn start() {
    if let Some(dir) = std::env::var_os("KALANGO_MIDIA_TESTE").map(PathBuf::from) {
        // the session test: each subfolder that appears is a drive
        glib::timeout_add_local(Duration::from_millis(500), move || {
            // a "drive" whose folder went away was removed
            let gone: Vec<PathBuf> = TEST_SEEN.with(|s| s.borrow().iter().filter(|p| !p.exists()).cloned().collect());
            for p in gone {
                TEST_SEEN.with(|s| s.borrow_mut().remove(&p));
                removed(&p.file_name().unwrap_or_default().to_string_lossy());
            }
            for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                let p = e.path();
                if !p.is_dir() || TEST_SEEN.with(|s| s.borrow().contains(&p)) || p.with_extension("ejetado").exists() {
                    continue;
                }
                TEST_SEEN.with(|s| s.borrow_mut().insert(p.clone()));
                let name = e.file_name().to_string_lossy().into_owned();
                arrived(Drive { id: format!("teste-{name}"), name, root: p, mount: None });
            }
            glib::ControlFlow::Continue
        });
        return;
    }
    if std::env::var_os("KALANGO_SHELL_DEV").is_some() {
        // the development session runs inside someone's desktop: its drives are that desktop's
        return;
    }
    let vm = gio::VolumeMonitor::get();
    vm.connect_volume_added(|_, v| {
        if !(v.can_mount() && v.should_automount()) || v.get_mount().is_some() {
            return;
        }
        let op = gtk::MountOperation::new(gtk::Window::NONE);
        v.mount(gio::MountMountFlags::NONE, Some(&op), gio::Cancellable::NONE, |r| {
            if let Err(e) = r {
                if !e.matches(gio::IOErrorEnum::FailedHandled) {
                    eprintln!("kalango-shell: mídia: {e}");
                }
            }
        });
    });
    vm.connect_drive_disconnected(|_, d| {
        if d.is_removable() || d.is_media_removable() {
            removed(&d.name());
        }
    });
    vm.connect_mount_added(|_, m| {
        if !is_removable(m) || m.is_shadowed() {
            return;
        }
        let Some(root) = m.root().path() else { return };
        let id = m.uuid().or_else(|| m.volume().and_then(|v| v.uuid())).map(|u| u.to_string()).unwrap_or_else(|| m.name().to_string());
        let name = m.drive().map(|d| d.name().to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| m.name().to_string());
        arrived(Drive { name, root, id, mount: Some(m.clone()) });
    });
    MONITOR.with(|x| *x.borrow_mut() = Some(vm));
}

#[cfg(test)]
mod tests {
    use super::new_photos;

    #[test]
    fn only_photos_pictures_does_not_have() {
        let base = std::env::temp_dir().join(format!("kalango-midia-{}", std::process::id()));
        let (drive, dest) = (base.join("drive/DCIM/100"), base.join("dest"));
        std::fs::create_dir_all(&drive).unwrap();
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(drive.join("IMG_1.JPG"), b"aaaa").unwrap();
        std::fs::write(drive.join("IMG_2.jpg"), b"bb").unwrap();
        std::fs::write(drive.join("notas.txt"), b"x").unwrap();
        // already there (same name and size)
        std::fs::write(dest.join("IMG_2.jpg"), b"bb").unwrap();
        let found = new_photos(&base.join("drive"), &dest);
        assert_eq!(found.len(), 1);
        assert!(found[0].ends_with("IMG_1.JPG"));
        let _ = std::fs::remove_dir_all(&base);
    }
}
