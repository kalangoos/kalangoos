//! The flow builder's canvas (design "Fluxo · construtor"): the blocks (widgets placed on a `gtk::Fixed` and scaled
//! with the zoom), the connections drawn over them (Bézier curves with their ports and way labels, the loop back up
//! dashed, the ones the last run travelled in teal, the one waiting for the person in gold), the zoom bar, the
//! minimap and the tip. It moves blocks, connects ports, takes blocks dropped from the palette and picks a
//! connection; what that means for the flow goes to the page through `on_event`. Geometry is `fluxo_modelo`'s.

use crate::conversa::ui_icon;
use crate::fluxo_modelo::{self as m, EdgeDoc, FlowDoc, Rect, TriggerDoc, View};
use crate::superficies::{caixa, rotulo};
use gtk::prelude::*;
use gtk::{gdk, glib, graphene, gsk, Align, Orientation};
use kalango_i18n::{tr, trf};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// What the page shows on the canvas.
#[derive(Clone, Debug, Default)]
pub struct CanvasData {
    pub flow: FlowDoc,
    pub triggers: Vec<TriggerDoc>,
    pub agents: HashMap<String, String>,
    /// each block's state in the run shown (done, failed, running, waiting)
    pub states: HashMap<String, String>,
    pub travelled: HashSet<(String, String)>,
    /// how many items a For each went through
    pub items: HashMap<String, usize>,
    /// a way's label with the run's numbers ("fatura 91%")
    pub labels: HashMap<(String, String), String>,
    /// problems found by the service, per block
    pub issues: HashMap<String, String>,
    pub selected: Option<String>,
    /// every block chosen (Shift+click, the band), the one above among them
    pub chosen: Vec<String>,
    /// a For each going through its items now (`run.item`): (the item it is on, how many)
    pub progress: HashMap<String, (usize, usize)>,
    /// after a test: each block's step ("Passo N") and the way the run went ("Caminho: A → B → …")
    pub step_no: HashMap<String, usize>,
    pub path: Option<String>,
    /// the sandboxes' machines as the run reports them (`run.sandbox`): "pronta em 0,8 s", "apagada"…
    pub sandbox_notes: HashMap<String, String>,
    /// the VirtualDesk machines by id (a sandbox's template by its name)
    pub templates: HashMap<String, String>,
}

pub enum CanvasEvent {
    /// a block or a trigger ("trigger:<id>") chosen, or none
    Select(Option<String>),
    /// Shift+click: a block added to the blocks chosen, or taken out
    Toggle(String),
    /// the band drawn on the empty canvas: the blocks inside it (with Shift: added to the ones chosen)
    SelectMany(Vec<String>, bool),
    /// blocks dragged to these places, and the ones that went into a sandbox (Some) or out of one (None)
    Moved(Vec<(String, f64, f64)>, Vec<(String, Option<String>)>),
    /// a connection drawn from a block's port (its way when the port has a name: None picks the first way free) to
    /// another block
    Connect(String, Option<Option<String>>, String),
    /// a connection's end dragged to another block
    Reconnect(EdgeDoc, String),
    /// the right button: on a block (Some) or on the empty canvas, at a screen and a world point
    Menu(Option<String>, (f64, f64), (f64, f64)),
    /// a double click on a block
    Activate(String),
    /// a palette item dropped at a world point
    Drop(String, (f64, f64)),
    /// a connection chosen (Delete removes it, "/" puts a block in it)
    Edge(Option<EdgeDoc>),
}

enum Drag {
    None,
    Pan { start: (f64, f64) },
    /// the blocks that move (each with where it started; a sandbox brings the blocks inside along)
    Move { id: String, ids: Vec<(String, (f64, f64))> },
    /// (`way`: the named port it left by)
    Connect { from: String, way: Option<Option<String>>, at: (f64, f64) },
    Reconnect { edge: EdgeDoc, at: (f64, f64) },
    Band { from: (f64, f64), at: (f64, f64) },
}

pub struct Canvas {
    pub root: gtk::Overlay,
    fixed: gtk::Fixed,
    edges: gtk::DrawingArea,
    minimap: gtk::DrawingArea,
    zoom_label: gtk::Label,
    pub tip: gtk::Box,
    pub banner: gtk::Box,
    pub banner_text: gtk::Label,
    pub banner_note: gtk::Label,
    /// "+" over the canvas in a narrow window (the palette in a popover)
    pub add_button: gtk::MenuButton,
    /// the zoom bar and the minimap (a narrow window has no room for them)
    controls: [gtk::Box; 2],
    view: Cell<View>,
    data: RefCell<CanvasData>,
    /// canvas id → (widget, its rectangle in the world)
    nodes: RefCell<Vec<(String, gtk::Box, Rect)>>,
    drag: RefCell<Drag>,
    moved: Cell<bool>,
    /// Shift held when the press began
    shift: Cell<bool>,
    pointer: Cell<(f64, f64)>,
    /// "Caminho: A → B → …" after a test
    path: gtk::Label,
    /// each sandbox's title width (its dashed border breaks there)
    captions: RefCell<HashMap<String, f64>>,
    pub edge: RefCell<Option<EdgeDoc>>,
    /// the connections' routes, kept while nothing they go around changes (a drag reroutes only what it touches)
    routes: RefCell<crate::flow_router::RouteCache>,
    /// each block's rows of named ports (their ports sit in line with them, wherever the text puts them)
    port_rows: RefCell<HashMap<String, Vec<gtk::Box>>>,
    fitted: Cell<bool>,
    pub on_event: RefCell<Option<Rc<dyn Fn(CanvasEvent)>>>,
}

/// Where the triggers sit: to the left of the block the run starts at, one under the other.
pub fn trigger_rect(flow: &FlowDoc, i: usize, h: f64) -> Rect {
    let (x, y) = flow.entry().map(|e| (e.x - 228.0, e.y)).unwrap_or((40.0, 60.0));
    Rect { x, y: y + i as f64 * (h + 40.0), w: m::node_width("trigger"), h }
}

/// A colour as cairo takes it (red, green, blue, alpha).
type Rgba = (f64, f64, f64, f64);

fn rgba(hex: &str, a: f64) -> Rgba {
    let c = gdk::RGBA::parse(crate::tema::cor(hex)).unwrap_or(gdk::RGBA::WHITE);
    (c.red() as f64, c.green() as f64, c.blue() as f64, a)
}

fn set(cr: &gtk::cairo::Context, c: (f64, f64, f64, f64)) {
    cr.set_source_rgba(c.0, c.1, c.2, c.3);
}

