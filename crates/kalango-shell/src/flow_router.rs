//! Where the flow builder's connections run (design "ligação para trás", "portas com nome"): an orthogonal router
//! around the cards. Its grid is sparse: the lines a margin away from each card's sides, the lines just outside them
//! (for gaps narrower than two margins) and the lines through every connection's stub; A* over it takes the shortest
//! way with the fewest bends, a cost for running in a card's margin, a much larger one for crossing a card (only
//! when there is no other way: overlapping or abutting cards) and a small one for sharing a stretch another connection
//! already runs on. A connection going forward keeps its Bézier curve when the curve touches no card; going back, or
//! when the curve would cross a card, it is routed. Parallel stretches are then moved apart by [`LANE_GAP`] where that
//! stays clear of the cards, and the turns are rounded only as far as they keep clear of them.

use crate::fluxo_modelo::{bezier, point_at, EdgePath, Rect, Route, Side};
use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// How far a routed connection keeps from the cards when it can.
pub const MARGIN: f64 = 12.0;
/// How far a connection runs straight out of its port before it may turn.
pub const STUB: f64 = 16.0;
/// How far apart two connections run side by side.
pub const LANE_GAP: f64 = 8.0;
/// The radius of a routed connection's turns (smaller where a card is close).
pub const RADIUS: f64 = 16.0;
/// A turn costs as much as this much length: a route takes a longer way to save a bend.
const BEND: i64 = 30 * SCALE;
/// Running in a card's margin, and through a card: each grid step there costs this much more.
const IN_MARGIN: i64 = 300 * SCALE;
const IN_CARD: i64 = 1_000_000 * SCALE;
/// How many states one link's search may look at: past it the link keeps its plain curve (a canvas with hundreds of
/// cards must not stall drawing; a connection is drawn either way).
pub const MAX_EXPANSIONS: usize = 40_000;
/// How far around its two ends a link's search first looks (it widens when no clear way is found there).
const WINDOW: f64 = 240.0;
/// Costs are kept in tenths of a unit (integers: the heap orders them exactly, so routes come out the same every time).
const SCALE: i64 = 10;

/// One connection to route: its ports (`Route`) and its two cards.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Link {
    pub route: Route,
    pub src: Rect,
    pub dst: Rect,
}

/// How a grid step lies: clear, in a card's margin, through a card.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Ground {
    Clear,
    Margin,
    Card,
}

/// The grid over the cards: its vertical lines (xs) and horizontal ones (ys), what each step between two neighbouring
/// points crosses, and how many routes already run on each step.
struct Grid {
    xs: Vec<f64>,
    ys: Vec<f64>,
    /// the step from (i, j) to (i + 1, j): index j * (nx - 1) + i
    across: Vec<Ground>,
    /// the step from (i, j) to (i, j + 1): index i * (ny - 1) + j
    down: Vec<Ground>,
    /// the search's work space, kept from one link to the next (a state counts only when its stamp is this search's)
    best: Vec<i64>,
    from: Vec<usize>,
    stamp: Vec<u32>,
    round: u32,
}

fn dedup(mut v: Vec<f64>) -> Vec<f64> {
    v.sort_by(f64::total_cmp);
    v.dedup_by(|a, b| (*a - *b).abs() < 0.25);
    v
}

/// The first index of `v` (sorted) whose value is at least `x`.
fn first_at_least(v: &[f64], x: f64) -> usize {
    v.partition_point(|a| *a < x)
}

impl Grid {
    fn new(cards: &[Rect], points: &[(f64, f64)]) -> Grid {
        let mut xs = Vec::with_capacity(cards.len() * 4 + points.len());
        let mut ys = Vec::with_capacity(cards.len() * 4 + points.len());
        for (k, c) in cards.iter().enumerate() {
            xs.extend([c.x - MARGIN, c.x + c.w + MARGIN]);
            ys.extend([c.y - MARGIN, c.y + c.h + MARGIN]);
            // (a card with another within two margins of it: the lines just outside it too, for the narrow gap)
            let close = cards.iter().enumerate().any(|(o, d)| o != k && d.x < c.x + c.w + 2.0 * MARGIN && c.x < d.x + d.w + 2.0 * MARGIN && d.y < c.y + c.h + 2.0 * MARGIN && c.y < d.y + d.h + 2.0 * MARGIN);
            if close {
                xs.extend([c.x - 1.0, c.x + c.w + 1.0]);
                ys.extend([c.y - 1.0, c.y + c.h + 1.0]);
            }
        }
        for p in points {
            xs.push(p.0);
            ys.push(p.1);
        }
        let (xs, ys) = (dedup(xs), dedup(ys));
        let (nx, ny) = (xs.len(), ys.len());
        let mut across = vec![Ground::Clear; ny * nx.saturating_sub(1)];
        let mut down = vec![Ground::Clear; nx * ny.saturating_sub(1)];
        // each card marks the steps whose middle lies in it (or in its margin): only the lines over its own span
        let mut mark = |c: &Rect, grow: f64, what: Ground| {
            let (x0, x1, y0, y1) = (c.x - grow, c.x + c.w + grow, c.y - grow, c.y + c.h + grow);
            let inside = |a: f64, lo: f64, hi: f64| a > lo + 0.01 && a < hi - 0.01;
            let (ia, ib) = (first_at_least(&xs, x0 - 0.5), first_at_least(&xs, x1 + 0.5).min(nx));
            let (ja, jb) = (first_at_least(&ys, y0 - 0.5), first_at_least(&ys, y1 + 0.5).min(ny));
            for (j, y) in ys.iter().enumerate().take(jb).skip(ja) {
                if !inside(*y, y0, y1) {
                    continue;
                }
                for i in ia.saturating_sub(1)..ib.min(nx.saturating_sub(1)) {
                    if inside((xs[i] + xs[i + 1]) / 2.0, x0, x1) {
                        let k = j * (nx - 1) + i;
                        across[k] = across[k].max(what);
                    }
                }
            }
            for (i, x) in xs.iter().enumerate().take(ib).skip(ia) {
                if !inside(*x, x0, x1) {
                    continue;
                }
                for j in ja.saturating_sub(1)..jb.min(ny.saturating_sub(1)) {
                    if inside((ys[j] + ys[j + 1]) / 2.0, y0, y1) {
                        let k = i * (ny - 1) + j;
                        down[k] = down[k].max(what);
                    }
                }
            }
        };
        for c in cards {
            mark(c, MARGIN - 0.5, Ground::Margin);
        }
        for c in cards {
            mark(c, 0.0, Ground::Card);
        }
        let states = nx * ny * 4 + 1;
        Grid { xs, ys, across, down, best: vec![0; states], from: vec![0; states], stamp: vec![0; states], round: 0 }
    }

