//! Pieces the widgets share (design "Widgets"): the header (glyph and CAPS title), the ring, the 3 px bar, the line
//! chart and text that changes with a 180 ms crossfade. Values move smoothly; charts slide left as new samples come
//! (320 ms ease-in-out); nothing blinks.

use crate::conversa::ui_icon;
use crate::superficies::{caixa, rotulo};
use gtk::prelude::*;
use gtk::{gdk, glib, Align, Orientation};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::{Duration, Instant};

pub const TEAL: &str = "#8ED2C3";
pub const ACCENT: &str = "#A5A1E2";
pub const GOLD: &str = "#E5C98C";
pub const BLUE: &str = "#7CC3DF";
pub const CORAL: &str = "#EFA2A6";
pub const MUTED: &str = "#8E8F99";

/// The widget's frame: a column with the design's 10 px gap and 4 px padding, at its size.
pub fn quadro(w: i32, h: i32) -> gtk::Box {
    let c = caixa(Orientation::Vertical, 10, "desk-widget");
    c.set_size_request(w, h);
    c
}

/// Glyph and CAPS title (the spark in the accent colour for AI widgets).
pub fn cabecalho(glifo: &str, titulo: &str) -> (gtk::Box, gtk::Label) {
    let r = caixa(Orientation::Horizontal, 6, "");
    if glifo == "spark" {
        r.append(&crate::superficies::estrela(13, ACCENT));
    } else {
        r.append(&ui_icon(glifo, MUTED, 13));
    }
    let l = rotulo(titulo, "desk-rotulo");
    l.set_xalign(0.0);
    // the rest of the row (a long network name is cut at the widget's edge, not before)
    l.set_hexpand(true);
    l.set_ellipsize(gtk::pango::EllipsizeMode::End);
    r.append(&l);
    (r, l)
}

fn rgba(c: &str) -> gdk::RGBA {
    gdk::RGBA::parse(c).unwrap_or(gdk::RGBA::WHITE)
}

fn ease(t: f64) -> f64 {
    if t < 0.5 { 2.0 * t * t } else { 1.0 - (-2.0 * t + 2.0).powi(2) / 2.0 }
}

/// A value that moves to its new target in 320 ms (ease-in-out), redrawing `area` on every frame.
#[derive(Clone)]
pub struct Suave {
    de: Rc<Cell<f64>>,
    para: Rc<Cell<f64>>,
    desde: Rc<Cell<Option<Instant>>>,
}

impl Suave {
    pub fn new(v: f64) -> Suave {
        Suave { de: Rc::new(Cell::new(v)), para: Rc::new(Cell::new(v)), desde: Rc::new(Cell::new(None)) }
    }

    pub fn agora(&self) -> f64 {
        match self.desde.get() {
            Some(t) => {
                let k = (t.elapsed().as_secs_f64() / 0.32).min(1.0);
                self.de.get() + (self.para.get() - self.de.get()) * ease(k)
            }
            None => self.para.get(),
        }
    }