impl Canvas {
    pub fn new() -> Rc<Canvas> {
        let root = gtk::Overlay::new();
        root.add_css_class("fluxo-tela");
        root.set_hexpand(true);
        root.set_vexpand(true);
        root.set_overflow(gtk::Overflow::Hidden);
        root.set_focusable(true);
        let fundo = gtk::DrawingArea::new();
        fundo.set_hexpand(true);
        fundo.set_vexpand(true);
        fundo.set_size_request(240, 200);
        root.set_child(Some(&fundo));
        let fixed = gtk::Fixed::new();
        root.add_overlay(&fixed);
        let edges = gtk::DrawingArea::new();
        edges.set_can_target(false);
        root.add_overlay(&edges);
        // the tip, or (running) what waits for the person
        let tip = caixa(Orientation::Horizontal, 7, "fluxo-dica");
        tip.append(&ui_icon("info", "#8E8F99", 14));
        tip.append(&rotulo(tr("Drag a block onto the canvas or type / over a connection"), "fluxo-dica-texto"));
        tip.set_halign(Align::Start);
        tip.set_valign(Align::Start);
        tip.set_margin_start(24);
        tip.set_margin_top(16);
        root.add_overlay(&tip);
        let path = rotulo("", "fluxo-percurso");
        // (above the zoom bar: the tip and the blocks stay clear)
        path.set_halign(Align::Start);
        path.set_valign(Align::End);
        path.set_margin_start(16);
        path.set_margin_end(160);
        path.set_margin_bottom(62);
        path.set_ellipsize(gtk::pango::EllipsizeMode::End);
        path.set_visible(false);
        path.set_can_target(false);
        root.add_overlay(&path);
        let banner = caixa(Orientation::Horizontal, 8, "fluxo-faixa");
        banner.append(&ui_icon("escudo", "#E5C98C", 14));
        let banner_text = rotulo("", "fluxo-faixa-texto");
        banner_text.set_ellipsize(gtk::pango::EllipsizeMode::End);
        banner.append(&banner_text);
        let banner_note = rotulo(tr("the rest of the flow waits, nothing was lost"), "fluxo-faixa-nota");
        banner_note.set_ellipsize(gtk::pango::EllipsizeMode::End);
        banner.append(&banner_note);
        banner.set_halign(Align::Start);
        banner.set_valign(Align::Start);
        banner.set_margin_start(24);
        banner.set_margin_end(24);
        banner.set_margin_top(14);
        banner.set_visible(false);
        root.add_overlay(&banner);
        // zoom
        let zoom = caixa(Orientation::Horizontal, 0, "fluxo-zoom");
        zoom.set_halign(Align::Start);
        zoom.set_valign(Align::End);
        zoom.set_margin_start(16);
        zoom.set_margin_bottom(16);
        let menos = gtk::Button::with_label("–");
        menos.add_css_class("fluxo-zoom-botao");
        let zoom_label = rotulo("100%", "fluxo-zoom-valor");
        let mais = gtk::Button::with_label("+");
        mais.add_css_class("fluxo-zoom-botao");
        let ajustar = gtk::Button::with_label(tr("Fit"));
        ajustar.add_css_class("fluxo-zoom-botao");
        ajustar.add_css_class("texto");
        for w in [menos.upcast_ref::<gtk::Widget>(), zoom_label.upcast_ref(), mais.upcast_ref(), ajustar.upcast_ref()] {
            zoom.append(w);
        }
        root.add_overlay(&zoom);
        // minimap
        let mapa = caixa(Orientation::Vertical, 0, "fluxo-mapa");
        mapa.set_halign(Align::End);
        mapa.set_valign(Align::End);
        mapa.set_margin_end(16);
        mapa.set_margin_bottom(16);
        let minimap = gtk::DrawingArea::new();
        minimap.set_content_width(128);
        minimap.set_content_height(80);
        mapa.append(&minimap);
        root.add_overlay(&mapa);
        let add_button = gtk::MenuButton::new();
        add_button.add_css_class("fluxo-mais");
        add_button.set_child(Some(&ui_icon("mais", "#1C1D24", 18)));
        add_button.set_tooltip_text(Some(tr("Add a block")));
        add_button.set_halign(Align::End);
        add_button.set_valign(Align::End);
        add_button.set_margin_end(24);
        add_button.set_margin_bottom(24);
        let pop = gtk::Popover::new();
        pop.add_css_class("fluxo-pop");
        add_button.set_popover(Some(&pop));
        add_button.set_visible(false);
        root.add_overlay(&add_button);

        let c = Rc::new(Canvas {
            root, fixed, edges, minimap, zoom_label, tip, banner, banner_text, banner_note, add_button, controls: [zoom.clone(), mapa.clone()], view: Cell::new(View::default()), data: RefCell::default(),
            nodes: RefCell::default(), drag: RefCell::new(Drag::None), moved: Cell::new(false), shift: Cell::new(false), pointer: Cell::new((0.0, 0.0)), path, captions: RefCell::default(), edge: RefCell::default(),
            fitted: Cell::new(false), on_event: RefCell::default(), routes: RefCell::default(), port_rows: RefCell::default(),
        });
        let w = Rc::downgrade(&c);
        c.edges.set_draw_func(move |_, cr, _, _| {
            if let Some(c) = w.upgrade() {
                c.draw_edges(cr);
            }
        });
        let w = Rc::downgrade(&c);
        c.minimap.set_draw_func(move |_, cr, wd, ht| {
            if let Some(c) = w.upgrade() {
                c.draw_minimap(cr, wd as f64, ht as f64);
            }
        });
        crate::tema::registrar(&c.edges, |w| w.queue_draw());
        crate::tema::registrar(&c.minimap, |w| w.queue_draw());
        for (b, f) in [(&menos, 1.0 / 1.2), (&mais, 1.2)] {
            let w = Rc::downgrade(&c);
            b.connect_clicked(move |_| {
                if let Some(c) = w.upgrade() {
                    let (wd, ht) = (c.root.width() as f64, c.root.height() as f64);
                    c.set_view(c.view.get().zoom_at((wd / 2.0, ht / 2.0), f));
                }
            });
        }
        let w = Rc::downgrade(&c);
        ajustar.connect_clicked(move |_| {
            if let Some(c) = w.upgrade() {
                c.fit();
            }
        });
        c.connect_input();
        // the first size: everything in view
        let w = Rc::downgrade(&c);
        fundo.connect_resize(move |_, _, _| {
            if let Some(c) = w.upgrade() {
                if !c.fitted.get() && !c.nodes.borrow().is_empty() {
                    c.fitted.set(true);
                    let c2 = c.clone();
                    glib::idle_add_local_once(move || c2.fit());
                }
                c.edges.queue_draw();
                c.minimap.queue_draw();
            }
        });
        c
    }

    /// A narrow window: the "+" over the canvas instead of the palette, no zoom bar nor minimap.
    pub fn set_narrow(&self, narrow: bool) {
        self.add_button.set_visible(narrow);
        for c in &self.controls {
            c.set_visible(!narrow);
        }
    }

    fn emit(&self, e: CanvasEvent) {
        let f = self.on_event.borrow().clone();
        if let Some(f) = f {
            f(e);
        }
    }

    pub fn set_view(&self, v: View) {
        self.view.set(v);
        self.zoom_label.set_text(&format!("{}%", (v.zoom * 100.0).round()));
        for (_, w, r) in self.nodes.borrow().iter() {
            self.place(w, r);
        }
        self.edges.queue_draw();
        self.minimap.queue_draw();
    }

    fn place(&self, w: &gtk::Box, r: &Rect) {
        let v = self.view.get();
        let (sx, sy) = v.to_screen((r.x, r.y));
        let t = gsk::Transform::new().translate(&graphene::Point::new(sx as f32, sy as f32)).scale(v.zoom as f32, v.zoom as f32);
        self.fixed.set_child_transform(w, Some(&t));
    }

    /// Everything in view (the button "Ajustar", and the first time the canvas shows a flow).
    pub fn fit(&self) {
        let (wd, ht) = (self.root.width() as f64, self.root.height() as f64);
        if wd < 10.0 {
            return;
        }
        if let Some(b) = m::bbox(self.nodes.borrow().iter().map(|(_, _, r)| *r)) {
            // (room for the tip above and the zoom bar below)
            let mut v = View::fit(Rect { x: b.x, y: b.y - 40.0, w: b.w, h: b.h + 90.0 }, wd, ht, 24.0);
            v.pan.0 = v.pan.0.min(24.0 - b.x * v.zoom).max(24.0 - b.x * v.zoom);
            self.set_view(v);
        }
        self.log_places();
    }

    /// Each block's place on the screen, for the session's scripts (`KALANGO_SHELL_DEBUG` only).
    fn log_places(&self) {
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_none() {
            return;
        }
        for (id, w, _) in self.nodes.borrow().iter() {
            crate::central::posicao_debug(&format!("fluxo bloco {id}"), w.upcast_ref());
        }
        // (and each port, in the window's coordinates)
        let Some(win) = self.root.root() else { return };
        let v = self.view.get();
        for (id, list) in self.ports() {
            for (i, (q, _)) in list.iter().enumerate() {
                let s = v.to_screen(*q);
                if let Some(p) = self.root.compute_point(&win, &graphene::Point::new(s.0 as f32, s.1 as f32)) {
                    eprintln!("debug central: fluxo porta {id} {i} em {:.0} {:.0}", p.x(), p.y());
                }
            }
        }
    }

    /// Shows the flow again from the start (a new flow: fit it once more).
    pub fn reset_fit(&self) {
        self.fitted.set(false);
    }

    /// Each block's size on the canvas (a sandbox's dashed area, and its corner under `SANDBOX_AREA`), for
    /// "Organizar o fluxo" and "Alinhar na vertical".
    pub fn sizes(&self) -> HashMap<String, (f64, f64)> {
        let d = self.data.borrow();
        let mut v = HashMap::new();
        for (id, _, r) in self.nodes.borrow().iter() {
            v.insert(id.clone(), (r.w, r.h));
            if d.flow.node(id).is_some_and(|n| n.kind == "sandbox") {
                v.insert(format!("{}{id}", m::SANDBOX_AREA), (r.x, r.y));
            }
        }
        v
    }

    /// A block's place on the screen (a popover points at it).
    pub fn screen_rect(&self, id: &str) -> Option<gdk::Rectangle> {
        let r = self.rect_of(id)?;
        let v = self.view.get();
        let (a, b) = (v.to_screen((r.x, r.y)), v.to_screen((r.x + r.w, r.y + r.h)));
        Some(gdk::Rectangle::new(a.0 as i32, a.1 as i32, (b.0 - a.0) as i32, (b.1 - a.1) as i32))
    }

