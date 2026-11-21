//! The canvas with live HTML (design "Chat · canvas com HTML ao vivo", 75452): a page the agent wrote in the
//! conversation, drawn as it is in the conversation's side bar. The file and the style sheets it links; Prévia,
//! Código or Dividido; the width of a computer, a tablet or a phone; "Abrir no navegador" (the person's own). A click on
//! an element outlines it, names it (`a.cta`) and opens a field next to it: the request goes to the agent with the
//! element. Its versions (each time the file changes while it is open): one is shown with a click, "Desfazer" puts the
//! one before back in the file, "Baixar .zip" puts the page and its styles in Downloads.
//!
//! The page is drawn by kalangod's Moli, the canvas's own session (`kalango/navegador`, sessao "canvas"): the HTML as
//! a data: address — none of the person's files is reachable from it, its requests go through the proxy that refuses
//! local addresses —, never by this process. Only its picture comes here.

use crate::conversa::ui_icon;
use crate::superficies::{caixa, rotulo};
use gtk::prelude::*;
use gtk::{gdk, gio, glib, Align, Orientation};
use kalango_i18n::{tr, trf};
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

/// The widths of Computador, Tablet and Celular (the page's px), and how much of the page is shown (the design's
/// preview is wider than tall for a computer).
const WIDTHS: [u32; 3] = [1100, 768, 390];
const HEIGHTS: [u32; 3] = [500, 640, 760];

/// The page's height in the browser for a preview `area` (w, h) px: as tall as the area's shape asks at the device's
/// `width` — the canvas fills its panel however wide or tall the panel is (design "Central · canvas largo") — never
/// under the device's `least` nor over three widths.
fn viewport_height(width: u32, area: (i32, i32), least: u32) -> u32 {
    if area.0 <= 0 || area.1 <= 0 {
        return least;
    }
    let h = (f64::from(width) * f64::from(area.1) / f64::from(area.0)).round() as u32;
    h.clamp(least, width * 3)
}

/// The style sheets a page links with a relative address (`<link rel="stylesheet" href="estilo.css">`).
pub fn linked_styles(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    let low = html.to_lowercase();
    let mut from = 0;
    while let Some(i) = low[from..].find("<link") {
        let start = from + i;
        let end = low[start..].find('>').map(|e| start + e).unwrap_or(low.len());
        let tag = &html[start..end];
        let tl = &low[start..end];
        if tl.contains("stylesheet") {
            if let Some(h) = tl.find("href=") {
                let rest = &tag[h + 5..];
                let q = rest.chars().next().unwrap_or('"');
                let val: String = if q == '"' || q == '\'' { rest[1..].chars().take_while(|c| *c != q).collect() } else { rest.chars().take_while(|c| !c.is_whitespace()).collect() };
                if !val.is_empty() && !val.contains("://") && !val.starts_with('/') && !val.contains("..") && !out.contains(&val) {
                    out.push(val);
                }
            }
        }
        from = end;
    }
    out
}

/// The page as one document: its linked local style sheets put inside it (a data: page cannot read files).
pub fn inline_styles(html: &str, dir: &Path) -> String {
    let mut s = html.to_string();
    for href in linked_styles(html) {
        let Ok(css) = std::fs::read_to_string(dir.join(&href)) else { continue };
        let style = format!("<style>/* {href} */\n{css}\n</style>");
        // (the link tag goes, the style takes its place)
        let low = s.to_lowercase();
        if let Some(i) = low.find(&href.to_lowercase()) {
            let start = low[..i].rfind("<link").unwrap_or(i);
            let end = low[i..].find('>').map(|e| i + e + 1).unwrap_or(s.len());
            s.replace_range(start..end, &style);
        }
    }
    s
}

/// The lines of the file where an element is (by its id, its classes or its tag): the first match and a few around.
pub fn lines_of(src: &str, selector: &str) -> (usize, usize) {
    let lines: Vec<&str> = src.lines().collect();
    if lines.is_empty() {
        return (1, 1);
    }
    let (tag, rest) = selector.split_once(['.', '#']).unwrap_or((selector, ""));
    let needle = if let Some(id) = selector.split('#').nth(1) {
        format!("id=\"{}\"", id.split('.').next().unwrap_or(id))
    } else if !rest.is_empty() {
        rest.split('.').next().unwrap_or("").to_string()
    } else {
        format!("<{tag}")
    };
    let hit = lines.iter().position(|l| l.contains(&needle)).or_else(|| lines.iter().position(|l| l.contains(&format!("<{tag}"))));
    match hit {
        Some(i) => (i.saturating_sub(2) + 1, (i + 4).min(lines.len())),
        None => (1, lines.len().min(6)),
    }
}

