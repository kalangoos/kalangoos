//! KalangoOS's own document reader (design "Leitor de documentos", 59670; app id os.kalango.Documentos;
//! `kalango-shell documentos [FILE]`): PDFs in tabs, read with poppler. The pages in a column of thumbnails and on the
//! desk (zoom, search, print, share); on the right the agent's panel, which slides in and can be put away: a summary
//! of the document, the places worth finding ("p. 2"), suggested questions and the answers to the person's own — each
//! citing its page: a click scrolls there and lights the words in gold for 1.2 s. Answered by the local model: the
//! file never leaves the computer.

use crate::conversa::ui_icon;
use crate::superficies::{caixa, rotulo};
use crate::Shell;
use gtk::prelude::*;
use gtk::{cairo, gdk, gio, glib, Align, Orientation};
use kalango_i18n::{tr, trf};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};

extern "C" {
    fn gdk_wayland_toplevel_set_application_id(toplevel: *mut gdk::ffi::GdkToplevel, id: *const std::ffi::c_char);
}

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

/// The zoom steps (1 = a point of the page per pixel).
const ZOOMS: &[f64] = &[0.5, 0.67, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0];
const THUMB: f64 = 96.0;
const HIGHLIGHT_US: i64 = 1_200_000;

pub fn is_document(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
}

/// Where the reader keeps what the agent said of a document, by its path and date.
fn cache_file(path: &Path) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    std::fs::metadata(path).and_then(|m| m.modified()).ok().hash(&mut h);
    glib::user_cache_dir().join("kalango").join("documentos").join(format!("{:016x}.json", h.finish()))
}

/// Text without its accents ("próxima revisão" → "proxima revisao"): what the person types and what the model
/// copies are found whatever their accents.
fn plain(s: &str) -> String {
    s.chars().map(|c| match c {
        'á' | 'à' | 'â' | 'ã' | 'ä' => 'a', 'Á' | 'À' | 'Â' | 'Ã' | 'Ä' => 'A',
        'é' | 'è' | 'ê' | 'ë' => 'e', 'É' | 'È' | 'Ê' | 'Ë' => 'E',
        'í' | 'ì' | 'î' | 'ï' => 'i', 'Í' | 'Ì' | 'Î' | 'Ï' => 'I',
        'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o', 'Ó' | 'Ò' | 'Ô' | 'Õ' | 'Ö' => 'O',
        'ú' | 'ù' | 'û' | 'ü' => 'u', 'Ú' | 'Ù' | 'Û' | 'Ü' => 'U',
        'ç' => 'c', 'Ç' => 'C', 'ñ' => 'n', 'Ñ' => 'N',
        c => c,
    }).collect()
}

/// Where words are on a page: without minding case or accents, across line ends.
fn find_on(page: &poppler::Page, q: &str) -> Vec<poppler::Rectangle> {
    page.find_text_with_options(&plain(q), poppler::FindFlags::IGNORE_DIACRITICS | poppler::FindFlags::MULTILINE)
}

/// The line of a page that shares the most words with a quote (the model's copy was not word for word): its start.
fn closest_line(text: &str, quote: &str) -> Option<String> {
    let words: Vec<String> = plain(quote).to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| w.chars().count() >= 4).map(str::to_string).collect();
    let (score, line) = text.lines().map(|l| {
        let low = plain(l).to_lowercase();
        (words.iter().filter(|w| low.contains(w.as_str())).count(), l)
    }).max_by_key(|(n, _)| *n)?;
    (score >= 2).then(|| line.split_whitespace().take(6).collect::<Vec<_>>().join(" "))
}

/// A few words of a quote, to find it when the model's copy is not exact: the whole, then shorter starts.
fn quote_tries(q: &str) -> Vec<String> {
    let words: Vec<&str> = q.split_whitespace().collect();
    let mut v = vec![words.join(" ")];
    for n in [6, 4, 3] {
        if words.len() > n {
            v.push(words[..n].join(" "));
        }
    }
    v.retain(|s| s.chars().count() >= 3);
    v.dedup();
    v
}

/// An open document: its pages on the desk, their thumbnails, and the agent's panel for it.
struct DocView {
    path: PathBuf,
    doc: poppler::Document,
    sizes: Vec<(f64, f64)>,
    texts: Vec<String>,
    zoom: Cell<f64>,
    root: gtk::Box,
    tab: gtk::Box,
    tab_label: gtk::Label,
    thumb_items: Vec<gtk::Box>,
    desk: gtk::ScrolledWindow,
    pages_box: gtk::Box,
    pages: Vec<gtk::DrawingArea>,
    page_label: gtk::Label,
    zoom_label: gtk::Label,
    find_bar: gtk::Box,
    find: gtk::SearchEntry,
    find_count: gtk::Label,
    hits: RefCell<Vec<(usize, poppler::Rectangle)>>,
    hit: Cell<usize>,
    panel: gtk::Revealer,
    summary: gtk::Box,
    places: gtk::Box,
    questions: gtk::Box,
    answers: gtk::Box,
    panel_scroll: gtk::ScrolledWindow,
    panel_ask: gtk::Entry,
    cache: RefCell<HashMap<(usize, i32), cairo::ImageSurface>>,
    thumb_cache: RefCell<HashMap<usize, cairo::ImageSurface>>,
    light: RefCell<Option<(usize, Vec<poppler::Rectangle>, i64)>>,
    current: Cell<usize>,
}

pub struct DocReader {
    window: gtk::ApplicationWindow,
    tabs: gtk::Box,
    stack: gtk::Stack,
    ask: gtk::Entry,
    empty: gtk::Label,
    views: RefCell<Vec<Rc<DocView>>>,
    shell: Weak<Shell>,
    /// a print in progress, and the one asked for meanwhile (started when it ends: two at once left one hanging,
    /// and the window could not close)
    printing: Cell<bool>,
    next_print: RefCell<Option<Weak<DocView>>>,
}

thread_local! {
    static READER: RefCell<Option<Rc<DocReader>>> = const { RefCell::new(None) };
}

/// Opens the reader, with a document in a new tab when one is given.
pub fn open(shell: &Rc<Shell>, file: Option<PathBuf>) {
    let r = READER.with(|r| r.borrow().clone()).unwrap_or_else(|| {
        let app = gio::Application::default().and_downcast::<gtk::Application>().expect("app");
        let r = DocReader::new(&app, shell);
        READER.with(|x| *x.borrow_mut() = Some(r.clone()));
        r
    });
    if !r.window.is_visible() {
        let (w, h) = crate::superficies::tamanho_de_janela(1500, 910);
        r.window.set_default_size(w, h);
    }
    r.window.present();
    if let Some(f) = file {
        r.open_file(&f, None);
    }
    r.empty.set_visible(r.views.borrow().is_empty());
}

fn icon_button(glyph: &str, px: i32, class: &str, tip: &str) -> gtk::Button {
    let b = gtk::Button::new();
    b.add_css_class(class);
    b.set_child(Some(&ui_icon(glyph, "#E2E2E6", px)));
    b.set_tooltip_text(Some(tip));
    b.set_valign(Align::Center);
    b
}

fn section(title: &str, spark: bool) -> gtk::Box {
    let r = caixa(Orientation::Horizontal, 8, "");
    if spark {
        r.append(&ui_icon("spark", "#A5A1E2", 14));
    }
    let l = rotulo(title, "doc-secao");
    l.set_xalign(0.0);
    r.append(&l);
    r
}

