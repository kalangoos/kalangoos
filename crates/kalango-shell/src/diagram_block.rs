//! A diagram in an answer (design "Chat · diagramas e fórmulas"): a ```mermaid block drawn in the conversation — the
//! flowchart subset models write (`flowchart LR|TD`, nodes `A[texto]`, `B{pergunta}`, `C(texto)`, `D((texto))`,
//! edges `-->`, `---`, `-.->`, `==>` with `|rótulo|` or `-- rótulo -->`, chains `A --> B --> C`). Laid out in layers
//! (longest path from the sources, the order inside a layer by its neighbours), drawn with the theme's colours: the
//! first nodes neutral, questions gold, the ends teal, the rest in the accent. "Diagrama"/"Código" switch the views;
//! ↓ keeps a PNG in Downloads.

use crate::conversa::ui_icon;
use crate::superficies::{caixa, rotulo};
use gtk::prelude::*;
use gtk::{Align, Orientation};
use kalango_i18n::{tr, trf};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shape {
    Rect,
    Round,
    Diamond,
    Circle,
}

#[derive(Clone, Debug)]
pub struct Node {
    /// its text (its id when it has none)
    pub text: String,
    pub shape: Shape,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Edge {
    pub from: usize,
    pub to: usize,
    pub label: String,
    pub dashed: bool,
}

#[derive(Clone, Debug)]
pub struct Graph {
    pub horizontal: bool,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

/// A node reference with its shape: `A`, `A[t]`, `A(t)`, `A{t}`, `A((t))`, `A([t])`, `A[(t)]`.
fn node_ref(s: &str) -> Option<(String, Option<(String, Shape)>)> {
    let s = s.trim().trim_end_matches(';').trim();
    if s.is_empty() {
        return None;
    }
    let cut = s.find(['[', '(', '{']).unwrap_or(s.len());
    let id = s[..cut].trim().to_string();
    if id.is_empty() || !id.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') {
        return None;
    }
    let rest = s[cut..].trim();
    if rest.is_empty() {
        return Some((id, None));
    }
    let strip = |t: &str, a: &str, b: &str| t.strip_prefix(a).and_then(|x| x.strip_suffix(b)).map(|x| x.trim().trim_matches('"').to_string());
    let shape = if let Some(t) = strip(rest, "((", "))") {
        (t, Shape::Circle)
    } else if let Some(t) = strip(rest, "([", "])").or_else(|| strip(rest, "[(", ")]")) {
        (t, Shape::Round)
    } else if let Some(t) = strip(rest, "{", "}") {
        (t, Shape::Diamond)
    } else if let Some(t) = strip(rest, "(", ")") {
        (t, Shape::Round)
    } else if let Some(t) = strip(rest, "[", "]") {
        (t, Shape::Rect)
    } else {
        return Some((id, None));
    };
    Some((id, Some(shape)))
}

/// The arrows a line may hold, longest first (so "-.->" is not read as "->").
const ARROWS: [&str; 6] = ["-.->", "==>", "-->", "---", "-.-", "->"];

/// Splits "A -->|sim| B --> C" into nodes and the edges between them (with their labels and kind).
fn chain(line: &str) -> Option<(Vec<String>, Vec<(String, bool)>)> {
    let mut parts = Vec::new();
    let mut links = Vec::new();
    let mut rest = line.trim().to_string();
    loop {
        // the next arrow outside brackets
        let mut found: Option<(usize, &str)> = None;
        let mut depth = 0i32;
        let bytes: Vec<char> = rest.chars().collect();
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                '[' | '(' | '{' => depth += 1,
                ']' | ')' | '}' => depth -= 1,
                _ if depth == 0 => {
                    let tail: String = bytes[i..].iter().collect();
                    // "-- texto -->": a label written inside the arrow
                    if tail.starts_with("-- ") || tail.starts_with("== ") {
                        found = Some((i, if tail.starts_with("==") { "==" } else { "--" }));
                        break;
                    }
                    if let Some(a) = ARROWS.iter().find(|a| tail.starts_with(**a)) {
                        found = Some((i, a));
                        break;
                    }
                }
                _ => {}
            }
            i += 1;
        }
        let Some((at, arrow)) = found else {
            parts.push(rest.trim().to_string());
            break;
        };
        let before: String = rest.chars().take(at).collect();
        let mut after: String = rest.chars().skip(at + arrow.chars().count()).collect();
        let mut label = String::new();
        if arrow == "--" || arrow == "==" {
            // "-- rótulo -->"
            let close = ["-->", "==>", "---"].iter().filter_map(|a| after.find(a).map(|k| (k, a.len()))).min();
            let (k, n) = close?;
            label = after[..k].trim().to_string();
            after = after[k + n..].to_string();
        } else if after.trim_start().starts_with('|') {
            let t = after.trim_start()[1..].to_string();
            let k = t.find('|')?;
            label = t[..k].trim().to_string();
            after = t[k + 1..].to_string();
        }
        parts.push(before.trim().to_string());
        links.push((label, arrow.contains('.')));
        rest = after;
    }
    (parts.len() >= 2).then_some((parts, links))
}