/// The page an answer holds: its last closed ```html block with markup in it (the model wrote the page in the
/// conversation instead of a file). The canvas shows it like a page the agent wrote to a file.
pub fn page_in_answer(md: &str) -> Option<String> {
    crate::fences::split(md).into_iter().rev().find_map(|p| match p {
        crate::fences::Piece::Code(f) if f.closed && matches!(crate::code_block::parse_info(&f.info).0.as_str(), "html" | "htm" | "xhtml") => {
            let low = f.body.to_lowercase();
            ["<html", "<body", "<!doctype", "<div", "<h1", "<p>", "<section", "<main"].iter().any(|t| low.contains(t)).then(|| f.body.clone())
        }
        _ => None,
    })
}

/// The lines the code under the page shows: beside the preview, the element picked (or the first 8 lines); in
/// Código and Dividir, the whole file. (first, last), from 1.
pub fn code_lines(total: usize, picked: Option<(usize, usize)>, preview: bool) -> (usize, usize) {
    match picked {
        Some(r) if preview => r,
        _ if !preview => (1, total.max(1)),
        _ => (1, total.clamp(1, 8)),
    }
}

fn clock_since(t: &glib::DateTime) -> String {
    let now = glib::DateTime::now_local().map(|n| n.to_unix()).unwrap_or(0);
    let m = (now - t.to_unix()) / 60;
    if m < 1 { tr("now").to_string() } else { trf("{n} min ago", &[("n", m.to_string())]) }
}

fn texture(b64: &str) -> Option<gdk::Texture> {
    let bytes = glib::base64_decode(b64);
    gdk::Texture::from_bytes(&glib::Bytes::from_owned(bytes)).ok()
}

struct Picked {
    selector: String,
    rect: (f64, f64, f64, f64),
}

pub struct HtmlCanvas {
    pub raiz: gtk::Box,
    files: gtk::Box,
    picture: gtk::Picture,
    marks: gtk::DrawingArea,
    ask_box: gtk::Box,
    ask: gtk::Entry,
    preview: gtk::Box,
    /// where the code block goes (built again for each change of lines)
    code: gtk::Box,
    versions: gtk::Box,
    modes: Vec<gtk::ToggleButton>,
    devices: Vec<gtk::ToggleButton>,
    status: gtk::Label,
    file: RefCell<Option<PathBuf>>,
    /// every version seen while open: when, and the file's text
    history: RefCell<Vec<(glib::DateTime, String)>>,
    shown: Cell<usize>,
    width: Cell<u32>,
    page: Cell<(f64, f64)>,
    picked: RefCell<Option<Picked>>,
    monitor: RefCell<Option<gio::FileMonitor>>,
    busy: Cell<bool>,
    /// the preview's size, and a counter that drops a redraw a newer resize replaced
    area: Cell<(i32, i32)>,
    resized: Cell<u32>,
    /// the page height last asked of the browser
    asked: Cell<u32>,
    /// a request about an element, to the agent of the conversation
    pub on_ask: RefCell<Option<Box<dyn Fn(String)>>>,
}

fn toggle(label: &str, class: &str) -> gtk::ToggleButton {
    let b = gtk::ToggleButton::with_label(label);
    b.add_css_class(class);
    b
}

fn icon_toggle(glyph: &str, tip: &str) -> gtk::ToggleButton {
    let b = gtk::ToggleButton::new();
    b.add_css_class("canvas-dispositivo");
    b.set_child(Some(&ui_icon(glyph, "#8E8F99", 16)));
    b.set_tooltip_text(Some(tip));
    b
}