fn rounded(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
    cr.arc(x + r, y + r, r, std::f64::consts::PI, 1.5 * std::f64::consts::PI);
    cr.close_path();
}

impl DocReader {
    fn new(app: &gtk::Application, shell: &Rc<Shell>) -> Rc<DocReader> {
        let (w, h) = crate::superficies::tamanho_de_janela(1500, 910);
        let window = gtk::ApplicationWindow::builder().application(app).title(tr("Documents")).default_width(w).default_height(h).build();
        // (its app id as its name: the state written by `kalango-shell estado` finds it by it)
        window.set_widget_name("os.kalango.Documentos");
        // (closed for a while, the window lets go of its surface and renderer and keeps its widgets: see
        // superficies::release_resources_when_hidden — not while it prints: the print dialog knows the window by its surface)
        crate::superficies::release_resources_when_hidden(&window, || {
            READER.with(|r| r.borrow().as_ref().is_none_or(|r| crate::release_policy::documents(r.printing.get())))
        });
        // (one window for the app, kept: closing hides it and opening shows it again — a closed GTK window shown again
        // could not be closed any more, and it kept text input from the other windows)
        window.set_hide_on_close(true);
        window.add_css_class("ed-janela");
        let sem_barra = gtk::Box::new(Orientation::Horizontal, 0);
        sem_barra.set_visible(false);
        window.set_titlebar(Some(&sem_barra));
        let bar = caixa(Orientation::Horizontal, 10, "ed-barra");
        bar.append(&crate::apps::imagem_kalango("documentos", 24));
        let tabs = caixa(Orientation::Horizontal, 4, "");
        bar.append(&tabs);
        let plus = icon_button("mais", 12, "doc-nova-aba", tr("Open a document"));
        crate::central::posicao_debug_sempre("documentos nova aba", plus.upcast_ref());
        bar.append(&plus);
        let space = caixa(Orientation::Horizontal, 0, "");
        space.set_hexpand(true);
        bar.append(&space);
        let ask = gtk::Entry::new();
        ask.add_css_class("ed-pedir");
        ask.set_size_request(340, 32);
        ask.set_valign(Align::Center);
        ask.set_placeholder_text(Some(tr("Ask about this document…")));
        ask.set_primary_icon_paintable(ui_icon("spark", "#A5A1E2", 13).paintable().as_ref());
        let ab = gtk::Overlay::new();
        ab.set_child(Some(&ask));
        let key = rotulo("Ctrl K", "tb-atalho");
        key.set_halign(Align::End);
        key.set_valign(Align::Center);
        key.set_margin_end(8);
        key.set_can_target(false);
        ab.add_overlay(&key);
        bar.append(&ab);
        let menu = gtk::MenuButton::new();
        menu.add_css_class("calc-menu");
        menu.set_child(Some(&ui_icon("menu", "#8E8F99", 16)));
        menu.set_valign(Align::Center);
        let pop = gtk::Popover::new();
        pop.add_css_class("menu");
        let items = caixa(Orientation::Vertical, 2, "menu-lista");
        pop.set_child(Some(&items));
        menu.set_popover(Some(&pop));
        bar.append(&menu);
        bar.append(&crate::configuracoes::controles_janela());
        let handle = gtk::WindowHandle::new();
        handle.set_child(Some(&bar));
        let line = gtk::Separator::new(Orientation::Horizontal);
        line.add_css_class("vid-linha");
        let stack = gtk::Stack::new();
        stack.set_vexpand(true);
        stack.set_hexpand(true);
        let empty = rotulo(tr("Open a PDF with + or drop it here."), "img-vazio");
        empty.set_halign(Align::Center);
        empty.set_valign(Align::Center);
        let body = gtk::Overlay::new();
        body.set_child(Some(&stack));
        body.add_overlay(&empty);
        body.set_vexpand(true);
        let root = caixa(Orientation::Vertical, 0, "doc-fundo");
        root.append(&handle);
        root.append(&line);
        root.append(&body);
        window.set_child(Some(&root));
        let r = Rc::new(DocReader { window, tabs, stack, ask, empty, views: RefCell::default(), shell: Rc::downgrade(shell), printing: Cell::new(false), next_print: RefCell::default() });
        r.wire(&plus, &items, &pop, &body);
        r
    }