    pub fn rects(&self) -> Vec<(String, Rect)> {
        self.nodes.borrow().iter().map(|(id, _, r)| (id.clone(), *r)).collect()
    }

    /// Draws the flow anew.
    pub fn set_data(self: &Rc<Self>, d: CanvasData) {
        *self.data.borrow_mut() = d;
        self.port_rows.borrow_mut().clear();
        // (new data — a flow opened, undo, redo, Organizar, a paste: every route worked out anew)
        self.routes.borrow_mut().clear();
        for (_, w, _) in self.nodes.borrow_mut().drain(..) {
            self.fixed.remove(&w);
        }
        let d = self.data.borrow();
        // the sandboxes' dashed areas go first: under the blocks, and the last to take a click
        let mut areas: Vec<(String, gtk::Box)> = Vec::new();
        for n in d.flow.nodes.iter().filter(|n| n.kind == "sandbox") {
            let w = self.sandbox_widget(n, &d);
            self.fixed.put(&w, 0.0, 0.0);
            areas.push((n.id.clone(), w));
        }
        let mut out = Vec::new();
        // the flow's input fields, on the start: its triggers, or the block a run starts at when it has none
        let input = m::inputs_line(&d.flow.inputs()).map(|text| m::Line { text, mono: true, wrap: true });
        let entry = d.flow.entry().map(|n| n.id.clone()).filter(|_| d.triggers.is_empty());
        for n in d.flow.nodes.iter().filter(|n| n.kind != "sandbox") {
            let (title, mut lines) = m::node_text(n, &d.agents, d.items.get(&n.id).copied());
            if entry.as_deref() == Some(n.id.as_str()) {
                lines.extend(input.clone());
            }
            let look = m::look(&n.kind);
            let w = self.node_widget(&n.id, look.caps, look.glyph, look.tone, &title, &lines, &d);
            let width = m::node_width(&n.kind);
            out.push((n.id.clone(), w, Rect { x: n.x, y: n.y, w: width, h: 0.0 }));
        }
        let mut trig = Vec::new();
        for t in &d.triggers {
            let (title, mut lines) = m::trigger_text(&t.config);
            lines.extend(input.clone());
            let id = format!("{}{}", m::TRIGGER_PREFIX, t.id);
            let glyph = m::trigger_glyph(t.config["kind"].as_str().unwrap_or(""));
            let w = self.node_widget(&id, m::look("trigger").caps, glyph, m::Tone::Gold, &title, &lines, &d);
            trig.push((id, w));
        }
        for (_, w, r) in out.iter_mut() {
            self.fixed.put(w, 0.0, 0.0);
            r.h = w.measure(Orientation::Vertical, r.w as i32).0 as f64;
        }
        // each dashed area holds the blocks inside it
        let mut all: Vec<(String, gtk::Box, Rect)> = Vec::new();
        for (id, w) in areas {
            let own = d.flow.node(&id).map(|n| (n.x, n.y)).unwrap_or((0.0, 0.0));
            let inside = out.iter().filter(|(k, _, _)| d.flow.node(k).is_some_and(|n| n.parent.as_deref() == Some(id.as_str()))).map(|(_, _, r)| *r);
            let r = m::sandbox_area(own, inside);
            w.set_size_request(r.w as i32, r.h as i32);
            let cw = w.first_child().map(|c| c.measure(Orientation::Horizontal, -1).1 as f64).unwrap_or(0.0);
            self.captions.borrow_mut().insert(id.clone(), cw.min(r.w - 24.0));
            all.push((id, w, r));
        }
        all.extend(out);
        for (i, (id, w)) in trig.into_iter().enumerate() {
            self.fixed.put(&w, 0.0, 0.0);
            let th = w.measure(Orientation::Vertical, m::node_width("trigger") as i32).0 as f64;
            all.push((id, w, trigger_rect(&d.flow, i, th)));
        }
        for (_, w, r) in &all {
            self.place(w, r);
        }
        self.path.set_text(&d.path.as_deref().map(|p| format!("{}: {p}", tr("Path"))).unwrap_or_default());
        self.path.set_visible(d.path.is_some());
        drop(d);
        *self.nodes.borrow_mut() = all;
        if !self.fitted.get() && self.root.width() > 10 && !self.nodes.borrow().is_empty() {
            self.fitted.set(true);
            self.fit();
        }
        self.edges.queue_draw();
        self.minimap.queue_draw();
        self.log_places();
    }

    /// A sandbox's dashed area (design "Bloco em sandbox"): its title on the border; the blocks inside sit over it.
    fn sandbox_widget(&self, n: &m::NodeDoc, d: &CanvasData) -> gtk::Box {
        let b = caixa(Orientation::Vertical, 0, "fluxo-sandbox");
        if d.chosen.contains(&n.id) || d.selected.as_deref() == Some(n.id.as_str()) {
            b.add_css_class("escolhido");
        }
        if n.disabled {
            b.add_css_class("desativado");
        }
        if d.issues.contains_key(&n.id) {
            b.add_css_class("problema");
            b.set_tooltip_text(d.issues.get(&n.id).map(String::as_str));
        }
        let cab = caixa(Orientation::Horizontal, 6, "fluxo-sandbox-cab");
        cab.set_halign(Align::Start);
        cab.append(&ui_icon("escudo", "#A5A1E2", 12));
        let tid = n.data["template"].as_str().unwrap_or("");
        let name = d.templates.get(tid).cloned().unwrap_or_else(|| tid.to_string());
        let mut caption = m::sandbox_caption(&n.data, &name);
        if !n.title.trim().is_empty() {
            caption = format!("{} · {caption}", n.title.trim());
        }
        let l = rotulo(&caption, "fluxo-sandbox-titulo");
        l.set_ellipsize(gtk::pango::EllipsizeMode::End);
        cab.append(&l);
        if let Some(k) = d.step_no.get(&n.id) {
            let st = rotulo(&trf("Step {n}", &[("n", k.to_string())]), "fluxo-no-passo");
            st.set_valign(Align::Center);
            cab.append(&st);
        }
        b.append(&cab);
        // what the run's machine did, at the bottom of the area
        if let Some(x) = d.sandbox_notes.get(&n.id) {
            let gap = caixa(Orientation::Vertical, 0, "");
            gap.set_vexpand(true);
            b.append(&gap);
            let l = rotulo(x, "fluxo-sandbox-nota");
            l.set_halign(Align::Start);
            l.set_ellipsize(gtk::pango::EllipsizeMode::End);
            b.append(&l);
        }
        b
    }

