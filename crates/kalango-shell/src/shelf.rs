//! The Shelf (design "Prateleira (recentes e feitos pelo agente)", 40500): beside the dock, only when there is
//! something new — a capture, a download (also one still coming), a file an agent made (the Timeline says so) — and it
//! goes by itself once seen. Its panel grows from the pile: the recent ones, the captures, the agents' files; each can
//! be opened, or dragged into any window or into the conversation with the agent.

use crate::conversa::ui_icon;
use crate::superficies::{caixa, rotulo};
use gtk::prelude::*;
use gtk::{gdk, gio, glib, Align, Orientation};
use kalango_i18n::{tr, trf};
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, SystemTime};

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

/// How long a thing counts as new (the pile shows), and how far back the panel looks.
const NEW_SECS: u64 = 3 * 3600;
const RECENT_SECS: u64 = 24 * 3600;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Capture,
    Download,
    Agent,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub kind: Kind,
    pub path: PathBuf,
    /// seconds since the epoch
    pub time: u64,
    /// who made it (an agent's file), or ""
    pub who: String,
    /// a download still coming (a .part file growing)
    pub coming: bool,
}

impl Item {
    pub fn name(&self) -> String {
        let n = self.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if self.coming {
            n.trim_end_matches(".part").trim_end_matches(".crdownload").trim_end_matches(".download").to_string()
        } else {
            n
        }
    }

    pub fn is_picture(&self) -> bool {
        self.path.extension().and_then(|e| e.to_str()).is_some_and(|e| ["png", "jpg", "jpeg", "webp", "gif"].contains(&e.to_lowercase().as_str()))
    }
}

fn now() -> u64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn mtime(p: &Path) -> u64 {
    std::fs::metadata(p).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0)
}

fn recent_files(dir: &Path, since: u64) -> Vec<(PathBuf, u64)> {
    std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_file())
        .filter(|p| !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')))
        .map(|p| { let t = mtime(&p); (p, t) }).filter(|(_, t)| *t >= since).collect()
}

fn downloads_dir() -> PathBuf {
    crate::pastas::downloads()
}

/// What the shelf holds (the last day's), newest first. Runs in a background thread.
pub fn scan() -> Vec<Item> {
    let since = now().saturating_sub(RECENT_SECS);
    let mut v: Vec<Item> = Vec::new();
    for d in crate::screenshot_tool::folders() {
        v.extend(recent_files(&d, since).into_iter().map(|(path, time)| Item { kind: Kind::Capture, path, time, who: String::new(), coming: false }));
    }
    for (path, time) in recent_files(&downloads_dir(), since) {
        let coming = path.extension().and_then(|e| e.to_str()).is_some_and(|e| ["part", "crdownload", "download"].contains(&e));
        v.push(Item { kind: Kind::Download, path, time, who: String::new(), coming });
    }
    // what the agents wrote (the Timeline: kalangod's records), still there
    if let Ok(recs) = crate::servicos::kalangod_registros(since as i64, 500) {
        for e in recs.iter().filter(|e| e["tool"] == "arquivos.escrever" && e["caller"].as_str().is_some_and(|c| c.contains("kalango-agente")) && e["status"] != "erro" && e["status"] != "negado") {
            let Some(p) = e["args"]["caminho"].as_str().map(PathBuf::from).filter(|p| p.is_file()) else { continue };
            if v.iter().any(|i| i.path == p) {
                continue;
            }
            v.push(Item { kind: Kind::Agent, path: p, time: e["time"].as_u64().unwrap_or(0), who: tr("Agent").to_string(), coming: false });
        }
    }
    v.sort_by(|a, b| b.time.cmp(&a.time));
    v.truncate(40);
    v
}

fn seen_file() -> PathBuf {
    glib::user_data_dir().join("kalango/prateleira-vista")
}

fn ago(t: u64) -> String {
    let s = now().saturating_sub(t);
    if s < 60 { tr("just now").to_string() } else if s < 3600 { trf("{n} min ago", &[("n", (s / 60).to_string())]) } else { trf("{n} h ago", &[("n", (s / 3600).to_string())]) }
}

