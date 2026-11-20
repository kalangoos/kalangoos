//! "Janelas lado a lado" (design 39375): between windows snapped side by side (a layout of the picker, or dragged to
//! the edges), a divider. Dragging it resizes the neighbours together — every window on one side of the line and
//! every one on the other, following the pointer 1:1 —, and a double click makes them equal. The windows are niri's
//! snapped (floating) windows, placed by `SnapWindow` in thousandths of the area they fill; the arrangement stays in
//! its workspace (the Space) and is there again when you come back to it.
//!
//! Where the dividers go is read from niri (every 400 ms, off the interface's thread): the windows of the focused
//! workspace that are floating, and the lines where one's edge meets another's.

use crate::servicos;
use crate::superficies::{caixa, camada};
use gtk::prelude::*;
use gtk::{gio, glib};
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use serde_json::json;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

/// The visible pill (design: 6 × 60, radius 3) and the room around it the pointer can grab.
const PILL_W: i32 = 6;
const PILL_L: i32 = 60;
const GRAB: i32 = 16;
/// A window is never made narrower (or shorter) than this by a divider.
const MIN: f64 = 240.0;
/// Edges this close count as meeting (niri's sizes are whole pixels; the gaps are 0).
const TOUCH: f64 = 2.5;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    fn right(&self) -> f64 {
        self.x + self.w
    }
    fn bottom(&self) -> f64 {
        self.y + self.h
    }
}

/// A line where windows meet: `vertical` (side by side, the line at x = `pos`) or horizontal (stacked, at y = `pos`);
/// `span` is the stretch along the line where both sides have windows; `before` are the windows left of (or above)
/// it, `after` right of (or below) it.
#[derive(Clone, Debug, PartialEq)]
pub struct Divider {
    pub vertical: bool,
    pub pos: f64,
    pub span: (f64, f64),
    pub before: Vec<(u64, Rect)>,
    pub after: Vec<(u64, Rect)>,
    /// what the windows fill together (the thousandths of SnapWindow are of this)
    pub area: Rect,
}

/// The dividers between these windows (id, rectangle in the output's logical pixels). `size`: the working area's
/// size (the screen less the top bar, the dock and pinned bars); it starts where the windows start (some window of a
/// layout is always at its left and at its top). The windows' own extent is not used for it: an app with a larger
/// minimum size (the calculator) runs past its region.
pub fn dividers(wins: &[(u64, Rect)], size: (f64, f64)) -> Vec<Divider> {
    if wins.len() < 2 || size.0 < 1.0 || size.1 < 1.0 {
        return vec![];
    }
    let area = {
        let x0 = wins.iter().map(|(_, r)| r.x).fold(f64::MAX, f64::min);
        let y0 = wins.iter().map(|(_, r)| r.y).fold(f64::MAX, f64::min);
        Rect { x: x0, y: y0, w: size.0, h: size.1 }
    };
    let mut out: Vec<Divider> = Vec::new();
    for vertical in [true, false] {
        // every edge where a window ends: the windows ending there and those starting there, with their stretch
        let end = |r: &Rect| if vertical { r.right() } else { r.bottom() };
        let start = |r: &Rect| if vertical { r.x } else { r.y };
        let along = |r: &Rect| if vertical { (r.y, r.bottom().min(area.bottom())) } else { (r.x, r.right().min(area.right())) };
        let mut lines: Vec<f64> = Vec::new();
        for (_, r) in wins {
            let e = end(r);
            if !lines.iter().any(|l| (l - e).abs() < TOUCH) {
                lines.push(e);
            }
        }
        for pos in lines {
            let before: Vec<(u64, Rect)> = wins.iter().filter(|(_, r)| (end(r) - pos).abs() < TOUCH).cloned().collect();
            let after: Vec<(u64, Rect)> = wins.iter().filter(|(_, r)| (start(r) - pos).abs() < TOUCH).cloned().collect();
            if before.is_empty() || after.is_empty() {
                continue;
            }
            let lo = |v: &[(u64, Rect)]| v.iter().map(|(_, r)| along(r).0).fold(f64::MAX, f64::min);
            let hi = |v: &[(u64, Rect)]| v.iter().map(|(_, r)| along(r).1).fold(f64::MIN, f64::max);
            let span = (lo(&before).max(lo(&after)), hi(&before).min(hi(&after)));
            if span.1 - span.0 < 40.0 {
                continue;
            }
            // only the windows that face the line across the shared stretch
            let faces = |r: &Rect| along(r).0 < span.1 - 1.0 && along(r).1 > span.0 + 1.0;
            let before: Vec<_> = before.into_iter().filter(|(_, r)| faces(r)).collect();
            let after: Vec<_> = after.into_iter().filter(|(_, r)| faces(r)).collect();
            out.push(Divider { vertical, pos, span, before, after, area });
        }
    }
    out
}