    #[allow(clippy::too_many_arguments)]
    fn node_widget(&self, id: &str, caps: &'static str, glyph: &str, tone: m::Tone, title: &str, lines: &[m::Line], d: &CanvasData) -> gtk::Box {
        let b = caixa(Orientation::Vertical, 0, "fluxo-no");
        let is_trigger = id.starts_with(m::TRIGGER_PREFIX);
        let kind = if is_trigger { "trigger".to_string() } else { d.flow.node(id).map(|n| n.kind.clone()).unwrap_or_default() };
        b.set_size_request(m::node_width(&kind) as i32, -1);
        if d.selected.as_deref() == Some(id) || d.chosen.iter().any(|c| c == id) {
            b.add_css_class("escolhido");
        }
        let node = d.flow.node(id);
        // (switched off: faded, a run passes its input straight on)
        if node.is_some_and(|n| n.disabled) {
            b.add_css_class("desativado");
            b.set_tooltip_text(Some(tr("Off: a run passes its input straight on")));
        }
        let state = d.states.get(id).map(String::as_str);
        if state == Some("waiting") {
            b.add_css_class("aguarda");
        }
        if d.issues.contains_key(id) {
            b.add_css_class("problema");
            b.set_tooltip_text(d.issues.get(id).map(String::as_str));
        }
        let cab = caixa(Orientation::Horizontal, 8, "");
        let tile = caixa(Orientation::Horizontal, 0, "fluxo-no-tile");
        tile.add_css_class(tone.class());
        tile.set_size_request(24, 24);
        tile.set_valign(Align::Center);
        let g = if glyph == "spark" { crate::superficies::estrela(12, tone.color()).upcast::<gtk::Widget>() } else { ui_icon(glyph, tone.color(), 14).upcast() };
        g.set_hexpand(true);
        tile.append(&g);
        tile.set_hexpand(false);
        cab.append(&tile);
        let c = rotulo(tr(caps), "fluxo-no-caps");
        c.add_css_class(tone.class());
        c.set_hexpand(true);
        c.set_xalign(0.0);
        cab.append(&c);
        // the run's dot: done, waiting, failed, running (a trigger counts as done when its flow ran)
        let dot_state = if is_trigger { (!d.states.is_empty()).then_some("done") } else { state };
        if let Some(s) = dot_state {
            let p = caixa(Orientation::Horizontal, 0, "fluxo-no-ponto");
            p.add_css_class(match s {
                "waiting" => "aguarda",
                "failed" => "falhou",
                "running" => "rodando",
                _ => "feito",
            });
            p.set_size_request(6, 6);
            p.set_valign(Align::Center);
            cab.append(&p);
        }
        b.append(&cab);
        let t = rotulo(title, "fluxo-no-titulo");
        t.set_xalign(0.0);
        t.set_wrap(true);
        t.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        t.set_lines(3);
        t.set_ellipsize(gtk::pango::EllipsizeMode::End);
        t.set_max_width_chars(10);
        b.append(&t);
        for (i, l) in lines.iter().take(3).chain(lines.iter().skip(3).filter(|l| l.wrap)).enumerate() {
            let x = rotulo(&l.text, if l.mono { "fluxo-no-mono" } else { "fluxo-no-linha" });
            x.set_xalign(0.0);
            if l.wrap {
                x.add_css_class("entrada");
                x.set_wrap(true);
                x.set_wrap_mode(gtk::pango::WrapMode::WordChar);
                x.set_lines(3);
                x.set_ellipsize(gtk::pango::EllipsizeMode::End);
                x.set_max_width_chars(10);
            } else if l.mono {
                x.set_ellipsize(gtk::pango::EllipsizeMode::End);
                x.set_max_width_chars(10);
            } else {
                x.set_wrap(true);
                x.set_lines(2);
                x.set_ellipsize(gtk::pango::EllipsizeMode::End);
                x.set_max_width_chars(10);
            }
            if i == 0 {
                x.add_css_class("primeira");
            }
            b.append(&x);
        }
        // after a test: the block's step in the run (under its words: the card keeps its width)
        if let Some(k) = d.step_no.get(id) {
            let st = rotulo(&trf("Step {n}", &[("n", k.to_string())]), "fluxo-no-passo");
            st.set_halign(Align::Start);
            st.set_margin_top(6);
            b.append(&st);
        }
        if let Some(n) = node {
            // several ways at once and no Join after them
            if d.flow.needs_join(id) {
                let row = caixa(Orientation::Horizontal, 6, "fluxo-no-aviso");
                row.append(&ui_icon("alerta", "#E5C98C", 12));
                let l = rotulo(tr("ways never meet: add a Join"), "fluxo-no-aviso-texto");
                l.set_wrap(true);
                l.set_max_width_chars(10);
                l.set_xalign(0.0);
                l.set_hexpand(true);
                row.append(&l);
                b.append(&row);
            }
            // one row per way out, last on the card: each port sits on the right side in line with its row (design
            // "portas com nome"; `m::port_points` puts them there)
            if m::named_ports(n) {
                let list = caixa(Orientation::Vertical, m::PORT_ROW_GAP as i32, "fluxo-no-saidas");
                if let Some(c) = m::ports_caption(n) {
                    let l = rotulo(c, "fluxo-no-saidas-modo");
                    l.set_xalign(0.0);
                    list.append(&l);
                }
                let mut rows = Vec::new();
                for p in m::out_ports(n) {
                    let row = caixa(Orientation::Horizontal, 6, "fluxo-no-saida");
                    rows.push(row.clone());
                    row.set_size_request(-1, m::PORT_ROW_H as i32);
                    row.set_tooltip_text(Some(&p.tip));
                    let dot = caixa(Orientation::Horizontal, 0, "fluxo-no-saida-ponto");
                    dot.set_size_request(5, 5);
                    dot.set_valign(Align::Center);
                    row.append(&dot);
                    let l = rotulo(&m::short_label(&p.label, m::PORT_LABEL_CHARS), "fluxo-no-mono");
                    l.set_ellipsize(gtk::pango::EllipsizeMode::End);
                    l.set_max_width_chars(10);
                    l.set_single_line_mode(true);
                    l.set_valign(Align::Center);
                    l.set_xalign(0.0);
                    l.set_hexpand(true);
                    row.append(&l);
                    list.append(&row);
                }
                self.port_rows.borrow_mut().insert(id.to_string(), rows);
                b.append(&list);
            }
        }
        b
    }

    fn rect_of(&self, id: &str) -> Option<Rect> {
        self.nodes.borrow().iter().find(|(i, _, _)| i == id).map(|(_, _, r)| *r)
    }

    /// Each block's ports where they sit now, with their ways (not the triggers' nor an End's).
    fn ports(&self) -> Vec<(String, Vec<((f64, f64), Option<String>)>)> {
        let d = self.data.borrow();
        self.nodes
            .borrow()
            .iter()
            .filter_map(|(id, _, r)| {
                let n = d.flow.node(id).filter(|n| n.kind != "end" && n.kind != "sandbox")?;
                Some((id.clone(), m::grab_points(n, *r, &self.port_places(n, *r))))
            })
            .collect()
    }

    /// Every block's card on the canvas (the triggers' too; not a sandbox's dashed area, which holds blocks): what a
    /// connection going back and a label keep clear of.
    fn cards(&self) -> Vec<Rect> {
        let d = self.data.borrow();
        self.nodes.borrow().iter().filter(|(id, _, _)| d.flow.node(id).is_none_or(|n| n.kind != "sandbox")).map(|(_, _, r)| *r).collect()
    }

    /// Where a block's ports are: in line with its rows as the card shows them (measured once the card is on screen:
    /// a bigger text makes taller rows), else where the design puts them.
    fn port_places(&self, n: &m::NodeDoc, r: Rect) -> Vec<(f64, f64)> {
        let fallback = m::port_points(n, r);
        if !m::named_ports(n) {
            return fallback;
        }
        let rows = self.port_rows.borrow();
        let card = self.nodes.borrow().iter().find(|(id, _, _)| *id == n.id).map(|(_, w, _)| w.clone());
        let (Some(rows), Some(card)) = (rows.get(&n.id), card) else { return fallback };
        // (the card's own coordinates are the world's: the zoom is a transform outside it)
        // (from the card's outer top: its own coordinates start inside its border and padding)
        let Some(top) = card.compute_bounds(&card).map(|b| b.y() as f64) else { return fallback };
        let middles: Option<Vec<f64>> = rows.iter().map(|row| row.compute_bounds(&card).filter(|b| b.height() > 0.0).map(|b| m::row_middle(top, b.y() as f64, b.height() as f64))).collect();
        match middles {
            Some(m) if m.len() == fallback.len() => m::port_points_from_rows(r, &m),
            _ => fallback,
        }
    }

    /// Where every connection drawn runs (with whether it is a trigger's): by its way's port out of its source, into
    /// its target's in port, around every card (`flow_router`); kept while nothing changes, and while a block is
    /// dragged only the routes it touches are worked out again.
    fn geometries(&self) -> Vec<(EdgeDoc, bool, m::Route, m::EdgePath)> {
        let edges = self.drawn_edges();
        let cards = self.cards();
        let d = self.data.borrow();
        let mut kept = Vec::new();
        let mut links = Vec::new();
        for (e, trigger) in edges {
            let (Some(a), Some(b)) = (self.rect_of(&e.source), self.rect_of(&e.target)) else { continue };
            let node = d.flow.node(&e.source);
            let ports = node.map(|n| self.port_places(n, a)).unwrap_or_default();
            let r = m::edge_route_at(node, e.handle.as_deref(), a, b, &ports);
            links.push(((e.source.clone(), e.target.clone(), e.handle.clone()), crate::flow_router::Link { route: r, src: a, dst: b }));
            kept.push((e, trigger, r));
        }
        let mut cache = self.routes.borrow_mut();
        let paths = cache.update(links, cards).to_vec();
        kept.into_iter().zip(paths).map(|((e, t, r), p)| (e, t, r, p)).collect()
    }

    /// The tip of the port under the pointer (what that way out does).
    fn port_tip(&self, world: (f64, f64)) -> Option<String> {
        let (id, way) = m::hit_port(&self.ports(), world, 7.0 / self.view.get().zoom)?;
        let d = self.data.borrow();
        let ports = m::out_ports(d.flow.node(&id)?);
        ports.into_iter().find(|p| p.handle == way).map(|p| p.tip)
    }