    fn wire(self: &Rc<Self>, plus: &gtk::Button, items: &gtk::Box, pop: &gtk::Popover, body: &gtk::Overlay) {
        let w = Rc::downgrade(self);
        plus.connect_clicked(move |_| {
            if let Some(r) = w.upgrade() {
                r.choose();
            }
        });
        let add = |label: &str, f: Box<dyn Fn(&Rc<DocReader>)>| {
            let b = gtk::Button::with_label(label);
            b.add_css_class("menu-item");
            if let Some(l) = b.child().and_downcast::<gtk::Label>() {
                l.set_xalign(0.0);
            }
            let (w, pop) = (Rc::downgrade(self), pop.clone());
            b.connect_clicked(move |_| {
                pop.popdown();
                if let Some(r) = w.upgrade() {
                    f(&r);
                }
            });
            items.append(&b);
        };
        add(tr("Open…"), Box::new(|r| r.choose()));
        add(tr("Print…"), Box::new(|r| {
            if let Some(v) = r.view() {
                r.print(&v);
            }
        }));
        add(tr("Show in the folder"), Box::new(|r| {
            if let (Some(v), Some(s)) = (r.view(), r.shell.upgrade()) {
                s.abrir_arquivos(Some(&v.path));
            }
        }));
        add(tr("Close the tab"), Box::new(|r| r.close_current()));
        // the request in the bar goes to the agent's panel of the document in view
        let w = Rc::downgrade(self);
        self.ask.connect_activate(move |e| {
            let t = e.text().trim().to_string();
            if let (Some(r), false) = (w.upgrade(), t.is_empty()) {
                e.set_text("");
                if let Some(v) = r.view() {
                    v.panel.set_reveal_child(true);
                    r.question(&v, t);
                }
            }
        });
        // a PDF dropped on the window opens in a new tab
        let drop = gtk::DropTarget::new(gio::File::static_type(), gdk::DragAction::COPY);
        let w = Rc::downgrade(self);
        drop.connect_drop(move |_, v, _, _| {
            let (Some(r), Ok(f)) = (w.upgrade(), v.get::<gio::File>()) else { return false };
            match f.path() {
                Some(p) if is_document(&p) => {
                    r.open_file(&p, None);
                    true
                }
                _ => false,
            }
        });
        body.add_controller(drop);
        // keys
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let w = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, k, _, m| {
            let Some(r) = w.upgrade() else { return glib::Propagation::Proceed };
            let ctrl = m.contains(gdk::ModifierType::CONTROL_MASK);
            let focus = gtk::prelude::GtkWindowExt::focus(&r.window);
            let typing = focus.is_some_and(|f| f.downcast_ref::<gtk::Text>().is_some() || f.is::<gtk::Entry>());
            let v = r.view();
            match (k, ctrl) {
                (gdk::Key::k, true) => {
                    r.ask.grab_focus();
                }
                (gdk::Key::f, true) => {
                    if let Some(v) = v {
                        v.find_bar.set_visible(true);
                        v.find.grab_focus();
                    }
                }
                (gdk::Key::o, true) | (gdk::Key::t, true) => r.choose(),
                (gdk::Key::w, true) => r.close_current(),
                (gdk::Key::p, true) => {
                    if let Some(v) = v {
                        r.print(&v);
                    }
                }
                (gdk::Key::plus | gdk::Key::equal | gdk::Key::KP_Add, true) => {
                    if let Some(v) = v {
                        r.zoom_step(&v, 1);
                    }
                }
                (gdk::Key::minus | gdk::Key::KP_Subtract, true) => {
                    if let Some(v) = v {
                        r.zoom_step(&v, -1);
                    }
                }
                (gdk::Key::_0, true) => {
                    if let Some(v) = v {
                        r.set_zoom(&v, 1.0);
                    }
                }
                (gdk::Key::Escape, _) => {
                    if let Some(v) = v.filter(|v| v.find_bar.is_visible()) {
                        v.find_bar.set_visible(false);
                        v.hits.borrow_mut().clear();
                        r.redraw(&v);
                    } else {
                        return glib::Propagation::Proceed;
                    }
                }
                _ if typing => return glib::Propagation::Proceed,
                (gdk::Key::Page_Down | gdk::Key::Right, false) => {
                    if let Some(v) = v {
                        r.go_page(&v, v.current.get() + 1);
                    }
                }
                (gdk::Key::Page_Up | gdk::Key::Left, false) => {
                    if let Some(v) = v {
                        r.go_page(&v, v.current.get().saturating_sub(1));
                    }
                }
                (gdk::Key::Home, false) => {
                    if let Some(v) = v {
                        r.go_page(&v, 0);
                    }
                }
                (gdk::Key::End, false) => {
                    if let Some(v) = v {
                        r.go_page(&v, v.pages.len().saturating_sub(1));
                    }
                }
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        self.window.add_controller(keys);
        let w = Rc::downgrade(self);
        self.stack.connect_visible_child_notify(move |_| {
            if let Some(r) = w.upgrade() {
                r.mark_tabs();
            }
        });
        let app_id = |j: &gtk::ApplicationWindow| {
            if let Some(s) = j.surface().filter(|s| s.type_().name().contains("Wayland")) {
                let id = std::ffi::CString::new("os.kalango.Documentos").unwrap();
                unsafe { gdk_wayland_toplevel_set_application_id(s.as_ptr() as *mut gdk::ffi::GdkToplevel, id.as_ptr()) };
            }
        };
        self.window.connect_realize(move |j| app_id(j));
        self.window.connect_map(move |j| {
            app_id(j);
            // (the keys are the app's: the question fields is focused only when asked, Ctrl+K)
            let j = j.clone();
            glib::idle_add_local_once(move || gtk::prelude::GtkWindowExt::set_focus(&j, None::<&gtk::Widget>));
        });
    }

    fn view(&self) -> Option<Rc<DocView>> {
        let name = self.stack.visible_child_name()?;
        self.views.borrow().iter().find(|v| v.path.to_string_lossy() == name.as_str()).cloned()
    }

    fn choose(self: &Rc<Self>) {
        let d = gtk::FileDialog::new();
        d.set_title(tr("Open a document"));
        let f = gtk::FileFilter::new();
        f.set_name(Some("PDF"));
        f.add_mime_type("application/pdf");
        f.add_suffix("pdf");
        let fs = gio::ListStore::new::<gtk::FileFilter>();
        fs.append(&f);
        d.set_filters(Some(&fs));
        let w = Rc::downgrade(self);
        d.open(Some(&self.window), gio::Cancellable::NONE, move |r| {
            if let (Some(me), Ok(f)) = (w.upgrade(), r) {
                if let Some(p) = f.path() {
                    me.open_file(&p, None);
                }
            }
        });
    }

    /// A document in a new tab (or its tab, when it is open already). A locked PDF asks for its password.
    fn open_file(self: &Rc<Self>, path: &Path, password: Option<&str>) {
        let key = path.to_string_lossy().to_string();
        if self.views.borrow().iter().any(|v| v.path == path) {
            self.stack.set_visible_child_name(&key);
            return;
        }
        let uri = gio::File::for_path(path).uri();
        let doc = match poppler::Document::from_file(&uri, password) {
            Ok(d) => d,
            Err(e) if e.matches(poppler::Error::Encrypted) => {
                self.ask_password(path, password.is_some());
                return;
            }
            Err(e) => {
                eprintln!("kalango-shell: documentos: {e}");
                self.empty.set_text(&trf("{file} could not be opened: {error}", &[("file", path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()), ("error", e.to_string())]));
                self.empty.set_visible(true);
                return;
            }
        };
        let v = self.build_view(path, doc);
        self.views.borrow_mut().push(v.clone());
        self.stack.add_named(&v.root, Some(&key));
        self.tabs.append(&v.tab);
        self.stack.set_visible_child_name(&key);
        self.empty.set_visible(false);
        self.mark_tabs();
        if debug() {
            eprintln!("debug documentos: aberto «{}» ({} páginas)", path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(), v.pages.len());
        }
        self.summarize(&v);
    }

    fn ask_password(self: &Rc<Self>, path: &Path, wrong: bool) {
        let d = gtk::Window::builder().transient_for(&self.window).modal(true).title(tr("Locked document")).resizable(false).build();
        d.add_css_class("ed-janela");
        let col = caixa(Orientation::Vertical, 10, "doc-senha");
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let t = rotulo(&trf("{file} is locked with a password.", &[("file", name)]), "doc-cartao-texto");
        t.set_wrap(true);
        col.append(&t);
        let e = gtk::PasswordEntry::new();
        e.set_show_peek_icon(true);
        col.append(&e);
        if wrong {
            col.append(&rotulo(tr("That password did not open it."), "doc-aviso"));
        }
        let ok = gtk::Button::with_label(tr("Open"));
        ok.add_css_class("doc-acao");
        ok.set_halign(Align::End);
        col.append(&ok);
        d.set_child(Some(&col));
        let go = {
            let (w, d, e, path) = (Rc::downgrade(self), d.clone(), e.clone(), path.to_path_buf());
            move || {
                let pw = e.text().to_string();
                d.close();
                if let Some(r) = w.upgrade() {
                    r.open_file(&path, Some(&pw));
                }
            }
        };
        let g = go.clone();
        ok.connect_clicked(move |_| g());
        e.connect_activate(move |_| go());
        d.present();
    }

    fn mark_tabs(&self) {
        let cur = self.stack.visible_child_name().map(|s| s.to_string());
        for v in self.views.borrow().iter() {
            if cur.as_deref() == Some(&*v.path.to_string_lossy()) {
                v.tab.add_css_class("atual");
                self.window.set_title(Some(&v.tab_label.text()));
            } else {
                v.tab.remove_css_class("atual");
            }
        }
    }

    fn close_current(self: &Rc<Self>) {
        if let Some(v) = self.view() {
            self.close(&v);
        }
    }

    fn close(self: &Rc<Self>, v: &Rc<DocView>) {
        self.tabs.remove(&v.tab);
        self.stack.remove(&v.root);
        self.views.borrow_mut().retain(|x| !Rc::ptr_eq(x, v));
        self.empty.set_visible(self.views.borrow().is_empty());
        self.mark_tabs();
    }

    fn build_view(self: &Rc<Self>, path: &Path, doc: poppler::Document) -> Rc<DocView> {
        let n = doc.n_pages().max(0) as usize;
        let mut sizes = Vec::with_capacity(n);
        let mut texts = Vec::with_capacity(n);
        for i in 0..n {
            let p = doc.page(i as i32);
            sizes.push(p.as_ref().map(|p| p.size()).unwrap_or((595.0, 842.0)));
            texts.push(p.and_then(|p| p.text()).map(|t| t.to_string()).unwrap_or_default());
        }
        // the tab
        let file_name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let tab = caixa(Orientation::Horizontal, 6, "doc-aba");
        let tab_b = gtk::Button::new();
        tab_b.add_css_class("doc-aba-nome");
        let tab_label = rotulo(&file_name, "");
        tab_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        tab_label.set_max_width_chars(22);
        tab_b.set_child(Some(&tab_label));
        let tab_x = icon_button("fechar", 8, "doc-aba-fechar", tr("Close the tab"));
        tab.append(&tab_b);
        tab.append(&tab_x);
        // the thumbnails
        let thumbs = caixa(Orientation::Vertical, 10, "doc-miniaturas");
        let ts = gtk::ScrolledWindow::new();
        ts.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        ts.set_child(Some(&thumbs));
        ts.add_css_class("doc-paginas");
        ts.set_size_request(150, -1);
        let mut thumb_items = Vec::new();
        // the desk
        let pages_box = caixa(Orientation::Vertical, 16, "doc-mesa-paginas");
        pages_box.set_halign(Align::Center);
        let desk = gtk::ScrolledWindow::new();
        desk.set_child(Some(&pages_box));
        desk.set_hexpand(true);
        desk.set_vexpand(true);
        desk.add_css_class("doc-mesa");
        let mut pages = Vec::new();
        for i in 0..n {
            let da = gtk::DrawingArea::new();
            pages_box.append(&da);
            pages.push(da);
            let item = caixa(Orientation::Vertical, 6, "doc-miniatura-item");
            let t = gtk::DrawingArea::new();
            let (w, h) = sizes[i];
            t.set_content_width(THUMB as i32);
            t.set_content_height((THUMB * h / w.max(1.0)) as i32);
            t.add_css_class("doc-miniatura");
            t.set_halign(Align::Center);
            t.set_cursor_from_name(Some("pointer"));
            item.append(&t);
            let num = rotulo(&(i + 1).to_string(), "doc-miniatura-num");
            item.append(&num);
            thumbs.append(&item);
            thumb_items.push(item);
        }
        pages_box.set_margin_top(28);
        pages_box.set_margin_bottom(90);
        // the floating tools
        let tools = caixa(Orientation::Horizontal, 4, "doc-ferramentas");
        tools.set_halign(Align::Center);
        tools.set_valign(Align::End);
        tools.set_margin_bottom(22);
        let zminus = icon_button("menos", 10, "doc-ferramenta", tr("Zoom out"));
        let zoom_label = rotulo("100%", "doc-ferramenta-texto");
        let zplus = icon_button("mais", 11, "doc-ferramenta", tr("Zoom in"));
        let page_label = rotulo(&format!("1 / {n}"), "doc-ferramenta-texto");
        let search_b = icon_button("busca", 12, "doc-ferramenta", tr("Find in the document"));
        let panel_b = icon_button("doc", 12, "doc-ferramenta", tr("The agent's panel"));
        let share_b = gtk::MenuButton::new();
        share_b.add_css_class("doc-ferramenta");
        share_b.set_child(Some(&ui_icon("compartilhar", "#E2E2E6", 12)));
        share_b.set_tooltip_text(Some(tr("Share")));
        share_b.set_valign(Align::Center);
        let print_b = icon_button("impressora", 12, "doc-ferramenta", tr("Print"));
        tools.append(&zminus);
        tools.append(&zoom_label);
        tools.append(&zplus);
        tools.append(&caixa(Orientation::Vertical, 0, "doc-divisor"));
        tools.append(&page_label);
        tools.append(&caixa(Orientation::Vertical, 0, "doc-divisor"));
        for (b, id) in [(search_b.upcast_ref::<gtk::Widget>(), "busca"), (panel_b.upcast_ref(), "painel"), (share_b.upcast_ref(), "compartilhar"), (print_b.upcast_ref(), "imprimir")] {
            crate::central::posicao_debug_sempre(&format!("documentos {id}"), b);
            tools.append(b);
        }
        // the find bar (Ctrl+F)
        let find_bar = caixa(Orientation::Horizontal, 8, "doc-busca");
        find_bar.set_halign(Align::Center);
        find_bar.set_valign(Align::Start);
        find_bar.set_margin_top(14);
        let find = gtk::SearchEntry::new();
        find.set_placeholder_text(Some(tr("Find in the document")));
        find.set_size_request(280, -1);
        let find_count = rotulo("", "doc-ferramenta-texto");
        find_bar.append(&find);
        find_bar.append(&find_count);
        find_bar.set_visible(false);
        let desk_over = gtk::Overlay::new();
        desk_over.set_child(Some(&desk));
        desk_over.add_overlay(&tools);
        desk_over.add_overlay(&find_bar);
        desk_over.set_hexpand(true);
        // the agent's panel
        let panel_col = caixa(Orientation::Vertical, 10, "doc-agente");
        panel_col.set_size_request(400, -1);
        panel_col.set_hexpand(false);
        let pscroll = gtk::ScrolledWindow::new();
        pscroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        pscroll.set_vexpand(true);
        let pin = caixa(Orientation::Vertical, 10, "");
        pin.append(&section(tr("DOCUMENT SUMMARY"), true));
        let summary = caixa(Orientation::Vertical, 10, "");
        summary.append(&rotulo(tr("The local model is reading it…"), "doc-fraco"));
        pin.append(&summary);
        let places_t = section(tr("WHERE IT IS IN THE TEXT"), false);
        places_t.set_margin_top(6);
        pin.append(&places_t);
        let places = caixa(Orientation::Vertical, 8, "");
        pin.append(&places);
        let q_t = section(tr("SUGGESTED QUESTIONS"), false);
        q_t.set_margin_top(6);
        pin.append(&q_t);
        let questions = caixa(Orientation::Vertical, 8, "");
        pin.append(&questions);
        let answers = caixa(Orientation::Vertical, 12, "");
        answers.set_margin_top(8);
        pin.append(&answers);
        pscroll.set_child(Some(&pin));
        panel_col.append(&pscroll);
        let panel_ask = gtk::Entry::new();
        panel_ask.add_css_class("doc-perguntar");
        panel_ask.set_placeholder_text(Some(tr("Ask about this document…")));
        panel_ask.set_primary_icon_paintable(ui_icon("spark", "#8E8F99", 13).paintable().as_ref());
        panel_ask.set_secondary_icon_paintable(ui_icon("enviar", "#8E8F99", 13).paintable().as_ref());
        crate::central::posicao_debug_sempre("documentos perguntar", panel_ask.upcast_ref());
        panel_col.append(&panel_ask);
        let foot = rotulo(tr("answered by the local model · the file does not leave the computer"), "doc-rodape");
        foot.set_xalign(0.0);
        foot.set_wrap(true);
        foot.set_max_width_chars(1);
        panel_col.append(&foot);
        let panel = gtk::Revealer::new();
        panel.set_transition_type(gtk::RevealerTransitionType::SlideLeft);
        panel.set_transition_duration(260);
        panel.set_child(Some(&panel_col));
        panel.set_reveal_child(true);
        let root = caixa(Orientation::Horizontal, 0, "");
        root.append(&ts);
        root.append(&desk_over);
        root.append(&panel);
        let v = Rc::new(DocView {
            path: path.to_path_buf(), doc, sizes, texts, zoom: Cell::new(1.0), root, tab, tab_label, thumb_items, desk, pages_box, pages,
            page_label, zoom_label, find_bar, find: find.clone(),
            find_count, hits: RefCell::default(), hit: Cell::new(0), panel, summary, places, questions, answers, panel_scroll: pscroll.clone(), panel_ask,
            cache: RefCell::default(), thumb_cache: RefCell::default(), light: RefCell::default(), current: Cell::new(0),
        });
        self.wire_view(&v, &tab_b, &tab_x, &zminus, &zplus, &search_b, &panel_b, &share_b, &print_b, &find);
        self.set_zoom(&v, 1.0);
        v
    }

    #[allow(clippy::too_many_arguments)]
    fn wire_view(self: &Rc<Self>, v: &Rc<DocView>, tab_b: &gtk::Button, tab_x: &gtk::Button, zminus: &gtk::Button, zplus: &gtk::Button,
        search_b: &gtk::Button, panel_b: &gtk::Button, share_b: &gtk::MenuButton, print_b: &gtk::Button, find: &gtk::SearchEntry) {
        let (wr, wv) = (Rc::downgrade(self), Rc::downgrade(v));
        let on = move |f: Box<dyn Fn(&Rc<DocReader>, &Rc<DocView>)>| {
            let (wr, wv) = (wr.clone(), wv.clone());
            move || {
                if let (Some(r), Some(v)) = (wr.upgrade(), wv.upgrade()) {
                    f(&r, &v);
                }
            }
        };
        let f = on(Box::new(|r, v| r.stack.set_visible_child_name(&v.path.to_string_lossy())));
        tab_b.connect_clicked(move |_| f());
        let f = on(Box::new(|r, v| r.close(v)));
        tab_x.connect_clicked(move |_| f());
        let f = on(Box::new(|r, v| r.zoom_step(v, -1)));
        zminus.connect_clicked(move |_| f());
        let f = on(Box::new(|r, v| r.zoom_step(v, 1)));
        zplus.connect_clicked(move |_| f());
        let f = on(Box::new(|_, v| {
            let show = !v.find_bar.is_visible();
            v.find_bar.set_visible(show);
            if show {
                v.find.grab_focus();
            }
        }));
        search_b.connect_clicked(move |_| f());
        let f = on(Box::new(|_, v| {
            let open = !v.panel.reveals_child();
            v.panel.set_reveal_child(open);
            if debug() {
                eprintln!("debug documentos: painel {open}");
            }
        }));
        panel_b.connect_clicked(move |_| f());
        let f = on(Box::new(|r, v| r.print(v)));
        print_b.connect_clicked(move |_| f());
        // share: the page's text, the path, the folder
        let sp = gtk::Popover::new();
        sp.add_css_class("menu");
        let items = caixa(Orientation::Vertical, 2, "menu-lista");
        let entries: Vec<(&str, Box<dyn Fn(&Rc<DocReader>, &Rc<DocView>)>)> = vec![
            (tr("Copy this page's text"), Box::new(|r, v| {
                let t = v.texts.get(v.current.get()).cloned().unwrap_or_default();
                r.window.clipboard().set_text(&t);
            })),
            (tr("Copy the path"), Box::new(|r, v| r.window.clipboard().set_text(&v.path.display().to_string()))),
            (tr("Show in the folder"), Box::new(|r, v| {
                if let Some(s) = r.shell.upgrade() {
                    s.abrir_arquivos(Some(&v.path));
                }
            })),
        ];
        for (label, g) in entries {
            let b = gtk::Button::with_label(label);
            b.add_css_class("menu-item");
            if let Some(l) = b.child().and_downcast::<gtk::Label>() {
                l.set_xalign(0.0);
            }
            let f = on(g);
            let sp2 = sp.clone();
            b.connect_clicked(move |_| {
                sp2.popdown();
                f();
            });
            items.append(&b);
        }
        sp.set_child(Some(&items));
        share_b.set_popover(Some(&sp));
        // find: as typed, every page; Enter goes to the next place
        let f = on(Box::new(|r, v| r.run_find(v)));
        find.connect_search_changed(move |_| f());
        let f = on(Box::new(|r, v| r.next_hit(v)));
        find.connect_activate(move |_| f());
        // the pages and the thumbnails draw themselves (rendered once a zoom, kept)
        for (i, da) in v.pages.iter().enumerate() {
            let wv = Rc::downgrade(v);
            da.set_draw_func(move |da, cr, w, h| {
                if let Some(v) = wv.upgrade() {
                    draw_page(&v, i, da, cr, f64::from(w), f64::from(h));
                }
            });
        }
        for (i, item) in v.thumb_items.iter().enumerate() {
            let Some(t) = item.first_child().and_downcast::<gtk::DrawingArea>() else { continue };
            let wv = Rc::downgrade(v);
            t.set_draw_func(move |t, cr, w, h| {
                if let Some(v) = wv.upgrade() {
                    draw_thumb(&v, i, t, cr, f64::from(w), f64::from(h));
                }
            });
            let g = gtk::GestureClick::new();
            let f = on(Box::new(move |r, v| r.go_page(v, i)));
            g.connect_released(move |_, _, _, _| f());
            t.add_controller(g);
        }
        // the page in view: the counter and the thumbnail
        let f = on(Box::new(|r, v| r.track_page(v)));
        v.desk.vadjustment().connect_value_changed(move |_| f());
        // Ctrl + the wheel zooms
        let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        wheel.set_propagation_phase(gtk::PropagationPhase::Capture);
        let (wr, wv) = (Rc::downgrade(self), Rc::downgrade(v));
        wheel.connect_scroll(move |c, _, dy| {
            if !c.current_event_state().contains(gdk::ModifierType::CONTROL_MASK) {
                return glib::Propagation::Proceed;
            }
            if let (Some(r), Some(v)) = (wr.upgrade(), wv.upgrade()) {
                r.zoom_step(&v, if dy < 0.0 { 1 } else { -1 });
            }
            glib::Propagation::Stop
        });
        v.desk.add_controller(wheel);
        // the panel's own question field
        let (wr, wv) = (Rc::downgrade(self), Rc::downgrade(v));
        v.panel_ask.connect_activate(move |e| {
            let t = e.text().trim().to_string();
            if let (Some(r), Some(v), false) = (wr.upgrade(), wv.upgrade(), t.is_empty()) {
                e.set_text("");
                r.question(&v, t);
            }
        });
        let (wr, wv) = (Rc::downgrade(self), Rc::downgrade(v));
        v.panel_ask.connect_icon_release(move |e, pos| {
            let t = e.text().trim().to_string();
            if let (Some(r), Some(v), gtk::EntryIconPosition::Secondary, false) = (wr.upgrade(), wv.upgrade(), pos, t.is_empty()) {
                e.set_text("");
                r.question(&v, t);
            }
        });
    }

    // ------------------------------------------------------------------------------------------- the pages

    fn set_zoom(&self, v: &Rc<DocView>, z: f64) {
        let keep = v.current.get();
        v.zoom.set(z);
        v.cache.borrow_mut().clear();
        for (i, da) in v.pages.iter().enumerate() {
            let (w, h) = v.sizes[i];
            da.set_content_width((w * z).round() as i32);
            da.set_content_height((h * z).round() as i32);
            da.queue_draw();
        }
        v.zoom_label.set_text(&format!("{}%", (z * 100.0).round()));
        // (the same page stays in view)
        let (wr, vv) = (v.clone(), keep);
        glib::idle_add_local_once(move || {
            if let Some(r) = READER.with(|r| r.borrow().clone()) {
                r.go_page(&wr, vv);
            }
        });
        if debug() {
            eprintln!("debug documentos: zoom {}%", (z * 100.0).round());
        }
    }

    fn zoom_step(&self, v: &Rc<DocView>, d: i32) {
        let z = v.zoom.get();
        let i = ZOOMS.iter().position(|x| (x - z).abs() < 0.001).map(|i| i as i32).unwrap_or(4);
        let j = (i + d).clamp(0, ZOOMS.len() as i32 - 1) as usize;
        self.set_zoom(v, ZOOMS[j]);
    }

    fn page_top(&self, v: &DocView, i: usize) -> Option<f64> {
        let p = v.pages.get(i)?.compute_point(&v.pages_box, &gtk::graphene::Point::new(0.0, 0.0))?;
        Some(f64::from(p.y()) + f64::from(v.pages_box.margin_top()))
    }

    fn go_page(&self, v: &Rc<DocView>, i: usize) {
        let i = i.min(v.pages.len().saturating_sub(1));
        if let Some(y) = self.page_top(v, i) {
            v.desk.vadjustment().set_value((y - 16.0).max(0.0));
        }
        v.current.set(i);
        self.mark_page(v);
    }

    fn track_page(&self, v: &Rc<DocView>) {
        let adj = v.desk.vadjustment();
        let mid = adj.value() + adj.page_size() / 3.0;
        let mut cur = 0;
        for i in 0..v.pages.len() {
            if self.page_top(v, i).is_some_and(|y| y <= mid) {
                cur = i;
            }
        }
        if cur != v.current.get() {
            v.current.set(cur);
            self.mark_page(v);
        }
    }

    fn mark_page(&self, v: &DocView) {
        let cur = v.current.get();
        v.page_label.set_text(&format!("{} / {}", cur + 1, v.pages.len()));
        for (i, item) in v.thumb_items.iter().enumerate() {
            if i == cur {
                item.add_css_class("atual");
            } else {
                item.remove_css_class("atual");
            }
        }
    }

    fn redraw(&self, v: &DocView) {
        for da in &v.pages {
            da.queue_draw();
        }
    }

    fn run_find(&self, v: &Rc<DocView>) {
        let q = v.find.text().trim().to_string();
        let mut hits = Vec::new();
        if q.chars().count() >= 2 {
            for i in 0..v.pages.len() {
                if let Some(p) = v.doc.page(i as i32) {
                    hits.extend(find_on(&p, &q).into_iter().map(|r| (i, r)));
                }
            }
        }
        v.find_count.set_text(&if q.is_empty() { String::new() } else { trf("{n} found", &[("n", hits.len().to_string())]) });
        if debug() && !q.is_empty() {
            eprintln!("debug documentos: busca «{q}» ({} lugares)", hits.len());
        }
        *v.hits.borrow_mut() = hits;
        v.hit.set(usize::MAX);
        self.redraw(v);
        self.next_hit(v);
    }

    fn next_hit(&self, v: &Rc<DocView>) {
        let hits = v.hits.borrow().clone();
        if hits.is_empty() {
            return;
        }
        let k = v.hit.get().wrapping_add(1) % hits.len();
        v.hit.set(k);
        let (i, r) = &hits[k];
        self.scroll_to_rect(v, *i, r);
        v.find_count.set_text(&trf("{k} of {n}", &[("k", (k + 1).to_string()), ("n", hits.len().to_string())]));
        self.redraw(v);
    }

    fn scroll_to_rect(&self, v: &Rc<DocView>, i: usize, r: &poppler::Rectangle) {
        let (_, ph) = v.sizes[i];
        let z = v.zoom.get();
        if let Some(top) = self.page_top(v, i) {
            let y = top + (ph - r.y2()) * z;
            let adj = v.desk.vadjustment();
            adj.set_value((y - adj.page_size() / 3.0).max(0.0));
        }
        v.current.set(i);
        self.mark_page(v);
    }

    /// Goes to a page and lights the quoted words in gold for 1.2 s (the agent's citations).
    fn cite(&self, v: &Rc<DocView>, page: usize, quote: &str) {
        let i = page.saturating_sub(1).min(v.pages.len().saturating_sub(1));
        let mut rects = Vec::new();
        if let Some(p) = v.doc.page(i as i32) {
            for q in quote_tries(quote) {
                rects = find_on(&p, &q);
                if !rects.is_empty() {
                    break;
                }
            }
            if rects.is_empty() {
                if let Some(line) = v.texts.get(i).and_then(|t| closest_line(t, quote)) {
                    rects = find_on(&p, &line);
                }
            }
        }
        if debug() {
            eprintln!("debug documentos: citação p. {page} ({} marcas)", rects.len());
        }
        match rects.first() {
            Some(r) => self.scroll_to_rect(v, i, r),
            None => self.go_page(v, i),
        }
        *v.light.borrow_mut() = Some((i, rects, glib::monotonic_time()));
        let da = v.pages[i].clone();
        let wv = Rc::downgrade(v);
        da.add_tick_callback(move |da, _| {
            da.queue_draw();
            let Some(v) = wv.upgrade() else { return glib::ControlFlow::Break };
            let done = v.light.borrow().as_ref().is_none_or(|(_, _, t0)| glib::monotonic_time() - t0 > HIGHLIGHT_US);
            if done {
                v.light.borrow_mut().take();
                da.queue_draw();
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
    }

    fn print(self: &Rc<Self>, v: &Rc<DocView>) {
        if self.printing.replace(true) {
            *self.next_print.borrow_mut() = Some(Rc::downgrade(v));
            if debug() {
                eprintln!("debug documentos: impressão na fila (outra em andamento)");
            }
            return;
        }
        let op = gtk::PrintOperation::new();
        op.set_n_pages(v.pages.len() as i32);
        op.set_job_name(&v.tab_label.text());
        let wv = Rc::downgrade(v);
        op.connect_draw_page(move |_, ctx, n| {
            let Some(v) = wv.upgrade() else { return };
            let Some(p) = v.doc.page(n) else { return };
            let (w, h) = p.size();
            let cr = ctx.cairo_context();
            let k = (ctx.width() / w).min(ctx.height() / h);
            cr.scale(k, k);
            p.render_for_printing(&cr);
        });
        // (asynchronous: a synchronous run spins a main loop inside this one until the portal answers — two prints in a
        // row left the window unable to close)
        op.set_allow_async(true);
        let w = Rc::downgrade(self);
        op.connect_done(move |_, r| {
            if debug() {
                eprintln!("debug documentos: impressão terminada ({r:?})");
            }
            let Some(me) = w.upgrade() else { return };
            me.printing.set(false);
            let next = me.next_print.borrow_mut().take().and_then(|v| v.upgrade());
            if let Some(v) = next {
                me.print(&v);
            }
        });
        if let Err(e) = op.run(gtk::PrintOperationAction::PrintDialog, Some(&self.window)) {
            eprintln!("kalango-shell: documentos: imprimir: {e}");
            self.printing.set(false);
        }
    }

    // ------------------------------------------------------------------------------------------- the agent

    fn pages_json(v: &DocView) -> Vec<serde_json::Value> {
        v.texts.iter().enumerate().take(80).map(|(i, t)| serde_json::json!({"n": i + 1, "texto": t})).collect()
    }

    /// The summary, the places worth finding and two questions (kept for the next time it opens).
    fn summarize(self: &Rc<Self>, v: &Rc<DocView>) {
        if let Some(out) = std::fs::read_to_string(cache_file(&v.path)).ok().and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok()) {
            self.show_summary(v, &out, true);
            return;
        }
        if v.texts.iter().all(|t| t.trim().is_empty()) {
            Self::clear(&v.summary);
            v.summary.append(&rotulo(tr("This document has no text to read (only pictures)."), "doc-fraco"));
            return;
        }
        let input = serde_json::json!({"nome": v.tab_label.text().to_string(), "paginas": Self::pages_json(v)});
        let (wr, wv) = (Rc::downgrade(self), Rc::downgrade(v));
        crate::lateral::em_segundo_plano(move || ask_agent("documento-resumo", &input), move |r| {
            let (Some(me), Some(v)) = (wr.upgrade(), wv.upgrade()) else { return };
            match r {
                Ok(out) => {
                    if let Some(d) = cache_file(&v.path).parent() {
                        let _ = std::fs::create_dir_all(d);
                    }
                    let _ = std::fs::write(cache_file(&v.path), out.to_string());
                    me.show_summary(&v, &out, false);
                }
                Err(e) => {
                    Self::clear(&v.summary);
                    let l = rotulo(&trf("The summary could not be made: {error}", &[("error", e)]), "doc-fraco");
                    l.set_wrap(true);
                    l.set_max_width_chars(1);
                    v.summary.append(&l);
                }
            }
        });
    }

    fn clear(b: &gtk::Box) {
        while let Some(c) = b.first_child() {
            b.remove(&c);
        }
    }

    fn page_chip(page: u64) -> gtk::Label {
        let l = rotulo(&format!("p. {page}"), "doc-pagina");
        l.set_valign(Align::Center);
        l
    }

    fn show_summary(self: &Rc<Self>, v: &Rc<DocView>, out: &serde_json::Value, cached: bool) {
        Self::clear(&v.summary);
        Self::clear(&v.places);
        Self::clear(&v.questions);
        let points: Vec<String> = out["resumo"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_string)).collect();
        for p in &points {
            let r = caixa(Orientation::Horizontal, 8, "");
            let dot = caixa(Orientation::Vertical, 0, "doc-ponto");
            dot.set_valign(Align::Start);
            dot.set_margin_top(6);
            r.append(&dot);
            let l = rotulo(p, "doc-resumo");
            l.set_wrap(true);
            l.set_xalign(0.0);
            l.set_hexpand(true);
            l.set_max_width_chars(1);
            r.append(&l);
            v.summary.append(&r);
        }
        let places: Vec<(String, u64, String)> = out["trechos"].as_array().into_iter().flatten()
            .map(|t| (t["titulo"].as_str().unwrap_or("").to_string(), t["pagina"].as_u64().unwrap_or(1), t["citacao"].as_str().unwrap_or("").to_string())).collect();
        for (k, (title, page, quote)) in places.iter().enumerate() {
            let b = gtk::Button::new();
            b.add_css_class("doc-trecho");
            let r = caixa(Orientation::Horizontal, 8, "");
            let l = rotulo(title, "");
            l.set_xalign(0.0);
            l.set_hexpand(true);
            l.set_ellipsize(gtk::pango::EllipsizeMode::End);
            l.set_max_width_chars(1);
            r.append(&l);
            r.append(&Self::page_chip(*page));
            b.set_child(Some(&r));
            crate::central::posicao_debug_sempre(&format!("documentos trecho {}", k + 1), b.upcast_ref());
            let (wr, wv, page, quote) = (Rc::downgrade(self), Rc::downgrade(v), *page as usize, quote.clone());
            b.connect_clicked(move |_| {
                if let (Some(r), Some(v)) = (wr.upgrade(), wv.upgrade()) {
                    r.cite(&v, page, &quote);
                }
            });
            v.places.append(&b);
        }
        for (k, q) in out["perguntas"].as_array().into_iter().flatten().filter_map(|x| x.as_str()).enumerate() {
            let b = gtk::Button::new();
            b.add_css_class("doc-sugestao");
            b.set_halign(Align::Start);
            let r = caixa(Orientation::Horizontal, 8, "");
            r.append(&ui_icon("spark", "#A5A1E2", 12));
            let l = rotulo(q, "");
            l.set_wrap(true);
            l.set_xalign(0.0);
            // (as wide as the words, up to the panel's width)
            l.set_max_width_chars(44);
            r.append(&l);
            b.set_child(Some(&r));
            crate::central::posicao_debug_sempre(&format!("documentos sugestao {}", k + 1), b.upcast_ref());
            let (wr, wv, q) = (Rc::downgrade(self), Rc::downgrade(v), q.to_string());
            b.connect_clicked(move |_| {
                if let (Some(r), Some(v)) = (wr.upgrade(), wv.upgrade()) {
                    r.question(&v, q.clone());
                }
            });
            v.questions.append(&b);
        }
        if debug() {
            eprintln!("debug documentos: resumo{} ({} pontos, {} trechos)", if cached { " guardado" } else { "" }, points.len(), places.len());
        }
    }

    /// A question about the document: the answer under the panel, with its page (a click lights the words).
    fn question(self: &Rc<Self>, v: &Rc<DocView>, q: String) {
        let block = caixa(Orientation::Vertical, 6, "doc-resposta");
        let ql = rotulo(&q, "doc-pergunta");
        ql.set_wrap(true);
        ql.set_xalign(0.0);
        ql.set_max_width_chars(1);
        block.append(&ql);
        let al = rotulo(tr("The local model is reading it…"), "doc-fraco");
        al.set_wrap(true);
        al.set_xalign(0.0);
        al.set_max_width_chars(1);
        block.append(&al);
        v.answers.append(&block);
        let input = serde_json::json!({"nome": v.tab_label.text().to_string(), "pergunta": q, "paginas": Self::pages_json(v)});
        let (wr, wv) = (Rc::downgrade(self), Rc::downgrade(v));
        crate::lateral::em_segundo_plano(move || ask_agent("documento-pergunta", &input), move |r| {
            let (Some(me), Some(v)) = (wr.upgrade(), wv.upgrade()) else { return };
            match r {
                Ok(out) => {
                    let text = out["resposta"].as_str().unwrap_or("").trim().to_string();
                    let page = out["pagina"].as_u64().unwrap_or(0);
                    let quote = out["citacao"].as_str().unwrap_or("").to_string();
                    al.set_text(&text);
                    al.remove_css_class("doc-fraco");
                    al.add_css_class("doc-resumo");
                    if page >= 1 && page as usize <= v.pages.len() {
                        let b = gtk::Button::new();
                        b.add_css_class("doc-citacao");
                        b.set_halign(Align::Start);
                        b.set_child(Some(&Self::page_chip(page)));
                        b.set_tooltip_text(Some(&quote));
                        crate::central::posicao_debug_sempre("documentos citacao", b.upcast_ref());
                        let (wr2, wv2) = (Rc::downgrade(&me), Rc::downgrade(&v));
                        b.connect_clicked(move |_| {
                            if let (Some(r), Some(v)) = (wr2.upgrade(), wv2.upgrade()) {
                                r.cite(&v, page as usize, &quote);
                            }
                        });
                        block.append(&b);
                    }
                    // (the answer in view: the panel scrolls down to it)
                    let sc = v.panel_scroll.clone();
                    glib::timeout_add_local_once(std::time::Duration::from_millis(120), move || {
                        let a = sc.vadjustment();
                        a.set_value(a.upper() - a.page_size());
                    });
                    if debug() {
                        eprintln!("debug documentos: resposta p. {page} ({} caracteres)", text.chars().count());
                    }
                }
                Err(e) => al.set_text(&trf("The agent could not answer: {error}", &[("error", e)])),
            }
        });
    }
}

fn ask_agent(cmd: &str, input: &serde_json::Value) -> Result<serde_json::Value, String> {
    use std::io::Write;
    let mut c = std::process::Command::new(crate::voz::agent_binary()).arg(cmd).stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().map_err(|e| e.to_string())?;
    c.stdin.take().ok_or("stdin")?.write_all(input.to_string().as_bytes()).map_err(|e| e.to_string())?;
    let o = c.wait_with_output().map_err(|e| e.to_string())?;
    serde_json::from_slice::<serde_json::Value>(&o.stdout).map_err(|_| String::from_utf8_lossy(&o.stderr).trim().lines().last().unwrap_or("").to_string())
}

/// A page on the desk: white paper, the page rendered once at this zoom (at the screen's scale), the find's places in
/// lavender and the cited words in gold, fading.
fn draw_page(v: &DocView, i: usize, da: &gtk::DrawingArea, cr: &cairo::Context, w: f64, h: f64) {
    cr.set_source_rgb(0.957, 0.957, 0.969);
    rounded(cr, 0.0, 0.0, w, h, 2.0);
    let _ = cr.fill();
    let z = v.zoom.get();
    let scale = da.scale_factor().max(1);
    let key = (i, (z * 100.0).round() as i32 * 10 + scale);
    if !v.cache.borrow().contains_key(&key) {
        let Some(p) = v.doc.page(i as i32) else { return };
        let (pw, ph) = ((w * f64::from(scale)).round() as i32, (h * f64::from(scale)).round() as i32);
        let Ok(s) = cairo::ImageSurface::create(cairo::Format::ARgb32, pw.max(1), ph.max(1)) else { return };
        if let Ok(c) = cairo::Context::new(&s) {
            c.set_source_rgb(0.957, 0.957, 0.969);
            let _ = c.paint();
            c.scale(z * f64::from(scale), z * f64::from(scale));
            p.render(&c);
        }
        let mut cache = v.cache.borrow_mut();
        // (a long document: only the pages near are kept)
        if cache.len() > 24 {
            let cur = v.current.get();
            cache.retain(|(k, _), _| k.abs_diff(cur) < 6);
        }
        cache.insert(key, s);
    }
    if let Some(s) = v.cache.borrow().get(&key) {
        cr.save().ok();
        rounded(cr, 0.0, 0.0, w, h, 2.0);
        cr.clip();
        cr.scale(1.0 / f64::from(scale), 1.0 / f64::from(scale));
        let _ = cr.set_source_surface(s, 0.0, 0.0);
        let _ = cr.paint();
        cr.restore().ok();
    }
    let (_, ph) = v.sizes[i];
    let rect = |cr: &cairo::Context, r: &poppler::Rectangle| {
        cr.rectangle(r.x1() * z - 2.0, (ph - r.y2()) * z - 1.0, (r.x2() - r.x1()) * z + 4.0, (r.y2() - r.y1()) * z + 2.0);
    };
    let hits = v.hits.borrow();
    let cur = v.hit.get();
    for (k, (pi, r)) in hits.iter().enumerate() {
        if *pi == i {
            let (a, b, c) = crate::tema::rgb("#A5A1E2");
            cr.set_source_rgba(a, b, c, if k == cur { 0.55 } else { 0.3 });
            rect(cr, r);
            let _ = cr.fill();
        }
    }
    if let Some((pi, rects, t0)) = v.light.borrow().as_ref() {
        if *pi == i {
            let x = ((glib::monotonic_time() - t0) as f64 / HIGHLIGHT_US as f64).clamp(0.0, 1.0);
            // (full for most of its time, then it fades)
            let a = if x < 0.7 { 0.55 } else { 0.55 * (1.0 - (x - 0.7) / 0.3) };
            cr.set_source_rgba(0xE5 as f64 / 255.0, 0xC9 as f64 / 255.0, 0x8C as f64 / 255.0, a);
            for r in rects {
                rect(cr, r);
                let _ = cr.fill();
            }
        }
    }
}

fn draw_thumb(v: &DocView, i: usize, t: &gtk::DrawingArea, cr: &cairo::Context, w: f64, h: f64) {
    let scale = t.scale_factor().max(1);
    if !v.thumb_cache.borrow().contains_key(&i) {
        let Some(p) = v.doc.page(i as i32) else { return };
        let (pw, _) = v.sizes[i];
        let k = w / pw.max(1.0) * f64::from(scale);
        let Ok(s) = cairo::ImageSurface::create(cairo::Format::ARgb32, (w * f64::from(scale)) as i32, (h * f64::from(scale)) as i32) else { return };
        if let Ok(c) = cairo::Context::new(&s) {
            c.set_source_rgb(0.957, 0.957, 0.969);
            let _ = c.paint();
            c.scale(k, k);
            p.render(&c);
        }
        v.thumb_cache.borrow_mut().insert(i, s);
    }
    if let Some(s) = v.thumb_cache.borrow().get(&i) {
        rounded(cr, 0.0, 0.0, w, h, 2.0);
        cr.clip();
        cr.scale(1.0 / f64::from(scale), 1.0 / f64::from(scale));
        let _ = cr.set_source_surface(s, 0.0, 0.0);
        let _ = cr.paint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_are_tried_shorter() {
        assert_eq!(quote_tries("a receita subiu 8% sobre setembro deste ano"),
            vec!["a receita subiu 8% sobre setembro deste ano", "a receita subiu 8% sobre setembro", "a receita subiu 8%", "a receita subiu"]);
        assert!(is_document(Path::new("/x/Relatório.PDF")));
        assert_eq!(plain("A próxima revisão está"), "A proxima revisao esta");
        let page = "Pendências\nHá três pendências: a fatura em duplicidade.\nA próxima revisão está marcada para 20 de novembro.";
        assert_eq!(closest_line(page, "a revisão seguinte foi marcada para novembro").as_deref(), Some("A próxima revisão está marcada para"));
        assert!(!is_document(Path::new("/x/nota.txt")));
    }
}
