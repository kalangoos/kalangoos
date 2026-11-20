//! The measure while a window is resized (design "Estados de janela", 63799, "Redimensionando"): the window outlined
//! in the accent with a handle at each corner, and its size ("870 × 400") under it. Any window — niri tells the size
//! as it changes (`WindowLayoutsChanged`); a resize by the pointer changes it many times in a row, a maximize or a
//! snap once, so the measure shows from the second change within a moment and goes 600 ms after the last one.

use crate::superficies::{caixa, camada, rotulo};
use gtk::prelude::*;
use gtk::{gio, glib, Align, Orientation};
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

/// Two changes this close: a resize in progress.
const BURST: Duration = Duration::from_millis(250);
const LINGER: Duration = Duration::from_millis(600);
/// The handles (10 × 10) sit on the corners: the outline is drawn this far out.
const OUT: i32 = 5;

/// The window's rectangle on its output, from a niri layout (`tile_pos_in_workspace_view` + `window_offset_in_tile`,
/// `window_size`); none when the window is not on a workspace in view.
pub fn rect(l: &Value) -> Option<(i32, i32, i32, i32)> {
    let p = l["tile_pos_in_workspace_view"].as_array()?;
    let o = l["window_offset_in_tile"].as_array();
    let off = |i: usize| o.and_then(|o| o.get(i)).and_then(Value::as_f64).unwrap_or(0.0);
    let s = l["window_size"].as_array()?;
    Some(((p.first()?.as_f64()? + off(0)).round() as i32, (p.get(1)?.as_f64()? + off(1)).round() as i32, s.first()?.as_i64()? as i32, s.get(1)?.as_i64()? as i32))
}

/// "870 × 400"
pub fn label(w: i32, h: i32) -> String {
    format!("{w} × {h}")
}

struct Measure {
    window: gtk::ApplicationWindow,
    frame: gtk::Overlay,
    chip: gtk::Label,
    /// the window watched: id, its last size, when it changed
    last: RefCell<Option<(u64, (i32, i32), Instant)>>,
    shown: Cell<bool>,
    hide_at: Cell<Option<Instant>>,
}

thread_local! {
    static MEASURE: RefCell<Option<Rc<Measure>>> = const { RefCell::new(None) };
}

fn measure() -> Option<Rc<Measure>> {
    if let Some(m) = MEASURE.with(|m| m.borrow().clone()) {
        return Some(m);
    }
    let app = gio::Application::default().and_downcast::<gtk::Application>()?;
    let window = camada(&app, "kalango-medida", Layer::Top, &[Edge::Top, Edge::Left]);
    window.set_exclusive_zone(-1);
    window.add_css_class("medida-janela");
    let col = caixa(Orientation::Vertical, 0, "");
    let frame = gtk::Overlay::new();
    let outline = caixa(Orientation::Vertical, 0, "medida-contorno");
    outline.set_margin_top(OUT);
    outline.set_margin_bottom(OUT);
    outline.set_margin_start(OUT);
    outline.set_margin_end(OUT);
    frame.set_child(Some(&outline));
    for (h, v) in [(Align::Start, Align::Start), (Align::End, Align::Start), (Align::Start, Align::End), (Align::End, Align::End)] {
        let handle = caixa(Orientation::Vertical, 0, "medida-alca");
        handle.set_size_request(10, 10);
        handle.set_halign(h);
        handle.set_valign(v);
        frame.add_overlay(&handle);
    }
    col.append(&frame);
    let chip = rotulo("", "medida-chip");
    chip.set_halign(Align::Center);
    chip.set_margin_top(10 - OUT);
    col.append(&chip);
    window.set_child(Some(&col));
    let m = Rc::new(Measure { window, frame, chip, last: RefCell::default(), shown: Cell::new(false), hide_at: Cell::new(None) });
    let w = Rc::downgrade(&m);
    glib::timeout_add_local(Duration::from_millis(100), move || {
        let Some(m) = w.upgrade() else { return glib::ControlFlow::Break };
        if m.shown.get() && m.hide_at.get().is_some_and(|t| Instant::now() >= t) {
            m.shown.set(false);
            m.window.set_visible(false);
            if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                eprintln!("debug medida: escondida");
            }
        }
        glib::ControlFlow::Continue
    });
    MEASURE.with(|x| *x.borrow_mut() = Some(m.clone()));
    Some(m)
}

/// niri's `WindowLayoutsChanged`: the focused window's new size.
pub fn layouts_changed(changes: &Value, focused: Option<u64>) {
    let Some(focused) = focused else { return };
    let Some(l) = changes.as_array().into_iter().flatten().find(|c| c[0].as_u64() == Some(focused)).map(|c| &c[1]) else { return };
    let Some((x, y, w, h)) = rect(l) else { return };
    let Some(m) = measure() else { return };
    let now = Instant::now();
    let prev = m.last.borrow_mut().replace((focused, (w, h), now));
    let Some((pid, psize, pt)) = prev else { return };
    if pid != focused || psize == (w, h) {
        // a move, or another window: nothing to measure (keep the time of the last change)
        if pid == focused {
            *m.last.borrow_mut() = Some((pid, psize, pt));
        }
        return;
    }
    if !m.shown.get() && now.duration_since(pt) > BURST {
        return;
    }
    m.frame.set_size_request(w + 2 * OUT, h + 2 * OUT);
    // (a layer surface only shrinks to its default size: the frame and the chip under it, exactly)
    m.window.set_default_size(w + 2 * OUT, h + 2 * OUT + (10 - OUT) + 21);
    m.window.set_margin(Edge::Left, x - OUT);
    m.window.set_margin(Edge::Top, y - OUT);
    m.chip.set_text(&label(w, h));
    m.hide_at.set(Some(now + LINGER));
    if !m.shown.replace(true) {
        m.window.present();
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug medida: redimensionando a janela {focused}");
        }
    }
    if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
        eprintln!("debug medida: {}", label(w, h));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_rectangle_and_its_measure() {
        let l = serde_json::json!({"tile_pos_in_workspace_view": [980.0, 540.0], "window_offset_in_tile": [0.0, 0.0], "window_size": [870, 400]});
        assert_eq!(rect(&l), Some((980, 540, 870, 400)));
        assert_eq!(label(870, 400), "870 × 400");
        // a window on a workspace out of view has no place on the output
        assert_eq!(rect(&serde_json::json!({"tile_pos_in_workspace_view": null, "window_size": [870, 400]})), None);
    }
}