    /// The connections drawn: the flow's and each trigger's to the block the run starts at.
    fn drawn_edges(&self) -> Vec<(EdgeDoc, bool)> {
        let d = self.data.borrow();
        let mut v: Vec<(EdgeDoc, bool)> = d.flow.edges.iter().map(|e| (e.clone(), false)).collect();
        if let Some(entry) = d.flow.entry() {
            for t in &d.triggers {
                v.push((EdgeDoc { source: format!("{}{}", m::TRIGGER_PREFIX, t.id), target: entry.id.clone(), handle: None }, true));
            }
        }
        v
    }

    fn draw_edges(&self, cr: &gtk::cairo::Context) {
        let v = self.view.get();
        let z = v.zoom;
        let d = self.data.borrow();
        let teal = rgba("#8ED2C3", 1.0);
        let grey = rgba("#6E6F7A", 1.0);
        let gold = rgba("#E5C98C", 1.0);
        let purple = rgba("#C7A7DC", 1.0);
        let accent = rgba("#A5A1E2", 1.0);
        let bg = rgba("#16171B", 1.0);
        let chip_bg = rgba("#121317", 1.0);
        let chip_line = rgba("#26272F", 1.0);
        let chosen = self.edge.borrow().clone();
        // the sandboxes' dashed areas, broken where their title sits
        for n in d.flow.nodes.iter().filter(|n| n.kind == "sandbox") {
            let Some(r) = self.rect_of(&n.id) else { continue };
            let sel = d.selected.as_deref() == Some(n.id.as_str()) || d.chosen.contains(&n.id);
            let (a, b) = (v.to_screen((r.x, r.y + 8.0)), v.to_screen((r.x + r.w, r.y + r.h)));
            let cap = self.captions.borrow().get(&n.id).copied().unwrap_or(0.0) * z;
            let issue = d.issues.contains_key(&n.id);
            // (the run's word on it: failed coral, done teal)
            let line = match d.states.get(&n.id).map(String::as_str) {
                _ if issue => rgba("#EFA2A6", 1.0),
                Some("failed") => rgba("#EFA2A6", 1.0),
                Some("done") => teal,
                _ => (accent.0, accent.1, accent.2, if n.disabled { 0.45 } else { 1.0 }),
            };
            set(cr, line);
            cr.set_line_width(if sel { 2.0 } else { 1.5 } * z.max(0.6));
            cr.set_dash(&[5.0 * z.max(0.6), 4.0 * z.max(0.6)], 0.0);
            let gap = (a.0 + 10.0 * z, a.0 + 14.0 * z + cap);
            cr.new_path();
            cr.move_to(gap.1.min(b.0), a.1);
            cr.line_to(b.0, a.1);
            cr.line_to(b.0, b.1);
            cr.line_to(a.0, b.1);
            cr.line_to(a.0, a.1);
            cr.line_to(gap.0, a.1);
            let _ = cr.stroke();
            cr.set_dash(&[], 0.0);
        }
        let geometries = self.geometries();
        let cards = self.cards();
        let fanout = |s: &str| d.flow.edges.iter().filter(|e| e.source == s).count();
        let mut ports: Vec<((f64, f64), (f64, f64, f64, f64))> = Vec::new();
        let _ = &bg;
        // (each label with the places it may go, best first, in the world: (spot, sits to its left))
        let mut labels: Vec<(Vec<((f64, f64), bool)>, String, Rgba)> = Vec::new();
        let moving = match &*self.drag.borrow() {
            Drag::Reconnect { edge, .. } => Some(edge.clone()),
            _ => None,
        };
        for (e, from_trigger, r, path) in &geometries {
            if moving.as_ref() == Some(e) {
                continue;
            }
            let (r, path) = (*r, path);
            let travelled = d.travelled.contains(&(e.source.clone(), e.target.clone())) || (*from_trigger && d.states.contains_key(&e.target));
            let waiting = d.states.get(&e.target).map(String::as_str) == Some("waiting") && travelled;
            let selected = chosen.as_ref() == Some(e);
            let color = if selected {
                accent
            } else if waiting {
                gold
            } else if e.handle.as_deref() == Some(m::LOW_CONFIDENCE) {
                // (the way a decision takes when it is not sure: the person decides — design "Bloco Decidir")
                gold
            } else if r.back {
                purple
            } else if travelled {
                teal
            } else {
                grey
            };
            cr.new_path();
            for (i, seg) in path.segments.iter().enumerate() {
                let p: Vec<(f64, f64)> = seg.iter().map(|q| v.to_screen(*q)).collect();
                if i == 0 {
                    cr.move_to(p[0].0, p[0].1);
                }
                cr.curve_to(p[1].0, p[1].1, p[2].0, p[2].1, p[3].0, p[3].1);
            }
            set(cr, color);
            cr.set_line_width(if selected { 2.0 } else { 1.5 } * z.max(0.6));
            if waiting || r.back {
                cr.set_dash(&[3.0 * z.max(0.6), 3.0 * z.max(0.6)], 0.0);
            } else {
                cr.set_dash(&[], 0.0);
            }
            let _ = cr.stroke();
            cr.set_dash(&[], 0.0);
            // (the source's out port and the target's in port, wherever the blocks sit)
            ports.push((v.to_screen(r.from), color));
            ports.push((v.to_screen(r.to), color));
            // the way's label (a run's numbers when it has them)
            let src = d.flow.node(&e.source);
            let src_kind = src.map(|n| n.kind.as_str()).unwrap_or("");
            let named = src.map(|n| m::way_name(n, e.handle.as_deref())).unwrap_or_else(|| m::way_label(src_kind, e.handle.as_deref()));
            let mut text = e.handle.as_ref().and_then(|h| d.labels.get(&(e.source.clone(), h.clone())).cloned()).unwrap_or_else(|| m::short_label(&named, m::PORT_LABEL_CHARS));
            // (a For each's "done" only reads when it also has its "empty list" way)
            if src_kind == "for_each" && e.handle.is_none() && !d.flow.edges.iter().any(|x| x.source == e.source && x.handle.as_deref() == Some(m::EMPTY_LIST)) {
                text.clear();
            }
            // (a named port says its way next to it: the line repeats it only going back, far from the port, or with
            // a run's numbers — design "portas com nome", "ligação para trás")
            let numbers = e.handle.as_ref().is_some_and(|h| d.labels.contains_key(&(e.source.clone(), h.clone())));
            if src.is_some_and(m::named_ports) && !r.back && !numbers {
                text.clear();
            }
            if !text.is_empty() {
                // (going back: halfway along its run around the blocks)
                // (along the line, where it covers no card)
                // (a routed connection: halfway along its longest stretch; a curve: along it)
                let spots: Vec<((f64, f64), bool)> = if r.back || !path.points.is_empty() {
                    vec![(path.label_at, false)]
                } else {
                    let t0 = if fanout(&e.source) >= 3 { 0.86 } else { 0.5 };
                    [t0, 0.4, 0.6, 0.3, 0.7, 0.2, 0.8].iter().map(|t| (m::point_at(&path.segments[0], *t), false)).collect()
                };
                let tc = if r.back {
                    purple
                } else if e.handle.as_deref() == Some(m::LOW_CONFIDENCE) {
                    gold
                } else if travelled {
                    teal
                } else {
                    rgba("#8E8F99", 1.0)
                };
                labels.push((spots, text, tc));
            }
        }
        // a For each's loop back (design "Fluxo · construtor": its agent runs once per item, inside the block; the
        // dashed curve from the block after it says so): faint until a run goes through it
        for n in d.flow.nodes.iter().filter(|n| n.kind == "for_each") {
            let Some(next) = d.flow.edges.iter().find(|e| e.source == n.id && e.handle.is_none() && !e.target.starts_with(m::TRIGGER_PREFIX)).map(|e| e.target.clone()) else { continue };
            if d.flow.edges.iter().any(|e| e.source == next && e.target == n.id) {
                continue;
            }
            let (Some(f), Some(a)) = (self.rect_of(&n.id), self.rect_of(&next)) else { continue };
            let (p0, p3) = (a.left_mid(), f.left_mid());
            let bend = p0.0.min(p3.0) - 26.0 - (p0.1 - p3.1).abs() * 0.18;
            let bz = [p0, (bend, p0.1), (bend, p3.1), p3];
            let alpha = if d.states.contains_key(&n.id) { 1.0 } else { 0.45 };
            let c = (purple.0, purple.1, purple.2, alpha);
            let p: Vec<(f64, f64)> = bz.iter().map(|q| v.to_screen(*q)).collect();
            cr.new_path();
            cr.move_to(p[0].0, p[0].1);
            cr.curve_to(p[1].0, p[1].1, p[2].0, p[2].1, p[3].0, p[3].1);
            set(cr, c);
            cr.set_line_width(1.5 * z.max(0.6));
            cr.set_dash(&[3.0 * z.max(0.6), 3.0 * z.max(0.6)], 0.0);
            let _ = cr.stroke();
            cr.set_dash(&[], 0.0);
            let word = m::item_word(n.data["items"].as_str().unwrap_or("content"));
            // (while it runs, the item it is on; once done, all of them)
            let at = d.progress.get(&n.id).copied().or_else(|| d.items.get(&n.id).map(|k| (*k, *k)));
            // (to the left of the curve near where it arrives; further along it, or over it, when a card is there)
            let ts = [0.88, 0.75, 0.62, 0.5, 0.38, 0.25, 0.12];
            let spots: Vec<((f64, f64), bool)> = ts.iter().map(|t| (m::point_at(&bz, *t), true)).chain(ts.iter().map(|t| (m::point_at(&bz, *t), false))).collect();
            labels.push((spots, format!("↻ {}", m::loop_label(&word, at)), c));
        }
        // the band being drawn on the empty canvas
        if let Drag::Band { from, at } = &*self.drag.borrow() {
            let (a, b) = (v.to_screen(*from), v.to_screen(*at));
            // (dashed, without a fill: design "seleção e menus")
            cr.rectangle(a.0.min(b.0), a.1.min(b.1), (a.0 - b.0).abs(), (a.1 - b.1).abs());
            set(cr, accent);
            cr.set_line_width(1.0);
            cr.set_dash(&[4.0, 3.0], 0.0);
            let _ = cr.stroke();
            cr.set_dash(&[], 0.0);
        }
        // a connection being drawn (or its end moved to another block)
        let drawing = match &*self.drag.borrow() {
            Drag::Connect { from, way, at } => Some((from.clone(), way.clone().flatten(), *at)),
            Drag::Reconnect { edge, at } => Some((edge.source.clone(), edge.handle.clone(), *at)),
            _ => None,
        };
        if let Some((from, way, at)) = drawing.as_ref() {
            if let Some(a) = self.rect_of(from) {
                let node = d.flow.node(from);
                let target = m::hit_node(&self.rects(), *at).filter(|t| t != from).and_then(|t| self.rect_of(t));
                let segments = match target {
                    Some(b) => {
                        let ports = node.map(|n| self.port_places(n, a)).unwrap_or_default();
                        let r = m::edge_route_at(node, way.as_deref(), a, b, &ports);
                        crate::flow_router::route_all(&[crate::flow_router::Link { route: r, src: a, dst: b }], &cards, &[]).remove(0).segments
                    }
                    None => {
                        // (from the port it left by, to the pointer)
                        let ports = node.map(|n| self.port_places(n, a)).unwrap_or_default();
                        let start = m::edge_route_at(node, way.as_deref(), a, Rect { x: a.x + a.w + 1.0, y: a.y, w: 0.0, h: a.h }, &ports).from;
                        let r = m::Route { from: start, from_side: m::Side::Right, to: *at, to_side: m::Side::Left, back: false };
                        vec![m::bezier(&r)]
                    }
                };
                cr.new_path();
                for (i, seg) in segments.iter().enumerate() {
                    let p: Vec<(f64, f64)> = seg.iter().map(|q| v.to_screen(*q)).collect();
                    if i == 0 {
                        cr.move_to(p[0].0, p[0].1);
                        ports.push((p[0], accent));
                    }
                    cr.curve_to(p[1].0, p[1].1, p[2].0, p[2].1, p[3].0, p[3].1);
                }
                set(cr, accent);
                cr.set_line_width(1.5 * z.max(0.6));
                let _ = cr.stroke();
            }
        }
        // the chosen block's free ports (to drag a new connection from)
        if let Some(sel) = d.selected.as_ref().filter(|s| !s.starts_with(m::TRIGGER_PREFIX)) {
            if let Some(r) = self.rect_of(sel) {
                if let Some(n) = d.flow.node(sel).filter(|n| n.kind != "end") {
                    for (q, _) in m::grab_points(n, r, &self.port_places(n, r)) {
                        let s = v.to_screen(q);
                        if !ports.iter().any(|(p, _)| (p.0 - s.0).abs() < 2.0 && (p.1 - s.1).abs() < 2.0) {
                            ports.push((s, accent));
                        }
                    }
                }
            }
        }
        // every named port shows, faint while nothing leaves by it (design "portas com nome")
        let free = (grey.0, grey.1, grey.2, 0.8);
        for (id, list) in self.ports() {
            if d.flow.node(&id).is_some_and(m::named_ports) {
                for (q, _) in list {
                    let s = v.to_screen(q);
                    if !ports.iter().any(|(p, _)| (p.0 - s.0).abs() < 2.0 && (p.1 - s.1).abs() < 2.0) {
                        ports.push((s, free));
                    }
                }
            }
        }
        let radius = 4.5 * z.clamp(0.6, 1.4);
        for (p, c) in ports {
            cr.new_path();
            cr.arc(p.0, p.1, radius, 0.0, std::f64::consts::TAU);
            set(cr, bg);
            let _ = cr.fill_preserve();
            set(cr, c);
            cr.set_line_width(1.5);
            let _ = cr.stroke();
        }
        let layout = pangocairo::functions::create_layout(cr);
        let mut fd = gtk::pango::FontDescription::from_string("Inter Semi-Bold 8.25");
        // (the labels shrink with the blocks, down to where they still read)
        fd.set_absolute_size(11.0 * z.clamp(0.5, 1.4) * gtk::pango::SCALE as f64);
        layout.set_font_description(Some(&fd));
        // (too small to read, and bigger than the cards around them: no labels below LABELS_MIN_ZOOM)
        let labels = if z < m::LABELS_MIN_ZOOM { Vec::new() } else { labels };
        for (spots, text, c) in labels {
            layout.set_text(&text);
            let (w, h) = layout.pixel_size();
            let (w, h) = (w as f64, h as f64);
            let (px, py) = (6.0, 2.0);
            // (the chip's size in the world, a little gap to the left of a loop's spot)
            let (cw, ch) = ((w + 2.0 * px + 2.0) / z, (h + 2.0 * py) / z);
            // (where it would cover a card wherever it goes, it is left out)
            let Some(corner) = m::label_place(&spots, cw, ch, &cards) else { continue };
            let (x, y) = v.to_screen(corner);
            rounded(cr, x, y, w + 2.0 * px, h + 2.0 * py, 4.0);
            set(cr, chip_bg);
            let _ = cr.fill_preserve();
            set(cr, chip_line);
            cr.set_line_width(1.0);
            let _ = cr.stroke();
            set(cr, c);
            cr.move_to(x + px, y + py);
            pangocairo::functions::show_layout(cr, &layout);
        }
    }