    fn index(&self, p: (f64, f64)) -> (usize, usize) {
        let near = |v: &[f64], x: f64| {
            let k = first_at_least(v, x - 0.25).min(v.len() - 1);
            if k > 0 && (v[k - 1] - x).abs() < (v[k] - x).abs() {
                k - 1
            } else {
                k
            }
        };
        (near(&self.xs, p.0), near(&self.ys, p.1))
    }

    /// The cheapest way from `a` (leaving in direction `start`) to `b` (arriving in direction `end`): its points.
    /// The cheapest way from `a` (leaving in direction `start`) to `b` (arriving in direction `end`): its points and
    /// its cost; None when [`MAX_EXPANSIONS`] states were looked at without reaching it.
    fn search(&mut self, a: (f64, f64), start: usize, b: (f64, f64), end: usize) -> Option<(Vec<(f64, f64)>, i64)> {
        let (nx, ny) = (self.xs.len(), self.ys.len());
        let (sa, sb) = (self.index(a), self.index(b));
        let id = |i: usize, j: usize, d: usize| (j * nx + i) * 4 + d;
        let goal = nx * ny * 4;
        self.round += 1;
        let round = self.round;
        let (xs, ys) = (&self.xs, &self.ys);
        let (best, from, stamp) = (&mut self.best, &mut self.from, &mut self.stamp);
        let get = |best: &Vec<i64>, stamp: &Vec<u32>, k: usize| if stamp[k] == round { best[k] } else { i64::MAX };
        // (as far as it is, plus a turn when the goal is not in line: never more than what is left)
        let h = |i: usize, j: usize| {
            let (dx, dy) = ((xs[i] - xs[sb.0]).abs(), (ys[j] - ys[sb.1]).abs());
            ((dx + dy) * SCALE as f64) as i64 + if dx > 0.25 && dy > 0.25 { BEND } else { 0 }
        };
        let mut heap = BinaryHeap::new();
        let s = id(sa.0, sa.1, start);
        best[s] = 0;
        from[s] = usize::MAX;
        stamp[s] = round;
        heap.push(Reverse((h(sa.0, sa.1), 0i64, s)));
        // directions: 0 right, 1 left, 2 down, 3 up
        let turn = |d: usize, e: usize| if d == e { 0 } else if d ^ 1 == e { 2 * BEND } else { BEND };
        let mut expanded = 0;
        while let Some(Reverse((_, g, k))) = heap.pop() {
            if g > get(best, stamp, k) {
                continue;
            }
            if k == goal {
                break;
            }
            expanded += 1;
            if expanded > MAX_EXPANSIONS {
                return None;
            }
            let (cell, d) = (k / 4, k % 4);
            let (i, j) = (cell % nx, cell / nx);
            if (i, j) == sb {
                let c = g + turn(d, end);
                if c < get(best, stamp, goal) {
                    best[goal] = c;
                    from[goal] = k;
                    stamp[goal] = round;
                    heap.push(Reverse((c, c, goal)));
                }
            }
            for e in 0..4 {
                if e == d ^ 1 {
                    continue;
                }
                let (ni, nj, len, ground) = match e {
                    0 if i + 1 < nx => (i + 1, j, xs[i + 1] - xs[i], self.across[j * (nx - 1) + i]),
                    1 if i > 0 => (i - 1, j, xs[i] - xs[i - 1], self.across[j * (nx - 1) + i - 1]),
                    2 if j + 1 < ny => (i, j + 1, ys[j + 1] - ys[j], self.down[i * (ny - 1) + j]),
                    3 if j > 0 => (i, j - 1, ys[j] - ys[j - 1], self.down[i * (ny - 1) + j - 1]),
                    _ => continue,
                };
                let cost = (len * SCALE as f64) as i64
                    + turn(d, e)
                    + match ground {
                        Ground::Clear => 0,
                        Ground::Margin => IN_MARGIN,
                        Ground::Card => IN_CARD,
                    };
                let nk = id(ni, nj, e);
                let ng = g + cost;
                if ng < get(best, stamp, nk) {
                    best[nk] = ng;
                    from[nk] = k;
                    stamp[nk] = round;
                    heap.push(Reverse((ng + h(ni, nj), ng, nk)));
                }
            }
        }
        // back from the goal: the cells passed, turned into points
        if stamp[goal] != round {
            return None;
        }
        let cost = best[goal];
        let mut cells = Vec::new();
        let mut k = from[goal];
        while k != usize::MAX {
            cells.push(k / 4);
            k = from[k];
        }
        cells.reverse();
        cells.dedup();
        Some((cells.into_iter().map(|c| (xs[c % nx], ys[c / nx])).collect(), cost))
    }
}

fn dir_of(s: Side) -> usize {
    match s {
        Side::Right => 0,
        Side::Left => 1,
        Side::Bottom => 2,
        Side::Top => 3,
    }
}

fn normal(s: Side) -> (f64, f64) {
    match s {
        Side::Right => (1.0, 0.0),
        Side::Left => (-1.0, 0.0),
        Side::Bottom => (0.0, 1.0),
        Side::Top => (0.0, -1.0),
    }
}

/// Whether the open segment from `a` to `b` (horizontal or vertical) passes through `c`'s inside.
fn segment_hits(a: (f64, f64), b: (f64, f64), c: &Rect) -> bool {
    let e = 0.5;
    let (x0, x1, y0, y1) = (a.0.min(b.0), a.0.max(b.0), a.1.min(b.1), a.1.max(b.1));
    x1 > c.x + e && x0 < c.x + c.w - e && y1 > c.y + e && y0 < c.y + c.h - e
}

/// Whether a point lies inside a card (not on its edge).
fn inside_any(p: (f64, f64), cards: &[Rect]) -> bool {
    cards.iter().any(|c| p.0 > c.x + 0.5 && p.0 < c.x + c.w - 0.5 && p.1 > c.y + 0.5 && p.1 < c.y + c.h - 0.5)
}

/// How many points to sample a cubic piece at: about one every 2 units of its length (at least 16).
fn samples(b: &[(f64, f64); 4]) -> usize {
    let hull: f64 = b.windows(2).map(|w| (w[1].0 - w[0].0).abs() + (w[1].1 - w[0].1).abs()).sum();
    ((hull / 2.0) as usize).clamp(16, 4000)
}