    pub fn ir(&self, v: f64, area: &impl IsA<gtk::Widget>) {
        if (v - self.para.get()).abs() < 1e-6 {
            return;
        }
        self.de.set(self.agora());
        self.para.set(v);
        self.desde.set(Some(Instant::now()));
        let s = self.clone();
        area.add_tick_callback(move |w, _| {
            w.queue_draw();
            if s.desde.get().is_some_and(|t| t.elapsed() >= Duration::from_millis(320)) {
                s.desde.set(None);
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
    }
}

/// A ring of `px` (track at 10% white, the value in its colour from the top, clockwise) with a centre for the number.
pub struct Anel {
    pub widget: gtk::Overlay,
    pub centro: gtk::Box,
    area: gtk::DrawingArea,
    valor: Suave,
}

impl Anel {
    pub fn new(px: i32, cor: &str) -> Anel {
        let area = gtk::DrawingArea::new();
        area.set_content_width(px);
        area.set_content_height(px);
        let valor = Suave::new(0.0);
        let (v, c) = (valor.clone(), rgba(cor));
        area.set_draw_func(move |_, cr, w, h| {
            let (cx, cy) = (f64::from(w) / 2.0, f64::from(h) / 2.0);
            let r = f64::from(w.min(h)) / 2.0 - 4.0;
            cr.set_line_width(6.0);
            cr.set_line_cap(gtk::cairo::LineCap::Round);
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.10);
            cr.arc(cx, cy, r, 0.0, std::f64::consts::TAU);
            let _ = cr.stroke();
            let x = v.agora().clamp(0.0, 1.0);
            if x > 0.001 {
                cr.set_source_rgba(f64::from(c.red()), f64::from(c.green()), f64::from(c.blue()), 1.0);
                let ini = -std::f64::consts::FRAC_PI_2;
                cr.arc(cx, cy, r, ini, ini + x * std::f64::consts::TAU);
                let _ = cr.stroke();
            }
        });
        let widget = gtk::Overlay::new();
        widget.set_child(Some(&area));
        widget.set_halign(Align::Start);
        let centro = caixa(Orientation::Vertical, 0, "");
        centro.set_halign(Align::Center);
        centro.set_valign(Align::Center);
        widget.add_overlay(&centro);
        Anel { widget, centro, area, valor }
    }

    /// Moves the ring to `v` (0…1).
    pub fn ir(&self, v: f64) {
        self.valor.ir(v, &self.area);
    }
}

/// A 3 px bar (track at 10% white) of `largura`.
pub fn barra(largura: i32, cor: &str) -> (gtk::DrawingArea, Suave) {
    let a = gtk::DrawingArea::new();
    a.set_content_width(largura);
    a.set_content_height(3);
    let v = Suave::new(0.0);
    let (s, c) = (v.clone(), rgba(cor));
    a.set_draw_func(move |_, cr, w, h| {
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.10);
        cr.rectangle(0.0, 0.0, f64::from(w), f64::from(h));
        let _ = cr.fill();
        let x = s.agora().clamp(0.0, 1.0);
        cr.set_source_rgba(f64::from(c.red()), f64::from(c.green()), f64::from(c.blue()), 1.0);
        cr.rectangle(0.0, 0.0, f64::from(w) * x, f64::from(h));
        let _ = cr.fill();
    });
    (a, v)
}

/// A line chart of the last samples (0…1), with the area under it at 13%; a new sample slides it left in 320 ms.
pub struct Grafico {
    pub area: gtk::DrawingArea,
    pontos: Rc<RefCell<VecDeque<f64>>>,
    desde: Rc<Cell<Option<Instant>>>,
    max: usize,
}

impl Grafico {
    pub fn new(largura: i32, altura: i32, max: usize, cor: &str) -> Grafico {
        let area = gtk::DrawingArea::new();
        area.set_content_width(largura);
        area.set_content_height(altura);
        let pontos: Rc<RefCell<VecDeque<f64>>> = Rc::default();
        let desde: Rc<Cell<Option<Instant>>> = Rc::default();
        let (p, d, c) = (pontos.clone(), desde.clone(), rgba(cor));
        area.set_draw_func(move |_, cr, w, h| {
            let p = p.borrow();
            if p.len() < 2 {
                return;
            }
            let passo = f64::from(w) / (max.saturating_sub(1).max(1)) as f64;
            // sliding: the newest sample comes in from the right
            let k = d.get().map(|t| ease((t.elapsed().as_secs_f64() / 0.32).min(1.0))).unwrap_or(1.0);
            let x0 = f64::from(w) - (p.len() - 1) as f64 * passo + (1.0 - k) * passo;
            let y = |v: f64| f64::from(h) - 2.0 - v.clamp(0.0, 1.0) * (f64::from(h) - 4.0);
            cr.move_to(x0, y(p[0]));
            for (i, v) in p.iter().enumerate().skip(1) {
                cr.line_to(x0 + i as f64 * passo, y(*v));
            }
            let linha = cr.copy_path();
            cr.line_to(x0 + (p.len() - 1) as f64 * passo, f64::from(h));
            cr.line_to(x0, f64::from(h));
            cr.close_path();
            cr.set_source_rgba(f64::from(c.red()), f64::from(c.green()), f64::from(c.blue()), 0.13);
            let _ = cr.fill();
            if let Ok(l) = linha {
                cr.append_path(&l);
            }
            cr.set_line_width(1.5);
            cr.set_line_join(gtk::cairo::LineJoin::Round);
            cr.set_source_rgba(f64::from(c.red()), f64::from(c.green()), f64::from(c.blue()), 1.0);
            let _ = cr.stroke();
        });
        Grafico { area, pontos, desde, max }
    }