    fn minimap_scale(&self, wd: f64, ht: f64) -> Option<(Rect, f64, (f64, f64))> {
        let b = m::bbox(self.nodes.borrow().iter().map(|(_, _, r)| *r))?;
        let b = Rect { x: b.x - 40.0, y: b.y - 40.0, w: b.w + 80.0, h: b.h + 80.0 };
        let s = (wd / b.w).min(ht / b.h);
        Some((b, s, ((wd - b.w * s) / 2.0, (ht - b.h * s) / 2.0)))
    }

    fn draw_minimap(&self, cr: &gtk::cairo::Context, wd: f64, ht: f64) {
        let Some((b, s, off)) = self.minimap_scale(wd, ht) else { return };
        set(cr, rgba("#3A3B46", 1.0));
        for (_, _, r) in self.nodes.borrow().iter() {
            cr.rectangle(off.0 + (r.x - b.x) * s, off.1 + (r.y - b.y) * s, (r.w * s).max(2.0), (r.h * s).max(2.0));
        }
        let _ = cr.fill();
        let v = self.view.get();
        let (w0, h0) = (self.root.width() as f64, self.root.height() as f64);
        let a = v.to_world((0.0, 0.0));
        let z = v.to_world((w0, h0));
        let (x0, y0) = ((off.0 + (a.0 - b.x) * s).max(0.5), (off.1 + (a.1 - b.y) * s).max(0.5));
        let (x1, y1) = ((off.0 + (z.0 - b.x) * s).min(wd - 0.5), (off.1 + (z.1 - b.y) * s).min(ht - 0.5));
        if x1 > x0 && y1 > y0 {
            rounded(cr, x0, y0, x1 - x0, y1 - y0, 2.0);
            set(cr, rgba("#A5A1E2", 1.0));
            cr.set_line_width(1.0);
            let _ = cr.stroke();
        }
    }