/// Whether a Bézier curve passes through a card or within `room` of one (its ends sit on the ports: the samples
/// next to them are left out, the port's own card counting only past them).
pub fn curve_hits(b: &[(f64, f64); 4], cards: &[Rect], room: f64) -> bool {
    // (only the cards near the curve: it lies inside the box of its four points)
    let (x0, x1) = (b.iter().map(|p| p.0).fold(f64::MAX, f64::min) - room, b.iter().map(|p| p.0).fold(f64::MIN, f64::max) + room);
    let (y0, y1) = (b.iter().map(|p| p.1).fold(f64::MAX, f64::min) - room, b.iter().map(|p| p.1).fold(f64::MIN, f64::max) + room);
    let cards: Vec<Rect> = cards.iter().filter(|c| c.x < x1 && x0 < c.x + c.w && c.y < y1 && y0 < c.y + c.h).copied().collect();
    if cards.is_empty() {
        return false;
    }
    let cards = &cards[..];
    let grown: Vec<Rect> = cards.iter().map(|c| Rect { x: c.x - room, y: c.y - room, w: c.w + 2.0 * room, h: c.h + 2.0 * room }).collect();
    let n = samples(b);
    let skip = n / 25 + 1;
    (skip..n - skip).map(|i| point_at(b, i as f64 / n as f64)).any(|p| inside_any(p, &grown)) || (1..n).map(|i| point_at(b, i as f64 / n as f64)).any(|p| inside_any(p, cards))
}

/// How far a port's stub may run: [`STUB`], or half of the way to the first card in front of it.
fn stub_len(p: (f64, f64), s: Side, own: Rect, cards: &[Rect]) -> f64 {
    let n = normal(s);
    let free = cards
        .iter()
        .filter(|c| **c != own)
        .filter_map(|c| {
            // (a card across the stub's line, in front of the port)
            let (along, across_lo, across_hi, at) = if n.0 != 0.0 { (if n.0 > 0.0 { c.x - p.0 } else { p.0 - (c.x + c.w) }, c.y, c.y + c.h, p.1) } else { (if n.1 > 0.0 { c.y - p.1 } else { p.1 - (c.y + c.h) }, c.x, c.x + c.w, p.0) };
            (at > across_lo && at < across_hi && along >= 0.0).then_some(along)
        })
        .fold(f64::INFINITY, f64::min);
    STUB.min(free / 2.0).max(1.0)
}

/// Drops the points in the middle of a straight run.
fn simplify(pts: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    let mut out: Vec<(f64, f64)> = Vec::with_capacity(pts.len());
    for p in pts {
        if out.last().is_some_and(|q| (q.0 - p.0).abs() < 1e-6 && (q.1 - p.1).abs() < 1e-6) {
            continue;
        }
        if out.len() >= 2 {
            let (a, b) = (out[out.len() - 2], out[out.len() - 1]);
            let straight = ((a.0 - b.0).abs() < 1e-6 && (b.0 - p.0).abs() < 1e-6) || ((a.1 - b.1).abs() < 1e-6 && (b.1 - p.1).abs() < 1e-6);
            if straight {
                out.pop();
            }
        }
        out.push(p);
    }
    out
}

/// Moves a route's inner stretches that lie on another route's stretch (the same line, overlapping) [`LANE_GAP`]
/// aside, one gap per route already there, when the moved stretch and the two it joins stay clear of the cards.
fn spread(pts: &mut [(f64, f64)], others: &[Vec<(f64, f64)>], cards: &[Rect]) {
    let n = pts.len();
    // (every stretch but the stubs themselves: the legs next to them move along the stub, which grows or shrinks)
    for k in 1..n.saturating_sub(2) {
        let (a, b) = (pts[k], pts[k + 1]);
        let flat = (a.1 - b.1).abs() < 1e-6;
        let on_line = |p: (f64, f64), q: (f64, f64)| {
            if flat {
                (p.1 - a.1).abs() < LANE_GAP / 2.0 && (q.1 - a.1).abs() < LANE_GAP / 2.0 && p.0.max(q.0).min(a.0.max(b.0)) - p.0.min(q.0).max(a.0.min(b.0)) > 1.0
            } else {
                (p.0 - a.0).abs() < LANE_GAP / 2.0 && (q.0 - a.0).abs() < LANE_GAP / 2.0 && p.1.max(q.1).min(a.1.max(b.1)) - p.1.min(q.1).max(a.1.min(b.1)) > 1.0
            }
        };
        let shared = others.iter().flat_map(|o| o.windows(2)).filter(|w| on_line(w[0], w[1])).count();
        if shared == 0 {
            continue;
        }
        // (try each side, the nearer gaps first)
        for step in [1.0, -1.0, 2.0, -2.0, 3.0, -3.0] {
            let off = step * LANE_GAP * shared as f64;
            let (na, nb) = if flat { ((a.0, a.1 + off), (b.0, b.1 + off)) } else { ((a.0 + off, a.1), (b.0 + off, b.1)) };
            let (pa, nb2) = (pts[k - 1], pts[k + 2]);
            // (a stub keeps its way out of the port and at least 4 of its length)
            let keeps = |port: (f64, f64), end: (f64, f64), moved: (f64, f64)| {
                let (d0, d1) = ((end.0 - port.0) + (end.1 - port.1), (moved.0 - port.0) + (moved.1 - port.1));
                d0.signum() == d1.signum() && d1.abs() >= 4.0 && ((moved.0 - port.0).abs() < 1e-6 || (moved.1 - port.1).abs() < 1e-6)
            };
            if (k == 1 && !keeps(pts[0], pts[1], na)) || (k + 2 == n - 1 && !keeps(pts[n - 1], pts[n - 2], nb)) {
                continue;
            }
            // (2 px off every card, at least: a stretch moved onto a card's edge would read as running along it)
            // (a stub starts on its card's edge: only the part it grew by is checked)
            let pa = if k == 1 { pts[1] } else { pa };
            let nb2 = if k + 2 == n - 1 { pts[k + 1] } else { nb2 };
            let clear = [(pa, na), (na, nb), (nb, nb2)].iter().all(|(p, q)| !cards.iter().any(|c| segment_hits(*p, *q, &Rect { x: c.x - 2.5, y: c.y - 2.5, w: c.w + 5.0, h: c.h + 5.0 })));
            let still = others.iter().flat_map(|o| o.windows(2)).filter(|w| {
                let (p, q) = (w[0], w[1]);
                if flat {
                    (p.1 - na.1).abs() < LANE_GAP / 2.0 && (q.1 - na.1).abs() < LANE_GAP / 2.0 && p.0.max(q.0).min(na.0.max(nb.0)) - p.0.min(q.0).max(na.0.min(nb.0)) > 1.0
                } else {
                    (p.0 - na.0).abs() < LANE_GAP / 2.0 && (q.0 - na.0).abs() < LANE_GAP / 2.0 && p.1.max(q.1).min(na.1.max(nb.1)) - p.1.min(q.1).max(na.1.min(nb.1)) > 1.0
                }
            });
            if clear && still.count() == 0 {
                pts[k] = na;
                pts[k + 1] = nb;
                break;
            }
        }
    }
}

