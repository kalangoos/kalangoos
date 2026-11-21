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
    // (a class given in place, "A[texto]:::destaque", is only looks: the node is the same)
    let s = s.find(":::").map(|k| s[..k].trim()).unwrap_or(s);
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

/// The arrows a line may hold, longest first (so "-.->" is not read as "->"); a longer one ("--->", "==>>") takes
/// its extra dashes, and "--o"/"--x" (circle and cross ends) are drawn as arrows.
const ARROWS: [&str; 11] = ["<-->", "-.->", "==>", "-->", "---", "===", "-.-", "--o", "--x", "->", "~~~"];

/// Splits "A -->|sim| B --> C" into nodes and the edges between them (with their labels and kind).
fn chain(line: &str) -> Option<(Vec<String>, Vec<(String, bool)>)> {
    let mut parts = Vec::new();
    let mut links = Vec::new();
    let mut rest = line.trim().to_string();
    loop {
        // the next arrow outside brackets
        let mut found: Option<(usize, String)> = None;
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
                        found = Some((i, if tail.starts_with("==") { "==" } else { "--" }.to_string()));
                        break;
                    }
                    if let Some(a) = ARROWS.iter().find(|a| tail.starts_with(**a) && !(a.ends_with(['o', 'x']) && tail[a.len()..].starts_with(|c: char| c.is_alphanumeric()))) {
                        // (the extra length of "--->" or "-..->" is the same arrow)
                        let extra = tail[a.len()..].chars().take_while(|c| matches!(c, '-' | '=' | '.' | '>')).count();
                        found = Some((i, tail[..a.len() + extra].to_string()));
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
            label = after[..k].trim().trim_matches('"').to_string();
            after = after[k + n..].to_string();
        } else if after.trim_start().starts_with('|') {
            let t = after.trim_start()[1..].to_string();
            let k = t.find('|')?;
            label = t[..k].trim().trim_matches('"').to_string();
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

/// A step of a sequence diagram.
#[derive(Clone, Debug, PartialEq)]
pub enum SeqItem {
    /// `A->>B: texto` (`-->>`/`-->` dashed: a reply; `-x`/`-)` drawn as arrows)
    Message { from: usize, to: usize, text: String, dashed: bool },
    /// `Note over A,B: texto`, `Note right of A: texto` (the participants it spans)
    Note { first: usize, last: usize, text: String },
    /// `loop`, `alt`, `opt`, `par`, `critical`, `break`, `rect` up to their `end`: a frame with its label
    Start { kind: String, label: String },
    /// `else` / `and` inside a frame: a dashed line across it
    Else { label: String },
    End,
}

/// A mermaid sequence diagram: who takes part (in order) and what happens.
#[derive(Clone, Debug, PartialEq)]
pub struct Sequence {
    pub participants: Vec<String>,
    pub items: Vec<SeqItem>,
}

/// The arrows of a sequence diagram, longest first.
const SEQ_ARROWS: [(&str, bool); 8] = [("-->>", true), ("->>", false), ("--x", true), ("-x", false), ("--)", true), ("-)", false), ("-->", true), ("->", false)];

/// Reads a mermaid sequence diagram (None when it is not one, or has no message).
pub fn parse_sequence(src: &str) -> Option<Sequence> {
    let mut lines = src.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("%%"));
    if lines.next()? != "sequenceDiagram" {
        return None;
    }
    let mut ids: Vec<String> = Vec::new();
    let mut seq = Sequence { participants: Vec::new(), items: Vec::new() };
    fn who(ids: &mut Vec<String>, seq: &mut Sequence, id: &str, label: Option<&str>) -> Option<usize> {
        let id = id.trim().trim_start_matches(['+', '-']).trim();
        if id.is_empty() || id.contains(char::is_whitespace) && !id.starts_with('"') {
            return None;
        }
        let id = id.trim_matches('"').to_string();
        let k = match ids.iter().position(|x| *x == id) {
            Some(k) => k,
            None => {
                ids.push(id.clone());
                seq.participants.push(id);
                ids.len() - 1
            }
        };
        if let Some(l) = label {
            seq.participants[k] = l.trim().trim_matches('"').to_string();
        }
        Some(k)
    }
    let mut depth = 0usize;
    for l in lines {
        let first = l.split_whitespace().next().unwrap_or("");
        let rest = l[first.len()..].trim();
        match first {
            "participant" | "actor" => {
                let (id, label) = match rest.split_once(" as ") {
                    Some((a, b)) => (a, Some(b)),
                    None => (rest, None),
                };
                who(&mut ids, &mut seq, id, label)?;
            }
            "loop" | "alt" | "opt" | "par" | "critical" | "break" | "rect" => {
                depth += 1;
                seq.items.push(SeqItem::Start { kind: first.to_string(), label: rest.to_string() });
            }
            "else" | "and" | "option" => seq.items.push(SeqItem::Else { label: rest.to_string() }),
            "end" => {
                depth = depth.checked_sub(1)?;
                seq.items.push(SeqItem::End);
            }
            "Note" | "note" => {
                let (place, text) = rest.split_once(':')?;
                let place = place.trim();
                let names = place.strip_prefix("over").or_else(|| place.strip_prefix("right of")).or_else(|| place.strip_prefix("left of"))?;
                let ks: Vec<usize> = names.split(',').map(|n| who(&mut ids, &mut seq, n, None)).collect::<Option<_>>()?;
                let (a, b) = (*ks.iter().min()?, *ks.iter().max()?);
                seq.items.push(SeqItem::Note { first: a, last: b, text: text.trim().to_string() });
            }
            "autonumber" | "activate" | "deactivate" | "title" | "box" | "accTitle:" | "accDescr:" | "create" | "destroy" | "links" | "link" => {}
            _ => {
                // a message: "A->>B: texto"
                let (at, (arrow, dashed)) = SEQ_ARROWS.iter().filter_map(|a| l.find(a.0).map(|k| (k, *a))).min_by_key(|(k, a)| (*k, std::cmp::Reverse(a.0.len())))?;
                let from = who(&mut ids, &mut seq, &l[..at], None)?;
                let after = &l[at + arrow.len()..];
                let (to, text) = after.split_once(':').unwrap_or((after, ""));
                let to = who(&mut ids, &mut seq, to, None)?;
                seq.items.push(SeqItem::Message { from, to, text: text.trim().to_string(), dashed });
            }
        }
    }
    (depth == 0 && seq.items.iter().any(|i| matches!(i, SeqItem::Message { .. }))).then_some(seq)
}

/// Where a sequence diagram's parts go: each participant's box (its centre is its lifeline), the top of each step,
/// the frames (x0, y0, x1, y1, the item that starts them, the tops of their `else` lines) and the whole size.
#[derive(Debug)]
pub struct SeqLayout {
    pub heads: Vec<(f64, f64, f64, f64)>,
    pub rows: Vec<f64>,
    pub frames: Vec<(f64, f64, f64, f64, usize, Vec<f64>)>,
    pub size: (f64, f64),
}

/// Lays a sequence diagram out with `measure(text, size) -> (w, h)` (pango in the shell, an estimate in tests).
pub fn layout_sequence(s: &Sequence, measure: &dyn Fn(&str, f64) -> (f64, f64)) -> SeqLayout {
    let n = s.participants.len();
    let widths: Vec<f64> = s.participants.iter().map(|p| (measure(p, 13.0).0 + 32.0).max(100.0)).collect();
    // the lifelines: far enough apart for the boxes, and for each message's label between its two ends
    let mut centre = vec![0.0f64; n];
    let mut x = 16.0;
    for k in 0..n {
        centre[k] = x + widths[k] / 2.0;
        x += widths[k] + 48.0;
    }
    for it in &s.items {
        let (a, b, need) = match it {
            SeqItem::Message { from, to, text, .. } if from != to => ((*from).min(*to), (*from).max(*to), measure(text, 11.0).0 + 32.0),
            SeqItem::Note { first, last, text } if first == last => (*first, (*first + 1).min(n - 1), measure(text, 12.0).0 / 2.0 + 24.0 + widths[*first] / 2.0),
            _ => continue,
        };
        if a == b {
            continue;
        }
        let have = centre[b] - centre[a];
        if have < need {
            for c in centre.iter_mut().skip(b) {
                *c += need - have;
            }
        }
    }
    let top = 16.0;
    let head_h = 36.0;
    let heads: Vec<(f64, f64, f64, f64)> = (0..n).map(|k| (centre[k] - widths[k] / 2.0, top, widths[k], head_h)).collect();
    let left = 16.0;
    let right = heads.last().map(|h| h.0 + h.2).unwrap_or(116.0);
    let mut y = top + head_h + 20.0;
    let mut rows = Vec::new();
    let mut frames = Vec::new();
    let mut open: Vec<(usize, f64, Vec<f64>)> = Vec::new();
    for (k, it) in s.items.iter().enumerate() {
        rows.push(y);
        y += match it {
            SeqItem::Message { from, to, .. } if from == to => 52.0,
            SeqItem::Message { .. } => 40.0,
            SeqItem::Note { text, .. } => measure(text, 12.0).1 + 28.0,
            SeqItem::Start { .. } => {
                open.push((k, y, Vec::new()));
                30.0
            }
            SeqItem::Else { .. } => {
                if let Some(o) = open.last_mut() {
                    o.2.push(y);
                }
                30.0
            }
            SeqItem::End => {
                if let Some((start, y0, elses)) = open.pop() {
                    let inset = 8.0 * (open.len() as f64 + 1.0);
                    frames.push((left - 8.0 + inset, y0, right + 8.0 - inset, y + 6.0, start, elses));
                }
                14.0
            }
        };
    }
    SeqLayout { heads, rows, frames, size: (right + 16.0 + 8.0, y + 12.0) }
}

struct Drawn {
    /// each node's box (x, y, w, h), its layer, and the whole drawing's size
    boxes: Vec<(f64, f64, f64, f64)>,
    layer: Vec<usize>,
    size: (f64, f64),
    /// laid out in rows ([`wrap_layout`]): each edge's path, and how many rows
    routes: Option<Vec<Vec<(f64, f64)>>>,
    rows: usize,
}

/// A long flowchart laid out in rows ("Fluxograma longo · em linhas"): the boxes, each edge's path (a polyline from
/// the source's border to the target's), the layers of each row and the size.
#[derive(Debug)]
pub struct Wrapped {
    pub boxes: Vec<(f64, f64, f64, f64)>,
    pub routes: Vec<Vec<(f64, f64)>>,
    pub rows: Vec<Vec<usize>>,
    pub size: (f64, f64),
}

/// The scale below which a fitted diagram is hard to read: under it, a long flowchart is laid out in rows.
pub const READABLE: f64 = 0.75;
/// Rows: the side margins (where the edges between rows far apart run), the least gap between columns, the gap
/// between rows (the turning edges and the channels), between the nodes of a column, and above and below.
const WRAP_SIDE: f64 = 28.0;
const WRAP_COL_GAP: f64 = 56.0;
const WRAP_ROW_GAP: f64 = 64.0;
const WRAP_NODE_GAP: f64 = 24.0;
const WRAP_EDGE: f64 = 16.0;

/// Lays a flowchart's layers out in rows, like text, in a snake: the first row left to right, the next right to left
/// (so the edge that turns a row goes straight down at its end), each layer a column with its nodes stacked (its
/// branches). Edges to the next column go side to side through the middle of the gap; the others (the turn, skips,
/// loops back) leave by the bottom or top of their node — or its flow side when other nodes are stacked there — and
/// run along the gap between rows, and along a side margin when the rows are further apart. `sizes`: each node's box;
/// `label_w`: an edge label's width (the gaps make room for them).
pub fn wrap_layout(g: &Graph, sizes: &[(f64, f64)], label_w: &dyn Fn(&str) -> f64, width: f64) -> Wrapped {
    let n = g.nodes.len();
    let (layer, place) = layout(g);
    let layers = layer.iter().max().map(|m| m + 1).unwrap_or(1);
    let cols: Vec<Vec<usize>> = (0..layers).map(|l| {
        let mut c: Vec<usize> = (0..n).filter(|&v| layer[v] == l).collect();
        c.sort_by_key(|&v| place[v]);
        c
    }).collect();
    let cw: Vec<f64> = cols.iter().map(|c| c.iter().map(|&v| sizes[v].0).fold(0.0, f64::max)).collect();
    let ch: Vec<f64> = cols.iter().map(|c| c.iter().map(|&v| sizes[v].1).sum::<f64>() + WRAP_NODE_GAP * (c.len().max(1) - 1) as f64).collect();
    // the gap after each column: room for the labels of the edges into the next one
    let mut gap = vec![WRAP_COL_GAP; layers];
    for e in &g.edges {
        if layer[e.to] == layer[e.from] + 1 && !e.label.is_empty() {
            gap[layer[e.from]] = gap[layer[e.from]].max(label_w(&e.label) + 24.0);
        }
    }
    // the rows: as many columns as fit
    let mut rows: Vec<Vec<usize>> = Vec::new();
    let mut cur: Vec<usize> = Vec::new();
    let mut used = 0.0;
    for (l, w) in cw.iter().enumerate() {
        let need = w + cur.last().map(|&p| gap[p]).unwrap_or(0.0);
        if !cur.is_empty() && used + need > width - 2.0 * WRAP_SIDE {
            rows.push(std::mem::take(&mut cur));
            cur.push(l);
            used = *w;
            continue;
        }
        cur.push(l);
        used += need;
    }
    rows.push(cur);
    let mut row_of = vec![0usize; layers];
    for (r, ls) in rows.iter().enumerate() {
        for &l in ls {
            row_of[l] = r;
        }
    }
    let rh: Vec<f64> = rows.iter().map(|ls| ls.iter().map(|&l| ch[l]).fold(0.0, f64::max)).collect();
    let mut row_top = Vec::new();
    let mut y = WRAP_EDGE;
    for h in &rh {
        row_top.push(y);
        y += h + WRAP_ROW_GAP;
    }
    let widest = rows.iter().map(|ls| ls.iter().map(|&l| cw[l]).sum::<f64>() + ls[..ls.len() - 1].iter().map(|&l| gap[l]).sum::<f64>()).fold(0.0, f64::max);
    let total_w = width.max(widest + 2.0 * WRAP_SIDE);
    let mut colx = vec![0.0f64; layers];
    for (r, ls) in rows.iter().enumerate() {
        if r % 2 == 0 {
            let mut x = WRAP_SIDE;
            for &l in ls {
                colx[l] = x;
                x += cw[l] + gap[l];
            }
        } else {
            let mut x = total_w - WRAP_SIDE;
            for &l in ls {
                colx[l] = x - cw[l];
                x -= cw[l] + gap[l];
            }
        }
    }
    let mut boxes = vec![(0.0, 0.0, 0.0, 0.0); n];
    for (l, c) in cols.iter().enumerate() {
        let r = row_of[l];
        let mut y = row_top[r] + (rh[r] - ch[l]) / 2.0;
        for &v in c {
            let (w, h) = sizes[v];
            boxes[v] = (colx[l] + (cw[l] - w) / 2.0, y, w, h);
            y += h + WRAP_NODE_GAP;
        }
    }
    let total_h = row_top.last().copied().unwrap_or(WRAP_EDGE) + rh.last().copied().unwrap_or(0.0) + WRAP_EDGE;
    let cx = |v: usize| boxes[v].0 + boxes[v].2 / 2.0;
    let cy = |v: usize| boxes[v].1 + boxes[v].3 / 2.0;
    let top_most = |v: usize| cols[layer[v]].first() == Some(&v);
    let bottom_most = |v: usize| cols[layer[v]].last() == Some(&v);
    let ltr = |v: usize| row_of[layer[v]] % 2 == 0;
    // out of a node to a channel at `ych` (below or above it); in from a channel to a node
    let out = |v: usize, below: bool, ych: f64, off: f64| -> Vec<(f64, f64)> {
        let b = boxes[v];
        if below && bottom_most(v) {
            return vec![(cx(v), b.1 + b.3), (cx(v), ych)];
        }
        if !below && top_most(v) {
            return vec![(cx(v), b.1), (cx(v), ych)];
        }
        // (sideways on the flow's side: the gap toward the next column, away from the branches coming in)
        let l = layer[v];
        if ltr(v) {
            let gx = colx[l] + cw[l] + 10.0 + off;
            vec![(b.0 + b.2, cy(v)), (gx, cy(v)), (gx, ych)]
        } else {
            let gx = colx[l] - 10.0 - off;
            vec![(b.0, cy(v)), (gx, cy(v)), (gx, ych)]
        }
    };
    let into = |v: usize, from_above: bool, ych: f64, off: f64| -> Vec<(f64, f64)> {
        let b = boxes[v];
        if from_above && top_most(v) {
            return vec![(cx(v), ych), (cx(v), b.1)];
        }
        if !from_above && bottom_most(v) {
            return vec![(cx(v), ych), (cx(v), b.1 + b.3)];
        }
        let l = layer[v];
        if ltr(v) {
            let gx = colx[l] - 10.0 - off;
            vec![(gx, ych), (gx, cy(v)), (b.0, cy(v))]
        } else {
            let gx = colx[l] + cw[l] + 10.0 + off;
            vec![(gx, ych), (gx, cy(v)), (b.0 + b.2, cy(v))]
        }
    };
    let mut routed = 0usize;
    let routes = g.edges.iter().map(|e| {
        let (a, b) = (e.from, e.to);
        let (ra, rb) = (row_of[layer[a]], row_of[layer[b]]);
        let pts = if ra == rb && layer[b] == layer[a] + 1 {
            // to the next column: side to side, turning in the middle of the gap
            let (sa, sb) = if ltr(a) { (boxes[a].0 + boxes[a].2, boxes[b].0) } else { (boxes[a].0, boxes[b].0 + boxes[b].2) };
            let xm = (sa + sb) / 2.0;
            vec![(sa, cy(a)), (xm, cy(a)), (xm, cy(b)), (sb, cy(b))]
        } else {
            // (each routed edge a little apart from the others in the same channels)
            let off = (routed % 4) as f64 * 5.0;
            routed += 1;
            let down = rb >= ra;
            let y1 = if down { row_top[ra] + rh[ra] + WRAP_ROW_GAP / 2.0 } else { row_top[ra] - WRAP_ROW_GAP / 2.0 } + off - 7.5;
            let mut p = out(a, down, y1, off);
            if rb == ra {
                p.extend(into(b, false, y1, off));
            } else if rb.abs_diff(ra) == 1 {
                p.extend(into(b, down, y1, off));
            } else {
                let xs = if ra % 2 == 0 { WRAP_SIDE / 2.0 - off / 2.0 } else { total_w - WRAP_SIDE / 2.0 + off / 2.0 };
                let y2 = if down { row_top[rb] - WRAP_ROW_GAP / 2.0 } else { row_top[rb] + rh[rb] + WRAP_ROW_GAP / 2.0 } + off - 7.5;
                p.push((xs, y1));
                p.push((xs, y2));
                p.extend(into(b, down, y2, off));
            }
            p
        };
        // (no point twice in a row)
        let mut clean: Vec<(f64, f64)> = Vec::with_capacity(pts.len());
        for q in pts {
            if clean.last().is_none_or(|l: &(f64, f64)| (l.0 - q.0).abs() > 0.01 || (l.1 - q.1).abs() > 0.01) {
                clean.push(q);
            }
        }
        clean
    }).collect();
    Wrapped { boxes, routes, rows, size: (total_w, total_h) }
}

/// Where an edge's label goes on its path: over the stretch into the target when that one is level (two branches
/// from one node keep their labels apart), else over the longest level stretch, else beside the longest one —
/// (x, y, level): centred on x above y, or to the right of x centred on y.
pub fn label_anchor(route: &[(f64, f64)]) -> (f64, f64, bool) {
    let segs: Vec<((f64, f64), (f64, f64))> = route.windows(2).map(|w| (w[0], w[1])).collect();
    let level = |s: &((f64, f64), (f64, f64))| (s.0 .1 - s.1 .1).abs() < 0.5;
    let pick = segs.last().filter(|s| level(s) && (s.0 .0 - s.1 .0).abs() > 20.0).copied()
        .or_else(|| segs.iter().filter(|s| level(s)).max_by(|a, b| (a.0 .0 - a.1 .0).abs().total_cmp(&(b.0 .0 - b.1 .0).abs())).copied());
    match pick {
        Some(s) => ((s.0 .0 + s.1 .0) / 2.0, s.0 .1, true),
        None => segs.iter().max_by(|a, b| (a.0 .1 - a.1 .1).abs().total_cmp(&(b.0 .1 - b.1 .1).abs()))
            .map(|s| (s.0 .0, (s.0 .1 + s.1 .1) / 2.0, false)).unwrap_or((route[0].0, route[0].1, true)),
    }
}

fn text_layout(cr: &gtk::cairo::Context, t: &str, size: f64) -> gtk::pango::Layout {
    let l = pangocairo::functions::create_layout(cr);
    let mut f = gtk::pango::FontDescription::from_string(crate::tema::fonte_ui());
    f.set_absolute_size(size * f64::from(gtk::pango::SCALE));
    l.set_font_description(Some(&f));
    l.set_text(t);
    l
}

/// Each node's box size, from its text in the shell's font.
fn node_sizes(cr: &gtk::cairo::Context, g: &Graph) -> Vec<(f64, f64)> {
    g.nodes.iter().map(|n| {
        let (w, h) = text_layout(cr, &n.text, 13.0).pixel_size();
        let (w, h) = (f64::from(w) + 32.0, f64::from(h) + 22.0);
        if n.shape == Shape::Diamond { (w * 1.35, h * 1.6) } else { (w.max(80.0), h.max(38.0)) }
    }).collect()
}

/// The layout for a block `width` wide: the layered one, or — when that would only fit below [`READABLE`] and rows
/// read better — the same flowchart in rows.
fn measure_in(cr: &gtk::cairo::Context, g: &Graph, width: f64) -> Drawn {
    let plain = measure(cr, g);
    if width <= 0.0 || fit_scale(width, plain.size) >= READABLE || g.nodes.len() < 4 {
        return plain;
    }
    let sizes = node_sizes(cr, g);
    let w = wrap_layout(g, &sizes, &|t: &str| f64::from(text_layout(cr, t, 11.0).pixel_size().0), width);
    if w.rows.len() < 2 || fit_scale(width, w.size) <= fit_scale(width, plain.size) {
        return plain;
    }
    Drawn { boxes: w.boxes, layer: plain.layer, size: w.size, routes: Some(w.routes), rows: w.rows.len() }
}

fn measure(cr: &gtk::cairo::Context, g: &Graph) -> Drawn {
    let (layer, place) = layout(g);
    let sizes = node_sizes(cr, g);
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
    Drawn { boxes, layer, size: if g.horizontal { (total_l, total_p) } else { (total_p, total_l) }, routes: None, rows: 1 }
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
    // in rows: each edge along its path, the arrow on its last stretch, the label where label_anchor says
    if let Some(routes) = &d.routes {
        for (e, r) in g.edges.iter().zip(routes) {
            if r.len() < 2 {
                continue;
            }
            cr.set_source_rgb(muted.0, muted.1, muted.2);
            if e.dashed {
                cr.set_dash(&[5.0, 4.0], 0.0);
            }
            cr.move_to(ox + r[0].0, oy + r[0].1);
            for p in &r[1..] {
                cr.line_to(ox + p.0, oy + p.1);
            }
            let _ = cr.stroke();
            cr.set_dash(&[], 0.0);
            let (p1, p0) = (r[r.len() - 1], r[r.len() - 2]);
            let (dx, dy) = (p1.0 - p0.0, p1.1 - p0.1);
            let len = (dx * dx + dy * dy).sqrt().max(1e-6);
            let (ux, uy) = (dx / len, dy / len);
            cr.move_to(ox + p1.0, oy + p1.1);
            cr.line_to(ox + p1.0 - ux * 7.0 - uy * 4.0, oy + p1.1 - uy * 7.0 + ux * 4.0);
            cr.line_to(ox + p1.0 - ux * 7.0 + uy * 4.0, oy + p1.1 - uy * 7.0 - ux * 4.0);
            cr.close_path();
            let _ = cr.fill();
            if !e.label.is_empty() {
                let l = text_layout(cr, &e.label, 11.0);
                let (lw, lh) = l.pixel_size();
                let (lw, lh) = (f64::from(lw), f64::from(lh));
                let (x, y, level) = label_anchor(r);
                let (lx, ly) = if level { (x - lw / 2.0, y - lh - 3.0) } else { (x + 6.0, y - lh / 2.0) };
                let panel = rgb("#101114");
                cr.set_source_rgb(panel.0, panel.1, panel.2);
                cr.rectangle(ox + lx - 3.0, oy + ly, lw + 6.0, lh);
                let _ = cr.fill();
                cr.set_source_rgb(muted.0, muted.1, muted.2);
                cr.move_to(ox + lx, oy + ly);
                pangocairo::functions::show_layout(cr, &l);
            }
        }
    }
    // edges first (under the boxes), each an elbow from side to side, with its arrow and label
    for e in g.edges.iter().filter(|_| d.routes.is_none()) {
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
    zoom: RefCell<Option<Rc<Zoom>>>,
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

/// A flowchart's size, and in how many rows, in a block `width` wide (0: as laid out, unwrapped).
fn size_of(g: &Graph, width: f64) -> ((f64, f64), usize) {
    gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, 1, 1).ok()
        .and_then(|s| gtk::cairo::Context::new(&s).ok()).map(|cr| { let d = measure_in(&cr, g, width); (d.size, d.rows) }).unwrap_or(((400.0, 200.0), 1))
}

impl Diagram {
    /// The graph as drawn in the width it has: a left-to-right chart that does not fit goes top to bottom.
    fn shown(&self) -> Rc<Graph> {
        let g = self.graph.borrow().clone();
        let w = self.width.get();
        if g.horizontal && w > 0 && size_of(&g, 0.0).0 .0 > f64::from(w) {
            let mut v = (*g).clone();
            v.horizontal = false;
            return Rc::new(v);
        }
        g
    }

    fn refit(&self) {
        let g = self.shown();
        let ((_, h), _) = size_of(&g, f64::from(self.width.get()));
        // (no width of its own: it fits the column, laid out down when across does not fit; then the zoom's scale)
        match self.zoom.borrow().as_ref() {
            Some(z) => z.apply(),
            None => {
                self.area.set_content_width(0);
                self.area.set_content_height(h.ceil() as i32);
            }
        }
        self.update_detail();
        while let Some(c) = self.code.first_child() {
            self.code.remove(&c);
        }
        self.code.append(&crate::code_block::new("mermaid", &self.src.borrow(), false));
        self.area.queue_draw();
    }

    /// "fluxograma · 20 nós · em 3 linhas" (the rows change with the block's width).
    fn update_detail(&self) {
        let g = self.shown();
        let (_, rows) = size_of(&g, f64::from(self.width.get()));
        let mut detail = format!("{} · {}", tr("flowchart"), trf("{n} nodes", &[("n", g.nodes.len().to_string())]));
        if rows > 1 {
            detail.push_str(&format!(" · {}", trf("in {n} rows", &[("n", rows.to_string())])));
            if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                eprintln!("debug diagrama: em linhas ({rows} linhas, {} nós)", g.nodes.len());
            }
        }
        if self.detail.text() != detail {
            self.detail.set_text(&detail);
        }
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
        let g = self.shown();
        let width = f64::from(self.width.get());
        let ((w, h), _) = size_of(&g, width);
        let paint = |cr: &gtk::cairo::Context| {
            let bg = crate::tema::rgb("#101114");
            cr.set_source_rgb(bg.0, bg.1, bg.2);
            let _ = cr.paint();
            let d = measure_in(cr, &g, width);
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
        let g = self.shown();
        let width = f64::from(self.width.get());
        let (size, _) = size_of(&g, width);
        let s = gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, (size.0 * 2.0) as i32, (size.1 * 2.0) as i32).map_err(|e| e.to_string())?;
        let cr = gtk::cairo::Context::new(&s).map_err(|e| e.to_string())?;
        cr.scale(2.0, 2.0);
        let bg = crate::tema::rgb("#101114");
        cr.set_source_rgb(bg.0, bg.1, bg.2);
        let _ = cr.paint();
        let d = measure_in(&cr, &g, width);
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

/// How tall a fitted diagram may be before it scrolls, and the smallest a fit makes it.
const FIT_HEIGHT: f64 = 560.0;
const FIT_MIN: f64 = 0.35;

/// The scale that fits a drawing of `size` in `width` (and in [`FIT_HEIGHT`]): never bigger than its own size, never
/// below [`FIT_MIN`] (the rest scrolls).
pub fn fit_scale(width: f64, size: (f64, f64)) -> f64 {
    if width <= 0.0 || size.0 <= 0.0 || size.1 <= 0.0 {
        return 1.0;
    }
    (width / size.0).min(FIT_HEIGHT / size.1).clamp(FIT_MIN, 1.0)
}

/// The next zoom step from `z` (25% steps, 25% – 300%).
pub fn zoom_step(z: f64, up: bool) -> f64 {
    // (the next quarter strictly above or below: from 62% that is 75% or 50%)
    let q = z / 0.25;
    let k = if up { (q + 1e-9).floor() + 1.0 } else { (q - 1e-9).ceil() - 1.0 };
    (k * 0.25).clamp(0.25, 3.0)
}

/// A diagram block's zoom (design "Chat · diagramas, documentos e realce de sintaxe" › Diagrama grande): fitted to
/// the column until − or + is pressed; the middle shows the scale and goes back to fitting.
pub struct Zoom {
    /// a scale chosen with − / + (None: fitted)
    pub manual: Cell<Option<f64>>,
    /// the column's width, as last seen while fitted
    pub view: Cell<f64>,
    mid: gtk::ToggleButton,
    area: gtk::DrawingArea,
    rol: gtk::ScrolledWindow,
    size: Box<dyn Fn() -> (f64, f64)>,
}

impl Zoom {
    /// The scale now.
    pub fn scale(&self) -> f64 {
        self.manual.get().unwrap_or_else(|| fit_scale(self.view.get(), (self.size)()))
    }

    /// The area's size, the label and the drawing again after a change.
    pub fn apply(&self) {
        let (w, h) = (self.size)();
        let z = self.scale();
        if self.manual.get().is_some() {
            self.area.set_content_width((w * z).ceil() as i32);
            self.area.set_hexpand(false);
            self.area.set_cursor_from_name(Some("grab"));
            self.mid.set_label(&format!("{:.0}%", z * 100.0));
            self.mid.set_active(false);
        } else {
            self.area.set_content_width(0);
            self.area.set_hexpand(true);
            self.area.set_cursor_from_name(None);
            self.mid.set_label(&trf("Fit · {pct}%", &[("pct", format!("{:.0}", z * 100.0))]));
            self.mid.set_active(true);
        }
        self.area.set_content_height((h * z).ceil() as i32);
        self.rol.set_policy(gtk::PolicyType::Automatic, if h * z > FIT_HEIGHT { gtk::PolicyType::Automatic } else { gtk::PolicyType::Never });
        self.area.queue_draw();
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug diagrama: zoom {:.0}%{} ({:.0}×{:.0})", z * 100.0, if self.manual.get().is_none() { " ajustado" } else { "" }, w * z, h * z);
        }
    }
}

/// The zoom group (− · Ajustar · 62% · +) for a block whose drawing is `area` in `rol`, `size` its size at 100%; drag
/// moves a zoomed drawing, Ctrl+wheel zooms.
fn zoom_controls(area: &gtk::DrawingArea, rol: &gtk::ScrolledWindow, size: impl Fn() -> (f64, f64) + 'static) -> (gtk::Box, Rc<Zoom>) {
    let seg = caixa(Orientation::Horizontal, 2, "tabela-segmentado");
    seg.set_valign(Align::Center);
    let minus = gtk::Button::with_label("−");
    let mid = gtk::ToggleButton::with_label("");
    let plus = gtk::Button::with_label("+");
    minus.set_tooltip_text(Some(tr("Zoom out")));
    plus.set_tooltip_text(Some(tr("Zoom in")));
    mid.set_tooltip_text(Some(tr("Fit to the width")));
    for b in [minus.upcast_ref::<gtk::Widget>(), mid.upcast_ref(), plus.upcast_ref()] {
        b.add_css_class("tabela-aba");
        seg.append(b);
    }
    minus.add_css_class("diagrama-zoom-passo");
    plus.add_css_class("diagrama-zoom-passo");
    rol.set_max_content_height(FIT_HEIGHT as i32);
    let z = Rc::new(Zoom { manual: Cell::new(None), view: Cell::new(0.0), mid: mid.clone(), area: area.clone(), rol: rol.clone(), size: Box::new(size) });
    let step = |z: &Rc<Zoom>, up: bool| {
        let now = z.scale();
        z.manual.set(Some(zoom_step(now, up)));
        z.apply();
    };
    let zw = Rc::downgrade(&z);
    minus.connect_clicked(move |_| if let Some(z) = zw.upgrade() { step(&z, false) });
    let zw = Rc::downgrade(&z);
    plus.connect_clicked(move |_| if let Some(z) = zw.upgrade() { step(&z, true) });
    let zw = Rc::downgrade(&z);
    mid.connect_clicked(move |_| {
        if let Some(z) = zw.upgrade() {
            z.manual.set(None);
            z.apply();
        }
    });
    // (the column's width while fitted: a zoomed drawing is as wide as itself)
    let zw = Rc::downgrade(&z);
    area.connect_resize(move |_, w, _| {
        let Some(z) = zw.upgrade() else { return };
        if z.manual.get().is_none() && (z.view.replace(f64::from(w)) - f64::from(w)).abs() > 0.5 {
            gtk::glib::idle_add_local_once(move || z.apply());
        }
    });
    // drag moves a zoomed drawing
    let drag = gtk::GestureDrag::new();
    let start = Rc::new(Cell::new((0.0, 0.0)));
    let (r1, s1) = (rol.clone(), start.clone());
    drag.connect_drag_begin(move |_, _, _| s1.set((r1.hadjustment().value(), r1.vadjustment().value())));
    let (r2, s2) = (rol.clone(), start);
    drag.connect_drag_update(move |_, dx, dy| {
        let (x, y) = s2.get();
        r2.hadjustment().set_value(x - dx);
        r2.vadjustment().set_value(y - dy);
    });
    area.add_controller(drag);
    // Ctrl + wheel zooms
    let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    let zw = Rc::downgrade(&z);
    wheel.connect_scroll(move |c, _, dy| {
        let Some(z) = zw.upgrade() else { return gtk::glib::Propagation::Proceed };
        if !c.current_event_state().contains(gtk::gdk::ModifierType::CONTROL_MASK) || dy == 0.0 {
            return gtk::glib::Propagation::Proceed;
        }
        step(&z, dy < 0.0);
        gtk::glib::Propagation::Stop
    });
    rol.add_controller(wheel);
    crate::central::posicao_debug("diagrama zoom mais", plus.upcast_ref());
    crate::central::posicao_debug("diagrama zoom ajustar", mid.upcast_ref());
    (seg, z)
}

fn rounded(cr: &gtk::cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    use std::f64::consts::{FRAC_PI_2, PI};
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, FRAC_PI_2, PI);
    cr.arc(x + r, y + r, r, PI, 3.0 * FRAC_PI_2);
    cr.close_path();
}

/// A sequence diagram drawn (design "Chat · diagramas, documentos e realce de sintaxe"): the participants as the
/// flowchart's nodes, dashed lifelines, messages in muted (replies dashed) with their labels on the panel, notes in
/// gold, loop/alt frames in the border colour with their label in the corner.
fn draw_sequence(cr: &gtk::cairo::Context, s: &Sequence, ox: f64, oy: f64) {
    let rgb = |h: &str| crate::tema::rgb(h);
    let measure = |t: &str, size: f64| {
        let (w, h) = text_layout(cr, t, size).pixel_size();
        (f64::from(w), f64::from(h))
    };
    let l = layout_sequence(s, &measure);
    let (accent, muted, border, gold, text, panel) = (rgb("#A5A1E2"), rgb("#8E8F99"), rgb("#26272F"), rgb("#E5C98C"), rgb("#E2E2E6"), rgb("#101114"));
    let label = |t: &str, size: f64, cx: f64, y: f64, c: (f64, f64, f64), bg: bool| {
        let lay = text_layout(cr, t, size);
        let (w, h) = lay.pixel_size();
        let (w, h) = (f64::from(w), f64::from(h));
        if bg {
            cr.set_source_rgb(panel.0, panel.1, panel.2);
            rounded(cr, cx - w / 2.0 - 5.0, y - 1.0, w + 10.0, h + 2.0, 3.0);
            let _ = cr.fill();
        }
        cr.set_source_rgb(c.0, c.1, c.2);
        cr.move_to(cx - w / 2.0, y);
        pangocairo::functions::show_layout(cr, &lay);
    };
    cr.set_line_width(1.0);
    // the frames (loop, alt…) under everything
    for (x0, y0, x1, y1, start, elses) in &l.frames {
        cr.set_source_rgb(border.0, border.1, border.2);
        rounded(cr, ox + x0, oy + y0, x1 - x0, y1 - y0, 4.0);
        let _ = cr.stroke();
        if let SeqItem::Start { kind, label: t } = &s.items[*start] {
            let lay = text_layout(cr, &format!("{kind}  {t}"), 11.0);
            cr.set_source_rgb(muted.0, muted.1, muted.2);
            cr.move_to(ox + x0 + 8.0, oy + y0 + 6.0);
            pangocairo::functions::show_layout(cr, &lay);
        }
        for ey in elses {
            cr.set_source_rgb(border.0, border.1, border.2);
            cr.set_dash(&[4.0, 4.0], 0.0);
            cr.move_to(ox + x0, oy + ey);
            cr.line_to(ox + x1, oy + ey);
            let _ = cr.stroke();
            cr.set_dash(&[], 0.0);
        }
    }
    // the else lines' labels (by the rows)
    for (k, it) in s.items.iter().enumerate() {
        if let SeqItem::Else { label: t } = it {
            let lay = text_layout(cr, &format!("[{t}]"), 11.0);
            cr.set_source_rgb(muted.0, muted.1, muted.2);
            let x0 = l.frames.iter().filter(|f| f.1 < l.rows[k] && l.rows[k] < f.3).map(|f| f.0).fold(f64::MIN, f64::max);
            cr.move_to(ox + x0 + 8.0, oy + l.rows[k] + 6.0);
            pangocairo::functions::show_layout(cr, &lay);
        }
    }
    // lifelines and the participants
    for (k, h) in l.heads.iter().enumerate() {
        let cx = h.0 + h.2 / 2.0;
        cr.set_source_rgb(border.0, border.1, border.2);
        cr.set_line_width(1.5);
        cr.set_dash(&[4.0, 4.0], 0.0);
        cr.move_to(ox + cx, oy + h.1 + h.3);
        cr.line_to(ox + cx, oy + l.size.1 - 12.0);
        let _ = cr.stroke();
        cr.set_dash(&[], 0.0);
        rounded(cr, ox + h.0, oy + h.1, h.2, h.3, 4.0);
        cr.set_source_rgb(panel.0, panel.1, panel.2);
        let _ = cr.fill_preserve();
        cr.set_source_rgba(accent.0, accent.1, accent.2, 0.12);
        let _ = cr.fill_preserve();
        cr.set_source_rgb(accent.0, accent.1, accent.2);
        let _ = cr.stroke();
        let lay = text_layout(cr, &s.participants[k], 13.0);
        let (w, th) = lay.pixel_size();
        cr.set_source_rgb(text.0, text.1, text.2);
        cr.move_to(ox + cx - f64::from(w) / 2.0, oy + h.1 + (h.3 - f64::from(th)) / 2.0);
        pangocairo::functions::show_layout(cr, &lay);
    }
    let centre = |k: usize| l.heads[k].0 + l.heads[k].2 / 2.0;
    for (k, it) in s.items.iter().enumerate() {
        let y = l.rows[k];
        match it {
            SeqItem::Message { from, to, text: t, dashed } => {
                cr.set_source_rgb(muted.0, muted.1, muted.2);
                cr.set_line_width(1.5);
                if *dashed {
                    cr.set_dash(&[5.0, 4.0], 0.0);
                }
                let (x0, x1, ay) = if from == to {
                    // to itself: out to the right and back, a little lower
                    let x = centre(*from);
                    cr.move_to(ox + x, oy + y + 22.0);
                    cr.line_to(ox + x + 34.0, oy + y + 22.0);
                    cr.line_to(ox + x + 34.0, oy + y + 40.0);
                    cr.line_to(ox + x + 2.0, oy + y + 40.0);
                    (x + 34.0, x, y + 40.0)
                } else {
                    let (x0, x1) = (centre(*from), centre(*to));
                    let dir = (x1 - x0).signum();
                    cr.move_to(ox + x0, oy + y + 22.0);
                    cr.line_to(ox + x1 - dir * 2.0, oy + y + 22.0);
                    (x0, x1, y + 22.0)
                };
                let _ = cr.stroke();
                cr.set_dash(&[], 0.0);
                let dir = (x1 - x0).signum();
                cr.move_to(ox + x1, oy + ay);
                cr.line_to(ox + x1 - dir * 8.0, oy + ay - 4.0);
                cr.line_to(ox + x1 - dir * 8.0, oy + ay + 4.0);
                cr.close_path();
                let _ = cr.fill();
                if !t.is_empty() {
                    let cx = if from == to { centre(*from) + 40.0 + measure(t, 11.0).0 / 2.0 } else { (x0 + x1) / 2.0 };
                    label(t, 11.0, ox + cx, oy + y + 2.0, muted, true);
                }
            }
            SeqItem::Note { first, last, text: t } => {
                let (w, h) = measure(t, 12.0);
                let (a, b) = (centre(*first), centre(*last));
                let width = (b - a + 100.0).max(w + 24.0);
                let x = (a + b) / 2.0 - width / 2.0;
                rounded(cr, ox + x, oy + y + 6.0, width, h + 14.0, 4.0);
                cr.set_source_rgb(panel.0, panel.1, panel.2);
                let _ = cr.fill_preserve();
                cr.set_source_rgba(gold.0, gold.1, gold.2, 0.12);
                let _ = cr.fill_preserve();
                cr.set_source_rgb(gold.0, gold.1, gold.2);
                cr.set_line_width(1.0);
                let _ = cr.stroke();
                label(t, 12.0, ox + (a + b) / 2.0, oy + y + 13.0, text, false);
            }
            _ => {}
        }
    }
}

/// The size a sequence diagram takes.
fn sequence_size(s: &Sequence) -> (f64, f64) {
    gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, 1, 1).ok().and_then(|x| gtk::cairo::Context::new(&x).ok())
        .map(|cr| layout_sequence(s, &|t: &str, size: f64| { let (w, h) = text_layout(&cr, t, size).pixel_size(); (f64::from(w), f64::from(h)) }).size)
        .unwrap_or((400.0, 200.0))
}