/// Where the line may go: each window keeps at least `MIN`.
pub fn clamp(d: &Divider, pos: f64) -> f64 {
    let lo = d.before.iter().map(|(_, r)| if d.vertical { r.x } else { r.y }).fold(f64::MIN, f64::max) + MIN;
    let hi = d.after.iter().map(|(_, r)| if d.vertical { r.right().min(d.area.right()) } else { r.bottom().min(d.area.bottom()) })
        .fold(f64::MAX, f64::min) - MIN;
    if lo > hi {
        return d.pos;
    }
    pos.clamp(lo, hi)
}

/// The double click: the line halfway, so both sides are equal.
pub fn middle(d: &Divider) -> f64 {
    let lo = d.before.iter().map(|(_, r)| if d.vertical { r.x } else { r.y }).fold(f64::MAX, f64::min);
    let hi = d.after.iter().map(|(_, r)| if d.vertical { r.right().min(d.area.right()) } else { r.bottom().min(d.area.bottom()) })
        .fold(f64::MIN, f64::max);
    clamp(d, ((lo + hi) / 2.0).round())
}

/// Each window's new region, in thousandths of the area ("x,y,w,h", as SnapWindow takes it), with the line at `pos`.
pub fn regions(d: &Divider, pos: f64) -> Vec<(u64, String)> {
    let a = d.area;
    let th = |v: f64, from: f64, size: f64| (((v - from) / size) * 1000.0).round().clamp(0.0, 1000.0) as i64;
    let region = |r: Rect| {
        let r = Rect { w: r.w.min(a.right() - r.x), h: r.h.min(a.bottom() - r.y), ..r };
        let (x0, y0) = (th(r.x, a.x, a.w), th(r.y, a.y, a.h));
        let (x1, y1) = (th(r.right(), a.x, a.w), th(r.bottom(), a.y, a.h));
        format!("{x0},{y0},{},{}", (x1 - x0).max(1), (y1 - y0).max(1))
    };
    let mut out = Vec::new();
    for (id, r) in &d.before {
        let n = if d.vertical { Rect { w: pos - r.x, ..*r } } else { Rect { h: pos - r.y, ..*r } };
        out.push((*id, region(n)));
    }
    for (id, r) in &d.after {
        let n = if d.vertical { Rect { x: pos, w: r.right() - pos, ..*r } } else { Rect { y: pos, h: r.bottom() - pos, ..*r } };
        out.push((*id, region(n)));
    }
    out
}

/// The floating windows of the focused workspace, as rectangles (the minimized ones are not on the screen).
fn look() -> Vec<(u64, Rect)> {
    crate::encaixe::workspace_windows().iter().filter(|w| w["is_floating"] == true && w["is_minimized"] != true).filter_map(|w| {
        let l = &w["layout"];
        let p = l["tile_pos_in_workspace_view"].as_array()?;
        let s = l["tile_size"].as_array()?;
        Some((w["id"].as_u64()?, Rect { x: p.first()?.as_f64()?, y: p.get(1)?.as_f64()?, w: s.first()?.as_f64()?, h: s.get(1)?.as_f64()? }))
    }).collect()
}