/// A line through `pts` as cubic pieces, each turn rounded by up to [`RADIUS`] (at most half of the shorter leg, and
/// only as far as the rounded turn stays out of every card).
pub fn rounded_polyline(pts: &[(f64, f64)], cards: &[Rect]) -> Vec<[(f64, f64); 4]> {
    let line = |a: (f64, f64), b: (f64, f64)| [a, (a.0 + (b.0 - a.0) / 3.0, a.1 + (b.1 - a.1) / 3.0), (a.0 + (b.0 - a.0) * 2.0 / 3.0, a.1 + (b.1 - a.1) * 2.0 / 3.0), b];
    let len = |a: (f64, f64), b: (f64, f64)| ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
    let towards = |a: (f64, f64), b: (f64, f64), k: f64| {
        let l = len(a, b).max(1e-9);
        (a.0 + (b.0 - a.0) / l * k, a.1 + (b.1 - a.1) / l * k)
    };
    let mut out = Vec::new();
    let mut at = pts[0];
    for i in 1..pts.len().saturating_sub(1) {
        let (a, b, c) = (pts[i - 1], pts[i], pts[i + 1]);
        let mut r = RADIUS.min(len(a, b) / 2.0).min(len(b, c) / 2.0);
        // (a quarter circle's handles: 0.5523 of the radius)
        let arc = |r: f64| {
            let (p1, p2) = (towards(b, a, r), towards(b, c, r));
            let k = 0.5523;
            (p1, [p1, (p1.0 + (b.0 - p1.0) * k, p1.1 + (b.1 - p1.1) * k), (p2.0 + (b.0 - p2.0) * k, p2.1 + (b.1 - p2.1) * k), p2])
        };
        while r > 0.5 && (1..12).any(|t| inside_any(point_at(&arc(r).1, t as f64 / 12.0), cards)) {
            r /= 2.0;
        }
        if r <= 0.5 {
            if len(at, b) > 1e-6 {
                out.push(line(at, b));
            }
            at = b;
            continue;
        }
        let (p1, piece) = arc(r);
        if len(at, p1) > 1e-6 {
            out.push(line(at, p1));
        }
        out.push(piece);
        at = piece[3];
    }
    let last = pts[pts.len() - 1];
    if len(at, last) > 1e-6 {
        out.push(line(at, last));
    }
    out
}

/// Where the label of a routed connection sits: halfway along its longest inner stretch.
fn label_spot(pts: &[(f64, f64)]) -> (f64, f64) {
    let inner = if pts.len() > 4 { &pts[1..pts.len() - 1] } else { pts };
    inner
        .windows(2)
        .max_by(|a, b| {
            let l = |w: &[(f64, f64)]| (w[0].0 - w[1].0).abs() + (w[0].1 - w[1].1).abs();
            l(a).total_cmp(&l(b))
        })
        .map(|w| ((w[0].0 + w[1].0) / 2.0, (w[0].1 + w[1].1) / 2.0))
        .unwrap_or(pts[0])
}

/// A plain curve as a drawn path (`window`: the search's when the curve is what a failed search fell back to).
fn curve_path(r: &Route, window: Option<Rect>) -> EdgePath {
    let b = bezier(r);
    EdgePath { segments: vec![b], label_at: point_at(&b, 0.5), points: Vec::new(), raw: Vec::new(), window }
}

/// Routes `links` among `cards` (every card on the canvas, the links' own among them). `keep[k]` (when given) is what
/// was worked out for link k before and still holds: a free curve stays as it is; a routed link's search result is
/// used again (its stretches are moved apart and its turns rounded anew, in order, as for the others). Returns each
/// link's drawn path, in order — the same as routing everything anew.
pub fn route_all(links: &[Link], cards: &[Rect], keep: &[Option<EdgePath>]) -> Vec<EdgePath> {
    let mut out = Vec::with_capacity(links.len());
    let mut drawn: Vec<Vec<(f64, f64)>> = Vec::new();
    for (k, l) in links.iter().enumerate() {
        let kept = keep.get(k).cloned().flatten();
        if let Some(p) = kept.as_ref().filter(|p| p.window.is_none()) {
            out.push(p.clone());
            continue;
        }
        // the links that need the router: going back, or a curve through a card
        if !l.route.back && !curve_hits(&bezier(&l.route), cards, 4.0) {
            out.push(curve_path(&l.route, None));
            continue;
        }
        let (raw, window) = match kept {
            Some(p) => (p.raw, p.window),
            None => search_link(l, cards),
        };
        let Some(window) = window else {
            out.push(curve_path(&l.route, None));
            continue;
        };
        // (no way found within the search's limit: the plain curve, as before routing)
        if raw.is_empty() {
            out.push(curve_path(&l.route, Some(window)));
            continue;
        }
        let mut pts = raw.clone();
        spread(&mut pts, &drawn, cards);
        let segments = rounded_polyline(&pts, cards);
        let label_at = label_spot(&pts);
        drawn.push(pts.clone());
        out.push(EdgePath { segments, label_at, points: pts, raw, window: Some(window) });
    }
    out
}

/// One link's search: over the cards near it only (a small grid, fast), wider when no clear way is found there, up
/// to every card. Returns its corners from port to port (empty when the search gave up) and the widest window used.
fn search_link(l: &Link, cards: &[Rect]) -> (Vec<(f64, f64)>, Option<Rect>) {
    let (n0, n1) = (normal(l.route.from_side), normal(l.route.to_side));
    let (s0, s1) = (stub_len(l.route.from, l.route.from_side, l.src, cards), stub_len(l.route.to, l.route.to_side, l.dst, cards));
    let a = (l.route.from.0 + n0.0 * s0, l.route.from.1 + n0.1 * s0);
    let b = (l.route.to.0 + n1.0 * s1, l.route.to.1 + n1.1 * s1);
    let start = dir_of(l.route.from_side);
    // (arriving into the card: against the in port's side)
    let end = dir_of(l.route.to_side) ^ 1;
    let mut grow = WINDOW;
    loop {
        let (x0, x1) = (a.0.min(b.0) - grow, a.0.max(b.0) + grow);
        let (y0, y1) = (a.1.min(b.1) - grow, a.1.max(b.1) + grow);
        let window = Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
        let near: Vec<Rect> = cards.iter().filter(|c| overlaps(c, &window)).copied().collect();
        let all = near.len() == cards.len();
        // (the window's own edges are lines too: a way around everything inside it)
        let mut grid = Grid::new(&near, &[a, b, (x0, y0), (x1, y1)]);
        let r = grid.search(a, start, b, end);
        let clear = r.as_ref().is_some_and(|(_, cost)| *cost < IN_CARD);
        if clear || all {
            let pts = match r {
                Some((mid, _)) => {
                    let mut pts = vec![l.route.from];
                    pts.extend(mid);
                    pts.push(l.route.to);
                    simplify(pts)
                }
                None => Vec::new(),
            };
            return (pts, Some(window));
        }
        grow *= 3.0;
    }
}