/// Reads a mermaid flowchart (None when it is not one this block draws).
pub fn parse(src: &str) -> Option<Graph> {
    let mut lines = src.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("%%"));
    let head = lines.next()?;
    let mut hw = head.split_whitespace();
    if !matches!(hw.next()?, "flowchart" | "graph") {
        return None;
    }
    let horizontal = matches!(hw.next().unwrap_or("TD"), "LR" | "RL");
    let mut g = Graph { horizontal, nodes: Vec::new(), edges: Vec::new() };
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut add = |g: &mut Graph, r: &str| -> Option<usize> {
        let (id, shape) = node_ref(r)?;
        let i = *index.entry(id.clone()).or_insert_with(|| {
            g.nodes.push(Node { text: id.clone(), shape: Shape::Rect });
            g.nodes.len() - 1
        });
        if let Some((t, s)) = shape {
            g.nodes[i].text = t;
            g.nodes[i].shape = s;
        }
        Some(i)
    };
    for l in lines {
        if l.starts_with("style ") || l.starts_with("classDef ") || l.starts_with("class ") || l.starts_with("linkStyle ") || l == "end" || l.starts_with("subgraph ") {
            continue;
        }
        // "A & B --> C" is several edges
        if let Some((parts, links)) = chain(l) {
            let ids: Vec<Vec<usize>> = parts.iter().map(|p| p.split(" & ").filter_map(|x| add(&mut g, x)).collect()).collect();
            for (k, (label, dashed)) in links.iter().enumerate() {
                for &a in &ids[k] {
                    for &b in &ids[k + 1] {
                        g.edges.push(Edge { from: a, to: b, label: label.clone(), dashed: *dashed });
                    }
                }
            }
        } else {
            add(&mut g, l);
        }
    }
    (!g.nodes.is_empty()).then_some(g)
}