pub struct Shelf {
    pub pill: gtk::Box,
    pile: gtk::DrawingArea,
    count: gtk::Label,
    pop: gtk::Popover,
    tabs: Vec<gtk::ToggleButton>,
    grid: gtk::FlowBox,
    list: gtk::Box,
    list_head: gtk::Box,
    items: RefCell<Vec<Item>>,
    seen: Cell<u64>,
    newest: Cell<u64>,
    tab: Cell<usize>,
    thumb: RefCell<Option<gtk::gdk_pixbuf::Pixbuf>>,
}

thread_local! {
    static SHELF: RefCell<Option<Rc<Shelf>>> = const { RefCell::new(None) };
}

/// The pile for the dock (hidden while there is nothing new).
pub fn widget() -> gtk::Box {
    if let Some(s) = SHELF.with(|s| s.borrow().clone()) {
        return s.pill.clone();
    }
    let s = Shelf::new();
    SHELF.with(|x| *x.borrow_mut() = Some(s.clone()));
    s.pill.clone()
}

/// The dock's direction: the pile sits after its icons (beside a bottom dock, below a side one).
pub fn follow(o: Orientation) {
    let p = widget();
    if o == Orientation::Vertical {
        p.set_margin_start(0);
        p.set_margin_top(10);
        p.set_halign(Align::Center);
    } else {
        p.set_margin_top(0);
        p.set_margin_start(10);
        p.set_valign(Align::Center);
    }
}

impl Shelf {
    fn new() -> Rc<Shelf> {
        let pill = caixa(Orientation::Horizontal, 0, "prat-pilha");
        pill.set_valign(Align::Center);
        pill.set_visible(false);
        let button = gtk::MenuButton::new();
        button.add_css_class("prat-botao");
        let inner = caixa(Orientation::Horizontal, 8, "");
        let pile = gtk::DrawingArea::new();
        pile.set_content_width(40);
        pile.set_content_height(34);
        inner.append(&pile);
        let count = rotulo("", "prat-contagem");
        inner.append(&count);
        button.set_child(Some(&inner));
        button.set_tooltip_text(Some(tr("Shelf")));
        crate::central::posicao_debug_sempre("prateleira", button.upcast_ref());
        pill.append(&button);
        // the panel
        let pop = gtk::Popover::new();
        pop.add_css_class("prat-painel");
        pop.set_position(gtk::PositionType::Top);
        let col = caixa(Orientation::Vertical, 10, "prat-coluna");
        col.set_size_request(400, -1);
        let head = caixa(Orientation::Horizontal, 6, "");
        let t = rotulo(tr("Shelf"), "prat-titulo");
        t.set_hexpand(true);
        t.set_xalign(0.0);
        head.append(&t);
        let mut tabs: Vec<gtk::ToggleButton> = Vec::new();
        for (i, n) in [tr("Recent"), tr("Captures"), tr("By the agent")].iter().enumerate() {
            let b = gtk::ToggleButton::with_label(n);
            b.add_css_class("prat-aba");
            if let Some(f) = tabs.first() {
                b.set_group(Some(f));
            }
            crate::central::posicao_debug_sempre(&format!("prateleira aba {i}"), b.upcast_ref());
            head.append(&b);
            tabs.push(b);
        }
        tabs[0].set_active(true);
        col.append(&head);
        let grid = gtk::FlowBox::new();
        grid.set_selection_mode(gtk::SelectionMode::None);
        grid.set_max_children_per_line(3);
        grid.set_min_children_per_line(3);
        grid.set_column_spacing(10);
        grid.set_row_spacing(10);
        col.append(&grid);
        let list_head = caixa(Orientation::Horizontal, 8, "");
        list_head.append(&ui_icon("spark", "#A5A1E2", 12));
        list_head.append(&rotulo(tr("MADE BY THE AGENTS"), "dsc-secao"));
        col.append(&list_head);
        let list = caixa(Orientation::Vertical, 8, "");
        col.append(&list);
        let hint = caixa(Orientation::Horizontal, 8, "");
        hint.append(&ui_icon("ponteiro", "#8E8F99", 12));
        let hl = rotulo(tr("Drag into any window or into the conversation with the agent"), "prat-dica");
        hl.set_xalign(0.0);
        hint.append(&hl);
        col.append(&hint);
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroll.set_propagate_natural_height(true);
        scroll.set_max_content_height(560);
        scroll.set_child(Some(&col));
        pop.set_child(Some(&scroll));
        button.set_popover(Some(&pop));
        // (Escape closes it: the dock takes the keyboard while it is open)
        crate::superficies::keyboard_while_open(&pop);
        let seen = std::fs::read_to_string(seen_file()).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
        let s = Rc::new(Shelf { pill, pile, count, pop, tabs, grid, list, list_head, items: RefCell::default(), seen: Cell::new(seen),
            newest: Cell::new(0), tab: Cell::new(0), thumb: RefCell::default() });
        s.wire();
        s
    }