/// The block for a sequence diagram: the flowchart's header ("Mermaid", what it holds, Diagrama/Código, ↓ PNG), the
/// drawing (wider than the column: it scrolls sideways).
fn new_sequence(src: &str, s: Sequence) -> gtk::Widget {
    let s = Rc::new(s);
    let bloco = caixa(Orientation::Vertical, 0, "md-bloco");
    let cab = caixa(Orientation::Horizontal, 8, "md-bloco-cab");
    cab.append(&ui_icon("camadas", "#8E8F99", 16));
    cab.append(&rotulo("Mermaid", "md-bloco-titulo"));
    let msgs = s.items.iter().filter(|i| matches!(i, SeqItem::Message { .. })).count();
    let det = rotulo(&format!("{} · {} · {}", tr("sequence"), trf("{n} participants", &[("n", s.participants.len().to_string())]), trf("{n} messages", &[("n", msgs.to_string())])), "md-bloco-detalhe");
    det.set_hexpand(true);
    det.set_xalign(0.0);
    det.set_ellipsize(gtk::pango::EllipsizeMode::End);
    det.set_max_width_chars(1);
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
    let area = gtk::DrawingArea::new();
    let rol = gtk::ScrolledWindow::new();
    rol.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Never);
    rol.set_propagate_natural_height(true);
    rol.set_child(Some(&area));
    rol.add_css_class("diagrama-area");
    let size = sequence_size(&s);
    let (zoom_bar, zoom) = zoom_controls(&area, &rol, move || size);
    cab.append(&zoom_bar);
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
    let s2 = s.clone();
    let zw = Rc::downgrade(&zoom);
    area.set_draw_func(move |_, cr, aw, _| {
        let z = zw.upgrade().map(|z| z.scale()).unwrap_or(1.0);
        cr.scale(z, z);
        draw_sequence(cr, &s2, ((f64::from(aw) / z - size.0) / 2.0).max(0.0), 0.0);
    });
    zoom.apply();
    // (the block keeps its zoom alive)
    unsafe {
        bloco.set_data("zoom", zoom);
    }
    vistas.add_named(&rol, Some("diagrama"));
    vistas.add_named(&crate::code_block::new("mermaid", src, false), Some("codigo"));
    bloco.append(&vistas);
    let v = vistas.clone();
    b_c.connect_toggled(move |b| v.set_visible_child_name(if b.is_active() { "codigo" } else { "diagrama" }));
    let s3 = s.clone();
    png.connect_clicked(move |b| {
        let dir = crate::pastas::downloads();
        let _ = std::fs::create_dir_all(&dir);
        let p = crate::widgets::nome_livre(&dir, tr("diagram"), "png");
        let (w, h) = sequence_size(&s3);
        let ok = (|| -> Result<(), String> {
            let img = gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, (w * 2.0) as i32, (h * 2.0) as i32).map_err(|e| e.to_string())?;
            let cr = gtk::cairo::Context::new(&img).map_err(|e| e.to_string())?;
            cr.scale(2.0, 2.0);
            let bg = crate::tema::rgb("#101114");
            cr.set_source_rgb(bg.0, bg.1, bg.2);
            let _ = cr.paint();
            draw_sequence(&cr, &s3, 0.0, 0.0);
            drop(cr);
            // (the pixels through a GDK texture, which writes PNG)
            img.flush();
            let (iw, ih, stride) = (img.width(), img.height(), img.stride() as usize);
            let mut img = img;
            let data = img.data().map_err(|e| e.to_string())?.to_vec();
            let tex = gtk::gdk::MemoryTexture::new(iw, ih, gtk::gdk::MemoryFormat::B8g8r8a8Premultiplied, &gtk::glib::Bytes::from_owned(data), stride);
            tex.save_to_png(&p).map_err(|e| e.to_string())
        })();
        match ok {
            Ok(()) => b.set_tooltip_text(Some(&trf("Saved in {folder}", &[("folder", dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())]))),
            Err(e) => eprintln!("kalango-shell: diagrama em PNG: {e}"),
        }
    });
    if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
        eprintln!("debug diagrama: sequência {} participantes, {msgs} mensagens", s.participants.len());
    }
    bloco.upcast()
}