fn apply(d: &Divider, pos: f64) {
    for (id, region) in regions(d, pos) {
        let _ = servicos::niri_acao(json!({"SnapWindow": {"id": id, "region": region}}));
    }
}

struct Handle {
    window: gtk::ApplicationWindow,
    divider: RefCell<Option<Divider>>,
}

impl Handle {
    fn place(&self, d: &Divider, pos: f64) {
        let mid = ((d.span.0 + d.span.1) / 2.0).round() as i32;
        let p = pos.round() as i32;
        if d.vertical {
            self.window.set_margin(Edge::Left, p - GRAB / 2);
            self.window.set_margin(Edge::Top, mid - PILL_L / 2);
        } else {
            self.window.set_margin(Edge::Left, mid - PILL_L / 2);
            self.window.set_margin(Edge::Top, p - GRAB / 2);
        }
    }
}

pub struct Dividers {
    /// the working area's size, measured by an invisible surface that fills it
    size: Rc<Cell<(i32, i32)>>,
    handles: RefCell<Vec<Rc<Handle>>>,
    dragging: Cell<bool>,
    busy: Cell<bool>,
    last: RefCell<Vec<Divider>>,
}

thread_local! {
    static DIVIDERS: RefCell<Option<Rc<Dividers>>> = const { RefCell::new(None) };
}

/// A transparent surface on the background that fills the working area (layer-shell places it inside the others'
/// reserved zones): its size is the area's.
fn measure(size: &Rc<Cell<(i32, i32)>>) {
    let Some(app) = gio::Application::default().and_downcast::<gtk::Application>() else { return };
    let w = camada(&app, "kalango-area-util", Layer::Background, &[Edge::Top, Edge::Bottom, Edge::Left, Edge::Right]);
    w.set_exclusive_zone(0);
    w.add_css_class("area-util");
    let probe = gtk::DrawingArea::new();
    let s = size.clone();
    probe.connect_resize(move |_, w, h| s.set((w, h)));
    w.set_child(Some(&probe));
    w.present();
}

pub fn start() {
    let size: Rc<Cell<(i32, i32)>> = Rc::default();
    measure(&size);
    let d = Rc::new(Dividers { size, handles: RefCell::default(), dragging: Cell::new(false), busy: Cell::new(false), last: RefCell::default() });
    DIVIDERS.with(|x| *x.borrow_mut() = Some(d.clone()));
    glib::timeout_add_local(Duration::from_millis(400), move || {
        d.tick();
        glib::ControlFlow::Continue
    });
}

impl Dividers {
    fn tick(self: &Rc<Self>) {
        if self.dragging.get() || self.busy.replace(true) {
            return;
        }
        let me = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(look, move |wins| {
            let Some(s) = me.upgrade() else { return };
            s.busy.set(false);
            if s.dragging.get() {
                return;
            }
            let (w, h) = s.size.get();
            let ds = dividers(&wins, (f64::from(w), f64::from(h)));
            if *s.last.borrow() == ds {
                return;
            }
            if debug() {
                let txt: Vec<String> = ds.iter().map(|d| format!("{} em {:.0} ({} | {})", if d.vertical { "vertical" } else { "horizontal" }, d.pos,
                    d.before.iter().map(|(i, _)| i.to_string()).collect::<Vec<_>>().join(","), d.after.iter().map(|(i, _)| i.to_string()).collect::<Vec<_>>().join(","))).collect();
                eprintln!("debug divisória: {} — {}", ds.len(), txt.join("; "));
            }
            s.show(&ds);
            *s.last.borrow_mut() = ds;
        });
    }