    fn wire(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        self.pile.set_draw_func(move |_, cr, width, height| {
            if let Some(s) = w.upgrade() {
                s.draw_pile(cr, f64::from(width), f64::from(height));
            }
        });
        for (i, t) in self.tabs.iter().enumerate() {
            let w = Rc::downgrade(self);
            t.connect_toggled(move |t| {
                if let (Some(s), true) = (w.upgrade(), t.is_active()) {
                    s.tab.set(i);
                    s.fill();
                }
            });
        }
        // opened: what is there is seen (the pile goes once the panel closes)
        let w = Rc::downgrade(self);
        self.pop.connect_show(move |_| {
            if let Some(s) = w.upgrade() {
                s.fill();
                s.seen.set(s.newest.get());
                if let Some(d) = seen_file().parent() {
                    let _ = std::fs::create_dir_all(d);
                }
                let _ = std::fs::write(seen_file(), s.newest.get().to_string());
                if debug() {
                    eprintln!("debug prateleira: aberta ({} itens: {})", s.items.borrow().len(),
                        s.items.borrow().iter().take(5).map(|i| i.name()).collect::<Vec<_>>().join(", "));
                }
            }
        });
        let w = Rc::downgrade(self);
        self.pop.connect_closed(move |_| {
            if let Some(s) = w.upgrade() {
                if debug() {
                    eprintln!("debug prateleira: fechada");
                }
                s.update_pill();
            }
        });
        // a look every 5 s (in the background)
        let w = Rc::downgrade(self);
        let busy = Rc::new(Cell::new(false));
        glib::timeout_add_local(Duration::from_secs(5), move || {
            let Some(s) = w.upgrade() else { return glib::ControlFlow::Break };
            if !busy.replace(true) {
                let (w2, b2) = (Rc::downgrade(&s), busy.clone());
                crate::lateral::em_segundo_plano(scan, move |items| {
                    b2.set(false);
                    if let Some(s) = w2.upgrade() {
                        s.take(items);
                    }
                });
            }
            glib::ControlFlow::Continue
        });
    }

    fn take(self: &Rc<Self>, items: Vec<Item>) {
        let newest = items.first().map(|i| i.time).unwrap_or(0);
        let arrived = newest > self.newest.get() && self.newest.get() != 0;
        if *self.items.borrow() == items {
            return;
        }
        self.newest.set(newest);
        // the newest picture on top of the pile
        let pic = items.iter().find(|i| i.is_picture()).map(|i| i.path.clone());
        *self.thumb.borrow_mut() = pic.and_then(|p| gtk::gdk_pixbuf::Pixbuf::from_file_at_scale(&p, 80, 60, true).ok());
        *self.items.borrow_mut() = items;
        self.update_pill();
        if self.pop.is_visible() {
            self.fill();
        }
        // a new thing: the pile pulses
        if arrived {
            self.pill.add_css_class("pulso");
            let p = self.pill.clone();
            glib::timeout_add_local_once(Duration::from_millis(700), move || p.remove_css_class("pulso"));
            if debug() {
                eprintln!("debug prateleira: novo «{}»", self.items.borrow().first().map(|i| i.name()).unwrap_or_default());
            }
            if self.pill.is_visible() {
                self.log_place();
            }
        }
    }