/// Whether two rectangles overlap (as a card counts in a search's window).
fn overlaps(c: &Rect, w: &Rect) -> bool {
    c.x < w.x + w.w && w.x < c.x + c.w && c.y < w.y + w.h && w.y < c.y + c.h
}

/// Whether a drawn connection passes through any of `cards` (sampled along every piece).
pub fn path_hits(p: &EdgePath, cards: &[Rect]) -> bool {
    p.segments.iter().any(|s| {
        let n = samples(s);
        (0..=n).map(|i| point_at(s, i as f64 / n as f64)).any(|q| inside_any(q, cards))
    })
}

/// The routes that `moved` cards (where they are now) may spoil: those leaving or reaching them, and those running
/// through or right next to them. One flag per link.
pub fn touched(links: &[Link], paths: &[EdgePath], moved: &[Rect]) -> Vec<bool> {
    let grown: Vec<Rect> = moved.iter().map(|c| Rect { x: c.x - MARGIN, y: c.y - MARGIN, w: c.w + 2.0 * MARGIN, h: c.h + 2.0 * MARGIN }).collect();
    links
        .iter()
        .zip(paths)
        .map(|(l, p)| {
            if moved.iter().any(|m| *m == l.src || *m == l.dst) {
                return true;
            }
            // (a path whose box misses every moved card cannot run through one: no need to sample it)
            let pts = p.segments.iter().flat_map(|s| s.iter());
            let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
            for q in pts {
                (x0, y0, x1, y1) = (x0.min(q.0), y0.min(q.1), x1.max(q.0), y1.max(q.1));
            }
            let near: Vec<Rect> = grown.iter().filter(|g| g.x < x1 && x0 < g.x + g.w && g.y < y1 && y0 < g.y + g.h).copied().collect();
            !near.is_empty() && path_hits(p, &near)
        })
        .collect()
}

/// A cache key for one link: its ends as they are drawn now.
pub type LinkKey = (String, String, Option<String>);

/// The routes the canvas drew last, by link, with the cards they were routed among.
#[derive(Clone, Debug, Default)]
pub struct RouteCache {
    pub cards: Vec<Rect>,
    pub links: Vec<(LinkKey, Link)>,
    pub paths: Vec<EdgePath>,
}

impl RouteCache {
    /// Forgets the routes (new data: a flow opened, undo, redo, Organizar, a paste; a drag let go).
    pub fn clear(&mut self) {
        *self = RouteCache::default();
    }