impl HtmlCanvas {
    pub fn new() -> Rc<HtmlCanvas> {
        let raiz = caixa(Orientation::Vertical, 12, "canvas-html");
        raiz.set_margin_top(14);
        raiz.set_margin_end(4);
        // the files, the modes, the widths and the person's browser
        let top = caixa(Orientation::Horizontal, 8, "");
        let files = caixa(Orientation::Horizontal, 6, "");
        files.set_hexpand(true);
        top.append(&files);
        let seg = caixa(Orientation::Horizontal, 0, "canvas-segmento");
        let modes: Vec<gtk::ToggleButton> = [tr("Preview"), tr("Code"), tr("Split")].iter().map(|t| toggle(t, "canvas-modo")).collect();
        for b in &modes {
            if let Some(f) = modes.first().filter(|f| *f != b) {
                b.set_group(Some(f));
            }
            seg.append(b);
        }
        modes[0].set_active(true);
        top.append(&seg);
        let devs = caixa(Orientation::Horizontal, 2, "");
        let devices = vec![icon_toggle("monitor", tr("Computer")), icon_toggle("celular", tr("Tablet")), icon_toggle("celular-d", tr("Phone"))];
        for b in &devices {
            if let Some(f) = devices.first().filter(|f| *f != b) {
                b.set_group(Some(f));
            }
            devs.append(b);
        }
        devices[0].set_active(true);
        top.append(&devs);
        let abrir = gtk::Button::new();
        abrir.add_css_class("canvas-abrir");
        let ar = caixa(Orientation::Horizontal, 6, "");
        ar.append(&ui_icon("globo", "#E2E2E6", 15));
        ar.append(&rotulo(tr("Open in the browser"), ""));
        abrir.set_child(Some(&ar));
        top.append(&abrir);
        raiz.append(&top);
        // the page, with the element picked outlined and the field next to it
        let preview = caixa(Orientation::Vertical, 0, "canvas-previa");
        let over = gtk::Overlay::new();
        let picture = gtk::Picture::new();
        picture.set_content_fit(gtk::ContentFit::Contain);
        picture.set_can_shrink(true);
        picture.set_vexpand(true);
        picture.set_valign(Align::Start);
        over.set_child(Some(&picture));
        let marks = gtk::DrawingArea::new();
        marks.set_can_target(false);
        over.add_overlay(&marks);
        let ask_box = caixa(Orientation::Horizontal, 8, "canvas-pedir");
        ask_box.set_halign(Align::Start);
        ask_box.set_valign(Align::Start);
        ask_box.append(&crate::superficies::estrela(13, "#A5A1E2"));
        let ask = gtk::Entry::new();
        ask.add_css_class("canvas-pedir-campo");
        ask.set_hexpand(true);
        ask.set_placeholder_text(Some(tr("ask for a change in this element")));
        ask_box.append(&ask);
        let send = gtk::Button::new();
        send.add_css_class("canvas-pedir-enviar");
        send.set_child(Some(&ui_icon("enviar", "#1C1D24", 14)));
        ask_box.append(&send);
        ask_box.set_size_request(380, -1);
        ask_box.set_visible(false);
        over.add_overlay(&ask_box);
        preview.append(&over);
        raiz.append(&preview);
        // the code: the lines of the element picked (or the whole file), as the conversation's code blocks show code
        let code = caixa(Orientation::Vertical, 0, "");
        raiz.append(&code);
        // the versions, Desfazer and the .zip
        let bottom = caixa(Orientation::Horizontal, 6, "");
        bottom.append(&ui_icon("relogio", "#8E8F99", 15));
        let versions = caixa(Orientation::Horizontal, 6, "");
        versions.set_hexpand(true);
        bottom.append(&versions);
        let desfazer = gtk::Button::new();
        desfazer.add_css_class("canvas-acao-texto");
        let dr = caixa(Orientation::Horizontal, 6, "");
        dr.append(&ui_icon("desfazer", "#8E8F99", 14));
        dr.append(&rotulo(tr("Undo"), ""));
        desfazer.set_child(Some(&dr));
        bottom.append(&desfazer);
        let zip = gtk::Button::new();
        zip.add_css_class("canvas-acao");
        let zr = caixa(Orientation::Horizontal, 6, "");
        zr.append(&ui_icon("baixar", "#E2E2E6", 14));
        zr.append(&rotulo(tr("Download .zip"), ""));
        zip.set_child(Some(&zr));
        bottom.append(&zip);
        raiz.append(&bottom);
        let status = rotulo("", "central-agente-sub");
        status.set_xalign(0.0);
        status.set_visible(false);
        raiz.append(&status);
        let c = Rc::new(HtmlCanvas {
            raiz, files, picture, marks, ask_box, ask, preview, code, versions, modes, devices, status,
            file: RefCell::default(), history: RefCell::default(), shown: Cell::new(0), width: Cell::new(WIDTHS[0]), page: Cell::new((1100.0, 500.0)),
            picked: RefCell::default(), monitor: RefCell::default(), busy: Cell::new(false), area: Cell::new((0, 0)), resized: Cell::new(0), asked: Cell::new(0), on_ask: RefCell::default(),
        });
        // the panel resized (the window, its edge, Expand): the page is drawn again at the new shape, once the size
        // settles
        let w = Rc::downgrade(&c);
        c.marks.connect_resize(move |_, aw, ah| {
            let Some(c) = w.upgrade() else { return };
            c.area.set((aw, ah));
            if c.file().is_none() || !c.reshaped() {
                return;
            }
            let n = c.resized.get().wrapping_add(1);
            c.resized.set(n);
            let w = Rc::downgrade(&c);
            glib::timeout_add_local_once(std::time::Duration::from_millis(350), move || {
                if let Some(c) = w.upgrade().filter(|c| c.resized.get() == n) {
                    c.draw();
                }
            });
        });
        // the outline of the element picked, and its name
        let w = Rc::downgrade(&c);
        c.marks.set_draw_func(move |_, cr, aw, ah| {
            let Some(c) = w.upgrade() else { return };
            let Some(p) = c.picked.borrow().as_ref().map(|p| (p.rect, p.selector.clone())) else { return };
            let Some((ox, oy, s)) = c.fit(f64::from(aw), f64::from(ah)) else { return };
            let (x, y, ww, hh) = p.0;
            let (a, b, d) = crate::tema::rgb("#A5A1E2");
            cr.set_source_rgb(a, b, d);
            cr.set_line_width(2.0);
            cr.rectangle(ox + x * s - 6.0, oy + y * s - 6.0, ww * s + 12.0, hh * s + 12.0);
            let _ = cr.stroke();
            // the name's label above it
            let layout = pangocairo::functions::create_layout(cr);
            layout.set_text(&p.1);
            layout.set_font_description(Some(&gtk::pango::FontDescription::from_string("JetBrains Mono 9")));
            let (tw, th) = layout.pixel_size();
            let (lx, ly) = (ox + x * s - 6.0, (oy + y * s - 6.0 - f64::from(th) - 4.0).max(0.0));
            cr.rectangle(lx, ly, f64::from(tw) + 8.0, f64::from(th) + 4.0);
            let _ = cr.fill();
            cr.set_source_rgb(0.11, 0.11, 0.14);
            cr.move_to(lx + 4.0, ly + 2.0);
            pangocairo::functions::show_layout(cr, &layout);
        });
        // a click on the page picks the element there
        let click = gtk::GestureClick::new();
        let w = Rc::downgrade(&c);
        click.connect_released(move |_, _, x, y| {
            let Some(c) = w.upgrade() else { return };
            let (aw, ah) = (f64::from(c.picture.width()), f64::from(c.picture.height()));
            let Some((ox, oy, s)) = c.fit(aw, ah) else { return };
            let (px, py) = ((x - ox) / s, (y - oy) / s);
            let (pw, ph) = c.page.get();
            if px < 0.0 || py < 0.0 || px > pw || py > ph {
                return;
            }
            c.pick(px, py);
        });
        c.picture.add_controller(click);
        // the request about it
        let w = Rc::downgrade(&c);
        let send_it = move || {
            let Some(c) = w.upgrade() else { return };
            let t = c.ask.text().trim().to_string();
            let Some((sel, file)) = c.picked.borrow().as_ref().map(|p| p.selector.clone()).zip(c.file.borrow().clone()) else { return };
            if t.is_empty() {
                return;
            }
            c.ask.set_text("");
            c.ask_box.set_visible(false);
            let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let msg = trf("In {file}, the element {element}: {request}. Change the file.", &[("file", name), ("element", format!("`{sel}`")), ("request", t)]);
            if debug() {
                eprintln!("debug canvas: pedido «{msg}»");
            }
            let f = c.on_ask.borrow();
            if let Some(f) = f.as_ref() {
                f(msg);
            }
        };
        let s2 = Rc::new(send_it);
        let s3 = s2.clone();
        c.ask.connect_activate(move |_| s3());
        send.connect_clicked(move |_| s2());
        // modes and widths
        for (i, b) in c.modes.iter().enumerate() {
            let w = Rc::downgrade(&c);
            b.connect_toggled(move |b| {
                if let Some(c) = w.upgrade().filter(|_| b.is_active()) {
                    c.preview.set_visible(i != 1);
                    c.show_code();
                    if debug() {
                        eprintln!("debug canvas: modo {}", ["prévia", "código", "dividido"][i]);
                    }
                }
            });
        }
        for (i, b) in c.devices.iter().enumerate() {
            let w = Rc::downgrade(&c);
            b.connect_toggled(move |b| {
                if let Some(c) = w.upgrade().filter(|_| b.is_active()) {
                    c.width.set(WIDTHS[i]);
                    c.picked.borrow_mut().take();
                    c.ask_box.set_visible(false);
                    c.draw();
                }
            });
        }
        let w = Rc::downgrade(&c);
        abrir.connect_clicked(move |b| {
            let Some(f) = w.upgrade().and_then(|c| c.file.borrow().clone()) else { return };
            let uri = gio::File::for_path(&f).uri();
            let _ = gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>);
            if debug() {
                eprintln!("debug canvas: abrir no navegador {uri}");
            }
            let _ = b;
        });
        let w = Rc::downgrade(&c);
        desfazer.connect_clicked(move |_| {
            if let Some(c) = w.upgrade() {
                c.undo();
            }
        });
        let w = Rc::downgrade(&c);
        zip.connect_clicked(move |_| {
            if let Some(c) = w.upgrade() {
                c.zip();
            }
        });
        crate::central::posicao_debug("canvas previa", c.picture.upcast_ref());
        crate::central::posicao_debug("canvas desfazer", desfazer.upcast_ref());
        crate::central::posicao_debug("canvas zip", zip.upcast_ref());
        crate::central::posicao_debug("canvas codigo", c.modes[1].upcast_ref());
        c
    }

    /// Where the page's picture is inside the widget, and its scale.
    fn fit(&self, aw: f64, ah: f64) -> Option<(f64, f64, f64)> {
        let (pw, ph) = self.page.get();
        if pw <= 0.0 || aw <= 0.0 {
            return None;
        }
        let s = (aw / pw).min(ah / ph);
        Some(((aw - pw * s) / 2.0, 0.0, s))
    }

    /// Another conversation, with no page: the canvas forgets this one.
    pub fn clear(&self) {
        self.file.borrow_mut().take();
        self.monitor.borrow_mut().take();
        self.history.borrow_mut().clear();
        self.picked.borrow_mut().take();
        self.ask_box.set_visible(false);
        self.picture.set_paintable(None::<&gdk::Paintable>);
    }

    pub fn file(&self) -> Option<PathBuf> {
        self.file.borrow().clone()
    }

    /// Shows a page the agent wrote (again: its newest version).
    pub fn open(self: &Rc<Self>, path: &Path) {
        let same = self.file.borrow().as_deref() == Some(path);
        if !same {
            *self.file.borrow_mut() = Some(path.to_path_buf());
            self.history.borrow_mut().clear();
            self.picked.borrow_mut().take();
            self.ask_box.set_visible(false);
            // a change to the file while open is a new version
            let m = gio::File::for_path(path).monitor_file(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE).ok();
            if let Some(m) = &m {
                let w = Rc::downgrade(self);
                m.connect_changed(move |_, _, _, ev| {
                    if matches!(ev, gio::FileMonitorEvent::ChangesDoneHint | gio::FileMonitorEvent::Created) {
                        if let Some(c) = w.upgrade() {
                            c.reload();
                        }
                    }
                });
            }
            *self.monitor.borrow_mut() = m;
        }
        self.reload();
    }

    fn reload(self: &Rc<Self>) {
        let Some(path) = self.file() else { return };
        let Ok(src) = std::fs::read_to_string(&path) else { return };
        let new = self.history.borrow().last().map(|(_, s)| s != &src).unwrap_or(true);
        if new {
            self.history.borrow_mut().push((glib::DateTime::now_local().unwrap_or_else(|_| glib::DateTime::from_unix_utc(0).unwrap()), src));
            if debug() {
                eprintln!("debug canvas: {} versão {}", path.display(), self.history.borrow().len());
            }
        }
        self.shown.set(self.history.borrow().len().saturating_sub(1));
        self.fill_files();
        self.fill_versions();
        self.show_code();
        self.draw();
    }

    fn source(&self) -> String {
        self.history.borrow().get(self.shown.get()).map(|(_, s)| s.clone()).unwrap_or_default()
    }

    fn fill_files(&self) {
        while let Some(x) = self.files.first_child() {
            self.files.remove(&x);
        }
        let Some(path) = self.file() else { return };
        let mut names = vec![(path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), true)];
        names.extend(linked_styles(&self.source()).into_iter().map(|n| (n, false)));
        for (n, main) in names {
            let chip = caixa(Orientation::Horizontal, 6, "canvas-arquivo");
            if main {
                chip.add_css_class("ativo");
            }
            chip.append(&ui_icon("codigo", if main { "#1C1D24" } else { "#8E8F99" }, 14));
            let l = rotulo(&n, "");
            l.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            l.set_max_width_chars(18);
            chip.append(&l);
            self.files.append(&chip);
        }
    }

    fn fill_versions(self: &Rc<Self>) {
        while let Some(x) = self.versions.first_child() {
            self.versions.remove(&x);
        }
        let h = self.history.borrow();
        for (i, (t, _)) in h.iter().enumerate().rev().take(5) {
            let b = gtk::Button::new();
            b.add_css_class("canvas-versao");
            if i == self.shown.get() {
                b.add_css_class("ativo");
            }
            b.set_child(Some(&rotulo(&format!("v{} · {}", i + 1, clock_since(t)), "")));
            let w = Rc::downgrade(self);
            b.connect_clicked(move |_| {
                if let Some(c) = w.upgrade() {
                    c.shown.set(i);
                    c.fill_versions();
                    c.show_code();
                    c.draw();
                    if debug() {
                        eprintln!("debug canvas: mostrando a versão {}", i + 1);
                    }
                }
            });
            self.versions.append(&b);
        }
    }

    fn show_code(&self) {
        let src = self.source();
        let name = self.file().and_then(|f| f.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_default();
        let picked = self.picked.borrow().as_ref().map(|p| lines_of(&src, &p.selector));
        let (a, z) = code_lines(src.lines().count(), picked, self.modes[0].is_active());
        let detail = format!("{name} · {}", trf("lines {from}–{to}", &[("from", a.to_string()), ("to", z.to_string())]));
        let text: Vec<&str> = src.lines().skip(a - 1).take(z + 1 - a).collect();
        while let Some(x) = self.code.first_child() {
            self.code.remove(&x);
        }
        // (the code alone: as tall as the panel lets it; beside the page: a few hundred pixels, it scrolls)
        let only_code = self.modes[1].is_active();
        self.code.set_vexpand(only_code);
        let block = crate::code_block::view("html", &text.join("\n"), &detail, a, if only_code { Some(0) } else { Some(420) });
        block.set_vexpand(only_code);
        self.code.append(&block);
    }

    /// The preview's shape asks for a page height other than the one drawn (more than 8 % off).
    fn reshaped(&self) -> bool {
        let width = self.width.get();
        let least = WIDTHS.iter().position(|w| *w == width).map(|i| HEIGHTS[i]).unwrap_or(HEIGHTS[0]);
        let want = f64::from(viewport_height(width, self.area.get(), least));
        let now = f64::from(self.asked.get());
        (want - now).abs() > now * 0.08
    }

    /// The page drawn again by the canvas's browser, at the width chosen.
    fn draw(self: &Rc<Self>) {
        let Some(path) = self.file() else { return };
        if self.busy.replace(true) {
            return;
        }
        let html = inline_styles(&self.source(), path.parent().unwrap_or(Path::new("/")));
        let width = self.width.get();
        let least = WIDTHS.iter().position(|w| *w == width).map(|i| HEIGHTS[i]).unwrap_or(HEIGHTS[0]);
        let height = viewport_height(width, self.area.get(), least);
        self.asked.set(height);
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(move || {
            crate::servicos::kalangod_navegador(json!({"acao": "html", "sessao": "canvas", "conteudo": html, "largura": width, "altura": height}))
        }, move |r: Result<Value, String>| {
            let Some(c) = w.upgrade() else { return };
            c.busy.set(false);
            match r {
                Ok(v) => {
                    c.page.set((v["largura"].as_f64().unwrap_or(f64::from(width)), v["altura"].as_f64().unwrap_or(f64::from(height))));
                    if let Some(t) = v["foto"].as_str().and_then(texture) {
                        c.picture.set_paintable(Some(&t));
                    }
                    c.status.set_visible(false);
                    c.marks.queue_draw();
                    if debug() {
                        eprintln!("debug canvas: página desenhada ({width} px, versão {}) · altura {height}", c.shown.get() + 1);
                    }
                    // (the panel took its size while the page was being drawn: once more at its shape)
                    if c.reshaped() {
                        c.draw();
                    }
                }
                Err(e) => {
                    c.status.set_text(&trf("The page could not be drawn: {reason}", &[("reason", e.clone())]));
                    c.status.set_visible(true);
                    eprintln!("kalango-shell: canvas: {e}");
                }
            }
        });
    }

    fn pick(self: &Rc<Self>, x: f64, y: f64) {
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(move || crate::servicos::kalangod_navegador(json!({"acao": "elemento", "sessao": "canvas", "x": x, "y": y})),
            move |r: Result<Value, String>| {
                let Some(c) = w.upgrade() else { return };
                let Ok(v) = r else { return };
                if let Some(t) = v["foto"].as_str().and_then(texture) {
                    c.picture.set_paintable(Some(&t));
                }
                let e = &v["elemento"];
                let Some(sel) = e["seletor"].as_str() else { return };
                let rect = (e["x"].as_f64().unwrap_or(0.0), e["y"].as_f64().unwrap_or(0.0), e["w"].as_f64().unwrap_or(0.0), e["h"].as_f64().unwrap_or(0.0));
                *c.picked.borrow_mut() = Some(Picked { selector: sel.to_string(), rect });
                // the field to the right of it (or under it, when there is no room)
                let (aw, ah) = (f64::from(c.picture.width()), f64::from(c.picture.height()));
                if let Some((ox, oy, s)) = c.fit(aw, ah) {
                    let right = ox + (rect.0 + rect.2) * s + 14.0;
                    let (mx, my) = if right + 380.0 <= aw { (right, oy + rect.1 * s) } else { (ox + rect.0 * s, oy + (rect.1 + rect.3) * s + 10.0) };
                    c.ask_box.set_margin_start(mx.max(0.0) as i32);
                    c.ask_box.set_margin_top(my.clamp(0.0, (ah - 40.0).max(0.0)) as i32);
                }
                c.ask_box.set_visible(true);
                c.ask.grab_focus();
                c.marks.queue_draw();
                c.show_code();
                crate::central::posicao_debug("canvas pedir", c.ask.upcast_ref());
                if debug() {
                    eprintln!("debug canvas: elemento {sel} em {:.0} {:.0} ({:.0}×{:.0})", rect.0, rect.1, rect.2, rect.3);
                }
            });
    }

    /// "Desfazer": the version before the one shown goes back into the file.
    fn undo(self: &Rc<Self>) {
        let i = self.shown.get();
        if i == 0 {
            return;
        }
        let Some(path) = self.file() else { return };
        let before = self.history.borrow()[i - 1].1.clone();
        if std::fs::write(&path, &before).is_ok() {
            if debug() {
                eprintln!("debug canvas: desfeito para a versão {i}");
            }
            // (the monitor adds it as the newest version)
            glib::timeout_add_local_once(Duration::from_millis(300), {
                let w = Rc::downgrade(self);
                move || {
                    if let Some(c) = w.upgrade() {
                        c.reload();
                    }
                }
            });
        }
    }

    /// "Baixar .zip": the page and its styles in Downloads.
    fn zip(self: &Rc<Self>) {
        let Some(path) = self.file() else { return };
        let dir = path.parent().unwrap_or(Path::new("/")).to_path_buf();
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "pagina".into());
        let mut names = vec![path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()];
        names.extend(linked_styles(&self.source()).into_iter().filter(|n| dir.join(n).is_file()));
        let out = crate::pastas::downloads().join(format!("{stem}.zip"));
        let _ = std::fs::create_dir_all(crate::pastas::downloads());
        let _ = std::fs::remove_file(&out);
        let ok = std::process::Command::new("zip").arg("-q").arg(&out).args(&names).current_dir(&dir).status().is_ok_and(|s| s.success());
        self.status.set_text(&if ok { trf("Saved in Downloads as “{file}”", &[("file", format!("{stem}.zip"))]) } else { tr("The .zip could not be made.").to_string() });
        self.status.set_visible(true);
        if debug() {
            eprintln!("debug canvas: zip {} ({ok}, {} arquivos)", out.display(), names.len());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_is_as_tall_as_the_preview_asks() {
        // a 756 × 600 preview at the desktop width: 873 px of page
        assert_eq!(viewport_height(1100, (756, 600), 500), 873);
        // a wide, short preview keeps the device's least
        assert_eq!(viewport_height(1100, (1400, 300), 500), 500);
        // a tall narrow one stops at three widths
        assert_eq!(viewport_height(390, (200, 2000), 760), 1170);
        // not laid out yet: the device's least
        assert_eq!(viewport_height(1100, (0, 0), 500), 500);
    }

    #[test]
    fn a_pages_style_sheets_go_inside_it() {
        let html = "<html><head><link rel=\"stylesheet\" href=\"estilo.css\"><link rel=stylesheet href='https://x.example/a.css'></head><body><a class=\"cta\">Baixar</a></body></html>";
        assert_eq!(linked_styles(html), ["estilo.css"]);
        let dir = std::env::temp_dir().join(format!("kalango-canvas-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("estilo.css"), ".cta { color: green }").unwrap();
        let one = inline_styles(html, &dir);
        assert!(one.contains("<style>/* estilo.css */\n.cta { color: green }\n</style>"), "{one}");
        assert!(!one.contains("href=\"estilo.css\""));
        // (a sheet on the web stays a link: it is fetched through the proxy)
        assert!(one.contains("x.example/a.css"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_page_written_in_the_answer() {
        let md = "Aqui está a página:\n\n```html\n<!DOCTYPE html>\n<html><body><h1>Olá</h1></body></html>\n```\n\nSalve como index.html.";
        assert_eq!(page_in_answer(md).as_deref(), Some("<!DOCTYPE html>\n<html><body><h1>Olá</h1></body></html>"));
        // the last page when there are two; none in other languages, nor while the block is still arriving
        let two = "```html\n<p>um</p>\n```\n```html\n<p>dois</p>\n```";
        assert_eq!(page_in_answer(two).as_deref(), Some("<p>dois</p>"));
        assert!(page_in_answer("```css\nbody { color: red }\n```").is_none());
        assert!(page_in_answer("```html\n<html><body>").is_none());
    }

    #[test]
    fn the_code_under_the_page() {
        assert_eq!(code_lines(159, None, true), (1, 8));
        assert_eq!(code_lines(5, None, true), (1, 5));
        assert_eq!(code_lines(159, Some((12, 14)), true), (12, 14));
        // Código and Dividir: the whole file, the element picked or not
        assert_eq!(code_lines(159, Some((12, 14)), false), (1, 159));
        assert_eq!(code_lines(0, None, false), (1, 1));
    }

    #[test]
    fn the_lines_of_an_element() {
        let src = "<html>\n<body>\n<section class=\"hero\">\n  <h1>O sistema</h1>\n  <a class=\"cta\" href=\"/baixar\">Baixar</a>\n</section>\n</body>\n</html>";
        assert_eq!(lines_of(src, "a.cta"), (3, 8));
        assert_eq!(lines_of(src, "h1"), (2, 7));
        assert_eq!(lines_of(src, "div.nada"), (1, 6));
    }
}
