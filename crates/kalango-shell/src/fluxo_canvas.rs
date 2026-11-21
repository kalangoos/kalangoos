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
use kalango_i18n::tr;
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
}

pub enum CanvasEvent {
    /// a block or a trigger ("trigger:<id>") chosen, or none
    Select(Option<String>),
    Moved(String, f64, f64),
    Connect(String, String),
    /// a palette item dropped at a world point
    Drop(String, (f64, f64)),
    /// a connection chosen (Delete removes it, "/" puts a block in it)
    Edge(Option<EdgeDoc>),
}

enum Drag {
    None,
    Pan { start: (f64, f64) },
    Move { id: String, start: (f64, f64) },
    Connect { from: String, at: (f64, f64) },
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
    pointer: Cell<(f64, f64)>,
    pub edge: RefCell<Option<EdgeDoc>>,
    fitted: Cell<bool>,
    pub on_event: RefCell<Option<Rc<dyn Fn(CanvasEvent)>>>,
}

/// Where the triggers sit: to the left of the block the run starts at, one under the other.
pub fn trigger_rect(flow: &FlowDoc, i: usize, h: f64) -> Rect {
    let (x, y) = flow.entry().map(|e| (e.x - 228.0, e.y)).unwrap_or((40.0, 60.0));
    Rect { x, y: y + i as f64 * (h + 40.0), w: m::node_width("trigger"), h }
}