    /// The routes for `links` among `cards`: the cache when nothing changed; when only some cards moved (the same
    /// links), only the routes those cards touch; else all of them.
    pub fn update(&mut self, links: Vec<(LinkKey, Link)>, cards: Vec<Rect>) -> &[EdgePath] {
        let same_links = self.links.len() == links.len() && self.links.iter().zip(&links).all(|(a, b)| a.0 == b.0);
        if same_links && self.cards == cards && self.links.iter().zip(&links).all(|(a, b)| a.1 == b.1) {
            return &self.paths;
        }
        let keep: Vec<Option<EdgePath>> = if same_links && self.cards.len() == cards.len() {
            // (the cards that moved: where they were and where they are)
            let (was, now): (Vec<Rect>, Vec<Rect>) = self.cards.iter().zip(&cards).filter(|(a, b)| a != b).map(|(a, b)| (*a, *b)).unzip();
            let plain: Vec<Link> = links.iter().map(|(_, l)| *l).collect();
            let spoilt = touched(&plain, &self.paths, &now);
            self.paths
                .iter()
                .zip(&spoilt)
                .zip(self.links.iter().zip(&links))
                .map(|((p, s), (a, b))| {
                    let same = a.1 == b.1;
                    match p.window {
                        // a free curve: while nothing lands on it
                        None => (same && !s).then(|| p.clone()),
                        // a search: while no card moved into or out of where it looked (its result would be the same)
                        Some(w) => (same && !was.iter().chain(&now).any(|c| overlaps(c, &w))).then(|| p.clone()),
                    }
                })
                .collect()
        } else {
            Vec::new()
        };
        let plain: Vec<Link> = links.iter().map(|(_, l)| *l).collect();
        self.paths = route_all(&plain, &cards, &keep);
        self.links = links;
        self.cards = cards;
        &self.paths
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fluxo_modelo::{edge_route, route, EdgeDoc, FlowDoc, NodeDoc};
    use std::collections::HashMap;

    fn card(x: f64, y: f64) -> Rect {
        Rect { x, y, w: 172.0, h: 90.0 }
    }

    fn link(src: Rect, dst: Rect) -> Link {
        Link { route: route(src, dst), src, dst }
    }

    /// The first card a drawn path runs through, if any.
    fn crossing(p: &EdgePath, cards: &[Rect]) -> Option<Rect> {
        p.segments.iter().flat_map(|s| {
            let n = samples(s);
            (0..=n).map(move |i| point_at(s, i as f64 / n as f64))
        }).find_map(|q| cards.iter().find(|c| q.0 > c.x + 0.5 && q.0 < c.x + c.w - 0.5 && q.1 > c.y + 0.5 && q.1 < c.y + c.h - 0.5).copied())
    }

    #[test]
    fn the_critics_layout_goes_around_c() {
        // Organizar's output: E above A and B, then C and D under them; D back up to B
        let (e, a, b, c, d) = (card(174.0, 60.0), card(60.0, 214.0), card(288.0, 214.0), card(174.0, 368.0), card(174.0, 522.0));
        let cards = [e, a, b, c, d];
        let links = [link(e, a), link(e, b), link(a, c), link(c, d), link(d, b)];
        assert!(links[4].route.back);
        let paths = route_all(&links, &cards, &[]);
        for (k, p) in paths.iter().enumerate() {
            assert_eq!(crossing(p, &cards), None, "link {k}");
        }
        // and a column: A → C past B in the middle (forward), C back to A
        let (a, b, c) = (card(0.0, 0.0), card(0.0, 200.0), card(0.0, 400.0));
        let paths = route_all(&[link(a, c), link(c, a), link(a, b)], &[a, b, c], &[]);
        for p in &paths {
            assert_eq!(crossing(p, &[a, b, c]), None);
        }
    }

    /// Round 1's layouts: a column whose last block links back to the first, a full row between the two ends, taller
    /// blocks in the same row.
    #[test]
    fn going_back_goes_around_every_card() {
        let (a, b, c) = (card(0.0, 0.0), card(0.0, 200.0), card(0.0, 400.0));
        assert_eq!(crossing(&route_all(&[link(c, a)], &[a, b, c], &[])[0], &[a, b, c]), None, "a column");
        let mut cards: Vec<Rect> = (-3..=3).map(|i| card(i as f64 * 228.0, 200.0)).collect();
        let (top, bottom) = (card(0.0, 0.0), card(228.0, 400.0));
        cards.extend([top, bottom]);
        assert_eq!(crossing(&route_all(&[link(bottom, top)], &cards, &[])[0], &cards), None, "a full row between");
        let (dst, mid, src) = (card(0.0, 0.0), Rect { x: 228.0, y: 0.0, w: 172.0, h: 200.0 }, card(456.0, 0.0));
        assert_eq!(crossing(&route_all(&[link(src, dst)], &[dst, mid, src], &[])[0], &[dst, mid, src]), None, "a taller block in the row");
    }

    #[test]
    fn a_route_leaves_by_its_port_and_arrives_at_its_port() {
        let (a, b, c) = (card(0.0, 0.0), card(0.0, 200.0), card(0.0, 400.0));
        let l = link(c, a);
        let p = &route_all(&[l], &[a, b, c], &[])[0];
        assert_eq!(p.points.first(), Some(&l.route.from));
        assert_eq!(p.points.last(), Some(&l.route.to));
        // out to the right of the source, in from the left of the target
        assert!(p.points[1].0 > l.route.from.0 && (p.points[1].1 - l.route.from.1).abs() < 1e-6);
        let n = p.points.len();
        assert!(p.points[n - 2].0 < l.route.to.0 && (p.points[n - 2].1 - l.route.to.1).abs() < 1e-6);
        // the pieces join up, from port to port
        assert_eq!(p.segments.first().unwrap()[0], l.route.from);
        assert_eq!(p.segments.last().unwrap()[3], l.route.to);
        for k in 1..p.segments.len() {
            assert_eq!(p.segments[k - 1][3], p.segments[k][0]);
        }
        // every stretch is horizontal or vertical
        for w in p.points.windows(2) {
            assert!((w[0].0 - w[1].0).abs() < 1e-6 || (w[0].1 - w[1].1).abs() < 1e-6, "{w:?}");
        }
    }

    #[test]
    fn a_free_forward_curve_stays_a_curve() {
        let (a, b) = (card(0.0, 0.0), card(228.0, 0.0));
        let p = &route_all(&[link(a, b)], &[a, b], &[])[0];
        assert!(p.points.is_empty(), "the design's curve");
        assert_eq!(p.segments.len(), 1);
    }

    #[test]
    fn parallel_routes_run_apart() {
        // two links back from the same tall block to the same block above: their runs do not lie on each other
        let (dst, src) = (card(0.0, 0.0), Rect { x: 0.0, y: 300.0, w: 172.0, h: 150.0 });
        let l1 = Link { route: Route { from: (172.0, 400.0), from_side: Side::Right, to: dst.left_mid(), to_side: Side::Left, back: true }, src, dst };
        let l2 = Link { route: Route { from: (172.0, 420.0), from_side: Side::Right, to: dst.left_mid(), to_side: Side::Left, back: true }, src, dst };
        let paths = route_all(&[l1, l2], &[src, dst], &[]);
        let inner = |p: &EdgePath| p.points[2..p.points.len() - 2].to_vec();
        for w1 in inner(&paths[0]).windows(2) {
            for w2 in inner(&paths[1]).windows(2) {
                let flat1 = (w1[0].1 - w1[1].1).abs() < 1e-6;
                let flat2 = (w2[0].1 - w2[1].1).abs() < 1e-6;
                if flat1 && flat2 && (w1[0].1 - w2[0].1).abs() < 1.0 {
                    let overlap = w1[0].0.max(w1[1].0).min(w2[0].0.max(w2[1].0)) - w1[0].0.min(w1[1].0).max(w2[0].0.min(w2[1].0));
                    assert!(overlap <= 1.0, "{w1:?} on {w2:?}");
                }
                if !flat1 && !flat2 && (w1[0].0 - w2[0].0).abs() < 1.0 {
                    let overlap = w1[0].1.max(w1[1].1).min(w2[0].1.max(w2[1].1)) - w1[0].1.min(w1[1].1).max(w2[0].1.min(w2[1].1));
                    assert!(overlap <= 1.0, "{w1:?} on {w2:?}");
                }
            }
        }
    }

    #[test]
    fn overlapping_cards_still_get_a_route_and_the_same_one_each_time() {
        let (a, b, c) = (card(0.0, 0.0), card(100.0, 40.0), card(60.0, 200.0));
        let links = [link(c, a), link(a, b)];
        let one = route_all(&links, &[a, b, c], &[]);
        let two = route_all(&links, &[a, b, c], &[]);
        assert_eq!(one, two);
        assert!(one.iter().all(|p| !p.segments.is_empty()));
    }

    /// A card dragged across a link and away again: the drawn picture is the same as routing the final layout from
    /// scratch (no detour left around a place that is empty now).
    #[test]
    fn after_a_drag_the_routes_are_those_of_the_layout_routed_anew() {
        let (a, b) = (card(0.0, 0.0), card(600.0, 0.0));
        let key = |k: &str| (k.to_string(), String::new(), None);
        let mut cache = RouteCache::default();
        // X starts below, crosses the link A → B, and stops below again
        for y in [300.0, 200.0, 100.0, 40.0, 0.0, -40.0, 100.0, 300.0] {
            let x = card(300.0, y);
            cache.update(vec![(key("ab"), link(a, b))], vec![a, b, x]);
            assert_eq!(cache.paths, route_all(&[link(a, b)], &[a, b, x], &[]), "X at {y}");
        }
        assert!(cache.paths[0].points.is_empty(), "back to the plain curve once X is gone");
        // a 7-card case: 1 → 4 routed around its own source and others, cards moving one by one
        let mut cards = vec![card(0.0, 0.0), card(300.0, 0.0), card(600.0, 0.0), card(300.0, 200.0), card(0.0, 400.0), card(600.0, 400.0), card(300.0, 400.0)];
        let pairs = [(1, 4), (0, 2), (3, 1), (5, 0), (6, 3), (2, 6)];
        let links = |cards: &[Rect]| pairs.iter().map(|(i, j)| ((i.to_string(), j.to_string(), None), link(cards[*i], cards[*j]))).collect::<Vec<_>>();
        let mut s = Seed(42);
        for step in 0..400 {
            let k = s.below(7) as usize;
            cards[k].x += s.below(160) as f64 - 80.0;
            cards[k].y += s.below(160) as f64 - 80.0;
            if step % 7 == 0 {
                // (undo, Organizar, a paste: new data)
                cache.clear();
            }
            let ls = links(&cards);
            cache.update(ls.clone(), cards.clone());
            let fresh = route_all(&ls.iter().map(|(_, l)| *l).collect::<Vec<_>>(), &cards, &[]);
            assert_eq!(cache.paths, fresh, "step {step}");
        }
    }

    /// Hundreds of cards do not stall the canvas: each link's search looks at a small grid around it, and gives up for
    /// the plain curve past its limit.
    #[test]
    fn two_hundred_cards_and_four_hundred_links_route_in_time() {
        let mut s = Seed(99);
        let cards: Vec<Rect> = (0..200).map(|i| Rect { x: (i % 20) as f64 * 228.0, y: (i / 20) as f64 * 190.0, w: 172.0, h: 60.0 + s.below(70) as f64 }).collect();
        let links: Vec<Link> = (0..400)
            .map(|_| {
                let (i, j) = (s.below(200) as usize, s.below(200) as usize);
                let j = if i == j { (j + 1) % 200 } else { j };
                link(cards[i], cards[j])
            })
            .collect();
        let t = std::time::Instant::now();
        let paths = route_all(&links, &cards, &[]);
        let took = t.elapsed().as_secs_f64() * 1000.0;
        eprintln!("200 cards, 400 links: {took:.1} ms");
        assert_eq!(paths.len(), 400);
        // (release: under 100 ms; a debug build is many times slower)
        let budget = if cfg!(debug_assertions) { 3000.0 } else { 100.0 };
        assert!(took < budget, "{took:.1} ms");
    }

    /// Two links leaving two ports of one card for the same block behind it do not lie on each other beside the card.
    #[test]
    fn links_from_two_ports_of_one_card_run_apart_by_their_stubs() {
        let (dst, src) = (card(0.0, 0.0), Rect { x: 0.0, y: 300.0, w: 172.0, h: 150.0 });
        let l1 = Link { route: Route { from: (172.0, 400.0), from_side: Side::Right, to: dst.left_mid(), to_side: Side::Left, back: true }, src, dst };
        let l2 = Link { route: Route { from: (172.0, 420.0), from_side: Side::Right, to: dst.left_mid(), to_side: Side::Left, back: true }, src, dst };
        let paths = route_all(&[l1, l2], &[src, dst], &[]);
        let overlap = |p: &EdgePath, q: &EdgePath| -> f64 {
            let mut worst: f64 = 0.0;
            for a in p.points.windows(2) {
                for b in q.points.windows(2) {
                    let va = (a[0].0 - a[1].0).abs() < 1e-6;
                    let vb = (b[0].0 - b[1].0).abs() < 1e-6;
                    if va && vb && (a[0].0 - b[0].0).abs() < 1.0 {
                        worst = worst.max(a[0].1.max(a[1].1).min(b[0].1.max(b[1].1)) - a[0].1.min(a[1].1).max(b[0].1.min(b[1].1)));
                    }
                    if !va && !vb && (a[0].1 - b[0].1).abs() < 1.0 {
                        worst = worst.max(a[0].0.max(a[1].0).min(b[0].0.max(b[1].0)) - a[0].0.min(a[1].0).max(b[0].0.min(b[1].0)));
                    }
                }
            }
            worst
        };
        assert!(overlap(&paths[0], &paths[1]) <= 16.0, "{:?} / {:?}", paths[0].points, paths[1].points);
    }

    /// A small seeded generator (the fuzz runs the same every time).
    struct Seed(u64);
    impl Seed {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            self.0 >> 33
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n.max(1)
        }
    }