    /// The samples (0…1), oldest first; the chart slides when one was added.
    pub fn mostrar(&self, v: &[f64]) {
        let novo: VecDeque<f64> = v.iter().rev().take(self.max).rev().copied().collect();
        let mudou = *self.pontos.borrow() != novo;
        if !mudou {
            return;
        }
        let deslizar = !self.pontos.borrow().is_empty();
        *self.pontos.borrow_mut() = novo;
        if deslizar {
            self.desde.set(Some(Instant::now()));
            let d = self.desde.clone();
            self.area.add_tick_callback(move |w, _| {
                w.queue_draw();
                if d.get().is_some_and(|t| t.elapsed() >= Duration::from_millis(320)) {
                    d.set(None);
                    return glib::ControlFlow::Break;
                }
                glib::ControlFlow::Continue
            });
        } else {
            self.area.queue_draw();
        }
    }
}

/// Text that changes with a 180 ms crossfade (two labels in a stack).
#[derive(Clone)]
pub struct Texto {
    pub stack: gtk::Stack,
    a: gtk::Label,
    b: gtk::Label,
    atual: Rc<Cell<bool>>,
}

impl Texto {
    pub fn new(classe: &str) -> Texto {
        let stack = gtk::Stack::new();
        stack.set_transition_type(gtk::StackTransitionType::Crossfade);
        stack.set_transition_duration(180);
        stack.set_hhomogeneous(false);
        stack.set_interpolate_size(true);
        let a = rotulo("", classe);
        let b = rotulo("", classe);
        for l in [&a, &b] {
            l.set_xalign(0.0);
        }
        stack.add_child(&a);
        stack.add_child(&b);
        stack.set_visible_child(&a);
        Texto { stack, a, b, atual: Rc::new(Cell::new(false)) }
    }

    pub fn alinhar(&self, x: f32) {
        self.a.set_xalign(x);
        self.b.set_xalign(x);
    }

    pub fn definir(&self, t: &str) {
        let (vis, outro) = if self.atual.get() { (&self.b, &self.a) } else { (&self.a, &self.b) };
        if vis.text() == t {
            return;
        }
        if vis.text().is_empty() {
            vis.set_text(t);
            return;
        }
        outro.set_text(t);
        self.stack.set_visible_child(outro);
        self.atual.set(!self.atual.get());
    }
}

/// "18,4" (one decimal, the comma of pt-BR).
pub fn decimal(v: f64) -> String {
    format!("{v:.1}").replace('.', ",")
}

/// A transfer rate: "820 KB/s", "18,4 MB/s".
pub fn taxa(bytes_s: f64) -> String {
    if bytes_s >= 1_000_000.0 {
        format!("{} MB/s", decimal(bytes_s / 1_000_000.0))
    } else if bytes_s >= 1000.0 {
        format!("{:.0} KB/s", bytes_s / 1000.0)
    } else {
        format!("{bytes_s:.0} B/s")
    }
}

/// A size: "412 GB", "1 TB", "1,6 TB".
pub fn tamanho(bytes: f64) -> String {
    let gb = bytes / 1e9;
    if gb >= 1000.0 {
        let tb = gb / 1000.0;
        if (tb - tb.round()).abs() < 0.05 { format!("{:.0} TB", tb.round()) } else { format!("{} TB", decimal(tb)) }
    } else {
        format!("{gb:.0} GB")
    }
}

/// Memory sizes as the system shows them (GiB, "32 GB" for 32 GiB).
pub const GIB: f64 = 1_073_741_824.0;

#[cfg(test)]
mod tests {
    #[test]
    fn numbers_in_pt_br() {
        assert_eq!(super::taxa(18_400_000.0), "18,4 MB/s");
        assert_eq!(super::taxa(1_200_000.0), "1,2 MB/s");
        assert_eq!(super::taxa(820_000.0), "820 KB/s");
        assert_eq!(super::tamanho(412e9), "412 GB");
        assert_eq!(super::tamanho(1.0e12), "1 TB");
        assert_eq!(super::tamanho(1.6e12), "1,6 TB");
    }
}