fn rgba(hex: &str, a: f64) -> (f64, f64, f64, f64) {
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
            nodes: RefCell::default(), drag: RefCell::new(Drag::None), moved: Cell::new(false), pointer: Cell::new((0.0, 0.0)), edge: RefCell::default(),
            fitted: Cell::new(false), on_event: RefCell::default(),
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
    }

    /// Shows the flow again from the start (a new flow: fit it once more).
    pub fn reset_fit(&self) {
        self.fitted.set(false);
    }

    pub fn rects(&self) -> Vec<(String, Rect)> {
        self.nodes.borrow().iter().map(|(id, _, r)| (id.clone(), *r)).collect()
    }

    /// Draws the flow anew.
    pub fn set_data(self: &Rc<Self>, d: CanvasData) {
        *self.data.borrow_mut() = d;
        for (_, w, _) in self.nodes.borrow_mut().drain(..) {
            self.fixed.remove(&w);
        }
        let d = self.data.borrow();
        let mut out = Vec::new();
        // the flow's input fields, on the start: its triggers, or the block a run starts at when it has none
        let input = m::inputs_line(&d.flow.inputs()).map(|text| m::Line { text, mono: true, wrap: true });
        let entry = d.flow.entry().map(|n| n.id.clone()).filter(|_| d.triggers.is_empty());
        for n in &d.flow.nodes {
            let (title, mut lines) = m::node_text(n, &d.agents, d.items.get(&n.id).copied());
            if entry.as_deref() == Some(n.id.as_str()) {
                lines.extend(input.clone());
            }
            let look = m::look(&n.kind);
            let w = self.node_widget(&n.id, look.caps, look.glyph, look.tone, &title, &lines, &d);
            let width = m::node_width(&n.kind);
            out.push((n.id.clone(), w, Rect { x: n.x, y: n.y, w: width, h: 0.0 }));
        }
        let mut th = 96.0;
        let mut trig = Vec::new();
        for t in &d.triggers {
            let (title, mut lines) = m::trigger_text(&t.config);
            lines.extend(input.clone());
            let id = format!("{}{}", m::TRIGGER_PREFIX, t.id);
            let glyph = m::trigger_glyph(t.config["kind"].as_str().unwrap_or(""));
            let w = self.node_widget(&id, m::look("trigger").caps, glyph, m::Tone::Gold, &title, &lines, &d);
            trig.push((id, w));
        }
        for (id, w, r) in out.iter_mut() {
            self.fixed.put(w, 0.0, 0.0);
            r.h = w.measure(Orientation::Vertical, r.w as i32).0 as f64;
            let _ = id;
        }
        for (i, (id, w)) in trig.into_iter().enumerate() {
            self.fixed.put(&w, 0.0, 0.0);
            th = w.measure(Orientation::Vertical, m::node_width("trigger") as i32).0 as f64;
            out.push((id, w, trigger_rect(&d.flow, i, th)));
        }
        let _ = th;
        for (_, w, r) in &out {
            self.place(w, r);
        }
        drop(d);
        *self.nodes.borrow_mut() = out;
        if !self.fitted.get() && self.root.width() > 10 && !self.nodes.borrow().is_empty() {
            self.fitted.set(true);
            self.fit();
        }
        self.edges.queue_draw();
        self.minimap.queue_draw();
    }

    #[allow(clippy::too_many_arguments)]
    fn node_widget(&self, id: &str, caps: &'static str, glyph: &str, tone: m::Tone, title: &str, lines: &[m::Line], d: &CanvasData) -> gtk::Box {
        let b = caixa(Orientation::Vertical, 0, "fluxo-no");
        let is_trigger = id.starts_with(m::TRIGGER_PREFIX);
        let kind = if is_trigger { "trigger".to_string() } else { d.flow.node(id).map(|n| n.kind.clone()).unwrap_or_default() };
        b.set_size_request(m::node_width(&kind) as i32, -1);
        if d.selected.as_deref() == Some(id) {
            b.add_css_class("escolhido");
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
        b
    }

    fn rect_of(&self, id: &str) -> Option<Rect> {
        self.nodes.borrow().iter().find(|(i, _, _)| i == id).map(|(_, _, r)| *r)
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
        let edges = self.drawn_edges();
        let fanout = |s: &str| d.flow.edges.iter().filter(|e| e.source == s).count();
        let mut ports: Vec<((f64, f64), (f64, f64, f64, f64))> = Vec::new();
        let mut labels: Vec<((f64, f64), String, (f64, f64, f64, f64), bool)> = Vec::new();
        for (e, from_trigger) in &edges {
            let (Some(a), Some(b)) = (self.rect_of(&e.source), self.rect_of(&e.target)) else { continue };
            let r = m::route(a, b);
            let bz = m::bezier(&r);
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
            let p: Vec<(f64, f64)> = bz.iter().map(|q| v.to_screen(*q)).collect();
            cr.new_path();
            cr.move_to(p[0].0, p[0].1);
            cr.curve_to(p[1].0, p[1].1, p[2].0, p[2].1, p[3].0, p[3].1);
            set(cr, color);
            cr.set_line_width(if selected { 2.0 } else { 1.5 } * z.max(0.6));
            if waiting || r.back {
                cr.set_dash(&[3.0 * z.max(0.6), 3.0 * z.max(0.6)], 0.0);
            } else {
                cr.set_dash(&[], 0.0);
            }
            let _ = cr.stroke();
            cr.set_dash(&[], 0.0);
            if !r.back {
                ports.push((p[0], color));
            }
            ports.push((p[3], color));
            // the way's label (a run's numbers when it has them)
            let src_kind = d.flow.node(&e.source).map(|n| n.kind.as_str()).unwrap_or("");
            let text = e.handle.as_ref().and_then(|h| d.labels.get(&(e.source.clone(), h.clone())).cloned()).unwrap_or_else(|| m::way_label(src_kind, e.handle.as_deref()));
            if !text.is_empty() {
                let t = if r.back { 0.12 } else if fanout(&e.source) >= 3 { 0.86 } else { 0.5 };
                let q = v.to_screen(m::point_at(&bz, t));
                let tc = if r.back {
                    purple
                } else if e.handle.as_deref() == Some(m::LOW_CONFIDENCE) {
                    gold
                } else if travelled {
                    teal
                } else {
                    rgba("#8E8F99", 1.0)
                };
                labels.push((q, if r.back { format!("↻ {text}") } else { text }, tc, r.back));
            }
        }
        // a connection being drawn
        if let Drag::Connect { from, at } = &*self.drag.borrow() {
            if let Some(a) = self.rect_of(from) {
                let target = m::hit_node(&self.rects(), *at).filter(|t| t != from).and_then(|t| self.rect_of(t));
                let (p0, p1) = match target {
                    Some(b) => {
                        let r = m::route(a, b);
                        (r, m::bezier(&r))
                    }
                    None => {
                        let r = m::Route { from: a.right_mid(), from_side: m::Side::Right, to: *at, to_side: m::Side::Left, back: false };
                        (r, m::bezier(&r))
                    }
                };
                let _ = p0;
                let p: Vec<(f64, f64)> = p1.iter().map(|q| v.to_screen(*q)).collect();
                cr.move_to(p[0].0, p[0].1);
                cr.curve_to(p[1].0, p[1].1, p[2].0, p[2].1, p[3].0, p[3].1);
                set(cr, accent);
                cr.set_line_width(1.5 * z.max(0.6));
                let _ = cr.stroke();
                ports.push((p[0], accent));
            }
        }
        // the chosen block's free ports (to drag a new connection from)
        if let Some(sel) = d.selected.as_ref().filter(|s| !s.starts_with(m::TRIGGER_PREFIX)) {
            if let Some(r) = self.rect_of(sel) {
                if d.flow.node(sel).is_some_and(|n| n.kind != "end") {
                    for q in m::out_ports(r) {
                        let s = v.to_screen(q);
                        if !ports.iter().any(|(p, _)| (p.0 - s.0).abs() < 2.0 && (p.1 - s.1).abs() < 2.0) {
                            ports.push((s, accent));
                        }
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
        fd.set_absolute_size(11.0 * z.clamp(0.7, 1.4) * gtk::pango::SCALE as f64);
        layout.set_font_description(Some(&fd));
        for (q, text, c, back) in labels {
            layout.set_text(&text);
            let (w, h) = layout.pixel_size();
            let (w, h) = (w as f64, h as f64);
            let (px, py) = (6.0, 2.0);
            // (a loop's label sits to the left of where it arrives)
            let (x, y) = if back { (q.0 - w - 2.0 * px - 2.0, q.1 - h / 2.0 - py) } else { (q.0 - w / 2.0 - px, q.1 - h / 2.0 - py) };
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

    fn connect_input(self: &Rc<Self>) {
        // drag: a port draws a connection, a block moves, the empty canvas pans; without moving it is a click
        let g = gtk::GestureDrag::new();
        g.set_button(gdk::BUTTON_PRIMARY);
        let w = Rc::downgrade(self);
        g.connect_drag_begin(move |_, x, y| {
            let Some(c) = w.upgrade() else { return };
            c.root.grab_focus();
            c.moved.set(false);
            let world = c.view.get().to_world((x, y));
            let rects = c.rects();
            let r = 7.0 / c.view.get().zoom;
            let port = m::hit_port(&rects, world, r).filter(|id| !id.starts_with(m::TRIGGER_PREFIX)).filter(|id| c.data.borrow().flow.node(id).is_some_and(|n| n.kind != "end")).map(str::to_string);
            *c.drag.borrow_mut() = if let Some(from) = port {
                Drag::Connect { from, at: world }
            } else if let Some(id) = m::hit_node(&rects, world) {
                let r = c.rect_of(id).unwrap();
                Drag::Move { id: id.to_string(), start: (r.x, r.y) }
            } else {
                Drag::Pan { start: c.view.get().pan }
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
                Drag::Pan { start: p } => {
                    let mut v = c.view.get();
                    v.pan = (p.0 + dx, p.1 + dy);
                    drop(d);
                    c.set_view(v);
                }
                Drag::Move { id, start: s } => {
                    if id.starts_with(m::TRIGGER_PREFIX) {
                        return;
                    }
                    let (nx, ny) = (s.0 + dx / z, s.1 + dy / z);
                    let id = id.clone();
                    drop(d);
                    // (the borrow ends before the change: an `if let` on it would hold it through its body)
                    let pos = c.nodes.borrow().iter().position(|(x, _, _)| *x == id);
                    if let Some(i) = pos {
                        let mut ns = c.nodes.borrow_mut();
                        ns[i].2.x = nx;
                        ns[i].2.y = ny;
                        let (wg, r) = (ns[i].1.clone(), ns[i].2);
                        drop(ns);
                        c.place(&wg, &r);
                    }
                    if let Some(n) = c.data.borrow_mut().flow.node_mut(&id) {
                        n.x = nx;
                        n.y = ny;
                    }
                    // (the triggers follow the first block)
                    c.restack_triggers();
                    c.edges.queue_draw();
                    c.minimap.queue_draw();
                }
                Drag::Connect { at, .. } => {
                    *at = c.view.get().to_world((start.0 + dx, start.1 + dy));
                    drop(d);
                    c.edges.queue_draw();
                }
                Drag::None => {}
            }
        });
        let w = Rc::downgrade(self);
        g.connect_drag_end(move |g, dx, dy| {
            let Some(c) = w.upgrade() else { return };
            let d = std::mem::replace(&mut *c.drag.borrow_mut(), Drag::None);
            let start = g.start_point().unwrap_or((0.0, 0.0));
            if !c.moved.get() {
                c.click(start);
                return;
            }
            match d {
                Drag::Move { id, .. } if !id.starts_with(m::TRIGGER_PREFIX) => {
                    if let Some(r) = c.rect_of(&id) {
                        c.emit(CanvasEvent::Moved(id, r.x, r.y));
                    }
                }
                Drag::Connect { from, .. } => {
                    let at = c.view.get().to_world((start.0 + dx, start.1 + dy));
                    let target = m::hit_node(&c.rects(), at).map(str::to_string);
                    c.edges.queue_draw();
                    if let Some(t) = target.filter(|t| *t != from && !t.starts_with(m::TRIGGER_PREFIX)) {
                        c.emit(CanvasEvent::Connect(from, t));
                    }
                }
                _ => {}
            }
        });
        self.root.add_controller(g);
        // the wheel pans; with Ctrl it zooms where the pointer is
        let mov = gtk::EventControllerMotion::new();
        let w = Rc::downgrade(self);
        mov.connect_motion(move |_, x, y| {
            if let Some(c) = w.upgrade() {
                c.pointer.set((x, y));
                let world = c.view.get().to_world((x, y));
                let rects = c.rects();
                let cursor = if m::hit_port(&rects, world, 7.0 / c.view.get().zoom).is_some() {
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
            self.emit(CanvasEvent::Select(Some(id.to_string())));
            return;
        }
        let tol = 6.0 / self.view.get().zoom;
        let hit = self.drawn_edges().into_iter().filter(|(_, t)| !t).find(|(e, _)| {
            let (Some(a), Some(b)) = (self.rect_of(&e.source), self.rect_of(&e.target)) else { return false };
            m::distance_to(&m::bezier(&m::route(a, b)), world) <= tol
        });
        let e = hit.map(|(e, _)| e);
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