    /// The pile shows while something is new (or the panel is open), with how many things the panel has.
    fn update_pill(&self) {
        let n = self.items.borrow().len();
        let new = self.newest.get() > self.seen.get() && now().saturating_sub(self.newest.get()) < NEW_SECS;
        let show = n > 0 && (new || self.pop.is_visible() || self.items.borrow().iter().any(|i| i.coming));
        self.count.set_text(&n.to_string());
        if show != self.pill.is_visible() && debug() {
            eprintln!("debug prateleira: {} ({n} itens)", if show { "à vista" } else { "escondida" });
            if show {
                self.log_place();
            }
        }
        self.pill.set_visible(show);
        self.pile.queue_draw();
    }

    /// For the session test: where the pile is on the screen (the dock is a layer at the bottom, centred).
    fn log_place(&self) {
        if !debug() {
            return;
        }
        let p = self.pill.clone();
        glib::timeout_add_local_once(Duration::from_millis(500), move || {
            use gtk4_layer_shell::LayerShell;
            let Some(win) = p.root().and_downcast::<gtk::Window>() else { return };
            let Some(b) = p.compute_bounds(&win) else { return };
            // (the screen as niri has it: GDK's monitor can keep an old size after a scale change)
            let (gw, gh) = crate::superficies::tamanho_da_saida()
                .or_else(|| gdk::Display::default().and_then(|d| d.monitors().item(0)).and_downcast::<gdk::Monitor>().map(|m| (m.geometry().width(), m.geometry().height())))
                .unwrap_or((1280, 800));
            // (a bottom dock is centred across, a side one down the left edge)
            let (wx, wy) = if win.is_anchor(gtk4_layer_shell::Edge::Left) && !win.is_anchor(gtk4_layer_shell::Edge::Right) {
                (win.margin(gtk4_layer_shell::Edge::Left), (gh - win.height()) / 2)
            } else {
                ((gw - win.width()) / 2, gh - win.margin(gtk4_layer_shell::Edge::Bottom) - win.height())
            };
            let (x, y) = (wx + (b.x() + b.width() / 2.0) as i32, wy + (b.y() + b.height() / 2.0) as i32);
            eprintln!("debug prateleira: pilha em {x} {y}");
        });
    }

    fn draw_pile(&self, cr: &gtk::cairo::Context, w: f64, h: f64) {
        // three cards a little apart, the newest picture on the front one
        for (k, a) in [(2.0, 0.35), (1.0, 0.6), (0.0, 1.0)] {
            let (x, y, cw, ch) = (k * 3.0 + 1.0, k * -2.0 + 6.0, w - 10.0, h - 10.0);
            cr.set_source_rgba(0.16, 0.17, 0.20, a);
            cr.rectangle(x, y, cw, ch);
            let _ = cr.fill();
            cr.set_source_rgba(0.35, 0.36, 0.42, a);
            cr.set_line_width(1.0);
            cr.rectangle(x + 0.5, y + 0.5, cw - 1.0, ch - 1.0);
            let _ = cr.stroke();
        }
        if let Some(png) = self.thumb.borrow().as_ref() {
            use gtk::gdk::prelude::GdkCairoContextExt;
            let (cw, ch) = (w - 10.0, h - 10.0);
            let k = (cw / f64::from(png.width())).max(ch / f64::from(png.height()));
            cr.save().ok();
            cr.rectangle(1.0, 6.0, cw, ch);
            cr.clip();
            cr.translate(1.0, 6.0);
            cr.scale(k, k);
            cr.set_source_pixbuf(png, 0.0, 0.0);
            let _ = cr.paint();
            cr.restore().ok();
        }
    }

