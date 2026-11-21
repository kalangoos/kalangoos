//! The edge of a side panel that the person drags to resize it (design "Chat · conversa com um agente seu", the
//! annotated edges): a 9 px hit area with the resize cursor; at rest only a 3×36 px grip in the border's colour, on
//! hover and while dragging a 1 px accent line with the grip in the accent. A double click puts the default back.

use gtk::prelude::*;
use gtk::gdk;
use std::cell::Cell;
use std::rc::Rc;

/// k_accent and k_border (dark values: the theme swaps them by value)
const ACCENT: &str = "#A5A1E2";
const BORDER: &str = "#26272F";

/// The hit area's width.
pub const HIT: i32 = 9;

/// A panel's width: the person's own (`saved`) or `share` of the window clamped to `default` (min, max), then always
/// within `limits` (min, max) and within the `room` the window leaves it (never under the least limit).
pub fn panel_width(window: i32, saved: Option<i32>, share: f64, default: (i32, i32), limits: (i32, i32), room: i32) -> i32 {
    let w = saved.unwrap_or_else(|| ((f64::from(window) * share).round() as i32).clamp(default.0, default.1));
    w.clamp(limits.0, limits.1).min(room).max(limits.0)
}

/// What the handle asks of its panel: its width now, a new one while dragging (`true` once the drag ends: keep it),
/// and back to the default.
pub struct Panel {
    pub width: Box<dyn Fn() -> i32>,
    pub resize: Box<dyn Fn(i32, bool)>,
    pub reset: Box<dyn Fn()>,
}

/// The handle. `grows` is the direction a drag widens the panel: -1 for a panel on the right (its left edge), 1 for
/// one on the left.
pub fn new(grows: i32, panel: Panel) -> gtk::DrawingArea {
    let h = gtk::DrawingArea::new();
    h.add_css_class("alca-redimensionar");
    h.set_content_width(HIT);
    h.set_size_request(HIT, -1);
    h.set_hexpand(false);
    h.set_vexpand(true);
    h.set_cursor_from_name(Some("col-resize"));
    let lit = Rc::new(Cell::new(false));
    let l = lit.clone();
    h.set_draw_func(move |_, cr, w, ht| {
        let rgba = |hex: &str| gdk::RGBA::parse(crate::tema::cor(hex)).unwrap_or(gdk::RGBA::BLACK);
        let x = f64::from(w / 2);
        if l.get() {
            let c = rgba(ACCENT);
            cr.set_source_rgba(c.red().into(), c.green().into(), c.blue().into(), c.alpha().into());
            cr.rectangle(x, 0.0, 1.0, f64::from(ht));
            let _ = cr.fill();
        }
        let c = rgba(if l.get() { ACCENT } else { BORDER });
        cr.set_source_rgba(c.red().into(), c.green().into(), c.blue().into(), c.alpha().into());
        let gy = f64::from((ht - 36) / 2);
        // (a grip with rounded ends: 3 px wide, 36 px tall)
        cr.new_sub_path();
        cr.arc(x + 0.5, gy + 1.5, 1.5, std::f64::consts::PI, 0.0);
        cr.arc(x + 0.5, gy + 34.5, 1.5, 0.0, std::f64::consts::PI);
        cr.close_path();
        let _ = cr.fill();
    });
    crate::tema::registrar(&h, |w| w.queue_draw());
    let dragging = Rc::new(Cell::new(false));
    let hover = gtk::EventControllerMotion::new();
    let (l, hh, d) = (lit.clone(), h.downgrade(), dragging.clone());
    hover.connect_enter(move |_, _, _| {
        l.set(true);
        if let Some(h) = hh.upgrade() {
            h.queue_draw();
        }
    });
    let (l, hh) = (lit.clone(), h.downgrade());
    hover.connect_leave(move |_| {
        if !d.get() {
            l.set(false);
            if let Some(h) = hh.upgrade() {
                h.queue_draw();
            }
        }
    });
    h.add_controller(hover);
    let panel = Rc::new(panel);
    let drag = gtk::GestureDrag::new();
    // (the handle moves with the panel while it is dragged: the pointer is followed on the window's surface, or the
    // offset would shrink as the handle runs after it)
    let start = Rc::new(Cell::new((0, 0.0))); // (width, x on the surface)
    let on_surface = |g: &gtk::GestureDrag| g.current_event().and_then(|e| e.position()).map(|(x, _)| x);
    let (p, s, d) = (panel.clone(), start.clone(), dragging.clone());
    drag.connect_drag_begin(move |g, _, _| {
        s.set(((p.width)(), on_surface(g).unwrap_or(0.0)));
        d.set(true);
    });
    let (p, s) = (panel.clone(), start.clone());
    drag.connect_drag_update(move |g, _, _| {
        let st = s.get();
        if let Some(x) = on_surface(g) {
            (p.resize)(st.0 + grows * (x - st.1).round() as i32, false);
        }
    });
    let (p, s, d, l, hh) = (panel.clone(), start, dragging, lit, h.downgrade());
    drag.connect_drag_end(move |g, _, _| {
        d.set(false);
        let st = s.get();
        if let Some(x) = on_surface(g).filter(|x| (x - st.1).abs() >= 1.0) {
            (p.resize)(st.0 + grows * (x - st.1).round() as i32, true);
        }
        // (the pointer may have left while dragging)
        if let Some(h) = hh.upgrade() {
            let inside = g.bounding_box_center().is_some_and(|(x, _)| x >= 0.0 && x <= f64::from(h.width()));
            l.set(inside);
            h.queue_draw();
        }
    });
    h.add_controller(drag);
    let twice = gtk::GestureClick::new();
    let p = panel;
    twice.connect_pressed(move |_, n, _, _| {
        if n == 2 {
            (p.reset)();
        }
    });
    h.add_controller(twice);
    h
}

#[cfg(test)]
mod tests {
    use super::panel_width;

    // the context on the right: 26 % of the window, 300–480 by default, 280–560 by hand
    fn context(window: i32, saved: Option<i32>) -> i32 {
        panel_width(window, saved, 0.26, (300, 480), (280, 560), i32::MAX)
    }

    #[test]
    fn the_default_is_a_share_of_the_window_clamped() {
        assert_eq!(context(1500, None), 390);
        assert_eq!(context(1000, None), 300);
        assert_eq!(context(3370, None), 480);
        // the list: 22 %, 240–360
        assert_eq!(panel_width(1300, None, 0.22, (240, 360), (220, 420), i32::MAX), 286);
        assert_eq!(panel_width(900, None, 0.22, (240, 360), (220, 420), i32::MAX), 240);
    }

    #[test]
    fn the_persons_width_holds_within_the_limits() {
        assert_eq!(context(3370, Some(520)), 520);
        assert_eq!(context(3370, Some(900)), 560);
        assert_eq!(context(3370, Some(100)), 280);
    }

    #[test]
    fn a_narrow_window_gives_way_down_to_the_least() {
        assert_eq!(panel_width(1500, Some(520), 0.26, (300, 480), (280, 560), 400), 400);
        assert_eq!(panel_width(1500, None, 0.26, (300, 480), (280, 560), 100), 280);
    }
}