    fn show(self: &Rc<Self>, ds: &[Divider]) {
        while self.handles.borrow().len() < ds.len() {
            let Some(h) = self.handle() else { return };
            self.handles.borrow_mut().push(h);
        }
        for (i, h) in self.handles.borrow().iter().enumerate() {
            match ds.get(i) {
                Some(d) => {
                    let (w, l) = if d.vertical { (GRAB, PILL_L) } else { (PILL_L, GRAB) };
                    h.window.set_default_size(w, l);
                    h.window.set_size_request(w, l);
                    if let Some(p) = h.window.child() {
                        p.set_size_request(if d.vertical { PILL_W } else { PILL_L }, if d.vertical { PILL_L } else { PILL_W });
                    }
                    h.window.set_cursor_from_name(Some(if d.vertical { "col-resize" } else { "row-resize" }));
                    h.place(d, d.pos);
                    *h.divider.borrow_mut() = Some(d.clone());
                    h.window.set_visible(true);
                    if debug() {
                        let mid = (d.span.0 + d.span.1) / 2.0;
                        let (x, y) = if d.vertical { (d.pos, mid) } else { (mid, d.pos) };
                        eprintln!("debug central: divisória {i} em {x:.0} {y:.0}");
                    }
                }
                None => {
                    *h.divider.borrow_mut() = None;
                    h.window.set_visible(false);
                }
            }
        }
    }