/// Each node's layer (the longest path from a source, edges that go back ignored) and its place in the layer.
pub fn layout(g: &Graph) -> (Vec<usize>, Vec<usize>) {
    let n = g.nodes.len();
    // edges that close a cycle (found by a depth-first walk) do not count for the layers
    let mut state = vec![0u8; n];
    let mut back = vec![false; g.edges.len()];
    fn dfs(v: usize, g: &Graph, state: &mut [u8], back: &mut [bool]) {
        state[v] = 1;
        for (k, e) in g.edges.iter().enumerate().filter(|(_, e)| e.from == v) {
            match state[e.to] {
                0 => dfs(e.to, g, state, back),
                1 => back[k] = true,
                _ => {}
            }
        }
        state[v] = 2;
    }
    for v in 0..n {
        if state[v] == 0 {
            dfs(v, g, &mut state, &mut back);
        }
    }
    let mut layer = vec![0usize; n];
    for _ in 0..n {
        let mut changed = false;
        for (k, e) in g.edges.iter().enumerate() {
            if !back[k] && e.from != e.to && layer[e.to] < layer[e.from] + 1 {
                layer[e.to] = layer[e.from] + 1;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    // the order inside each layer: first seen, then by the mean place of the nodes before them (two passes)
    let layers = layer.iter().max().map(|m| m + 1).unwrap_or(1);
    let mut order: Vec<f64> = (0..n).map(|i| i as f64).collect();
    for _ in 0..2 {
        for l in 1..layers {
            let mut ins: Vec<usize> = (0..n).filter(|&v| layer[v] == l).collect();
            let key = |v: usize| -> f64 {
                let prev: Vec<f64> = g.edges.iter().filter(|e| e.to == v && layer[e.from] < l).map(|e| order[e.from]).collect();
                if prev.is_empty() { order[v] } else { prev.iter().sum::<f64>() / prev.len() as f64 }
            };
            ins.sort_by(|a, b| key(*a).total_cmp(&key(*b)));
            for (k, v) in ins.into_iter().enumerate() {
                order[v] = k as f64;
            }
        }
    }
    let mut place = vec![0usize; n];
    for l in 0..layers {
        let mut ins: Vec<usize> = (0..n).filter(|&v| layer[v] == l).collect();
        ins.sort_by(|a, b| order[*a].total_cmp(&order[*b]));
        for (k, v) in ins.into_iter().enumerate() {
            place[v] = k;
        }
    }
    (layer, place)
}

struct Drawn {
    /// each node's box (x, y, w, h), its layer, and the whole drawing's size
    boxes: Vec<(f64, f64, f64, f64)>,
    layer: Vec<usize>,
    size: (f64, f64),
}

fn text_layout(cr: &gtk::cairo::Context, t: &str, size: f64) -> gtk::pango::Layout {
    let l = pangocairo::functions::create_layout(cr);
    let mut f = gtk::pango::FontDescription::from_string(crate::tema::fonte_ui());
    f.set_absolute_size(size * f64::from(gtk::pango::SCALE));
    l.set_font_description(Some(&f));
    l.set_text(t);
    l
}

fn measure(cr: &gtk::cairo::Context, g: &Graph) -> Drawn {
    let (layer, place) = layout(g);
    let sizes: Vec<(f64, f64)> = g.nodes.iter().map(|n| {
        let (w, h) = text_layout(cr, &n.text, 13.0).pixel_size();
        let (w, h) = (f64::from(w) + 32.0, f64::from(h) + 22.0);
        if n.shape == Shape::Diamond { (w * 1.35, h * 1.6) } else { (w.max(80.0), h.max(38.0)) }
    }).collect();
    let layers = layer.iter().max().map(|m| m + 1).unwrap_or(1);
    let (gap_l, gap_p) = (if g.horizontal { 64.0 } else { 52.0 }, if g.horizontal { 36.0 } else { 40.0 });
    // the room of each layer (across the flow) and of each place (along it)
    let mut span_l = vec![0.0f64; layers];
    let places = place.iter().max().map(|m| m + 1).unwrap_or(1);
    let mut span_p = vec![0.0f64; places];
    for (i, (w, h)) in sizes.iter().enumerate() {
        let (along, across) = if g.horizontal { (*w, *h) } else { (*h, *w) };
        span_l[layer[i]] = span_l[layer[i]].max(along);
        span_p[place[i]] = span_p[place[i]].max(across);
    }
    let start_l: Vec<f64> = span_l.iter().scan(16.0, |acc, s| { let v = *acc; *acc += s + gap_l; Some(v) }).collect();
    let start_p: Vec<f64> = span_p.iter().scan(16.0, |acc, s| { let v = *acc; *acc += s + gap_p; Some(v) }).collect();
    let boxes: Vec<(f64, f64, f64, f64)> = sizes.iter().enumerate().map(|(i, (w, h))| {
        let (l, p) = (layer[i], place[i]);
        if g.horizontal {
            (start_l[l] + (span_l[l] - w) / 2.0, start_p[p] + (span_p[p] - h) / 2.0, *w, *h)
        } else {
            (start_p[p] + (span_p[p] - w) / 2.0, start_l[l] + (span_l[l] - h) / 2.0, *w, *h)
        }
    }).collect();
    let total_l = start_l.last().unwrap_or(&16.0) + span_l.last().unwrap_or(&0.0) + 16.0;
    // (room at the side for the edges that go around a layer)
    let detour = if g.edges.iter().any(|e| layer[e.to].abs_diff(layer[e.from]) > 1) { 48.0 } else { 0.0 };
    let total_p = start_p.last().unwrap_or(&16.0) + span_p.last().unwrap_or(&0.0) + 16.0 + detour;
    Drawn { boxes, layer, size: if g.horizontal { (total_l, total_p) } else { (total_p, total_l) } }
}

/// The node's colour: the sources neutral, questions gold, the ends teal, the rest the accent.
fn colour(g: &Graph, i: usize) -> &'static str {
    if g.nodes[i].shape == Shape::Diamond {
        "#E5C98C"
    } else if !g.edges.iter().any(|e| e.to == i) {
        "#8E8F99"
    } else if !g.edges.iter().any(|e| e.from == i) {
        "#8ED2C3"
    } else {
        "#A5A1E2"
    }
}

/// Where an edge leaves or reaches a box: the middle of the side facing the other end.
fn port(b: (f64, f64, f64, f64), toward: (f64, f64), diamond: bool) -> (f64, f64) {
    let (cx, cy) = (b.0 + b.2 / 2.0, b.1 + b.3 / 2.0);
    let (dx, dy) = (toward.0 - cx, toward.1 - cy);
    let _ = diamond;
    if dx.abs() * b.3 > dy.abs() * b.2 {
        (if dx > 0.0 { b.0 + b.2 } else { b.0 }, cy)
    } else {
        (cx, if dy > 0.0 { b.1 + b.3 } else { b.1 })
    }
}

fn draw(cr: &gtk::cairo::Context, g: &Graph, d: &Drawn, ox: f64, oy: f64, square: bool, mono: bool) {
    let rgb = |h: &str| crate::tema::rgb(h);
    let muted = rgb("#8E8F99");
    cr.set_line_width(1.5);
    // edges first (under the boxes), each an elbow from side to side, with its arrow and label
    for e in &g.edges {
        let (a, b) = (d.boxes[e.from], d.boxes[e.to]);
        let ca = (a.0 + a.2 / 2.0, a.1 + a.3 / 2.0);
        let cb = (b.0 + b.2 / 2.0, b.1 + b.3 / 2.0);
        cr.set_source_rgb(muted.0, muted.1, muted.2);
        if e.dashed {
            cr.set_dash(&[5.0, 4.0], 0.0);
        }
        // an edge that skips a layer goes around the layers between, at their side (not through their nodes)
        let desvio = d.layer[e.to].abs_diff(d.layer[e.from]) > 1;
        let (p0, mid, p1) = if desvio {
            let (lo, hi) = (d.layer[e.from].min(d.layer[e.to]), d.layer[e.from].max(d.layer[e.to]));
            let between = (0..g.nodes.len()).filter(|&k| d.layer[k] >= lo && d.layer[k] <= hi);
            if g.horizontal {
                let low = between.map(|k| d.boxes[k].1 + d.boxes[k].3).fold(0.0f64, f64::max) + 18.0;
                let (s0, s1) = ((ca.0, a.1 + a.3), (cb.0, b.1 + b.3));
                cr.move_to(ox + s0.0, oy + s0.1);
                cr.line_to(ox + s0.0, oy + low);
                cr.line_to(ox + s1.0, oy + low);
                cr.line_to(ox + s1.0, oy + s1.1);
                (s0, (s1.0, low), s1)
            } else {
                let right = between.map(|k| d.boxes[k].0 + d.boxes[k].2).fold(0.0f64, f64::max) + 18.0;
                let (s0, s1) = ((a.0 + a.2, ca.1), (b.0 + b.2, cb.1));
                cr.move_to(ox + s0.0, oy + s0.1);
                cr.line_to(ox + right, oy + s0.1);
                cr.line_to(ox + right, oy + s1.1);
                cr.line_to(ox + s1.0, oy + s1.1);
                (s0, (right, s1.1), s1)
            }
        } else {
            let p0 = port(a, cb, g.nodes[e.from].shape == Shape::Diamond);
            let p1 = port(b, ca, g.nodes[e.to].shape == Shape::Diamond);
            let horizontal_start = p0.1 == ca.1;
            let mid = if horizontal_start { (p1.0, p0.1) } else { (p0.0, p1.1) };
            cr.move_to(ox + p0.0, oy + p0.1);
            cr.line_to(ox + mid.0, oy + mid.1);
            cr.line_to(ox + p1.0, oy + p1.1);
            (p0, mid, p1)
        };
        let _ = cr.stroke();
        cr.set_dash(&[], 0.0);
        // the arrowhead, along the last segment (a straight edge: its elbow is its end, the whole edge says the way)
        let (mut dx, mut dy) = (p1.0 - mid.0, p1.1 - mid.1);
        if dx.abs() + dy.abs() < 1.0 {
            (dx, dy) = (p1.0 - p0.0, p1.1 - p0.1);
        }
        let len = (dx * dx + dy * dy).sqrt().max(1e-6);
        let (ux, uy) = (dx / len, dy / len);
        cr.move_to(ox + p1.0, oy + p1.1);
        cr.line_to(ox + p1.0 - ux * 7.0 - uy * 4.0, oy + p1.1 - uy * 7.0 + ux * 4.0);
        cr.line_to(ox + p1.0 - ux * 7.0 + uy * 4.0, oy + p1.1 - uy * 7.0 - ux * 4.0);
        cr.close_path();
        let _ = cr.fill();
        if !e.label.is_empty() && desvio {
            // (a detour's label beside its long side)
            let l = text_layout(cr, &e.label, 11.0);
            let (lw, lh) = l.pixel_size();
            if g.horizontal {
                cr.move_to(ox + (p0.0 + p1.0) / 2.0 - f64::from(lw) / 2.0, oy + mid.1 + 3.0);
            } else {
                cr.move_to(ox + mid.0 + 6.0, oy + (p0.1 + p1.1) / 2.0 - f64::from(lh) / 2.0);
            }
            pangocairo::functions::show_layout(cr, &l);
        } else if !e.label.is_empty() {
            let l = text_layout(cr, &e.label, 11.0);
            let (lw, lh) = l.pixel_size();
            let (mx, my) = if (mid.0 - p0.0).abs() + (mid.1 - p0.1).abs() > (p1.0 - mid.0).abs() + (p1.1 - mid.1).abs() {
                ((p0.0 + mid.0) / 2.0, (p0.1 + mid.1) / 2.0)
            } else {
                ((mid.0 + p1.0) / 2.0, (mid.1 + p1.1) / 2.0)
            };
            cr.move_to(ox + mx - f64::from(lw) / 2.0 + if (mid.0 - p0.0).abs() < 1.0 { f64::from(lw) / 2.0 + 6.0 } else { 0.0 }, oy + my - f64::from(lh) - 3.0);
            pangocairo::functions::show_layout(cr, &l);
        }
    }
    for (i, n) in g.nodes.iter().enumerate() {
        let (x, y, w, h) = d.boxes[i];
        let (x, y) = (ox + x, oy + y);
        let c = rgb(if mono { "#8E8F99" } else { colour(g, i) });
        let fill = rgb("#1C1D23");
        cr.new_path();
        match n.shape {
            Shape::Diamond => {
                cr.move_to(x + w / 2.0, y);
                cr.line_to(x + w, y + h / 2.0);
                cr.line_to(x + w / 2.0, y + h);
                cr.line_to(x, y + h / 2.0);
                cr.close_path();
            }
            Shape::Circle => {
                cr.arc(x + w / 2.0, y + h / 2.0, w.min(h) / 2.0, 0.0, std::f64::consts::TAU);
            }
            _ => {
                let r = if n.shape == Shape::Round { h / 2.0 } else if square { 0.5 } else { 4.0 };
                cr.arc(x + w - r, y + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
                cr.arc(x + w - r, y + h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
                cr.arc(x + r, y + h - r, r, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
                cr.arc(x + r, y + r, r, std::f64::consts::PI, 3.0 * std::f64::consts::FRAC_PI_2);
                cr.close_path();
            }
        }
        cr.set_source_rgb(fill.0, fill.1, fill.2);
        let _ = cr.fill_preserve();
        cr.set_source_rgb(c.0, c.1, c.2);
        let _ = cr.stroke();
        let l = text_layout(cr, &n.text, 13.0);
        let (lw, lh) = l.pixel_size();
        let t = rgb("#E2E2E6");
        cr.set_source_rgb(t.0, t.1, t.2);
        cr.move_to(x + (w - f64::from(lw)) / 2.0, y + (h - f64::from(lh)) / 2.0);
        pangocairo::functions::show_layout(cr, &l);
    }
}

/// A diagram on screen: its source (edited from the context's Canvas tab), its graph and how it looks.
pub struct Diagram {
    pub src: RefCell<String>,
    graph: RefCell<Rc<Graph>>,
    pub square: Cell<bool>,
    /// one colour for every node instead of the theme's
    pub mono: Cell<bool>,
    /// the width it has (a left-to-right chart wider than that is drawn top to bottom)
    width: Cell<i32>,
    area: gtk::DrawingArea,
    detail: gtk::Label,
    code: gtk::Box,
}

thread_local! {
    /// the latest diagram of the conversations (the one the Canvas tab edits) and who wants to know of a new one
    static CURRENT: RefCell<Option<std::rc::Weak<Diagram>>> = const { RefCell::new(None) };
    static LISTENERS: RefCell<Vec<Box<dyn Fn()>>> = const { RefCell::new(Vec::new()) };
}

/// The diagram the Canvas tab edits (the latest one drawn, while it is on screen).
pub fn current() -> Option<Rc<Diagram>> {
    CURRENT.with(|c| c.borrow().as_ref().and_then(|w| w.upgrade()))
}

/// Called when a new diagram is drawn.
pub fn on_new(f: impl Fn() + 'static) {
    LISTENERS.with(|l| l.borrow_mut().push(Box::new(f)));
}

fn size_of(g: &Graph) -> (f64, f64) {
    gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, 1, 1).ok()
        .and_then(|s| gtk::cairo::Context::new(&s).ok()).map(|cr| measure(&cr, g).size).unwrap_or((400.0, 200.0))
}

impl Diagram {
    /// The graph as drawn in the width it has: a left-to-right chart that does not fit goes top to bottom.
    fn shown(&self) -> Rc<Graph> {
        let g = self.graph.borrow().clone();
        let w = self.width.get();
        if g.horizontal && w > 0 && size_of(&g).0 > f64::from(w) {
            let mut v = (*g).clone();
            v.horizontal = false;
            return Rc::new(v);
        }
        g
    }

    fn refit(&self) {
        let g = self.shown();
        let (_, h) = size_of(&g);
        // (no width of its own: it fits the column, laid out down when across does not fit)
        self.area.set_content_width(0);
        self.area.set_content_height(h.ceil() as i32);
        self.detail.set_text(&format!("{} · {}", tr("flowchart"), trf("{n} nodes", &[("n", g.nodes.len().to_string())])));
        while let Some(c) = self.code.first_child() {
            self.code.remove(&c);
        }
        self.code.append(&crate::code_block::new("mermaid", &self.src.borrow(), false));
        self.area.queue_draw();
    }

    /// A new source (edited): drawn again when it reads as a flowchart; false when it does not (kept as it was).
    pub fn set_source(&self, src: &str) -> bool {
        let Some(g) = parse(src) else { return false };
        *self.src.borrow_mut() = src.to_string();
        *self.graph.borrow_mut() = Rc::new(g);
        self.refit();
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug diagrama: editado ({} nós)", self.graph.borrow().nodes.len());
        }
        true
    }

    /// Left to right or top to bottom (the source's first line says it).
    pub fn set_horizontal(&self, h: bool) {
        let src = self.src.borrow().clone();
        let mut lines: Vec<String> = src.lines().map(str::to_string).collect();
        if let Some(first) = lines.iter_mut().find(|l| !l.trim().is_empty() && !l.trim().starts_with("%%")) {
            let kw = first.split_whitespace().next().unwrap_or("flowchart").to_string();
            *first = format!("{kw} {}", if h { "LR" } else { "TD" });
        }
        self.set_source(&lines.join("\n"));
    }

    pub fn horizontal(&self) -> bool {
        self.graph.borrow().horizontal
    }

    pub fn set_mono(&self, m: bool) {
        self.mono.set(m);
        self.area.queue_draw();
    }

    pub fn set_square(&self, sq: bool) {
        self.square.set(sq);
        self.area.queue_draw();
    }

    /// The drawing as a vector file: SVG or PDF (by the path's extension), on the panel's dark.
    pub fn save_vector(&self, p: &std::path::Path) -> Result<(), String> {
        let g = self.graph.borrow().clone();
        let (w, h) = size_of(&g);
        let paint = |cr: &gtk::cairo::Context| {
            let bg = crate::tema::rgb("#101114");
            cr.set_source_rgb(bg.0, bg.1, bg.2);
            let _ = cr.paint();
            let d = measure(cr, &g);
            draw(cr, &g, &d, 0.0, 0.0, self.square.get(), self.mono.get());
        };
        if p.extension().is_some_and(|e| e == "pdf") {
            let s = gtk::cairo::PdfSurface::new(w, h, p).map_err(|e| e.to_string())?;
            let cr = gtk::cairo::Context::new(&s).map_err(|e| e.to_string())?;
            paint(&cr);
            drop(cr);
            s.finish();
        } else {
            let s = gtk::cairo::SvgSurface::new(w, h, Some(p)).map_err(|e| e.to_string())?;
            let cr = gtk::cairo::Context::new(&s).map_err(|e| e.to_string())?;
            paint(&cr);
            drop(cr);
            s.finish();
        }
        Ok(())
    }

    /// The drawing as a PNG (twice its size, on the panel's dark).
    pub fn save_png(&self, p: &std::path::Path) -> Result<(), String> {
        let g = self.graph.borrow().clone();
        let size = size_of(&g);
        let s = gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, (size.0 * 2.0) as i32, (size.1 * 2.0) as i32).map_err(|e| e.to_string())?;
        let cr = gtk::cairo::Context::new(&s).map_err(|e| e.to_string())?;
        cr.scale(2.0, 2.0);
        let bg = crate::tema::rgb("#101114");
        cr.set_source_rgb(bg.0, bg.1, bg.2);
        let _ = cr.paint();
        let d = measure(&cr, &g);
        draw(&cr, &g, &d, 0.0, 0.0, self.square.get(), self.mono.get());
        drop(cr);
        // (the pixels through a GDK texture, which writes PNG)
        s.flush();
        let (w, h, stride) = (s.width(), s.height(), s.stride() as usize);
        let mut s = s;
        let data = s.data().map_err(|e| e.to_string())?.to_vec();
        let tex = gtk::gdk::MemoryTexture::new(w, h, gtk::gdk::MemoryFormat::B8g8r8a8Premultiplied, &gtk::glib::Bytes::from_owned(data), stride);
        tex.save_to_png(p).map_err(|e| e.to_string())
    }
}

/// The block for a mermaid source (None when it is not a flowchart: the code block shows it instead). `conversa`: an
/// answer of the agent's (the Canvas tab edits it), not a document's preview.
pub fn new(src: &str, conversa: bool) -> Option<gtk::Widget> {
    let g = Rc::new(parse(src)?);
    let bloco = caixa(Orientation::Vertical, 0, "md-bloco");
    let cab = caixa(Orientation::Horizontal, 8, "md-bloco-cab");
    cab.append(&ui_icon("camadas", "#8E8F99", 16));
    cab.append(&rotulo("Mermaid", "md-bloco-titulo"));
    let det = rotulo("", "md-bloco-detalhe");
    det.set_hexpand(true);
    det.set_xalign(0.0);
    cab.append(&det);
    let seg = caixa(Orientation::Horizontal, 2, "tabela-segmentado");
    seg.set_valign(Align::Center);
    let b_d = gtk::ToggleButton::with_label(tr("Diagram"));
    let b_c = gtk::ToggleButton::with_label(tr("Code"));
    for b in [&b_d, &b_c] {
        b.add_css_class("tabela-aba");
        seg.append(b);
    }
    b_c.set_group(Some(&b_d));
    b_d.set_active(true);
    cab.append(&seg);
    let png = gtk::Button::new();
    png.add_css_class("md-bloco-icone");
    png.set_child(Some(&ui_icon("download", "#8E8F99", 16)));
    png.set_tooltip_text(Some(tr("Save as PNG in Downloads")));
    png.set_valign(Align::Center);
    cab.append(&png);
    bloco.append(&cab);
    bloco.append(&caixa(Orientation::Horizontal, 0, "md-bloco-div"));
    let vistas = gtk::Stack::new();
    vistas.set_vhomogeneous(false);
    vistas.set_interpolate_size(true);
    let area = gtk::DrawingArea::new();
    area.set_hexpand(true);
    let rol = gtk::ScrolledWindow::new();
    rol.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Never);
    rol.set_propagate_natural_height(true);
    rol.set_child(Some(&area));
    rol.add_css_class("diagrama-area");
    vistas.add_named(&rol, Some("diagrama"));
    let code = caixa(Orientation::Vertical, 0, "");
    vistas.add_named(&code, Some("codigo"));
    bloco.append(&vistas);
    let d = Rc::new(Diagram { src: RefCell::new(src.to_string()), graph: RefCell::new(g.clone()), square: Cell::new(false), mono: Cell::new(false), width: Cell::new(0), area: area.clone(), detail: det, code });
    d.refit();
    let dw = Rc::downgrade(&d);
    area.connect_resize(move |_, w, _| {
        let Some(d) = dw.upgrade() else { return };
        if d.width.replace(w) != w {
            let d2 = d.clone();
            gtk::glib::idle_add_local_once(move || {
                let (_, h) = size_of(&d2.shown());
                d2.area.set_content_height(h.ceil() as i32);
            });
        }
    });
    let dw = Rc::downgrade(&d);
    area.set_draw_func(move |_, cr, w, _| {
        let Some(d) = dw.upgrade() else { return };
        let g = d.shown();
        let m = measure(cr, &g);
        draw(cr, &g, &m, ((f64::from(w) - m.size.0) / 2.0).max(0.0), 0.0, d.square.get(), d.mono.get());
    });
    let v = vistas.clone();
    b_c.connect_toggled(move |b| v.set_visible_child_name(if b.is_active() { "codigo" } else { "diagrama" }));
    let dw = Rc::downgrade(&d);
    png.connect_clicked(move |b| {
        let Some(d) = dw.upgrade() else { return };
        let dir = crate::pastas::downloads();
        let _ = std::fs::create_dir_all(&dir);
        let p = crate::widgets::nome_livre(&dir, tr("diagram"), "png");
        match d.save_png(&p) {
            Ok(()) => {
                b.set_tooltip_text(Some(&trf("Saved in {folder}", &[("folder", dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())])));
                if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                    eprintln!("debug diagrama: png salvo {}", p.display());
                }
            }
            Err(e) => eprintln!("kalango-shell: diagrama em PNG: {e}"),
        }
    });
    // (the block keeps its diagram; the Canvas tab edits the latest one)
    unsafe {
        bloco.set_data("diagrama", d.clone());
    }
    if conversa {
        CURRENT.with(|c| *c.borrow_mut() = Some(Rc::downgrade(&d)));
        LISTENERS.with(|l| {
            for f in l.borrow().iter() {
                f();
            }
        });
    }
    if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
        eprintln!("debug diagrama: fluxograma {} nós, {} ligações", g.nodes.len(), g.edges.len());
    }
    Some(bloco.upcast())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DESIGN: &str = "flowchart LR\n    A[Pedido] --> B[Entender a intenção]\n    B --> C{Risco alto?}\n    C -->|sim| D[Pedir aprovação]\n    C -->|não| E[Executar]\n    D -->|aprovado| E\n    E --> F[Registrar na Linha do tempo]";

    #[test]
    fn the_designs_flowchart() {
        let g = parse(DESIGN).unwrap();
        assert!(g.horizontal);
        assert_eq!(g.nodes.len(), 6);
        assert_eq!(g.edges.len(), 6);
        assert_eq!(g.nodes[2].shape, Shape::Diamond);
        assert_eq!(g.nodes[2].text, "Risco alto?");
        assert!(g.edges.iter().any(|e| e.label == "sim" && g.nodes[e.to].text == "Pedir aprovação"));
        let (layer, _) = layout(&g);
        // Executar after Pedir aprovação (the longest path), Registrar last
        assert!(layer[4] > layer[3]);
        assert!(layer[5] > layer[4]);
    }

    #[test]
    fn other_writings() {
        let g = parse("graph TD\nA((Início)) -- sim --> B([Fim])\nB -.-> C & D").unwrap();
        assert!(!g.horizontal);
        assert_eq!(g.nodes[0].shape, Shape::Circle);
        assert_eq!(g.edges[0].label, "sim");
        assert_eq!(g.edges.len(), 3);
        assert!(g.edges[1].dashed);
        assert!(parse("sequenceDiagram\nA->>B: oi").is_none());
        // a cycle still lays out
        let g = parse("flowchart LR\nA-->B\nB-->A").unwrap();
        let (layer, _) = layout(&g);
        assert_ne!(layer[0], layer[1]);
    }
}