    /// Centres the view on the minimap's point.
    fn minimap_go(&self, p: (f64, f64)) {
        let (wd, ht) = (self.minimap.width() as f64, self.minimap.height() as f64);
        let Some((b, s, off)) = self.minimap_scale(wd, ht) else { return };
        let world = (b.x + (p.0 - off.0) / s, b.y + (p.1 - off.1) / s);
        let mut v = self.view.get();
        v.pan = (self.root.width() as f64 / 2.0 - world.0 * v.zoom, self.root.height() as f64 / 2.0 - world.1 * v.zoom);
        self.set_view(v);
    }

    /// The blocks a press on `id` moves: the chosen ones when it is one of them, else itself; a sandbox brings the
    /// blocks inside along.
    fn moving_set(&self, id: &str) -> Vec<(String, (f64, f64))> {
        let d = self.data.borrow();
        let mut ids: Vec<String> = if d.chosen.len() > 1 && d.chosen.iter().any(|c| c == id) { d.chosen.clone() } else { vec![id.to_string()] };
        let boxes: Vec<String> = ids.iter().filter(|i| d.flow.node(i).is_some_and(|n| n.kind == "sandbox")).cloned().collect();
        for n in d.flow.nodes.iter().filter(|n| n.parent.as_ref().is_some_and(|p| boxes.contains(p))) {
            if !ids.contains(&n.id) {
                ids.push(n.id.clone());
            }
        }
        drop(d);
        ids.into_iter().filter(|i| !i.starts_with(m::TRIGGER_PREFIX)).filter_map(|i| self.rect_of(&i).map(|r| (i, (r.x, r.y)))).collect()
    }

    /// The connection whose arriving end is under `p` (to drag it to another block).
    fn hit_edge_end(&self, p: (f64, f64), radius: f64) -> Option<EdgeDoc> {
        self.geometries().into_iter().filter(|(_, trigger, _, _)| !trigger).map(|(e, _, r, _)| (e, r.to)).find(|(_, to)| (to.0 - p.0).powi(2) + (to.1 - p.1).powi(2) <= radius * radius).map(|(e, _)| e)
    }

    fn connect_input(self: &Rc<Self>) {
        // drag: a port draws a connection, a connection's end goes to another block, a block moves (with the others
        // chosen), the empty canvas draws a band; without moving it is a click. The middle button pans.
        let g = gtk::GestureDrag::new();
        g.set_button(gdk::BUTTON_PRIMARY);
        let w = Rc::downgrade(self);
        g.connect_drag_begin(move |g, x, y| {
            let Some(c) = w.upgrade() else { return };
            c.root.grab_focus();
            c.moved.set(false);
            c.shift.set(g.current_event_state().contains(gdk::ModifierType::SHIFT_MASK));
            let world = c.view.get().to_world((x, y));
            let rects = c.rects();
            let r = 7.0 / c.view.get().zoom;
            let port = m::hit_port(&c.ports(), world, r);
            let end = if port.is_none() { c.hit_edge_end(world, r) } else { None };
            *c.drag.borrow_mut() = if let Some((from, way)) = port {
                // (a named port connects its own way)
                let named = c.data.borrow().flow.node(&from).is_some_and(m::named_ports);
                Drag::Connect { from, way: named.then_some(way), at: world }
            } else if let Some(edge) = end {
                Drag::Reconnect { edge, at: world }
            } else if let Some(id) = m::hit_node(&rects, world) {
                let id = id.to_string();
                Drag::Move { ids: c.moving_set(&id), id }
            } else {
                Drag::Band { from: world, at: world }
            };
        });
        let w = Rc::downgrade(self);
        g.connect_drag_update(move |g, dx, dy| {
            let Some(c) = w.upgrade() else { return };
            if !c.moved.get() && dx.abs() + dy.abs() < 4.0 {
                return;
            }
            c.moved.set(true);
            let z = c.view.get().zoom;
            let start = g.start_point().unwrap_or((0.0, 0.0));
            let mut d = c.drag.borrow_mut();
            match &mut *d {
                Drag::Pan { .. } | Drag::None => {}
                Drag::Move { ids, .. } => {
                    let ids = ids.clone();
                    drop(d);
                    // (the borrows end before each change)
                    for (id, s) in &ids {
                        let (nx, ny) = (s.0 + dx / z, s.1 + dy / z);
                        let pos = c.nodes.borrow().iter().position(|(x, _, _)| x == id);
                        if let Some(i) = pos {
                            let mut ns = c.nodes.borrow_mut();
                            ns[i].2.x = nx;
                            ns[i].2.y = ny;
                            let (wg, r) = (ns[i].1.clone(), ns[i].2);
                            drop(ns);
                            c.place(&wg, &r);
                        }
                        if let Some(n) = c.data.borrow_mut().flow.node_mut(id) {
                            n.x = nx;
                            n.y = ny;
                        }
                    }
                    // (the triggers follow the first block)
                    c.restack_triggers();
                    c.edges.queue_draw();
                    c.minimap.queue_draw();
                }
                Drag::Connect { at, .. } | Drag::Reconnect { at, .. } | Drag::Band { at, .. } => {
                    *at = c.view.get().to_world((start.0 + dx, start.1 + dy));
                    drop(d);
                    c.edges.queue_draw();
                }
            }
        });
        let w = Rc::downgrade(self);
        g.connect_drag_end(move |g, dx, dy| {
            let Some(c) = w.upgrade() else { return };
            let d = std::mem::replace(&mut *c.drag.borrow_mut(), Drag::None);
            // (a drag let go: the routes worked out anew, as for a flow just opened)
            c.routes.borrow_mut().clear();
            c.edges.queue_draw();
            let start = g.start_point().unwrap_or((0.0, 0.0));
            if !c.moved.get() {
                c.click(start);
                return;
            }
            let at = c.view.get().to_world((start.0 + dx, start.1 + dy));
            match d {
                Drag::Move { id, ids } => {
                    let flow = c.data.borrow().flow.clone();
                    let moved: Vec<String> = ids.iter().map(|(i, _)| i.clone()).collect();
                    let places: Vec<(String, f64, f64)> = ids.iter().filter_map(|(i, _)| c.rect_of(i).map(|r| (i.clone(), r.x, r.y))).collect();
                    // a block let go over a sandbox's dashed area goes into it; out of it, back into the flow
                    let areas: Vec<(String, Rect)> = flow.nodes.iter().filter(|n| n.kind == "sandbox" && !moved.contains(&n.id)).filter_map(|n| c.rect_of(&n.id).map(|r| (n.id.clone(), r))).collect();
                    let mut into = Vec::new();
                    for n in flow.nodes.iter().filter(|n| moved.contains(&n.id) && n.kind != "sandbox") {
                        // (the blocks inside a sandbox that moves along stay in it)
                        if n.parent.as_ref().is_some_and(|p| moved.contains(p)) {
                            continue;
                        }
                        let Some(r) = c.rect_of(&n.id) else { continue };
                        let to = m::sandbox_at(&areas, (r.x + r.w / 2.0, r.y + r.h / 2.0)).map(str::to_string);
                        if to != n.parent {
                            into.push((n.id.clone(), to));
                        }
                    }
                    c.emit(CanvasEvent::Moved(places, into));
                    // (a block moved alone is the chosen one: Delete, Duplicate and the inspector act on it)
                    let chosen = { let d = c.data.borrow(); d.selected.as_deref() == Some(id.as_str()) || d.chosen.contains(&id) };
                    if !chosen && !id.starts_with(m::TRIGGER_PREFIX) {
                        *c.edge.borrow_mut() = None;
                        c.emit(CanvasEvent::Select(Some(id)));
                    }
                }
                Drag::Connect { from, way, .. } => {
                    let target = m::hit_node(&c.rects(), at).map(str::to_string);
                    c.edges.queue_draw();
                    if let Some(t) = target.filter(|t| *t != from && !t.starts_with(m::TRIGGER_PREFIX)) {
                        c.emit(CanvasEvent::Connect(from, way, t));
                    }
                }
                Drag::Reconnect { edge, .. } => {
                    let target = m::hit_node(&c.rects(), at).map(str::to_string);
                    c.edges.queue_draw();
                    if let Some(t) = target.filter(|t| *t != edge.source && *t != edge.target && !t.starts_with(m::TRIGGER_PREFIX)) {
                        c.emit(CanvasEvent::Reconnect(edge, t));
                    }
                }
                Drag::Band { from, at } => {
                    let band = Rect { x: from.0.min(at.0), y: from.1.min(at.1), w: (from.0 - at.0).abs(), h: (from.1 - at.1).abs() };
                    let inside: Vec<String> = c
                        .rects()
                        .into_iter()
                        .filter(|(id, r)| !id.starts_with(m::TRIGGER_PREFIX) && r.x >= band.x && r.y >= band.y && r.x + r.w <= band.x + band.w && r.y + r.h <= band.y + band.h)
                        .map(|(id, _)| id)
                        .collect();
                    c.edges.queue_draw();
                    *c.edge.borrow_mut() = None;
                    c.emit(CanvasEvent::SelectMany(inside, c.shift.get()));
                }
                _ => {}
            }
        });
        self.root.add_controller(g);
        let pan = gtk::GestureDrag::new();
        pan.set_button(gdk::BUTTON_MIDDLE);
        let w = Rc::downgrade(self);
        pan.connect_drag_begin(move |_, _, _| {
            if let Some(c) = w.upgrade() {
                *c.drag.borrow_mut() = Drag::Pan { start: c.view.get().pan };
            }
        });
        let w = Rc::downgrade(self);
        pan.connect_drag_update(move |_, dx, dy| {
            let Some(c) = w.upgrade() else { return };
            let start = match &*c.drag.borrow() {
                Drag::Pan { start } => *start,
                _ => return,
            };
            let mut v = c.view.get();
            v.pan = (start.0 + dx, start.1 + dy);
            c.set_view(v);
        });
        let w = Rc::downgrade(self);
        pan.connect_drag_end(move |_, _, _| {
            if let Some(c) = w.upgrade() {
                *c.drag.borrow_mut() = Drag::None;
            }
        });
        self.root.add_controller(pan);
        // the right button: the block's menu, or the canvas' (design "seleção e menus")
        let menu = gtk::GestureClick::new();
        menu.set_button(gdk::BUTTON_SECONDARY);
        let w = Rc::downgrade(self);
        menu.connect_pressed(move |_, _, x, y| {
            let Some(c) = w.upgrade() else { return };
            c.root.grab_focus();
            let world = c.view.get().to_world((x, y));
            let id = m::hit_node(&c.rects(), world).filter(|i| !i.starts_with(m::TRIGGER_PREFIX)).map(str::to_string);
            c.emit(CanvasEvent::Menu(id, (x, y), world));
        });
        self.root.add_controller(menu);
        let twice = gtk::GestureClick::new();
        twice.set_button(gdk::BUTTON_PRIMARY);
        let w = Rc::downgrade(self);
        twice.connect_pressed(move |_, n, x, y| {
            let Some(c) = w.upgrade() else { return };
            if n == 2 {
                let world = c.view.get().to_world((x, y));
                if let Some(id) = m::hit_node(&c.rects(), world).filter(|i| !i.starts_with(m::TRIGGER_PREFIX)).map(str::to_string) {
                    c.emit(CanvasEvent::Activate(id));
                }
            }
        });
        self.root.add_controller(twice);
        // the wheel pans; with Ctrl it zooms where the pointer is
        let mov = gtk::EventControllerMotion::new();
        let w = Rc::downgrade(self);
        mov.connect_motion(move |_, x, y| {
            if let Some(c) = w.upgrade() {
                c.pointer.set((x, y));
                let world = c.view.get().to_world((x, y));
                let rects = c.rects();
                let r = 7.0 / c.view.get().zoom;
                let cursor = if m::hit_port(&c.ports(), world, r).is_some() || c.hit_edge_end(world, r).is_some() {
                    "crosshair"
                } else if m::hit_node(&rects, world).is_some() {
                    "grab"
                } else {
                    "default"
                };
                c.root.set_cursor_from_name(Some(cursor));
            }
        });
        self.root.add_controller(mov);
        // a port's tip: its way's whole name and what it does
        self.root.set_has_tooltip(true);
        let w = Rc::downgrade(self);
        self.root.connect_query_tooltip(move |_, x, y, _, tt| {
            let Some(c) = w.upgrade() else { return false };
            match c.port_tip(c.view.get().to_world((x as f64, y as f64))) {
                Some(t) => {
                    tt.set_text(Some(&t));
                    true
                }
                None => false,
            }
        });
        let sc = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
        let w = Rc::downgrade(self);
        sc.connect_scroll(move |sc, dx, dy| {
            let Some(c) = w.upgrade() else { return glib::Propagation::Proceed };
            let ctrl = sc.current_event_state().contains(gdk::ModifierType::CONTROL_MASK);
            let wheel = sc.unit() == gdk::ScrollUnit::Wheel;
            if ctrl {
                let f = if dy < 0.0 { 1.1 } else { 1.0 / 1.1 };
                c.set_view(c.view.get().zoom_at(c.pointer.get(), f));
            } else {
                let k = if wheel { 40.0 } else { 1.0 };
                let mut v = c.view.get();
                v.pan = (v.pan.0 - dx * k, v.pan.1 - dy * k);
                c.set_view(v);
            }
            glib::Propagation::Stop
        });
        self.root.add_controller(sc);
        // blocks dropped from the palette
        let drop = gtk::DropTarget::new(glib::types::Type::STRING, gdk::DragAction::COPY);
        let w = Rc::downgrade(self);
        drop.connect_drop(move |_, v, x, y| {
            let (Some(c), Ok(item)) = (w.upgrade(), v.get::<String>()) else { return false };
            let world = c.view.get().to_world((x, y));
            c.emit(CanvasEvent::Drop(item, world));
            true
        });
        self.root.add_controller(drop);
        // the minimap moves the view
        let mg = gtk::GestureDrag::new();
        let w = Rc::downgrade(self);
        mg.connect_drag_begin(move |_, x, y| {
            if let Some(c) = w.upgrade() {
                c.minimap_go((x, y));
            }
        });
        let w = Rc::downgrade(self);
        mg.connect_drag_update(move |g, dx, dy| {
            if let (Some(c), Some(s)) = (w.upgrade(), g.start_point()) {
                c.minimap_go((s.0 + dx, s.1 + dy));
            }
        });
        self.minimap.add_controller(mg);
    }