    fn handle(self: &Rc<Self>) -> Option<Rc<Handle>> {
        let app = gio::Application::default().and_downcast::<gtk::Application>()?;
        let window = camada(&app, "kalango-divisoria", Layer::Top, &[Edge::Top, Edge::Left]);
        window.set_exclusive_zone(-1);
        window.add_css_class("divisoria-janela");
        let pill = caixa(gtk::Orientation::Vertical, 0, "divisoria");
        pill.set_halign(gtk::Align::Center);
        pill.set_valign(gtk::Align::Center);
        window.set_child(Some(&pill));
        let h = Rc::new(Handle { window, divider: RefCell::default() });
        // dragging: the windows on both sides follow the pointer 1:1
        let drag = gtk::GestureDrag::new();
        let start: Rc<Cell<f64>> = Rc::default();
        let (hw, me, st) = (Rc::downgrade(&h), Rc::downgrade(self), start.clone());
        drag.connect_drag_begin(move |_, _, _| {
            let (Some(h), Some(me)) = (hw.upgrade(), me.upgrade()) else { return };
            let pos = h.divider.borrow().as_ref().map(|d| d.pos);
            if let Some(pos) = pos {
                me.dragging.set(true);
                st.set(pos);
                if let Some(c) = h.window.child() {
                    c.add_css_class("arrastando");
                }
            }
        });
        let (hw, st) = (Rc::downgrade(&h), start.clone());
        let last_sent: Rc<Cell<f64>> = Rc::new(Cell::new(f64::NAN));
        let ls = last_sent.clone();
        drag.connect_drag_update(move |_, dx, dy| {
            let Some(h) = hw.upgrade() else { return };
            let Some(d) = h.divider.borrow().clone() else { return };
            let pos = clamp(&d, (st.get() + if d.vertical { dx } else { dy }).round());
            if pos == ls.get() {
                return;
            }
            ls.set(pos);
            apply(&d, pos);
            h.place(&d, pos);
        });
        let (hw, me, st, ls) = (Rc::downgrade(&h), Rc::downgrade(self), start, last_sent);
        drag.connect_drag_end(move |_, dx, dy| {
            let (Some(h), Some(me)) = (hw.upgrade(), me.upgrade()) else { return };
            if let Some(c) = h.window.child() {
                c.remove_css_class("arrastando");
            }
            if let Some(d) = h.divider.borrow().clone() {
                let pos = clamp(&d, (st.get() + if d.vertical { dx } else { dy }).round());
                if pos != d.pos {
                    apply(&d, pos);
                    if debug() {
                        eprintln!("debug divisória: arrastada de {:.0} para {pos:.0} ({} janelas)", d.pos, d.before.len() + d.after.len());
                    }
                }
            }
            ls.set(f64::NAN);
            // niri moves the windows; the next look puts the divider where they are now
            let m = Rc::downgrade(&me);
            glib::timeout_add_local_once(Duration::from_millis(250), move || {
                if let Some(m) = m.upgrade() {
                    m.dragging.set(false);
                    m.last.borrow_mut().clear();
                }
            });
        });
        h.window.add_controller(drag);
        // a double click: equal sides
        let click = gtk::GestureClick::new();
        let (hw, me) = (Rc::downgrade(&h), Rc::downgrade(self));
        click.connect_pressed(move |_, n, _, _| {
            if n != 2 {
                return;
            }
            let (Some(h), Some(me)) = (hw.upgrade(), me.upgrade()) else { return };
            let Some(d) = h.divider.borrow().clone() else { return };
            let pos = middle(&d);
            apply(&d, pos);
            h.place(&d, pos);
            me.last.borrow_mut().clear();
            if debug() {
                eprintln!("debug divisória: igualada em {pos:.0}");
            }
        });
        h.window.add_controller(click);
        Some(h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect { x, y, w, h }
    }

    /// "Foco + pilha" on a 1920×1080 screen under the top bar: the editor on the left, the terminal and the browser
    /// stacked on the right.
    const SIZE: (f64, f64) = (1920.0, 1048.0);

    fn focus_stack() -> Vec<(u64, Rect)> {
        vec![(1, r(0.0, 32.0, 1281.0, 1048.0)), (2, r(1281.0, 32.0, 639.0, 524.0)), (3, r(1281.0, 556.0, 639.0, 524.0))]
    }

    #[test]
    fn the_lines_where_snapped_windows_meet() {
        let ds = dividers(&focus_stack(), SIZE);
        assert_eq!(ds.len(), 2);
        let v = ds.iter().find(|d| d.vertical).unwrap();
        assert_eq!(v.pos, 1281.0);
        assert_eq!(v.before.iter().map(|(i, _)| *i).collect::<Vec<_>>(), [1]);
        assert_eq!(v.after.iter().map(|(i, _)| *i).collect::<Vec<_>>(), [2, 3]);
        assert_eq!(v.span, (32.0, 1080.0));
        let h = ds.iter().find(|d| !d.vertical).unwrap();
        assert_eq!((h.pos, h.before.len(), h.after.len()), (556.0, 1, 1));
        // windows apart, or one alone: nothing
        assert!(dividers(&[(1, r(0.0, 0.0, 500.0, 500.0)), (2, r(700.0, 0.0, 500.0, 500.0))], SIZE).is_empty());
        assert!(dividers(&[(1, r(0.0, 0.0, 500.0, 500.0))], SIZE).is_empty());
    }

    #[test]
    fn moving_the_line_resizes_both_sides_together() {
        let ds = dividers(&focus_stack(), SIZE);
        let v = ds.iter().find(|d| d.vertical).unwrap();
        // the editor to half the screen: the stack grows with it
        let regs = regions(v, 960.0);
        assert_eq!(regs, [(1, "0,0,500,1000".to_string()), (2, "500,0,500,500".to_string()), (3, "500,500,500,500".to_string())]);
        // never narrower than the minimum, and the double click halves
        assert_eq!(clamp(v, 50.0), MIN);
        assert_eq!(clamp(v, 1900.0), 1920.0 - MIN);
        assert_eq!(middle(v), 960.0);
    }

    #[test]
    fn an_app_taller_than_its_region_does_not_stretch_the_others() {
        // the calculator's minimum height runs 55 px past the bottom of the area
        let wins = vec![(1, r(0.0, 32.0, 1152.0, 1048.0)), (2, r(1152.0, 32.0, 768.0, 489.0)), (3, r(1152.0, 521.0, 768.0, 614.0))];
        let ds = dividers(&wins, SIZE);
        let v = ds.iter().find(|d| d.vertical).unwrap();
        assert_eq!(v.span, (32.0, 1080.0));
        let regs = regions(v, 960.0);
        assert_eq!(regs[0], (1, "0,0,500,1000".to_string()));
        assert_eq!(regs[2], (3, "500,467,500,533".to_string()));
    }
}