    /// Random cards apart from each other by at least `gap` (so a way between them always exists).
    fn scatter(s: &mut Seed, n: usize, gap: f64) -> Vec<Rect> {
        let mut cards: Vec<Rect> = Vec::new();
        let mut tries = 0;
        while cards.len() < n && tries < 4000 {
            tries += 1;
            let c = Rect { x: s.below(1400) as f64, y: s.below(1000) as f64, w: 120.0 + s.below(80) as f64, h: 60.0 + s.below(100) as f64 };
            if cards.iter().all(|o| c.x > o.x + o.w + gap || o.x > c.x + c.w + gap || c.y > o.y + o.h + gap || o.y > c.y + c.h + gap) {
                cards.push(c);
            }
        }
        cards
    }

    fn check(cards: &[Rect], links: &[Link], what: &str) -> usize {
        let paths = route_all(links, cards, &[]);
        let mut bad = 0;
        for (k, p) in paths.iter().enumerate() {
            if let Some(c) = crossing(p, cards) {
                bad += 1;
                eprintln!("{what}: link {k} {:?} runs through {c:?} by {:?} (src {:?} dst {:?})", links[k].route, p.points, links[k].src, links[k].dst);
            }
        }
        bad
    }

    #[test]
    fn fuzz_no_route_crosses_a_card_when_a_way_exists() {
        let mut s = Seed(20261121);
        let mut bad = 0;
        let mut total = 0;
        // random layouts: cards at least 8 apart (a way always exists), links between random pairs
        for round in 0..2000 {
            let gap = if round % 4 == 0 { 8.0 } else { 40.0 };
            let count = 4 + s.below(9) as usize;
            let cards = scatter(&mut s, count, gap);
            let n = cards.len();
            let many = n + s.below(n as u64) as usize;
            let links: Vec<Link> = (0..many)
                .filter_map(|_| {
                    let (i, j) = (s.below(n as u64) as usize, s.below(n as u64) as usize);
                    (i != j).then(|| link(cards[i], cards[j]))
                })
                .collect();
            total += links.len();
            bad += check(&cards, &links, &format!("random {round}"));
        }
        // Organizar's layouts: random flows arranged, with links going back
        for round in 0..1000 {
            let n = 4 + s.below(10) as usize;
            let mut f = FlowDoc::default();
            let mut sizes = HashMap::new();
            for i in 0..n {
                let id = format!("n{i}");
                f.nodes.push(NodeDoc { id: id.clone(), kind: "message".into(), ..Default::default() });
                sizes.insert(id, (172.0, 60.0 + s.below(110) as f64));
            }
            for i in 1..n {
                let from = s.below(i as u64) as usize;
                f.edges.push(EdgeDoc { source: format!("n{from}"), target: format!("n{i}"), handle: None });
            }
            for _ in 0..1 + s.below(3) {
                let (i, j) = (s.below(n as u64) as usize, s.below(n as u64) as usize);
                if i > j {
                    f.edges.push(EdgeDoc { source: format!("n{i}"), target: format!("n{j}"), handle: None });
                }
            }
            f.arrange(&sizes);
            let rect = |id: &str| {
                let nd = f.node(id).unwrap();
                let (w, h) = sizes[id];
                Rect { x: nd.x, y: nd.y, w, h }
            };
            let cards: Vec<Rect> = f.nodes.iter().map(|nd| rect(&nd.id)).collect();
            let links: Vec<Link> = f.edges.iter().map(|e| {
                let (a, b) = (rect(&e.source), rect(&e.target));
                Link { route: edge_route(f.node(&e.source), e.handle.as_deref(), a, b), src: a, dst: b }
            }).collect();
            total += links.len();
            bad += check(&cards, &links, &format!("arranged {round}"));
        }
        assert!(total > 20_000, "{total}");
        assert_eq!(bad, 0, "{bad} of {total} links run through a card");
    }