    fn restack_triggers(&self) {
        let d = self.data.borrow();
        let mut ns = self.nodes.borrow_mut();
        let mut i = 0;
        for (id, w, r) in ns.iter_mut() {
            if id.starts_with(m::TRIGGER_PREFIX) {
                *r = trigger_rect(&d.flow, i, r.h);
                i += 1;
                self.place(w, r);
            }
        }
    }

    /// A click: a block or a trigger is chosen; on a connection, the connection; on nothing, nothing.
    fn click(&self, p: (f64, f64)) {
        let world = self.view.get().to_world(p);
        let rects = self.rects();
        if let Some(id) = m::hit_node(&rects, world) {
            *self.edge.borrow_mut() = None;
            if self.shift.get() && !id.starts_with(m::TRIGGER_PREFIX) {
                self.emit(CanvasEvent::Toggle(id.to_string()));
            } else {
                self.emit(CanvasEvent::Select(Some(id.to_string())));
            }
            return;
        }
        let tol = 6.0 / self.view.get().zoom;
        let e = self.geometries().into_iter().filter(|(_, t, _, _)| !t).find(|(_, _, _, path)| m::distance_to_path(path, world) <= tol).map(|(e, _, _, _)| e);
        *self.edge.borrow_mut() = e.clone();
        self.edges.queue_draw();
        self.emit(CanvasEvent::Edge(e.clone()));
        if e.is_none() {
            self.emit(CanvasEvent::Select(None));
        }
    }

    /// The canvas' middle as a world point (where a block added by a click goes when nothing is chosen).
    pub fn center_world(&self) -> (f64, f64) {
        self.view.get().to_world((self.root.width() as f64 / 2.0, self.root.height() as f64 / 2.0))
    }

    /// Brings a block into view (when the inspector or a run points at it).
    pub fn reveal(&self, id: &str) {
        let Some(r) = self.rect_of(id) else { return };
        let v = self.view.get();
        let (a, b) = (v.to_screen((r.x, r.y)), v.to_screen((r.x + r.w, r.y + r.h)));
        let (wd, ht) = (self.root.width() as f64, self.root.height() as f64);
        if a.0 < 0.0 || a.1 < 0.0 || b.0 > wd || b.1 > ht {
            let mut v = v;
            v.pan = (wd / 2.0 - (r.x + r.w / 2.0) * v.zoom, ht / 2.0 - (r.y + r.h / 2.0) * v.zoom);
            self.set_view(v);
        }
    }
}

pub(crate) fn rounded(cr: &gtk::cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
    cr.arc(x + r, y + r, r, std::f64::consts::PI, 1.5 * std::f64::consts::PI);
    cr.close_path();
}