/// The block for a mermaid source (None when it is not a flowchart: the code block shows it instead). `conversa`: an
/// answer of the agent's (the Canvas tab edits it), not a document's preview.
pub fn new(src: &str, conversa: bool) -> Option<gtk::Widget> {
    let Some(g) = parse(src) else {
        return parse_sequence(src).map(|s| new_sequence(src, s));
    };
    let g = Rc::new(g);
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
    let d = Rc::new(Diagram { src: RefCell::new(src.to_string()), graph: RefCell::new(g.clone()), square: Cell::new(false), mono: Cell::new(false), width: Cell::new(0), area: area.clone(), detail: det, code, zoom: RefCell::new(None) });
    let dw = Rc::downgrade(&d);
    let (zoom_bar, zoom) = zoom_controls(&area, &rol, move || dw.upgrade().map(|d| size_of(&d.shown(), f64::from(d.width.get())).0).unwrap_or((400.0, 200.0)));
    cab.insert_child_after(&zoom_bar, Some(&seg));
    *d.zoom.borrow_mut() = Some(zoom.clone());
    d.refit();
    let dw = Rc::downgrade(&d);
    area.connect_resize(move |_, w, _| {
        let Some(d) = dw.upgrade() else { return };
        // (the column's width, while fitted: a left-to-right chart wider than it is drawn top to bottom)
        if d.zoom.borrow().as_ref().is_some_and(|z| z.manual.get().is_none()) && d.width.replace(w) != w {
            let d2 = d.clone();
            gtk::glib::idle_add_local_once(move || {
                if let Some(z) = d2.zoom.borrow().as_ref() {
                    z.apply();
                }
                d2.update_detail();
            });
        }
    });
    let dw = Rc::downgrade(&d);
    area.set_draw_func(move |_, cr, w, _| {
        let Some(d) = dw.upgrade() else { return };
        let g = d.shown();
        let z = d.zoom.borrow().as_ref().map(|z| z.scale()).unwrap_or(1.0);
        cr.scale(z, z);
        let m = measure_in(cr, &g, f64::from(d.width.get()));
        draw(cr, &g, &m, ((f64::from(w) / z - m.size.0) / 2.0).max(0.0), 0.0, d.square.get(), d.mono.get());
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

    /// A text's size as the shell's font gives it, roughly (tests have no fonts to measure).
    fn estimate(t: &str, size: f64) -> (f64, f64) {
        (t.chars().count() as f64 * size * 0.55, size * 1.3)
    }

    #[test]
    fn sequence_diagrams() {
        let src = "sequenceDiagram\n    autonumber\n    participant P as Pessoa\n    participant A as Agente\n    actor K as kalangod\n    P->>+A: crie a página\n    loop até ficar pronta\n        A->>K: arquivos.escrever\n        alt aprovado\n            K-->>A: ok\n        else negado\n            K--xA: você não permitiu\n        end\n    end\n    Note over A,K: tudo na Linha do tempo\n    A-->>-P: pronto\n    A->>A: revisa";
        let s = parse_sequence(src).unwrap();
        assert_eq!(s.participants, ["Pessoa", "Agente", "kalangod"]);
        assert_eq!(s.items.iter().filter(|i| matches!(i, SeqItem::Message { .. })).count(), 6);
        assert_eq!(s.items[0], SeqItem::Message { from: 0, to: 1, text: "crie a página".into(), dashed: false });
        assert!(s.items.contains(&SeqItem::Message { from: 2, to: 1, text: "você não permitiu".into(), dashed: true }));
        assert!(s.items.contains(&SeqItem::Note { first: 1, last: 2, text: "tudo na Linha do tempo".into() }));
        let l = layout_sequence(&s, &estimate);
        // the boxes side by side, apart; each label fits between its two lifelines
        for w in l.heads.windows(2) {
            assert!(w[0].0 + w[0].2 < w[1].0, "{:?}", l.heads);
        }
        for it in &s.items {
            if let SeqItem::Message { from, to, text, .. } = it {
                if from != to {
                    let (a, b) = (l.heads[*from], l.heads[*to]);
                    assert!(((a.0 + a.2 / 2.0) - (b.0 + b.2 / 2.0)).abs() > estimate(text, 11.0).0, "{text}");
                }
            }
        }
        // the steps go down, the two frames nest, everything inside the size
        assert!(l.rows.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(l.frames.len(), 2);
        let (inner, outer) = (&l.frames[0], &l.frames[1]);
        assert!(outer.0 < inner.0 && inner.2 < outer.2 && outer.1 < inner.1 && inner.3 < outer.3);
        assert_eq!(inner.5.len(), 1);
        assert!(l.rows.last().unwrap() + 40.0 <= l.size.1);
        assert!(l.heads.iter().all(|h| h.0 + h.2 <= l.size.0));
        // participants met only in messages; what is not a sequence diagram
        let s = parse_sequence("sequenceDiagram\nAlice->>John: Olá\nJohn-->>Alice: Oi!").unwrap();
        assert_eq!(s.participants, ["Alice", "John"]);
        assert!(parse_sequence("sequenceDiagram\nparticipant A").is_none());
        assert!(parse_sequence("sequenceDiagram\nloop x\nA->>B: y").is_none());
        assert!(parse_sequence("flowchart TD\nA-->B").is_none());
        assert!(parse_sequence("<diagrama>\n<!-- Elementos aqui -->\n</diagrama>").is_none());
    }

    /// The resident memory now, in KiB.
    fn rss_kib() -> u64 {
        std::fs::read_to_string("/proc/self/statm").ok().and_then(|s| s.split_whitespace().nth(1).and_then(|p| p.parse::<u64>().ok())).map(|p| p * 4).unwrap_or(0)
    }

    #[test]
    fn thirty_nodes_render() {
        // 30 steps, a decision every 5 with a way back: laid out and drawn (the shell's pango and cairo) on a surface
        let mut src = String::from("flowchart TD\n");
        for i in 0..29 {
            src.push_str(&format!("  N{i}[Passo {i} do projeto] --> N{}[Passo {} do projeto]\n", i + 1, i + 1));
            if i % 5 == 4 {
                src.push_str(&format!("  N{i} -->|refazer| N{}\n", i - 3));
            }
        }
        let g = parse(&src).unwrap();
        assert_eq!(g.nodes.len(), 30);
        // (the first round also starts fontconfig and pango: the second is what a diagram costs)
        for round in ["primeiro", "segundo"] {
            let before = rss_kib();
            let t = std::time::Instant::now();
            let surface = gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, 1, 1).unwrap();
            let cr = gtk::cairo::Context::new(&surface).unwrap();
            let d = measure(&cr, &g);
            let laid = t.elapsed();
            let img = gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, d.size.0.ceil() as i32, d.size.1.ceil() as i32).unwrap();
            let cr = gtk::cairo::Context::new(&img).unwrap();
            let d = measure(&cr, &g);
            draw(&cr, &g, &d, 0.0, 0.0, false, false);
            drop(cr);
            let all = t.elapsed();
            let after = rss_kib();
            eprintln!("fluxograma de 30 nós ({round}): layout {:.2} ms, layout+desenho {:.2} ms, {:.0}×{:.0} px, memória +{} KiB (superfície {} KiB)",
                laid.as_secs_f64() * 1000.0, all.as_secs_f64() * 1000.0, d.size.0, d.size.1, after.saturating_sub(before), img.stride() as i64 * img.height() as i64 / 1024);
            if round == "segundo" {
                assert!(all.as_millis() < 500, "{all:?}");
            }
        }
        let surface = gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, 1, 1).unwrap();
        let d = measure(&gtk::cairo::Context::new(&surface).unwrap(), &g);
        // the same chart in a 700 px block: in rows, readable
        let cr = gtk::cairo::Context::new(&surface).unwrap();
        let t = std::time::Instant::now();
        let w = measure_in(&cr, &g, 700.0);
        let laid = t.elapsed();
        let img = gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, w.size.0.ceil() as i32, w.size.1.ceil() as i32).unwrap();
        let cr2 = gtk::cairo::Context::new(&img).unwrap();
        let t = std::time::Instant::now();
        let w2 = measure_in(&cr2, &g, 700.0);
        draw(&cr2, &g, &w2, 0.0, 0.0, false, false);
        eprintln!("fluxograma de 30 nós em linhas: {} linhas, {:.0}×{:.0} px (ajuste {:.0}%), layout {:.2} ms, layout+desenho {:.2} ms",
            w.rows, w.size.0, w.size.1, fit_scale(700.0, w.size) * 100.0, laid.as_secs_f64() * 1000.0, t.elapsed().as_secs_f64() * 1000.0);
        assert!(w.rows > 1 && w.routes.is_some());
        // no box over another
        for (i, a) in d.boxes.iter().enumerate() {
            for b in d.boxes.iter().skip(i + 1) {
                assert!(a.0 + a.2 <= b.0 || b.0 + b.2 <= a.0 || a.1 + a.3 <= b.1 || b.1 + b.3 <= a.1, "{a:?} {b:?}");
            }
        }
    }

    /// Node boxes as the shell's font would make them, roughly.
    fn est_sizes(g: &Graph) -> Vec<(f64, f64)> {
        g.nodes.iter().map(|n| {
            let w = (n.text.chars().count() as f64 * 7.0 + 32.0).max(80.0);
            if n.shape == Shape::Diamond { (w * 1.35, 60.0) } else { (w, 38.0) }
        }).collect()
    }

    /// What every wrapped layout must keep: no box over another, each edge from its source's border to its target's
    /// in straight level or upright stretches that cross no other box, everything inside the size, and the layers
    /// read in order along the snake.
    fn check_wrapped(g: &Graph, w: &Wrapped) {
        let inside = |b: (f64, f64, f64, f64), x: f64, y: f64| x > b.0 + 1.0 && x < b.0 + b.2 - 1.0 && y > b.1 + 1.0 && y < b.1 + b.3 - 1.0;
        let on_border = |b: (f64, f64, f64, f64), p: (f64, f64)| {
            let (x, y) = p;
            let in_x = x >= b.0 - 0.5 && x <= b.0 + b.2 + 0.5;
            let in_y = y >= b.1 - 0.5 && y <= b.1 + b.3 + 0.5;
            in_x && in_y && ((x - b.0).abs() < 0.6 || (x - b.0 - b.2).abs() < 0.6 || (y - b.1).abs() < 0.6 || (y - b.1 - b.3).abs() < 0.6)
        };
        for (i, a) in w.boxes.iter().enumerate() {
            for b in w.boxes.iter().skip(i + 1) {
                assert!(a.0 + a.2 <= b.0 || b.0 + b.2 <= a.0 || a.1 + a.3 <= b.1 || b.1 + b.3 <= a.1, "boxes overlap: {a:?} {b:?}");
            }
            assert!(a.0 >= 0.0 && a.1 >= 0.0 && a.0 + a.2 <= w.size.0 && a.1 + a.3 <= w.size.1, "box outside: {a:?} {:?}", w.size);
        }
        assert_eq!(w.routes.len(), g.edges.len());
        for (e, r) in g.edges.iter().zip(&w.routes) {
            assert!(r.len() >= 2, "{r:?}");
            assert!(on_border(w.boxes[e.from], r[0]), "edge {}→{} leaves off its source: {r:?}", e.from, e.to);
            assert!(on_border(w.boxes[e.to], *r.last().unwrap()), "edge {}→{} ends off its target: {r:?}", e.from, e.to);
            for s in r.windows(2) {
                let ((x0, y0), (x1, y1)) = (s[0], s[1]);
                assert!((x0 - x1).abs() < 0.01 || (y0 - y1).abs() < 0.01, "a slanted stretch: {r:?}");
                assert!(x0 >= 0.0 && x0 <= w.size.0 && y0 >= 0.0 && y0 <= w.size.1, "a route outside: {r:?}");
                for (k, b) in w.boxes.iter().enumerate() {
                    if k == e.from || k == e.to {
                        continue;
                    }
                    // (sampled along the stretch: no point of it inside another node)
                    for t in 0..=20 {
                        let t = f64::from(t) / 20.0;
                        assert!(!inside(*b, x0 + (x1 - x0) * t, y0 + (y1 - y0) * t), "edge {}→{} crosses node {k}: {r:?} {b:?}", e.from, e.to);
                    }
                }
            }
        }
        // the layers in reading order: rows in turn, left to right then right to left
        let (layer, _) = layout(g);
        let flat: Vec<usize> = w.rows.iter().flatten().copied().collect();
        assert_eq!(flat, (0..flat.len()).collect::<Vec<_>>());
        for (r, ls) in w.rows.iter().enumerate() {
            let xs: Vec<f64> = ls.iter().map(|&l| (0..g.nodes.len()).filter(|&v| layer[v] == l).map(|v| w.boxes[v].0).fold(f64::MAX, f64::min)).collect();
            assert!(xs.windows(2).all(|p| if r % 2 == 0 { p[0] < p[1] } else { p[0] > p[1] }), "row {r} out of order: {xs:?}");
        }
    }

    #[test]
    fn long_chains_wrap_into_rows() {
        // the 20 steps LFM3 wrote (2026-11-21), in a block 700 px wide
        let names = ["Iniciar projeto", "Definir objetivo", "Criar equipe", "Estabelecer cronograma", "Realizar análise de mercado", "Identificar stakeholders", "Coletar requisitos", "Priorizar requisitos", "Escrever especificações", "Design da arquitetura", "Criar banco de dados", "Implementar módulo A", "Teste módulo A", "Implementar módulo B", "Teste módulo B", "Integrar", "Teste de integração", "Deploy", "Monitorar", "Encerrar"];
        let mut src = String::from("flowchart TD\n");
        for i in 0..names.len() - 1 {
            src.push_str(&format!("  N{i}[{}] --> N{}[{}]\n", names[i], i + 1, names[i + 1]));
        }
        let g = parse(&src).unwrap();
        let sizes = est_sizes(&g);
        let w = wrap_layout(&g, &sizes, &|t: &str| t.chars().count() as f64 * 6.0, 700.0);
        assert!(w.rows.len() >= 4, "{:?}", w.rows);
        check_wrapped(&g, &w);
        // readable where the column would have shrunk it to 35%
        assert!(fit_scale(700.0, w.size) >= READABLE, "{:?}", w.size);
        // the edge that turns a row goes down at its end
        let turn = w.rows[0].last().copied().unwrap();
        let r = &w.routes[turn];
        assert!(r.first().unwrap().1 < r.last().unwrap().1, "{r:?}");
    }

    #[test]
    fn rows_keep_branches_loops_and_labels() {
        // the design's sheet: a decision with two branches, a loop back over a row, labels on the edges
        let src = "flowchart TD\nA[Início] --> B[Definir requisitos] --> C[Design] --> D[Implementar] --> E[Testar] --> F{Passou nos testes?}\nF -->|sim| G[Revisar código]\nF -->|não| H[Corrigir]\nH -->|refazer| D\nG --> I[Deploy] --> J[Monitorar] --> K[Documentar] --> L[Fim]\nA -.-> L";
        let g = parse(src).unwrap();
        let sizes = est_sizes(&g);
        for width in [520.0, 758.0, 1000.0] {
            let w = wrap_layout(&g, &sizes, &|t: &str| t.chars().count() as f64 * 6.0, width);
            assert!(w.rows.len() >= 2, "{width}: {:?}", w.rows);
            check_wrapped(&g, &w);
            // the two branches' labels apart; the loop goes up to its target
            let lab = |t: &str| label_anchor(&w.routes[g.edges.iter().position(|e| e.label == t).unwrap()]);
            let (sim, nao) = (lab("sim"), lab("não"));
            assert!((sim.0 - nao.0).abs() + (sim.1 - nao.1).abs() > 12.0, "{width}: {sim:?} {nao:?}");
            let back = &w.routes[g.edges.iter().position(|e| e.label == "refazer").unwrap()];
            assert!(back.last().unwrap().1 < back.first().unwrap().1, "{width}: {back:?}");
        }
    }

    #[test]
    fn zoom_fits_and_steps() {
        // a small diagram stays at its size; a wide one fits the column; a tall one fits 560 px, never below 35%
        assert_eq!(fit_scale(700.0, (400.0, 200.0)), 1.0);
        assert!((fit_scale(700.0, (1400.0, 300.0)) - 0.5).abs() < 1e-9);
        assert!((fit_scale(700.0, (230.0, 1120.0)) - 0.5).abs() < 1e-9);
        assert_eq!(fit_scale(700.0, (230.0, 2740.0)), FIT_MIN);
        assert_eq!(fit_scale(0.0, (230.0, 2740.0)), 1.0);
        assert_eq!(zoom_step(0.62, true), 0.75);
        assert_eq!(zoom_step(0.62, false), 0.5);
        assert_eq!(zoom_step(0.25, false), 0.25);
        assert_eq!(zoom_step(3.0, true), 3.0);
        assert_eq!(zoom_step(1.0, true), 1.25);
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
        // what models write besides: quoted labels, classes in place, longer and other arrows, styling lines
        let g = parse("graph LR\nA[\"Início (agora)\"]:::k --->|\"sim\"| B\nB --o C\nC <--> D\nclassDef k fill:#fff;\nclass A k\nstyle B stroke:#f00\nsubgraph S [Grupo]\ndirection TB\nD --> E\nend").unwrap();
        assert_eq!(g.nodes[0].text, "Início (agora)");
        assert_eq!(g.nodes.len(), 5);
        assert_eq!(g.edges.len(), 4);
        assert_eq!(g.edges[0].label, "sim");
        // a cycle still lays out
        let g = parse("flowchart LR\nA-->B\nB-->A").unwrap();
        let (layer, _) = layout(&g);
        assert_ne!(layer[0], layer[1]);
    }
}