    fn drag_source(w: &impl IsA<gtk::Widget>, path: &Path) {
        let d = gtk::DragSource::new();
        d.set_actions(gdk::DragAction::COPY);
        let f = gio::File::for_path(path);
        d.connect_prepare(move |_, _, _| Some(gdk::ContentProvider::for_value(&gdk::FileList::from_array(&[f.clone()]).to_value())));
        w.add_controller(d);
    }

    fn open(path: &Path) {
        let _ = crate::apps::open_default(&gio::File::for_path(path), None);
        if debug() {
            eprintln!("debug prateleira: abrir «{}»", path.display());
        }
    }

    /// The panel: pictures as a grid, the agents' files and the downloads as a list.
    fn fill(self: &Rc<Self>) {
        while let Some(c) = self.grid.first_child() {
            self.grid.remove(&c);
        }
        while let Some(c) = self.list.first_child() {
            self.list.remove(&c);
        }
        let tab = self.tab.get();
        let items = self.items.borrow().clone();
        let pics: Vec<&Item> = items.iter().filter(|i| i.is_picture() && (tab == 0 || (tab == 1 && i.kind == Kind::Capture) || (tab == 2 && i.kind == Kind::Agent))).take(9).collect();
        for (k, i) in pics.iter().enumerate() {
            let b = gtk::Button::new();
            b.add_css_class("prat-miniatura");
            let col = caixa(Orientation::Vertical, 4, "");
            let pic = gtk::Picture::for_filename(&i.path);
            pic.set_content_fit(gtk::ContentFit::Cover);
            pic.set_size_request(120, 76);
            col.append(&pic);
            let l = rotulo(&i.name(), "prat-nome");
            l.set_xalign(0.0);
            l.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            l.set_max_width_chars(14);
            col.append(&l);
            b.set_child(Some(&col));
            Self::drag_source(&b, &i.path);
            crate::central::posicao_debug_sempre(&format!("prateleira item {}", k + 1), b.upcast_ref());
            let p = i.path.clone();
            b.connect_clicked(move |_| Self::open(&p));
            self.grid.append(&b);
        }
        let rest: Vec<&Item> = items.iter().filter(|i| !i.is_picture() && (tab == 0 || (tab == 1 && i.kind == Kind::Capture) || (tab == 2 && i.kind == Kind::Agent))).take(8).collect();
        self.list_head.set_visible(rest.iter().any(|i| i.kind == Kind::Agent));
        for i in rest {
            let b = gtk::Button::new();
            b.add_css_class("prat-linha");
            let r = caixa(Orientation::Horizontal, 10, "");
            r.append(&ui_icon(if i.kind == Kind::Download { "download" } else { "doc" }, "#8E8F99", 14));
            let c = caixa(Orientation::Vertical, 1, "");
            c.set_hexpand(true);
            let n = rotulo(&i.name(), "prat-linha-nome");
            n.set_xalign(0.0);
            n.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            n.set_max_width_chars(1);
            c.append(&n);
            let sub = if i.coming {
                let size = std::fs::metadata(&i.path).map(|m| m.len()).unwrap_or(0);
                trf("downloading · {size}", &[("size", crate::disks_data::amount(size))])
            } else if i.kind == Kind::Agent {
                format!("{} · {}", i.who, ago(i.time))
            } else {
                ago(i.time)
            };
            let s = rotulo(&sub, "prat-linha-sub");
            s.set_xalign(0.0);
            c.append(&s);
            r.append(&c);
            r.append(&ui_icon("chevron", "#8E8F99", 12));
            b.set_child(Some(&r));
            Self::drag_source(&b, &i.path);
            let p = i.path.clone();
            b.connect_clicked(move |_| Self::open(&p));
            self.list.append(&b);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_kinds() {
        let i = Item { kind: Kind::Download, path: PathBuf::from("/x/kalangoos-0.2.iso.part"), time: 0, who: String::new(), coming: true };
        assert_eq!(i.name(), "kalangoos-0.2.iso");
        assert!(!i.is_picture());
        let c = Item { kind: Kind::Capture, path: PathBuf::from("/x/Captura 02:01.png"), time: 0, who: String::new(), coming: false };
        assert!(c.is_picture());
    }
}