    #[test]
    fn fifty_cards_and_fifty_links_route_quickly_and_a_drag_reroutes_only_what_it_touches() {
        let mut s = Seed(7);
        let cards: Vec<Rect> = (0..50).map(|i| Rect { x: (i % 8) as f64 * 228.0, y: (i / 8) as f64 * 190.0, w: 172.0, h: 70.0 + s.below(60) as f64 }).collect();
        let links: Vec<(LinkKey, Link)> = (0..50)
            .map(|k| {
                let (i, j) = (s.below(50) as usize, s.below(50) as usize);
                let j = if i == j { (j + 1) % 50 } else { j };
                ((format!("{i}"), format!("{j}"), Some(format!("{k}"))), link(cards[i], cards[j]))
            })
            .collect();
        let mut cache = RouteCache::default();
        let t = std::time::Instant::now();
        cache.update(links.clone(), cards.clone());
        let full = t.elapsed();
        // a drag: card 20 moves a little each frame; only its links (and those it lands on) are routed again
        let mut frames = Vec::new();
        for f in 1..=30 {
            let mut moved = cards.clone();
            moved[20].x += f as f64 * 3.0;
            moved[20].y += f as f64 * 2.0;
            let ls: Vec<(LinkKey, Link)> = links
                .iter()
                .map(|(k, l)| {
                    let src = if l.src == cards[20] { moved[20] } else { l.src };
                    let dst = if l.dst == cards[20] { moved[20] } else { l.dst };
                    (k.clone(), Link { route: route(src, dst), src, dst })
                })
                .collect();
            let t = std::time::Instant::now();
            cache.update(ls, moved.clone());
            frames.push(t.elapsed());
        }
        let worst = frames.iter().max().unwrap();
        let mean = frames.iter().map(|d| d.as_secs_f64()).sum::<f64>() / frames.len() as f64;
        eprintln!("frames (ms): {:?}", frames.iter().map(|d| (d.as_secs_f64() * 100_000.0).round() / 100.0).collect::<Vec<_>>());
        // (and after the drag, still no route through a card)
        for (k, p) in cache.paths.iter().enumerate() {
            assert_eq!(crossing(p, &cache.cards), None, "link {k}");
        }
        eprintln!("50 cards, 50 links: all routed in {:.2} ms; a dragging frame {:.2} ms on average, at most {:.2} ms", full.as_secs_f64() * 1000.0, mean * 1000.0, worst.as_secs_f64() * 1000.0);
        // (a dragging frame keeps the routes it does not touch: the very same paths)
        let untouched = cache.links.iter().zip(&cache.paths).filter(|((_, l), _)| l.src != cards[20] && l.dst != cards[20]).count();
        assert!(untouched > 40);
        let rerouted = cache.paths.iter().filter(|p| !p.points.is_empty()).count();
        assert!(rerouted > 0);
        // (release: a first routing in a few ms, a dragging frame in about 2; a debug build is many times slower)
        let scale = if cfg!(debug_assertions) { 25.0 } else { 1.0 };
        assert!(full.as_secs_f64() * 1000.0 < 20.0 * scale, "{full:?}");
        assert!(mean * 1000.0 < 2.0 * scale, "{mean}");
    }
}

#[cfg(test)]
mod timing {
    use super::*;
    use crate::fluxo_modelo::route;
    #[test]
    fn where_a_frame_goes() {
        let cards: Vec<Rect> = (0..50).map(|i| Rect { x: (i % 8) as f64 * 228.0, y: (i / 8) as f64 * 190.0, w: 172.0, h: 70.0 + (i * 13 % 60) as f64 }).collect();
        let links: Vec<Link> = (0..50).map(|k| { let (i, j) = (k, (k * 17 + 5) % 50); let j = if i == j { (j + 1) % 50 } else { j }; Link { route: route(cards[i], cards[j]), src: cards[i], dst: cards[j] } }).collect();
        let pts: Vec<(f64, f64)> = links.iter().flat_map(|l| [l.route.from, l.route.to]).collect();
        let t = std::time::Instant::now();
        for _ in 0..100 { std::hint::black_box(Grid::new(&cards, &pts)); }
        eprintln!("grid: {:.3} ms", t.elapsed().as_secs_f64() * 10.0);
        let mut g = Grid::new(&cards, &pts);
        let _ = &mut g;
        let back: Vec<Link> = links.iter().filter(|l| l.route.back).copied().collect();
        let t = std::time::Instant::now();
        for _ in 0..20 { std::hint::black_box(route_all(&back, &cards, &[])); }
        eprintln!("{} links back routed: {:.3} ms ({} x {} lines)", back.len(), t.elapsed().as_secs_f64() * 1000.0 / 20.0, g.xs.len(), g.ys.len());
        let paths = route_all(&links, &cards, &[]);
        let t = std::time::Instant::now();
        for _ in 0..100 { std::hint::black_box(touched(&links, &paths, &[cards[20], cards[21]])); }
        eprintln!("touched: {:.3} ms", t.elapsed().as_secs_f64() * 10.0);
        let t = std::time::Instant::now();
        for _ in 0..100 { for l in &links { std::hint::black_box(curve_hits(&bezier(&l.route), &cards, 4.0)); } }
        eprintln!("curves: {:.3} ms", t.elapsed().as_secs_f64() * 10.0);
    }
}
