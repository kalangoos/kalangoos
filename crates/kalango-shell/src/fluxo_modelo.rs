//! The flow builder's model (design "Fluxo · construtor", "Execuções"): a flow of kalango-fluxos as the screens edit
//! it — blocks with their place on the canvas, connections with their way out, the flow's triggers — and the pure
//! parts of drawing and reading it: each kind's look, the ways out and their labels, where a connection leaves and
//! arrives, hit-testing, the canvas' zoom, the last run's marks and the decisions' numbers. No GTK here: the canvas
//! (`fluxo_canvas`), the inspector (`fluxo_inspetor`) and the pages (`fluxo_construtor`, `fluxo_execucoes`) draw it.

use kalango_i18n::{n_, tr};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};

/// A block of the flow, as saved by kalango-fluxos (`{id, position, type, data}`), with its name on the canvas.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NodeDoc {
    pub id: String,
    pub x: f64,
    pub y: f64,
    pub kind: String,
    pub data: Value,
    /// the name the person gave it (empty: the block's own words name it)
    pub title: String,
    /// switched off: a run hands its input straight on (the engine's node-level `disabled`)
    pub disabled: bool,
    /// the "Em sandbox" block it is inside (`parent`), None for the flow itself
    pub parent: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct EdgeDoc {
    pub source: String,
    pub target: String,
    /// the way out of the source; None = the default way
    pub handle: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FlowDoc {
    pub nodes: Vec<NodeDoc>,
    pub edges: Vec<EdgeDoc>,
    /// what the screens do not edit (limits…), kept as it came
    pub rest: Map<String, Value>,
}

/// A trigger of the flow (`triggers.list`): its config is the service's `Trigger` (`{"kind": …}`).
#[derive(Clone, Debug, PartialEq)]
pub struct TriggerDoc {
    pub id: String,
    pub config: Value,
    pub enabled: bool,
    /// already in the service (a new one waits for the flow to be saved first)
    pub saved: bool,
}

/// The canvas names a trigger "trigger:<id>" (blocks and triggers share the selection).
pub const TRIGGER_PREFIX: &str = "trigger:";

impl NodeDoc {
    pub fn from_json(v: &Value) -> Option<NodeDoc> {
        Some(NodeDoc {
            id: v["id"].as_str()?.to_string(),
            x: v["position"]["x"].as_f64().unwrap_or(0.0),
            y: v["position"]["y"].as_f64().unwrap_or(0.0),
            kind: v["type"].as_str().unwrap_or("end").to_string(),
            data: v.get("data").cloned().unwrap_or(Value::Null),
            title: v["title"].as_str().unwrap_or("").to_string(),
            disabled: v["disabled"].as_bool().unwrap_or(false),
            parent: v["parent"].as_str().filter(|p| !p.is_empty()).map(str::to_string),
        })
    }

    pub fn to_json(&self) -> Value {
        let mut v = json!({"id": self.id, "position": {"x": self.x.round(), "y": self.y.round()}, "type": self.kind});
        if !self.data.is_null() {
            v["data"] = self.data.clone();
        }
        if !self.title.is_empty() {
            v["title"] = json!(self.title);
        }
        if self.disabled {
            v["disabled"] = json!(true);
        }
        if let Some(p) = &self.parent {
            v["parent"] = json!(p);
        }
        v
    }
}

impl FlowDoc {
    pub fn from_json(v: &Value) -> FlowDoc {
        let mut rest = v.as_object().cloned().unwrap_or_default();
        rest.remove("nodes");
        rest.remove("edges");
        FlowDoc {
            nodes: v["nodes"].as_array().into_iter().flatten().filter_map(NodeDoc::from_json).collect(),
            edges: v["edges"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|e| {
                    Some(EdgeDoc { source: e["source"].as_str()?.into(), target: e["target"].as_str()?.into(), handle: e["sourceHandle"].as_str().map(str::to_string) })
                })
                .collect(),
            rest,
        }
    }

    pub fn to_json(&self) -> Value {
        let mut m = self.rest.clone();
        m.insert("nodes".into(), Value::Array(self.nodes.iter().map(NodeDoc::to_json).collect()));
        m.insert(
            "edges".into(),
            Value::Array(
                self.edges
                    .iter()
                    .map(|e| {
                        let mut v = json!({"source": e.source, "target": e.target});
                        if let Some(h) = &e.handle {
                            v["sourceHandle"] = json!(h);
                        }
                        v
                    })
                    .collect(),
            ),
        );
        Value::Object(m)
    }

    pub fn node(&self, id: &str) -> Option<&NodeDoc> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn node_mut(&mut self, id: &str) -> Option<&mut NodeDoc> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }

    /// A new block's id: its kind and the first free number.
    pub fn new_id(&self, kind: &str) -> String {
        (1..).map(|i| format!("{kind}_{i}")).find(|id| self.node(id).is_none()).unwrap()
    }

    /// The block a run starts at (as the engine finds it): the first block nothing leads to; else the first.
    pub fn entry(&self) -> Option<&NodeDoc> {
        let targets: HashSet<&str> = self.edges.iter().map(|e| e.target.as_str()).collect();
        // (the blocks inside a sandbox start the sandbox's own run, not the flow's)
        let top = |n: &&NodeDoc| n.parent.is_none();
        self.nodes.iter().filter(top).find(|n| !targets.contains(n.id.as_str())).or(self.nodes.iter().find(top))
    }

    /// Removes a block (a sandbox with the blocks inside it) and its connections.
    pub fn remove_node(&mut self, id: &str) {
        let gone: HashSet<String> = self.nodes.iter().filter(|n| n.id == id || n.parent.as_deref() == Some(id)).map(|n| n.id.clone()).collect();
        self.nodes.retain(|n| !gone.contains(&n.id));
        self.edges.retain(|e| !gone.contains(&e.source) && !gone.contains(&e.target));
    }

    /// Removes a block; one in the middle of a path (everything it leads to is one block) leaves the blocks before it
    /// connected to the one after it, by the same ways: A → B → C without B is A → C (design "seleção e menus").
    /// True when it reconnected.
    pub fn remove_node_reconnecting(&mut self, id: &str) -> bool {
        let ins: Vec<EdgeDoc> = self.edges.iter().filter(|e| e.target == id && e.source != id).cloned().collect();
        let mut outs: Vec<String> = self.edges.iter().filter(|e| e.source == id && e.target != id).map(|e| e.target.clone()).collect();
        outs.sort();
        outs.dedup();
        self.remove_node(id);
        let [next] = outs.as_slice() else { return false };
        if ins.is_empty() || self.node(next).is_none() {
            return false;
        }
        for e in ins {
            let linked = self.edges.iter().any(|x| x.source == e.source && x.target == *next);
            if e.source != *next && !linked && self.node(&e.source).is_some() {
                self.edges.push(EdgeDoc { source: e.source, target: next.clone(), handle: e.handle });
            }
        }
        true
    }

    /// Whether a connection may join two blocks: both in the flow itself, or both inside the same sandbox (what is
    /// outside does not see the machine; only the sandbox block connects to the outside).
    pub fn may_connect(&self, a: &str, b: &str) -> bool {
        match (self.node(a), self.node(b)) {
            (Some(x), Some(y)) => x.parent == y.parent,
            _ => false,
        }
    }

    /// Puts a block into a sandbox (or back into the flow with None): the connections that would cross the
    /// sandbox's edge go.
    pub fn set_parent(&mut self, id: &str, parent: Option<&str>) {
        if parent == Some(id) || self.node(id).is_none_or(|n| n.kind == "sandbox" && parent.is_some()) {
            return;
        }
        if let Some(n) = self.node_mut(id) {
            if n.parent.as_deref() == parent {
                return;
            }
            n.parent = parent.map(str::to_string);
        }
        let parents: HashMap<String, Option<String>> = self.nodes.iter().map(|n| (n.id.clone(), n.parent.clone())).collect();
        self.edges.retain(|e| !(e.source == id || e.target == id) || parents.get(&e.source) == parents.get(&e.target));
    }

    /// Keeps each sandbox right: blocks whose sandbox is gone are back in the flow, and a sandbox's `entry` is one of
    /// its blocks (the first nothing inside leads to).
    pub fn fix_sandboxes(&mut self) {
        let boxes: HashSet<String> = self.nodes.iter().filter(|n| n.kind == "sandbox").map(|n| n.id.clone()).collect();
        for n in self.nodes.iter_mut() {
            if n.parent.as_ref().is_some_and(|p| !boxes.contains(p)) {
                n.parent = None;
            }
        }
        for b in &boxes {
            let kids: Vec<String> = self.nodes.iter().filter(|n| n.parent.as_deref() == Some(b.as_str())).map(|n| n.id.clone()).collect();
            let cur = self.node(b).and_then(|n| n.data["entry"].as_str()).unwrap_or("").to_string();
            let led: HashSet<&str> = self.edges.iter().filter(|e| kids.contains(&e.source)).map(|e| e.target.as_str()).collect();
            let first = kids.iter().find(|k| !led.contains(k.as_str())).or(kids.first()).cloned().unwrap_or_default();
            let ok = kids.contains(&cur) && !led.contains(cur.as_str());
            if !ok && cur != first {
                if let Some(n) = self.node_mut(b) {
                    if !n.data.is_object() {
                        n.data = json!({});
                    }
                    n.data["entry"] = json!(first);
                }
            }
        }
    }

    /// The blocks `ids` with the blocks inside the sandboxes among them, and the connections among all of them: what
    /// Ctrl+C copies and Ctrl+D duplicates.
    pub fn copy_nodes(&self, ids: &[String]) -> (Vec<NodeDoc>, Vec<EdgeDoc>) {
        let set: HashSet<&str> = ids.iter().map(String::as_str).collect();
        let nodes: Vec<NodeDoc> = self.nodes.iter().filter(|n| set.contains(n.id.as_str()) || n.parent.as_deref().is_some_and(|p| set.contains(p))).cloned().collect();
        let all: HashSet<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
        let edges = self.edges.iter().filter(|e| all.contains(e.source.as_str()) && all.contains(e.target.as_str())).cloned().collect();
        (nodes, edges)
    }

    /// Adds copies of `nodes` (moved by `dx`, `dy`) with new ids, and `edges` among them. A copy inside a sandbox
    /// copied along goes into the new one; else into the same sandbox when it still exists. Returns the new ids of the
    /// blocks that were not inside a copied sandbox (the ones to choose).
    pub fn paste_nodes(&mut self, nodes: &[NodeDoc], edges: &[EdgeDoc], dx: f64, dy: f64) -> Vec<String> {
        let mut ids: HashMap<String, String> = HashMap::new();
        for n in nodes {
            let id = self.new_id(&n.kind);
            ids.insert(n.id.clone(), id.clone());
            // (reserve the id before the next one is chosen)
            self.nodes.push(NodeDoc { id, x: n.x + dx, y: n.y + dy, ..n.clone() });
        }
        let mut top = Vec::new();
        for n in nodes {
            let new = ids[&n.id].clone();
            let parent = match &n.parent {
                Some(p) if ids.contains_key(p) => Some(ids[p].clone()),
                Some(p) if self.node(p).is_some_and(|x| x.kind == "sandbox") => Some(p.clone()),
                _ => None,
            };
            if !n.parent.as_ref().is_some_and(|p| ids.contains_key(p)) {
                top.push(new.clone());
            }
            if let Some(x) = self.node_mut(&new) {
                x.parent = parent;
                if x.kind == "sandbox" {
                    let entry = x.data["entry"].as_str().and_then(|e| ids.get(e)).cloned().unwrap_or_default();
                    x.data["entry"] = json!(entry);
                }
            }
        }
        for e in edges {
            if let (Some(a), Some(b)) = (ids.get(&e.source), ids.get(&e.target)) {
                self.edges.push(EdgeDoc { source: a.clone(), target: b.clone(), handle: e.handle.clone() });
            }
        }
        self.fix_sandboxes();
        top
    }

    /// "Organizar o fluxo": the blocks top-down along the connections — the block a run starts at on top, each block
    /// one row below the first block that leads to it, the blocks of a row side by side around the same middle.
    /// `sizes` are the blocks' sizes on the canvas (a sandbox's is its dashed area; the blocks inside move with it).
    pub fn arrange(&mut self, sizes: &HashMap<String, (f64, f64)>) {
        let top: Vec<String> = self.nodes.iter().filter(|n| n.parent.is_none()).map(|n| n.id.clone()).collect();
        let Some(entry) = self.entry().map(|n| n.id.clone()) else { return };
        let mut row: HashMap<String, usize> = HashMap::from([(entry.clone(), 0)]);
        let mut order = vec![entry.clone()];
        let mut queue = std::collections::VecDeque::from([entry.clone()]);
        let visit = |queue: &mut std::collections::VecDeque<String>, row: &mut HashMap<String, usize>, order: &mut Vec<String>, flow: &FlowDoc| {
            while let Some(cur) = queue.pop_front() {
                let r = row[&cur];
                for e in flow.edges.iter().filter(|e| e.source == cur) {
                    if top.contains(&e.target) && !row.contains_key(&e.target) {
                        row.insert(e.target.clone(), r + 1);
                        order.push(e.target.clone());
                        queue.push_back(e.target.clone());
                    }
                }
            }
        };
        visit(&mut queue, &mut row, &mut order, self);
        // (blocks no run reaches yet go below, each with what follows it)
        for id in &top {
            if !row.contains_key(id) {
                let r = row.values().max().map_or(0, |m| m + 1);
                row.insert(id.clone(), r);
                order.push(id.clone());
                queue.push_back(id.clone());
                visit(&mut queue, &mut row, &mut order, self);
            }
        }
        let size = |id: &str| sizes.get(id).copied().unwrap_or((node_width(self.node(id).map(|n| n.kind.as_str()).unwrap_or("")), 90.0));
        let start = self.node(&entry).map(|n| (n.x + size(&entry).0 / 2.0, n.y)).unwrap_or((260.0, 60.0));
        let rows = row.values().max().map_or(0, |m| m + 1);
        let mut y = start.1;
        let mut moves: Vec<(String, f64, f64)> = Vec::new();
        for r in 0..rows {
            let ids: Vec<&String> = order.iter().filter(|id| row[*id] == r).collect();
            let width: f64 = ids.iter().map(|id| size(id).0).sum::<f64>() + 56.0 * ids.len().saturating_sub(1) as f64;
            let mut x = start.0 - width / 2.0;
            let mut h: f64 = 0.0;
            for id in ids {
                let (w, hh) = size(id);
                moves.push((id.clone(), x, y));
                x += w + 56.0;
                h = h.max(hh);
            }
            y += h + 64.0;
        }
        for (id, x, y) in moves {
            // (a sandbox's place is its dashed area's corner: the blocks inside follow it)
            let (ox, oy) = match sizes.get(&format!("{SANDBOX_AREA}{id}")) {
                Some(&(ax, ay)) => (ax, ay),
                None => self.node(&id).map(|n| (n.x, n.y)).unwrap_or((x, y)),
            };
            let (dx, dy) = (x - ox, y - oy);
            for n in self.nodes.iter_mut().filter(|n| n.id == id || n.parent.as_deref() == Some(id.as_str())) {
                n.x += dx;
                n.y += dy;
            }
        }
    }

    /// "Alinhar na vertical": the blocks on one vertical line (the topmost's middle), one under the other in the order
    /// they were, with room between them.
    pub fn align_vertical(&mut self, ids: &[String], sizes: &HashMap<String, (f64, f64)>) {
        let mut v: Vec<(String, f64, f64)> = ids.iter().filter_map(|id| self.node(id).map(|n| (n.id.clone(), n.x, n.y))).collect();
        v.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));
        let Some(first) = v.first().cloned() else { return };
        let size = |id: &str| sizes.get(id).copied().unwrap_or((172.0, 90.0));
        let mid = first.1 + size(&first.0).0 / 2.0;
        let mut y = first.2;
        for (id, _, oy) in v {
            let (w, h) = size(&id);
            let ny = oy.max(y);
            if let Some(n) = self.node_mut(&id) {
                n.x = mid - w / 2.0;
                n.y = ny;
            }
            y = ny + h + 48.0;
        }
    }

    /// A block that sends the run several ways at once (In parallel, or "follow every way that holds") with no Join
    /// after it: the ways never meet again, and what comes after runs once per way.
    pub fn needs_join(&self, id: &str) -> bool {
        let Some(n) = self.node(id) else { return false };
        if n.kind != "parallel" && n.data["multiple_ways"] != json!(true) {
            return false;
        }
        let mut targets: Vec<&str> = self.edges.iter().filter(|e| e.source == id).map(|e| e.target.as_str()).collect();
        targets.sort_unstable();
        targets.dedup();
        targets.len() >= 2 && !downstream(self, id, "").iter().any(|x| x != id && self.node(x).is_some_and(|n| n.kind == "join"))
    }

    /// The variables a run of this flow has by the time a block reads them: the input fields, the names blocks save
    /// their output as and the Variables blocks' names (for the For each's list).
    pub fn variable_names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.inputs().iter().filter_map(|f| f["name"].as_str().map(str::to_string)).collect();
        for n in &self.nodes {
            if let Some(s) = n.data["save_as"].as_str().filter(|s| !s.is_empty()) {
                v.push(s.to_string());
            }
            for x in n.data["variables"].as_array().into_iter().flatten() {
                if let Some(s) = x["name"].as_str().filter(|s| !s.is_empty()) {
                    v.push(s.to_string());
                }
            }
        }
        let mut seen = HashSet::new();
        v.retain(|x| seen.insert(x.clone()));
        v
    }

    /// Connects `source` to `target` by the first way out of `source` still free (a way already used by another
    /// connection stays, except the default way, which may fan out). Returns false when that connection exists or
    /// every way of `source` is taken.
    pub fn connect(&mut self, source: &str, target: &str) -> bool {
        if source == target || self.edges.iter().any(|e| e.source == source && e.target == target) {
            return false;
        }
        let Some(n) = self.node(source) else { return false };
        let used: HashSet<Option<String>> = self.edges.iter().filter(|e| e.source == source).map(|e| e.handle.clone()).collect();
        let ws = ways(n);
        let Some(handle) = ws.iter().find(|w| w.is_none() || !used.contains(*w)).cloned() else { return false };
        self.edges.push(EdgeDoc { source: source.into(), target: target.into(), handle });
        true
    }

    /// Sends way `handle` of `source` to `target` (None: that way leads nowhere).
    pub fn set_way(&mut self, source: &str, handle: Option<&str>, target: Option<&str>) {
        self.edges.retain(|e| !(e.source == source && e.handle.as_deref() == handle));
        if let Some(t) = target {
            self.edges.push(EdgeDoc { source: source.into(), target: t.into(), handle: handle.map(str::to_string) });
        }
    }
}

/// The canvas' key for a sandbox's dashed area in the sizes `arrange` gets (its corner).
pub const SANDBOX_AREA: &str = "area:";

/// A sandbox's dashed area around the blocks inside it (room for its title above), or its own size when empty.
pub fn sandbox_area(own: (f64, f64), inside: impl IntoIterator<Item = Rect>) -> Rect {
    match bbox(inside) {
        Some(b) => Rect { x: b.x - 24.0, y: b.y - 44.0, w: b.w + 48.0, h: b.h + 68.0 },
        None => Rect { x: own.0, y: own.1, w: 380.0, h: 200.0 },
    }
}

/// The sandbox a block dropped with its middle at `p` goes into: the topmost dashed area holding the point.
pub fn sandbox_at(areas: &[(String, Rect)], p: (f64, f64)) -> Option<&str> {
    areas.iter().rev().find(|(_, r)| r.contains(p)).map(|(id, _)| id.as_str())
}

/// A sandbox's networks and ends: (saved id, the inspector's name, the dashed area's words).
pub const SANDBOX_NETWORKS: [(&str, &str, &str); 3] =
    [("nenhuma", n_("No network"), n_("no network")), ("internet", n_("Internet only"), n_("internet only")), ("casa", n_("Home network"), n_("home network"))];
pub const SANDBOX_ENDS: [(&str, &str, &str); 3] =
    [("apagar", n_("Delete"), n_("delete at the end")), ("guardar", n_("Keep it"), n_("keep at the end")), ("se_falhar", n_("If it fails"), n_("keep if it fails"))];

/// The dashed area's title: "EM SANDBOX · Ubuntu base · sem rede · apagar no fim".
pub fn sandbox_caption(d: &Value, template: &str) -> String {
    let word = |list: &[(&str, &str, &'static str)], v: &str| list.iter().find(|x| x.0 == v).or(list.first()).map(|x| tr(x.2).to_string()).unwrap_or_default();
    let t = if template.trim().is_empty() { tr("choose the machine").to_string() } else { template.to_string() };
    format!("{} · {t} · {} · {}", tr("IN SANDBOX"), word(&SANDBOX_NETWORKS, d["network"].as_str().unwrap_or("")), word(&SANDBOX_ENDS, d["on_end"].as_str().unwrap_or("")))
}

/// A crossing list (`inputs`/`outputs`: `[{from, to}]`) as the inspector writes it, one per line: "{arquivo} → /entrada/".
pub fn crossings_text(v: &Value) -> String {
    v.as_array().into_iter().flatten().map(|c| format!("{} → {}", c["from"].as_str().unwrap_or(""), c["to"].as_str().unwrap_or(""))).collect::<Vec<_>>().join("\n")
}

/// The inspector's lines back as crossings ("from → to"; "->" works too; a line without an arrow is skipped).
pub fn parse_crossings(text: &str) -> Value {
    Value::Array(
        text.lines()
            .filter_map(|l| {
                let (a, b) = l.split_once('→').or_else(|| l.split_once("->"))?;
                let (a, b) = (a.trim(), b.trim());
                (!a.is_empty() && !b.is_empty()).then(|| json!({"from": a, "to": b}))
            })
            .collect(),
    )
}

/// The VirtualDesk machines from `vm.listar` (`{maquinas: [{id, nome, sistema}]}`): (id, "nome · sistema").
pub fn machine_list(v: &Value) -> Vec<(String, String)> {
    let list = v["maquinas"].as_array().or_else(|| v["valor"]["maquinas"].as_array());
    list.into_iter()
        .flatten()
        .filter_map(|m| {
            let id = m["id"].as_str()?.to_string();
            let name = m["nome"].as_str().filter(|n| !n.is_empty()).unwrap_or(&id).to_string();
            Some((id, name))
        })
        .collect()
}

/// What a sandbox's machine did, from `run.sandbox` (`{machine, state: ready|deleted|kept, millis}`): "máquina
/// vm-3 pronta em 0,8 s", "máquina vm-3 apagada", "máquina vm-3 guardada".
pub fn sandbox_note(d: &Value) -> String {
    let machine = d["machine"].as_str().unwrap_or("");
    match d["state"].as_str().unwrap_or("") {
        "ready" => match d["millis"].as_u64() {
            Some(ms) => kalango_i18n::trf("machine {m} ready in {t}", &[("m", machine.to_string()), ("t", duration_text(ms as f64 / 1000.0))]),
            None => kalango_i18n::trf("machine {m} ready", &[("m", machine.to_string())]),
        },
        "kept" => kalango_i18n::trf("machine {m} kept", &[("m", machine.to_string())]),
        _ => kalango_i18n::trf("machine {m} deleted", &[("m", machine.to_string())]),
    }
}

/// An agent's "Se falhar" from its three boxes ("2", "3 s", "60 s"): None when all are empty (no retry). The
/// numbers are read from the start of each box; an empty attempts or wait takes the engine's default (3, 2 s).
pub fn retry_from(attempts: &str, wait: &str, limit: &str) -> Option<Value> {
    let num = |t: &str| -> Option<f64> {
        let n: String = t.trim().chars().take_while(|c| c.is_ascii_digit() || *c == ',' || *c == '.').collect();
        n.replace(',', ".").parse().ok()
    };
    let (a, w, l) = (num(attempts), num(wait), num(limit));
    if a.is_none() && w.is_none() && l.is_none() {
        return None;
    }
    let mut v = json!({"max_attempts": a.map(|x| x as u64).unwrap_or(3), "initial_delay_s": w.unwrap_or(2.0)});
    if let Some(l) = l {
        v["timeout_s"] = json!(l as u64);
    }
    Some(v)
}

/// The order in which a run went through the blocks ("Passo N"): each block's first step.
pub fn step_numbers(steps: &[Step]) -> HashMap<String, usize> {
    let mut v = HashMap::new();
    for s in steps {
        let next = v.len() + 1;
        v.entry(s.node.clone()).or_insert(next);
    }
    v
}

/// "Caminho: A → B → C": the blocks a run went through, by name, each once.
pub fn path_text(steps: &[Step], name: impl Fn(&str) -> String) -> String {
    let mut seen = HashSet::new();
    let names: Vec<String> = steps.iter().filter(|s| seen.insert(s.node.clone())).map(|s| name(&s.node)).collect();
    names.join(" → ")
}

/// A card's rows for its ways out when it has several of its own (an agent's routes, a condition's rules): what
/// each is called.
pub fn way_rows(n: &NodeDoc) -> Vec<String> {
    match n.kind.as_str() {
        "agent" => n.data["routes"].as_array().into_iter().flatten().filter_map(|r| r["id"].as_str().map(str::to_string)).collect(),
        "condition" if n.data["conditions"].as_array().is_some_and(|c| c.len() > 1) => {
            n.data["conditions"].as_array().into_iter().flatten().map(|c| format!("{} · {}", c["id"].as_str().unwrap_or(""), rule_text(&c["data"]))).collect()
        }
        _ => vec![],
    }
}


// ── the kinds of block ──────────────────────────────────────────────────────────────────────────────────────────

/// How a kind looks: its caps label, its glyph and its tone (the tile's and the label's colour).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KindLook {
    pub caps: &'static str,
    pub glyph: &'static str,
    pub tone: Tone,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Gold,
    Accent,
    Blue,
    Purple,
    Teal,
    Coral,
    Grey,
}

impl Tone {
    /// The tone's colour in the dark tokens (the theme maps it).
    pub fn color(self) -> &'static str {
        match self {
            Tone::Gold => "#E5C98C",
            Tone::Accent => "#A5A1E2",
            Tone::Blue => "#7CC3DF",
            Tone::Purple => "#C7A7DC",
            Tone::Teal => "#8ED2C3",
            Tone::Coral => "#EFA2A6",
            Tone::Grey => "#8E8F99",
        }
    }

    pub fn class(self) -> &'static str {
        match self {
            Tone::Gold => "tom-ouro",
            Tone::Accent => "tom-acento",
            Tone::Blue => "tom-azul",
            Tone::Purple => "tom-roxo",
            Tone::Teal => "tom-verde",
            Tone::Coral => "tom-coral",
            Tone::Grey => "tom-cinza",
        }
    }
}

pub fn look(kind: &str) -> KindLook {
    let (caps, glyph, tone) = match kind {
        "agent" => (n_("AGENT"), "spark", Tone::Accent),
        "message" => (n_("MESSAGE"), "doc", Tone::Grey),
        "wait" => (n_("WAIT"), "relogio", Tone::Purple),
        "condition" => (n_("CONDITION"), "compartilhar", Tone::Blue),
        "ask" => (n_("QUESTION"), "usuario", Tone::Gold),
        "parallel" => (n_("PARALLEL"), "apps-pontos", Tone::Blue),
        "join" => (n_("JOIN"), "apps-pontos", Tone::Blue),
        "set_variables" => (n_("VARIABLES"), "lista", Tone::Grey),
        "for_each" => (n_("FOR EACH"), "anel", Tone::Purple),
        "system_action" => (n_("ACTION"), "terminal", Tone::Teal),
        "approval" => (n_("APPROVAL"), "escudo", Tone::Coral),
        "notify" => (n_("NOTIFY"), "sino", Tone::Gold),
        "subflow" => (n_("SUBFLOW"), "raio", Tone::Accent),
        "decide" => (n_("DECIDE"), "compartilhar", Tone::Blue),
        "transcribe" => (n_("TRANSCRIBE"), "microfone", Tone::Blue),
        "describe_image" => (n_("DESCRIBE"), "olho", Tone::Teal),
        "speak" => (n_("SPEAK"), "fone", Tone::Blue),
        "trigger" => (n_("TRIGGER"), "raio", Tone::Gold),
        "sandbox" => (n_("IN SANDBOX"), "escudo", Tone::Accent),
        _ => (n_("END"), "quadrado", Tone::Grey),
    };
    KindLook { caps, glyph, tone }
}

/// A trigger's glyph by its kind.
pub fn trigger_glyph(kind: &str) -> &'static str {
    match kind {
        "schedule" => "relogio",
        "file" => "pasta",
        "notification" => "sino",
        "webhook" => "ramo",
        "system" => "cpu",
        _ => "raio",
    }
}

/// The palette (design: Gatilhos, Agentes, Lógica, Multimodal, Ações): (group, [(id, name, glyph, tone, badge)]).
/// An id "trigger:<kind>" adds a trigger; "block:<kind>[:<preset>]" a block.
pub type PaletteItem = (&'static str, &'static str, &'static str, Tone, &'static str);

pub fn palette() -> Vec<(&'static str, Vec<PaletteItem>)> {
    vec![
        (
            n_("TRIGGERS"),
            vec![
                ("trigger:schedule", n_("Schedule"), "relogio", Tone::Gold, ""),
                ("trigger:file", n_("File or folder"), "pasta", Tone::Gold, ""),
                ("trigger:notification", n_("Notification"), "sino", Tone::Gold, ""),
                ("trigger:webhook", n_("Local webhook"), "ramo", Tone::Gold, ""),
                ("trigger:system", n_("System event"), "cpu", Tone::Gold, ""),
            ],
        ),
        (n_("AGENTS"), vec![("block:agent", n_("Agent"), "spark", Tone::Accent, ""), ("block:agent:team", n_("Team"), "lista", Tone::Blue, "")]),
        (
            n_("LOGIC"),
            vec![
                ("block:condition", n_("Condition"), "compartilhar", Tone::Blue, ""),
                ("block:decide", n_("Decide"), "compartilhar", Tone::Blue, "d1"),
                ("block:for_each", n_("For each"), "anel", Tone::Purple, ""),
                ("block:condition:repeat", n_("Repeat until"), "anel", Tone::Purple, ""),
                ("block:wait", n_("Wait"), "relogio", Tone::Purple, ""),
                ("block:parallel", n_("In parallel"), "apps-pontos", Tone::Blue, ""),
                ("block:sandbox", n_("In sandbox"), "escudo", Tone::Coral, ""),
                ("block:join", n_("Join"), "apps-pontos", Tone::Blue, ""),
                ("block:set_variables", n_("Variables"), "lista", Tone::Grey, ""),
            ],
        ),
        (
            n_("MULTIMODAL"),
            vec![
                ("block:transcribe", n_("Transcribe audio"), "microfone", Tone::Blue, ""),
                ("block:describe_image", n_("Describe image or screen"), "olho", Tone::Teal, ""),
                ("block:speak", n_("Speak (voice)"), "fone", Tone::Blue, ""),
                ("block:agent:multimodal", n_("Multimodal agent"), "spark", Tone::Accent, ""),
            ],
        ),
        (
            n_("ACTIONS"),
            vec![
                ("block:system_action", n_("System action"), "terminal", Tone::Teal, ""),
                ("block:system_action:mcp", n_("MCP connector"), "codigo", Tone::Coral, ""),
                ("block:approval", n_("Ask for approval"), "escudo", Tone::Coral, ""),
                ("block:notify", n_("Notify"), "sino", Tone::Gold, ""),
                ("block:ask", n_("Ask a question"), "usuario", Tone::Gold, ""),
                ("block:message", n_("Message"), "doc", Tone::Grey, ""),
                ("block:subflow", n_("Subflow"), "raio", Tone::Accent, ""),
                ("block:end", n_("End"), "quadrado", Tone::Grey, ""),
            ],
        ),
    ]
}

/// Whether a palette item matches the search (its name, translated or not, without accents' trouble).
pub fn palette_matches(name: &str, translated: &str, query: &str) -> bool {
    let q = fold(query.trim());
    q.is_empty() || fold(name).contains(&q) || fold(translated).contains(&q)
}

/// Lower case without the Portuguese accents (for searching).
pub fn fold(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ã' => 'a',
            'é' | 'ê' => 'e',
            'í' => 'i',
            'ó' | 'ô' | 'õ' => 'o',
            'ú' | 'ü' => 'u',
            'ç' => 'c',
            c => c,
        })
        .collect()
}

/// A new block's data, valid as the service checks it where it can be (the person fills in the rest).
pub fn default_data(kind: &str, preset: &str) -> Value {
    match (kind, preset) {
        ("agent", _) => json!({"agent_id": "", "message": ""}),
        ("message", _) => json!({"content": ""}),
        ("wait", _) => json!({"seconds": 60}),
        ("condition", "repeat") => json!({"conditions": [{"id": "sim", "type": "previous-output", "data": {"field": "content", "operator": "contains", "value": "pronto"}}]}),
        ("condition", _) => json!({"conditions": [{"id": "sim", "type": "previous-output", "data": {"field": "content", "operator": "equals", "value": ""}}]}),
        ("ask", _) => json!({"message": "", "save_as": "resposta"}),
        ("set_variables", _) => json!({"variables": []}),
        ("for_each", _) => json!({"items": "content", "agent_id": "", "message": ""}),
        ("system_action", "mcp") => json!({"tool": "mcp.", "arguments": {}}),
        ("system_action", _) => json!({"tool": "", "arguments": {}}),
        ("approval", _) => json!({"question": "", "details": "", "timeout_s": 7200}),
        ("notify", _) => json!({"title": "", "body": ""}),
        ("subflow", _) => json!({"flow_id": ""}),
        ("decide", _) => json!({"model": "decisao", "state": "", "route": "rota", "threshold": 0.8,
            "questions": {"rota": {"type": "noul", "instructions": ""}}}),
        ("transcribe", _) => json!({"file": "", "language": "Portuguese", "save_as": "transcricao"}),
        ("describe_image", _) => json!({"file": "", "question": "", "save_as": "descricao"}),
        ("speak", _) => json!({"text": "", "voice": "female", "out": "~/Música/fala.wav"}),
        ("sandbox", _) => json!({"template": "", "network": "nenhuma", "on_end": "apagar", "inputs": [], "outputs": [], "entry": ""}),
        _ => Value::Null,
    }
}

/// A new trigger's config (valid: the person adjusts it).
pub fn default_trigger(kind: &str, flow_id: &str) -> Value {
    match kind {
        "schedule" => json!({"kind": "schedule", "days": [], "hour": 9, "minute": 0}),
        "file" => json!({"kind": "file", "path": "~/Downloads", "events": ["created"], "pattern": ""}),
        "webhook" => json!({"kind": "webhook", "key_secret": format!("webhook-{flow_id}")}),
        "system" => json!({"kind": "system", "event": "usb", "filter": ""}),
        "manual" => json!({"kind": "manual"}),
        _ => json!({"kind": "notification", "app": "", "group_by_sender": false}),
    }
}

// ── ways out ────────────────────────────────────────────────────────────────────────────────────────────────────

/// The ways out of a block, in order (None = the default way): a condition's ids and its "else", a decision's
/// answers and "low confidence", an approval's yes and no, an agent's routes and its default.
pub fn ways(n: &NodeDoc) -> Vec<Option<String>> {
    match n.kind.as_str() {
        "condition" => {
            let mut v: Vec<Option<String>> = n.data["conditions"].as_array().into_iter().flatten().filter_map(|c| c["id"].as_str().map(|s| Some(s.to_string()))).collect();
            v.push(Some(ELSE_HANDLE.into()));
            v
        }
        "decide" => decide_ways(&n.data).into_iter().map(Some).collect(),
        "approval" => vec![Some("sim".into()), Some("não".into())],
        "agent" => {
            let mut v: Vec<Option<String>> = n.data["routes"].as_array().into_iter().flatten().filter_map(|r| r["id"].as_str().map(|s| Some(s.to_string()))).collect();
            v.push(None);
            v
        }
        "for_each" => vec![None, Some(EMPTY_LIST.into())],
        "end" => vec![],
        _ => vec![None],
    }
}

/// The blocks `from` leads to (itself included), not going through `stop`: what moves along when a block is put
/// into the connection `stop` → `from`.
pub fn downstream(flow: &FlowDoc, from: &str, stop: &str) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    let mut todo = vec![from.to_string()];
    while let Some(id) = todo.pop() {
        if id == stop || seen.contains(&id) || flow.node(&id).is_none() {
            continue;
        }
        todo.extend(flow.edges.iter().filter(|e| e.source == id).map(|e| e.target.clone()));
        seen.push(id);
    }
    seen
}

/// Where a block of width `w` put into the connection from `a` to `b` pushes `b` and what follows it: one step
/// along the connection (as far as `a` is from `b`, at least the block's size). None for a connection going back
/// or sideways (the new block finds a free spot instead).
pub fn insert_step(a: Rect, b: Rect, w: f64) -> Option<(f64, f64)> {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    if dy.abs() > dx.abs() {
        (dy > 0.0).then(|| (0.0, dy.max(a.h + 60.0)))
    } else {
        (dx > 0.0).then(|| (dx.max(w + 56.0), 0.0))
    }
}

/// What a For each goes through, as one of them is called: "var:anexos" → "anexo"; the input or a field without a
/// plural → "item".
pub fn item_word(items: &str) -> String {
    let name = items.strip_prefix("var:").or_else(|| items.strip_prefix("content.")).unwrap_or("");
    let last = name.rsplit('.').next().unwrap_or("");
    match last.strip_suffix('s') {
        Some(one) if one.chars().count() > 1 && one.chars().all(|c| c.is_alphanumeric() || c == '_') => one.replace('_', " "),
        _ => tr("item").to_string(),
    }
}

/// The For each's loop back on the canvas (design "Fluxo · construtor": "próximo anexo (1 de 2)"): the next item,
/// and how far the run went — the item it is on while it runs (`run.item`), all of them once it is done.
pub fn loop_label(word: &str, items: Option<(usize, usize)>) -> String {
    let next = kalango_i18n::trf("next {item}", &[("item", word.to_string())]);
    match items {
        Some((i, n)) if n > 0 => format!("{next} ({})", kalango_i18n::trf("{i} of {n}", &[("i", i.min(n).to_string()), ("n", n.to_string())])),
        _ => next,
    }
}

/// Undo and redo of the flow builder: the flow before each change (newest last) and the changes undone. Changes
/// closer than [`History::MERGE_MS`] to the one before (a word typed, a block dragged) are one step.
#[derive(Clone, Debug, Default)]
pub struct History {
    past: Vec<FlowDoc>,
    future: Vec<FlowDoc>,
    /// the flow as the last change left it, and when (ms)
    recorded: FlowDoc,
    at: i64,
}

impl History {
    pub const STEPS: usize = 100;
    pub const MERGE_MS: i64 = 1000;

    pub fn new(doc: &FlowDoc) -> History {
        History { recorded: doc.clone(), ..Default::default() }
    }

    /// The flow after a change, at `now_ms`.
    pub fn record(&mut self, doc: &FlowDoc, now_ms: i64) {
        if *doc == self.recorded {
            return;
        }
        let before = std::mem::replace(&mut self.recorded, doc.clone());
        if now_ms - self.at > Self::MERGE_MS || self.past.is_empty() {
            self.past.push(before);
            if self.past.len() > Self::STEPS {
                self.past.remove(0);
            }
        }
        self.at = now_ms;
        self.future.clear();
    }

    /// The flow before the last change (`current` goes to redo).
    pub fn undo(&mut self, current: FlowDoc) -> Option<FlowDoc> {
        let prev = self.past.pop()?;
        self.future.push(current);
        self.restored(&prev);
        Some(prev)
    }

    /// The change undone last, again.
    pub fn redo(&mut self, current: FlowDoc) -> Option<FlowDoc> {
        let next = self.future.pop()?;
        self.past.push(current);
        self.restored(&next);
        Some(next)
    }

    fn restored(&mut self, doc: &FlowDoc) {
        self.recorded = doc.clone();
        // (the next change is a step of its own)
        self.at = i64::MIN / 2;
    }
}

pub const ELSE_HANDLE: &str = "bottom-handle";
/// The way a For each takes when its list is empty (the engine's "empty").
pub const EMPTY_LIST: &str = "empty";
pub const LOW_CONFIDENCE: &str = "pouca_confianca";

/// The decision block's ways (as fluxos-core `DecideBlock::ways`): each option of the route question ("true"/"false"
/// for yes/no, the level's index for a score) and "pouca_confianca".
pub fn decide_ways(d: &Value) -> Vec<String> {
    let route = d["route"].as_str().unwrap_or("");
    let q = &d["questions"][route];
    let mut v: Vec<String> = match q["type"].as_str() {
        Some("choice") => q["criteria"].as_object().map(|c| c.keys().cloned().collect()).unwrap_or_default(),
        Some("score") => (0..q["criteria"].as_array().map(Vec::len).unwrap_or(0)).map(|i| i.to_string()).collect(),
        _ => vec!["true".into(), "false".into()],
    };
    v.push(LOW_CONFIDENCE.into());
    v
}

/// What a way out is called on the canvas and in the inspector.
pub fn way_label(kind: &str, handle: Option<&str>) -> String {
    match (kind, handle) {
        ("for_each", None) => tr("all done").into(),
        ("for_each", Some(EMPTY_LIST)) => tr("empty list").into(),
        (_, None) | (_, Some("default")) => String::new(),
        ("condition", Some(ELSE_HANDLE)) => tr("no").into(),
        (_, Some(ELSE_HANDLE)) => tr("else").into(),
        (_, Some(LOW_CONFIDENCE)) => tr("low confidence").into(),
        ("decide", Some("true")) => tr("yes").into(),
        ("decide", Some("false")) => tr("no").into(),
        (_, Some(h)) => h.to_string(),
    }
}

// ── geometry ────────────────────────────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn contains(&self, p: (f64, f64)) -> bool {
        p.0 >= self.x && p.0 <= self.x + self.w && p.1 >= self.y && p.1 <= self.y + self.h
    }
    pub fn right_mid(&self) -> (f64, f64) {
        (self.x + self.w, self.y + self.h / 2.0)
    }
    pub fn left_mid(&self) -> (f64, f64) {
        (self.x, self.y + self.h / 2.0)
    }
    pub fn top_mid(&self) -> (f64, f64) {
        (self.x + self.w / 2.0, self.y)
    }
    pub fn bottom_mid(&self) -> (f64, f64) {
        (self.x + self.w / 2.0, self.y + self.h)
    }
}

/// A block's width on the canvas (the design's 172; the decision's question reads wider).
pub fn node_width(kind: &str) -> f64 {
    if kind == "decide" { 188.0 } else { 172.0 }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

impl Side {
    fn normal(self) -> (f64, f64) {
        match self {
            Side::Left => (-1.0, 0.0),
            Side::Right => (1.0, 0.0),
            Side::Top => (0.0, -1.0),
            Side::Bottom => (0.0, 1.0),
        }
    }
}

/// Where a connection leaves and arrives: right → left when the target is to the right, bottom → top when it is
/// below, and a loop on the left (dashed) when it goes back up to a block above.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Route {
    pub from: (f64, f64),
    pub from_side: Side,
    pub to: (f64, f64),
    pub to_side: Side,
    pub back: bool,
}

pub fn route(src: Rect, dst: Rect) -> Route {
    let overlap_x = dst.x < src.x + src.w && src.x < dst.x + dst.w;
    if dst.y + dst.h <= src.y && overlap_x {
        return Route { from: src.left_mid(), from_side: Side::Left, to: dst.left_mid(), to_side: Side::Left, back: true };
    }
    if dst.x >= src.x + src.w - 1.0 {
        Route { from: src.right_mid(), from_side: Side::Right, to: dst.left_mid(), to_side: Side::Left, back: false }
    } else if dst.y >= src.y + src.h - 1.0 {
        Route { from: src.bottom_mid(), from_side: Side::Bottom, to: dst.top_mid(), to_side: Side::Top, back: false }
    } else if dst.x + dst.w <= src.x + 1.0 {
        Route { from: src.left_mid(), from_side: Side::Left, to: dst.right_mid(), to_side: Side::Right, back: false }
    } else {
        Route { from: src.top_mid(), from_side: Side::Top, to: dst.bottom_mid(), to_side: Side::Bottom, back: false }
    }
}

/// The cubic Bézier of a route: the handles go out along each side's normal (a loop bulges further out).
pub fn bezier(r: &Route) -> [(f64, f64); 4] {
    let d = ((r.to.0 - r.from.0).powi(2) + (r.to.1 - r.from.1).powi(2)).sqrt();
    let k = if r.back { (d * 0.35).clamp(40.0, 90.0) } else { (d * 0.4).clamp(16.0, 120.0) };
    let (n0, n1) = (r.from_side.normal(), r.to_side.normal());
    [r.from, (r.from.0 + n0.0 * k, r.from.1 + n0.1 * k), (r.to.0 + n1.0 * k, r.to.1 + n1.1 * k), r.to]
}

pub fn point_at(b: &[(f64, f64); 4], t: f64) -> (f64, f64) {
    let u = 1.0 - t;
    let (a, bb, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    (a * b[0].0 + bb * b[1].0 + c * b[2].0 + d * b[3].0, a * b[0].1 + bb * b[1].1 + c * b[2].1 + d * b[3].1)
}

/// The shortest distance from `p` to the curve (sampled), to pick a connection with the pointer.
pub fn distance_to(b: &[(f64, f64); 4], p: (f64, f64)) -> f64 {
    (0..=40).map(|i| point_at(b, i as f64 / 40.0)).map(|q| ((q.0 - p.0).powi(2) + (q.1 - p.1).powi(2)).sqrt()).fold(f64::INFINITY, f64::min)
}

/// The output ports of a block (where a new connection is dragged from): right and bottom.
pub fn out_ports(r: Rect) -> [(f64, f64); 2] {
    [r.right_mid(), r.bottom_mid()]
}

/// The block whose output port is under `p` (within `radius`), topmost first.
pub fn hit_port(rects: &[(String, Rect)], p: (f64, f64), radius: f64) -> Option<&str> {
    rects.iter().rev().find(|(_, r)| out_ports(*r).iter().any(|q| (q.0 - p.0).powi(2) + (q.1 - p.1).powi(2) <= radius * radius)).map(|(id, _)| id.as_str())
}

/// The block under `p`, topmost first (the last drawn is on top).
pub fn hit_node(rects: &[(String, Rect)], p: (f64, f64)) -> Option<&str> {
    rects.iter().rev().find(|(_, r)| r.contains(p)).map(|(id, _)| id.as_str())
}

pub fn bbox(rects: impl IntoIterator<Item = Rect>) -> Option<Rect> {
    rects.into_iter().fold(None, |acc: Option<Rect>, r| {
        Some(match acc {
            None => r,
            Some(a) => {
                let (x0, y0) = (a.x.min(r.x), a.y.min(r.y));
                let (x1, y1) = ((a.x + a.w).max(r.x + r.w), (a.y + a.h).max(r.y + r.h));
                Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }
            }
        })
    })
}

/// A free place for a new block next to `after` (to its right, else below), not over another block.
pub fn free_spot(rects: &[(String, Rect)], after: Option<Rect>, w: f64, h: f64) -> (f64, f64) {
    let free = |x: f64, y: f64| {
        let r = Rect { x: x - 20.0, y: y - 20.0, w: w + 40.0, h: h + 40.0 };
        !rects.iter().any(|(_, o)| o.x < r.x + r.w && r.x < o.x + o.w && o.y < r.y + r.h && r.y < o.y + o.h)
    };
    let (bx, by) = match after {
        Some(a) => (a.x + a.w + 56.0, a.y),
        None => match bbox(rects.iter().map(|(_, r)| *r)) {
            Some(b) => (b.x, b.y + b.h + 56.0),
            None => (40.0, 60.0),
        },
    };
    // to its right (going down), else below it (going down)
    let below = after.map(|a| (a.x, a.y + a.h + 56.0));
    for i in 0..30 {
        let step = i as f64 * 130.0;
        if free(bx, by + step) {
            return (bx, by + step);
        }
        if let Some((x, y)) = below.filter(|(x, y)| free(*x, *y + step)) {
            return (x, y + step);
        }
    }
    (bx, by)
}

/// The canvas' view: a screen point is `pan + world × zoom`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub zoom: f64,
    pub pan: (f64, f64),
}

pub const ZOOM_MIN: f64 = 0.3;
pub const ZOOM_MAX: f64 = 2.0;

impl Default for View {
    fn default() -> View {
        View { zoom: 1.0, pan: (24.0, 0.0) }
    }
}

impl View {
    pub fn to_screen(self, p: (f64, f64)) -> (f64, f64) {
        (self.pan.0 + p.0 * self.zoom, self.pan.1 + p.1 * self.zoom)
    }
    pub fn to_world(self, p: (f64, f64)) -> (f64, f64) {
        ((p.0 - self.pan.0) / self.zoom, (p.1 - self.pan.1) / self.zoom)
    }
    /// Zooms by `factor` keeping the world point under `at` (a screen point) where it is.
    pub fn zoom_at(&self, at: (f64, f64), factor: f64) -> View {
        let z = (self.zoom * factor).clamp(ZOOM_MIN, ZOOM_MAX);
        let w = self.to_world(at);
        View { zoom: z, pan: (at.0 - w.0 * z, at.1 - w.1 * z) }
    }
    /// Everything in view, with a margin, at most at 100%.
    pub fn fit(b: Rect, width: f64, height: f64, margin: f64) -> View {
        let z = ((width - 2.0 * margin) / b.w.max(1.0)).min((height - 2.0 * margin) / b.h.max(1.0)).clamp(ZOOM_MIN, 1.0);
        View { zoom: z, pan: (margin + (width - 2.0 * margin - b.w * z) / 2.0 - b.x * z, margin - b.y * z) }
    }
}

// ── words on the blocks ─────────────────────────────────────────────────────────────────────────────────────────

/// The comparisons of a rule (`field`/`operator`/`value` or `checks`), flat, with their operator's id.
pub fn rule_leaves(rule: &Value) -> Vec<(String, String, Value)> {
    match rule["checks"].as_array() {
        Some(c) if !c.is_empty() => c
            .iter()
            .map(|x| {
                let f = x["field"].as_str().or(rule["field"].as_str()).unwrap_or("content");
                (f.to_string(), x["operator"].as_str().unwrap_or("equals").to_string(), x.get("value").cloned().unwrap_or(Value::Null))
            })
            .collect(),
        _ => vec![(rule["field"].as_str().unwrap_or("content").to_string(), rule["operator"].as_str().unwrap_or("equals").to_string(), rule.get("value").cloned().unwrap_or(Value::Null))],
    }
}

/// A rule's comparisons back in the saved shape (`checks`, the first one also in `field`/`operator`/`value`).
pub fn rule_from_leaves(leaves: &[(String, String, Value)], any: bool) -> Value {
    let checks: Vec<Value> = leaves.iter().map(|(f, o, v)| json!({"field": f, "operator": o, "value": v})).collect();
    let mut r = json!({"checks": checks});
    if let Some((f, o, v)) = leaves.first() {
        r["field"] = json!(f);
        r["operator"] = json!(o);
        r["value"] = v.clone();
    }
    if any {
        r["match"] = json!("any");
    }
    r
}

/// The 14 operators: (id, name, symbol on the block).
pub const OPERATORS: &[(&str, &str, &str)] = &[
    ("equals", n_("is equal to"), "=="),
    ("not_equals", n_("is not"), "!="),
    ("contains", n_("contains"), "∋"),
    ("not_contains", n_("does not contain"), "∌"),
    ("starts_with", n_("starts with"), "^="),
    ("ends_with", n_("ends with"), "$="),
    ("greater_than", n_("greater than"), ">"),
    ("greater_than_or_equal", n_("at least"), "≥"),
    ("less_than", n_("less than"), "<"),
    ("less_than_or_equal", n_("at most"), "≤"),
    ("matches", n_("matches"), "~"),
    ("not_matches", n_("does not match"), "!~"),
    ("is_defined", n_("is filled"), "?"),
    ("is_not_defined", n_("is empty"), "∅"),
];

/// Whether an operator compares with a value ("is filled" and "is empty" do not).
pub fn takes_value(op: &str) -> bool {
    !matches!(op, "is_defined" | "is_not_defined")
}

/// How a field reads on a block: `var:classe.tipo` → `{tipo}`… the last part of the path; `content` → `{entrada}`.
pub fn field_text(field: &str) -> String {
    let f = field.trim();
    if f.is_empty() || f == "content" || f == "output" {
        return format!("{{{}}}", tr("input"));
    }
    let path = f.strip_prefix("var:").or_else(|| f.strip_prefix("content.")).unwrap_or(f);
    format!("{{{}}}", path.rsplit('.').next().unwrap_or(path))
}

pub fn value_text(v: &Value) -> String {
    match v {
        Value::String(s) => format!("\"{s}\""),
        Value::Null => String::new(),
        v => v.to_string(),
    }
}

/// The first comparison of a condition as the block shows it: `{tipo} == "fatura"`.
pub fn rule_text(rule: &Value) -> String {
    let leaves = rule_leaves(rule);
    let Some((f, o, v)) = leaves.first() else { return String::new() };
    let sym = OPERATORS.iter().find(|x| x.0 == o).map(|x| x.2).unwrap_or(o);
    let value = if takes_value(o) { value_text(v) } else { String::new() };
    let mut s = format!("{} {sym} {value}", field_text(f)).trim().to_string();
    if leaves.len() > 1 {
        s.push_str(&format!(" +{}", leaves.len() - 1));
    }
    s
}

/// A line under a block's name: plain (muted) or code (mono).
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub text: String,
    pub mono: bool,
    /// a code line that wraps instead of being cut (the start block's "entrada: …")
    pub wrap: bool,
}

fn plain(t: impl Into<String>) -> Line {
    Line { text: t.into(), mono: false, wrap: false }
}
fn mono(t: impl Into<String>) -> Line {
    Line { text: t.into(), mono: true, wrap: false }
}

// ── the flow's structured input ─────────────────────────────────────────────────────────────────────────────────

/// The kinds a field of the flow's input can have (kalango-fluxos `fields::KINDS`) and their names on the screen.
pub const FIELD_KINDS: [(&str, &str); 6] =
    [("text", n_("text")), ("number", n_("number")), ("integer", n_("integer")), ("boolean", n_("yes/no")), ("list", n_("list")), ("object", n_("object"))];

/// A field's kind as the screen names it ("texto", "lista"…).
pub fn field_kind_name(kind: &str) -> &'static str {
    tr(FIELD_KINDS.iter().find(|k| k.0 == kind).map(|k| k.1).unwrap_or("text"))
}

/// What a form's box shows before anything is typed, by kind.
pub fn field_hint(kind: &str) -> &'static str {
    match kind {
        "number" => "189.90",
        "integer" => "30",
        "boolean" => tr("yes or no"),
        "list" => "[\"a\", \"b\"]",
        "object" => "{\"a\": 1}",
        _ => "",
    }
}

impl FlowDoc {
    /// The fields every run must bring (`inputs`): `{name, kind, required, description}`.
    pub fn inputs(&self) -> Vec<Value> {
        self.rest.get("inputs").and_then(Value::as_array).cloned().unwrap_or_default()
    }

    /// How a trigger's event fills the fields (`input_map`: field → template such as "{evento.de}").
    pub fn input_map(&self) -> Map<String, Value> {
        self.rest.get("input_map").and_then(Value::as_object).cloned().unwrap_or_default()
    }

    /// Sets one field's template (empty: the field is no longer mapped).
    pub fn set_mapping(&mut self, field: &str, template: &str) {
        let mut m = self.input_map();
        set_template(&mut m, field, template);
        self.set_input_map(m);
    }

    fn set_input_map(&mut self, m: Map<String, Value>) {
        if m.is_empty() {
            self.rest.remove("input_map");
        } else {
            self.rest.insert("input_map".into(), Value::Object(m));
        }
    }

    /// A field renamed: its mapping follows it (also through an empty name while the person retypes it).
    pub fn rename_input(&mut self, old: &str, new: &str) {
        let mut m = self.input_map();
        if let Some(v) = m.remove(old) {
            m.insert(new.to_string(), v);
        }
        self.set_input_map(m);
    }

    /// A field removed: its mapping goes with it.
    pub fn forget_mapping(&mut self, field: &str) {
        let mut m = self.input_map();
        m.remove(field);
        self.set_input_map(m);
    }

    /// Replaces the fields (none: the input is free text again).
    pub fn set_inputs(&mut self, fields: Vec<Value>) {
        if fields.is_empty() {
            self.rest.remove("inputs");
        } else {
            self.rest.insert("inputs".into(), Value::Array(fields));
        }
    }
}

/// Sets or removes (empty) a field's template in a mapping (the flow's `input_map`, a Subflow's `input`).
pub fn set_template(m: &mut Map<String, Value>, field: &str, template: &str) {
    if template.trim().is_empty() {
        m.remove(field);
    } else {
        m.insert(field.to_string(), json!(template.trim()));
    }
}

/// A problem the service finds in a mapping (stable English: `"x" is not an input field`, `the required field "x"
/// is not filled`), after its prefix ("trigger mapping: ", "<block id>: ").
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MapIssue {
    NotAField(String),
    Unfilled(String),
}

impl MapIssue {
    pub fn parse(msg: &str) -> Option<MapIssue> {
        let quoted = |s: &str| s.strip_prefix('"').and_then(|s| s.split_once('"')).map(|(f, rest)| (f.to_string(), rest.to_string()));
        if let Some((f, rest)) = quoted(msg.trim()) {
            return (rest == " is not an input field").then_some(MapIssue::NotAField(f));
        }
        let (f, rest) = quoted(msg.trim().strip_prefix("the required field ")?)?;
        (rest == " is not filled").then_some(MapIssue::Unfilled(f))
    }

    pub fn field(&self) -> &str {
        match self {
            MapIssue::NotAField(f) | MapIssue::Unfilled(f) => f,
        }
    }

    pub fn text(&self) -> String {
        match self {
            MapIssue::NotAField(f) => kalango_i18n::trf("\"{f}\" is not an input field", &[("f", f.clone())]),
            MapIssue::Unfilled(f) => kalango_i18n::trf("the required field \"{f}\" is not filled", &[("f", f.clone())]),
        }
    }
}

/// The trigger mapping's problems among the flow's ("trigger mapping: …", joined with "; ").
pub fn trigger_map_issues(flow_issues: &str) -> Vec<MapIssue> {
    flow_issues.split("; ").filter_map(|m| m.strip_prefix("trigger mapping: ")).filter_map(MapIssue::parse).collect()
}

/// A problem as the screen says it: the mappings' ones in the person's language, the rest as the service wrote them.
pub fn issue_text(msg: &str) -> String {
    msg.split("; ")
        .map(|m| {
            let (prefix, rest) = match m.strip_prefix("trigger mapping: ") {
                Some(r) => (Some(tr("trigger mapping")), r),
                None => (None, m),
            };
            if let Some(t) = input_issue_text(m) {
                return t;
            }
            match MapIssue::parse(rest) {
                Some(i) => prefix.map(|p| format!("{p}: {}", i.text())).unwrap_or_else(|| i.text()),
                None => service_text(m),
            }
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// What kalango-fluxos says in English (its checks before saving, its failures' sentences: fluxos-core `flow.rs`
/// and `failure.rs`), as the screens say it. The service's texts are stable by design: each one is matched whole.
const SERVICE_TEXTS: &[&str] = &[
    n_("no connection reaches this block"),
    n_("choose the agent of this block"),
    n_("choose the agent that runs for each item"),
    n_("choose the flow"),
    n_("add at least one rule"),
    n_("the flow has no blocks"),
    n_("two blocks have the same id"),
    n_("at most 20 variables per block"),
    n_("save_as must be a variable name (letters, digits and _)"),
    n_("max_items goes from 1 to 50"),
    n_("items is \"content\", \"content.<field>\" or \"var:<variable>\""),
    n_("the decision must expire between 1 minute and 7 days"),
    n_("the decision model is decisao or decisao-omni"),
    n_("ask at least one question"),
    n_("the confidence goes from 0 to 1"),
    n_("retry needs max_attempts 1–5, initial_delay 0–30 s and timeout 1–300 s (or none)"),
    n_("The agent went past its time limit and the run was stopped."),
    n_("The model provider refused because of a usage limit (too many requests or quota used up)."),
    n_("The conversation went past the largest size the model accepts."),
    n_("The model provider refused the token (invalid key or no permission)."),
    n_("The model failed or was not running."),
    n_("The model refused the request (parameters, content or model not valid)."),
    n_("Could not connect to or call a connector (MCP)."),
    n_("One of the agent's tools failed during the run."),
    n_("Network failure calling an outside service."),
    n_("The agent no longer exists or was not available."),
    n_("The input did not fit the fields it needs."),
    n_("The system refused the action (permissions, allowed folders or your decision)."),
    n_("Content read during the task tried to give orders; the task was stopped."),
    n_("The agent reached its usage limit."),
    n_("The run stopped before the end: the service restarted or the connection closed."),
    n_("You stopped the run."),
    n_("Internal error while running."),
];

/// One of the service's sentences in the person's language (see [`SERVICE_TEXTS`]); "<failure> Technical detail:
/// <hints>." keeps its hints as they came. Anything else stays as it came.
pub fn service_text(msg: &str) -> String {
    let whole = |m: &str| SERVICE_TEXTS.iter().find(|t| **t == m).map(|t| tr(t).to_string());
    if let Some((head, hints)) = msg.split_once(" Technical detail: ") {
        if let Some(t) = whole(head) {
            return format!("{t} {} {hints}", tr("Technical detail:"));
        }
    }
    if let Some(t) = whole(msg) {
        return t;
    }
    let quoted = |pre: &str, post: &str| msg.strip_prefix(pre).and_then(|r| r.strip_suffix(post)).map(str::to_string);
    if let Some(a) = quoted("the system does not offer the action \"", "\"") {
        return kalango_i18n::trf("the system does not offer the action \"{a}\"", &[("a", a)]);
    }
    if let Some(v) = quoted("\"", "\" is not a variable name") {
        return kalango_i18n::trf("\"{v}\" is not a variable name", &[("v", v)]);
    }
    if let Some(o) = quoted("unknown operation \"", "\"") {
        return kalango_i18n::trf("unknown operation \"{o}\"", &[("o", o)]);
    }
    msg.to_string()
}

/// The service's words on the input fields ("input: the field name "x" must be a variable name"…), translated.
fn input_issue_text(msg: &str) -> Option<String> {
    let rest = msg.strip_prefix("input: ")?;
    let quoted = |s: &str, pre: &str, post: &str| s.strip_prefix(pre).and_then(|r| r.strip_suffix(post)).map(str::to_string);
    if let Some(f) = quoted(rest, "the field name \"", "\" must be a variable name") {
        return Some(kalango_i18n::trf("the field name \"{f}\" must be a variable name (letters, digits, _)", &[("f", f)]));
    }
    if let Some(f) = quoted(rest, "the field \"", "\" appears twice") {
        return Some(kalango_i18n::trf("the field \"{f}\" appears twice", &[("f", f)]));
    }
    None
}

/// A `flows.save` refusal ("a; subflow_1: b; c") split by where it belongs: a block's id (when it names one of
/// `nodes`) or "" for the flow.
pub fn split_issues(msg: &str, nodes: &[String]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for part in msg.split("; ") {
        let (k, v) = match part.split_once(": ") {
            Some((id, rest)) if nodes.iter().any(|n| n == id) => (id.to_string(), rest.to_string()),
            _ => (String::new(), part.to_string()),
        };
        match out.iter_mut().find(|(x, _)| *x == k) {
            Some((_, t)) => {
                *t += "; ";
                *t += &v;
            }
            None => out.push((k, v)),
        }
    }
    out
}

/// What a run received, in one line (Execuções' "Gatilho" column): a structured input as its values
/// (`"Conta de luz" · 189.9 · 2 itens`, in the order of the flow's `fields`), any other text as it came.
pub fn input_summary(input: &str, fields: &[Value]) -> String {
    let Ok(Value::Object(o)) = serde_json::from_str::<Value>(input.trim()) else { return input.replace('\n', " ") };
    let pos = |k: &str| fields.iter().position(|f| f["name"] == json!(k)).unwrap_or(usize::MAX);
    let mut items: Vec<(&String, &Value)> = o.iter().collect();
    items.sort_by_key(|(k, _)| pos(k));
    items
        .into_iter()
        .map(|(_, x)| match x {
            Value::String(t) => format!("\"{t}\""),
            Value::Array(a) => format!("{} {}", a.len(), if a.len() == 1 { tr("item") } else { tr("items") }),
            Value::Object(_) => "{…}".to_string(),
            other => value_text(other),
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

/// A new field's name: "campo", then "campo_2", "campo_3"… (the first free one).
pub fn new_field_name(fields: &[Value]) -> String {
    let taken = |n: &str| fields.iter().any(|f| f["name"] == json!(n));
    std::iter::once("campo".to_string()).chain((2..).map(|i| format!("campo_{i}"))).find(|n| !taken(n)).unwrap()
}

/// The start block's line: "entrada: {remetente}, {assunto}" (None without fields).
pub fn inputs_line(fields: &[Value]) -> Option<String> {
    let names: Vec<String> = fields.iter().filter_map(|f| f["name"].as_str()).map(|n| format!("{{{n}}}")).collect();
    (!names.is_empty()).then(|| format!("{}: {}", tr("input"), names.join(", ")))
}

/// A test form's values as the object `flows.run` checks: empty boxes are left out (the service names the
/// required ones that are missing), the rest go as typed text (the service reads "30" as 30 and "sim" as true).
pub fn form_object(values: &[(String, String)]) -> Value {
    Value::Object(values.iter().filter(|(_, v)| !v.trim().is_empty()).map(|(k, v)| (k.clone(), json!(v.trim()))).collect())
}

/// How a caller passes the fields (HTTP and MCP): `{"remetente": …, "assunto": …}`.
pub fn call_example(fields: &[Value]) -> String {
    let names: Vec<String> = fields.iter().filter_map(|f| f["name"].as_str()).take(3).map(|n| format!("\"{n}\": …")).collect();
    let more = if fields.len() > 3 { ", …" } else { "" };
    format!("flows.run {} {{{}{more}}}", tr("with"), names.join(", "))
}

/// What the decision models are called on the screen.
pub fn decision_model_name(id: &str) -> &'static str {
    if id == "decisao-omni" { "d1-Omni-2B" } else { "d1-2B" }
}

/// A kind's name as the palette says it ("Ask for approval", "Notify"…): a block with nothing that names it yet.
pub fn kind_name(kind: &str) -> String {
    let id = format!("block:{kind}");
    palette().into_iter().flat_map(|(_, items)| items).find(|i| i.0 == id).map(|i| tr(i.1).to_string()).unwrap_or_else(|| tr(look(kind).caps).to_string())
}

/// The key under which [`node_text`]'s names hold a system action's description ("tool:arquivos.mover").
pub fn tool_key(tool: &str) -> String {
    format!("tool:{tool}")
}

/// The key under which [`node_text`]'s names hold another flow's name ("flow:arquivar_fatura").
pub fn flow_key(flow: &str) -> String {
    format!("flow:{flow}")
}

/// A system action's description as a name: "Move ou renomeia um arquivo ou pasta [nível usuario; …]" → its words
/// without the bracket.
pub fn tool_title(description: &str) -> String {
    // (the first clause: "A data e a hora agora neste computador: o dia, …" names it by what comes before the colon)
    description.split(" [").next().unwrap_or("").split([':', ';']).next().unwrap_or("").trim().trim_end_matches('.').to_string()
}

/// A block's name and the lines under it. `agents` names the agents by id, the system actions by [`tool_key`] and
/// the other flows by [`flow_key`]; `items` is how many items the last run went through (For each).
pub fn node_text(n: &NodeDoc, agents: &HashMap<String, String>, items: Option<usize>) -> (String, Vec<Line>) {
    let d = &n.data;
    let s = |k: &str| d[k].as_str().unwrap_or("").trim().to_string();
    let agent = |id: &str| agents.get(id).cloned().unwrap_or_else(|| id.to_string());
    let (auto, lines): (String, Vec<Line>) = match n.kind.as_str() {
        "agent" => {
            let id = s("agent_id");
            let mut l = vec![plain(if id.is_empty() { tr("choose the agent").to_string() } else { agent(&id) })];
            if let Some(v) = d["save_as"].as_str().filter(|v| !v.is_empty()) {
                l.push(plain(format!("{}: {{{v}}}", tr("output"))));
            }
            (if id.is_empty() { tr("Agent").into() } else { agent(&id) }, l)
        }
        "message" => (tr("Message").into(), vec![plain(s("content"))]),
        "wait" => {
            let sec = d["seconds"].as_u64().unwrap_or(0);
            (tr("Wait").into(), vec![mono(if sec.is_multiple_of(60) { format!("{} min", sec / 60) } else { format!("{sec} s") })])
        }
        "condition" => {
            let first = d["conditions"].get(0).map(|c| rule_text(&c["data"])).unwrap_or_default();
            (tr("Condition").into(), vec![mono(first)])
        }
        "ask" => (s("message"), vec![plain(tr("waits for your answer"))]),
        "parallel" => (tr("In parallel").into(), vec![plain(tr("every way at once"))]),
        "join" => (tr("Join").into(), vec![plain(tr("waits for every way"))]),
        "set_variables" => {
            let names: Vec<String> = d["variables"].as_array().into_iter().flatten().filter_map(|v| v["name"].as_str().map(|n| format!("{{{n}}}"))).collect();
            (tr("Variables").into(), vec![mono(names.join(", "))])
        }
        "for_each" => {
            let items_src = s("items");
            let src = if items_src.is_empty() || items_src == "content" { field_text("content") } else { field_text(&items_src) };
            let mut t = src;
            if let Some(k) = items {
                t.push_str(&format!(" · {k} {}", if k == 1 { tr("item") } else { tr("items") }));
            }
            (tr("Each item").into(), vec![mono(t)])
        }
        "system_action" => {
            let tool = s("tool");
            let mut l = vec![plain(tool.clone())];
            if let Some(v) = d["arguments"].as_object().and_then(|a| a.values().filter_map(|v| v.as_str()).find(|v| v.starts_with('~') || v.starts_with('/'))) {
                l.push(mono(v.to_string()));
            }
            l.push(plain(tr("kalangod · user level")));
            let named = agents.get(&tool_key(&tool)).map(|d| tool_title(d)).filter(|t| !t.is_empty());
            (if tool.is_empty() { tr("System action").into() } else { named.unwrap_or(tool) }, l)
        }
        "approval" => (s("question"), vec![plain(if s("details").is_empty() { tr("you decide on the approval screen").to_string() } else { s("details") })]),
        "notify" => (s("title"), vec![plain(s("body"))]),
        "subflow" => (agents.get(&flow_key(&s("flow_id"))).cloned().filter(|x| !x.trim().is_empty()).unwrap_or_else(|| tr("Subflow").into()), vec![mono(s("flow_id"))]),
        "decide" => {
            let pct = (d["threshold"].as_f64().unwrap_or(0.8) * 100.0).round();
            let route = d["route"].as_str().unwrap_or("");
            (
                d["questions"][route]["instructions"].as_str().unwrap_or("").to_string(),
                vec![plain(format!("{} · {} · {}", decision_model_name(d["model"].as_str().unwrap_or("")), tr("local"), tr("1 pass"))), plain(format!("{} {pct}%", tr("confidence threshold")))],
            )
        }
        "transcribe" => (tr("Transcribe audio").into(), vec![mono(s("file")), plain("LFM3-Audio")]),
        "describe_image" => (s("question"), vec![mono(if s("file").is_empty() { tr("the screen").to_string() } else { s("file") }), plain("LFM3-VL")]),
        "speak" => (s("text"), vec![mono(s("out")), plain("LFM3-Audio")]),
        "sandbox" => (tr("In sandbox").into(), vec![]),
        _ => (tr("End").into(), vec![plain(tr("ends the flow"))]),
    };
    // where the block keeps its output ("Salva em {var}"; an agent's says "saída: {…}" already)
    let mut lines = lines;
    if n.kind != "agent" {
        if let Some(v) = d["save_as"].as_str().filter(|v| !v.trim().is_empty()) {
            lines.push(plain(kalango_i18n::trf("saves in {var}", &[("var", format!("{{{}}}", v.trim()))])));
        }
    }
    let title = if !n.title.trim().is_empty() {
        n.title.clone()
    } else if auto.trim().is_empty() {
        kind_name(&n.kind)
    } else {
        auto
    };
    (title, lines.into_iter().filter(|l| !l.text.trim().is_empty()).collect())
}

/// A trigger's name and line on the canvas.
pub fn trigger_text(t: &Value) -> (String, Vec<Line>) {
    let s = |k: &str| t[k].as_str().unwrap_or("").to_string();
    match t["kind"].as_str().unwrap_or("") {
        "schedule" => {
            let when = if s("cron").is_empty() { format!("{:02}:{:02}", t["hour"].as_u64().unwrap_or(0), t["minute"].as_u64().unwrap_or(0)) } else { s("cron") };
            let days: Vec<u8> = t["days"].as_array().map(|a| a.iter().filter_map(|d| d.as_u64()).map(|d| d as u8).collect()).unwrap_or_default();
            (tr("Schedule").into(), vec![mono(format!("{} · {when}", days_text(days)))])
        }
        "file" => (tr("File or folder").into(), vec![mono(s("path")), plain(s("pattern"))]),
        "notification" => {
            let app = s("app");
            let rule = t.get("rule").filter(|r| !r.is_null()).map(rule_text).unwrap_or_default();
            (if app.is_empty() { tr("Any notification").into() } else { format!("{} {app}", tr("New notification from")) }, vec![plain(rule)])
        }
        "webhook" => (tr("Local webhook").into(), vec![plain(tr("only from this computer, with a key"))]),
        "system" => (tr("System event").into(), vec![mono(format!("{} {}", s("event"), s("filter")).trim().to_string())]),
        _ => (tr("By hand").into(), vec![]),
    }
    .clone_filtered()
}

trait Filtered {
    fn clone_filtered(self) -> Self;
}
impl Filtered for (String, Vec<Line>) {
    fn clone_filtered(self) -> Self {
        (self.0, self.1.into_iter().filter(|l| !l.text.trim().is_empty()).collect())
    }
}

/// "todo dia", "dias úteis", "seg, qua".
pub fn days_text(days: Vec<u8>) -> String {
    const NAMES: [&str; 7] = [n_("Mon"), n_("Tue"), n_("Wed"), n_("Thu"), n_("Fri"), n_("Sat"), n_("Sun")];
    let mut d = days;
    d.sort_unstable();
    d.dedup();
    if d.is_empty() || d.len() == 7 {
        tr("every day").into()
    } else if d == [1, 2, 3, 4, 5] {
        tr("weekdays").into()
    } else {
        d.iter().filter(|x| (1..=7).contains(*x)).map(|x| tr(NAMES[*x as usize - 1]).to_lowercase()).collect::<Vec<_>>().join(", ")
    }
}

/// "notificação do Correio", "todo dia 09:00"… — the header's "gatilho: …".
pub fn trigger_summary(t: &Value) -> String {
    match t["kind"].as_str().unwrap_or("") {
        "notification" if !t["app"].as_str().unwrap_or("").is_empty() => format!("{} {}", tr("notification from"), t["app"].as_str().unwrap_or("")),
        "notification" => tr("any notification").into(),
        "schedule" => trigger_text(t).1.first().map(|l| l.text.clone()).unwrap_or_default(),
        "file" => format!("{} {}", tr("file in"), t["path"].as_str().unwrap_or("")),
        "webhook" => tr("local webhook").into(),
        "system" => format!("{} {}", tr("system event"), t["event"].as_str().unwrap_or("")),
        _ => tr("by hand").into(),
    }
}

// ── the last run on the canvas ──────────────────────────────────────────────────────────────────────────────────

/// A step of a run (`runs.get`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Step {
    pub seq: i64,
    pub node: String,
    pub kind: String,
    pub status: String,
    pub input: String,
    pub output: String,
    pub detail: String,
    pub started: i64,
    pub finished: Option<i64>,
}

impl Step {
    pub fn from_json(v: &Value) -> Step {
        let s = |k: &str| v[k].as_str().unwrap_or("").to_string();
        Step { seq: v["seq"].as_i64().unwrap_or(0), node: s("node_id"), kind: s("kind"), status: s("status"), input: s("input"), output: s("output"), detail: service_text(&s("detail")), started: v["started_at"].as_i64().unwrap_or(0), finished: v["finished_at"].as_i64() }
    }
}

/// How a run marks the canvas: each block's state ("done", "failed", "running", "waiting") and the connections it
/// travelled (each step to the next one, when they are connected).
pub fn run_marks(flow: &FlowDoc, steps: &[Step], waiting_node: Option<&str>) -> (HashMap<String, String>, HashSet<(String, String)>) {
    let mut states: HashMap<String, String> = HashMap::new();
    for s in steps {
        let st = match s.status.as_str() {
            "done" => "done",
            "failed" => "failed",
            "running" => "running",
            _ => "done",
        };
        states.insert(s.node.clone(), st.to_string());
    }
    if let Some(w) = waiting_node {
        states.insert(w.to_string(), "waiting".into());
    }
    let mut travelled = HashSet::new();
    let mut order: Vec<&str> = steps.iter().map(|s| s.node.as_str()).collect();
    if let Some(w) = waiting_node {
        order.push(w);
    }
    for pair in order.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if a != b && flow.edges.iter().any(|e| e.source == a && e.target == b) {
            travelled.insert((a.to_string(), b.to_string()));
        }
    }
    // a loop back to a block already visited counts too
    for pair in order.windows(2) {
        if let Some(e) = flow.edges.iter().find(|e| e.source == pair[0] && e.target == pair[1]) {
            travelled.insert((e.source.clone(), e.target.clone()));
        }
    }
    // a sandbox's blocks run between it and what follows it: each level (the flow, each sandbox) counts on its own
    let parent = |id: &str| flow.node(id).and_then(|n| n.parent.clone());
    let levels: HashSet<Option<String>> = order.iter().map(|id| parent(id)).collect();
    for level in levels {
        let mine: Vec<&str> = order.iter().copied().filter(|id| parent(id) == level).collect();
        for pair in mine.windows(2) {
            if pair[0] != pair[1] && flow.edges.iter().any(|e| e.source == pair[0] && e.target == pair[1]) {
                travelled.insert((pair[0].to_string(), pair[1].to_string()));
            }
        }
    }
    (states, travelled)
}

/// The way a step took, from its detail ("→ sim", "… → fatura · 41 ms", "allow_once").
pub fn way_taken(s: &Step) -> Option<String> {
    match s.kind.as_str() {
        "approval" => Some(if s.detail == "allow_once" || s.detail == "allow_always" { "sim".into() } else { "não".into() }),
        _ => {
            let after = s.detail.rsplit_once("→ ")?.1;
            Some(after.split(" · ").next().unwrap_or(after).trim().to_string()).filter(|x| !x.is_empty())
        }
    }
}

// ── decisions ───────────────────────────────────────────────────────────────────────────────────────────────────

/// A decision step read back: the options with their probabilities (highest first), the confidence, the way taken,
/// the time and whether it came from the cache.
#[derive(Clone, Debug, PartialEq)]
pub struct Decision {
    pub options: Vec<(String, f64)>,
    pub confidence: f64,
    pub chosen: String,
    pub millis: Option<u64>,
    pub cached: bool,
}

/// From a decide step: its output (`{answers, usage}`) gives the probabilities; its detail the way and the time.
pub fn parse_decision(s: &Step, route: &str) -> Option<Decision> {
    if s.kind != "decide" {
        return None;
    }
    let out: Value = serde_json::from_str(&s.output).unwrap_or(Value::Null);
    let answers = out["answers"].as_object();
    let answer = answers.and_then(|a| a.get(route).or_else(|| a.values().next())).cloned().unwrap_or(Value::Null);
    let mut options: Vec<(String, f64)> = match answer["probabilities"].as_object() {
        Some(p) => p.iter().map(|(k, v)| (k.clone(), v.as_f64().unwrap_or(0.0))).collect(),
        None => match answer["noul"].as_f64() {
            Some(p) => vec![("true".into(), p), ("false".into(), 1.0 - p)],
            None => vec![],
        },
    };
    options.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let confidence = answer["confidence"].as_f64().or_else(|| answer["noul"].as_f64().map(|p| (p - 0.5).abs() * 2.0)).or_else(|| {
        let c = s.detail.split("confiança ").nth(1)?.split_whitespace().next()?;
        c.replace(',', ".").parse().ok()
    })?;
    let chosen = way_taken(s).unwrap_or_default();
    let millis = s.detail.split(" · ").filter_map(|p| p.trim().strip_suffix(" ms").or_else(|| p.trim().strip_suffix(" ms (cache)"))).filter_map(|n| n.trim().parse().ok()).last();
    Some(Decision { options, confidence, chosen, millis, cached: s.detail.contains("(cache)") })
}

/// The decisions of a flow over its runs (design "Decisões desta rotina").
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DecisionStats {
    pub runs: usize,
    pub alone: usize,
    pub asked: usize,
    pub median_ms: Option<u64>,
    pub p95_ms: Option<u64>,
    pub cached: usize,
}

pub fn decision_stats(runs: &[Vec<Step>]) -> DecisionStats {
    let mut st = DecisionStats { runs: runs.len(), ..Default::default() };
    let mut times = Vec::new();
    for steps in runs {
        for s in steps.iter().filter(|s| s.kind == "decide") {
            let Some(d) = parse_decision(s, "") else { continue };
            if d.chosen == LOW_CONFIDENCE { st.asked += 1 } else { st.alone += 1 }
            if d.cached {
                st.cached += 1;
            }
            if let Some(m) = d.millis {
                times.push(m);
            }
        }
    }
    times.sort_unstable();
    if !times.is_empty() {
        st.median_ms = Some(times[times.len() / 2]);
        st.p95_ms = Some(times[((times.len() as f64 * 0.95).ceil() as usize).clamp(1, times.len()) - 1]);
    }
    st
}

// ── numbers ─────────────────────────────────────────────────────────────────────────────────────────────────────

/// A decimal as the interface writes it (a comma in Portuguese).
pub fn decimal(x: f64, places: usize) -> String {
    let s = format!("{x:.places$}");
    if tr(".") == "," { s.replace('.', ",") } else { s }
}

/// "4,1 s", "41 ms", "2 min".
pub fn duration_text(secs: f64) -> String {
    if secs < 1.0 {
        format!("{} ms", (secs * 1000.0).round())
    } else if secs < 60.0 {
        format!("{} s", decimal(secs, 1))
    } else {
        format!("{} min", (secs / 60.0).round())
    }
}

/// "812", "3,1 mil" (thousands).
pub fn tokens_text(n: i64) -> String {
    if n < 1000 { n.to_string() } else if n < 10_000 { format!("{} {}", decimal(n as f64 / 1000.0, 1), tr("k")) } else { format!("{} {}", n / 1000, tr("k")) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_flows_input_fields_round_trip_and_name_the_start() {
        let mut f = flow();
        assert!(f.inputs().is_empty());
        assert_eq!(inputs_line(&f.inputs()), None);
        let fields = vec![json!({"name": "remetente", "kind": "text", "required": true}), json!({"name": "anexos", "kind": "list"})];
        f.set_inputs(fields.clone());
        let back = FlowDoc::from_json(&f.to_json());
        assert_eq!(back.inputs(), fields);
        assert_eq!(inputs_line(&back.inputs()).unwrap(), format!("{}: {{remetente}}, {{anexos}}", tr("input")));
        // no fields: the key goes away (the service reads free text again)
        f.set_inputs(vec![]);
        assert!(f.to_json().get("inputs").is_none());
    }

    #[test]
    fn the_trigger_mapping_follows_renames_and_empties() {
        let mut f = flow();
        f.set_mapping("remetente", " {evento.de} ");
        f.set_mapping("anexos", "{evento.anexos}");
        assert_eq!(f.to_json()["input_map"], json!({"remetente": "{evento.de}", "anexos": "{evento.anexos}"}));
        f.rename_input("remetente", "");
        f.rename_input("", "de");
        assert_eq!(f.input_map().get("de"), Some(&json!("{evento.de}")));
        f.forget_mapping("de");
        f.set_mapping("anexos", "");
        assert!(f.to_json().get("input_map").is_none());
    }

    #[test]
    fn mapping_problems_read_from_the_services_words() {
        let i = trigger_map_issues("input: x; trigger mapping: \"cor\" is not an input field; trigger mapping: the required field \"assunto\" is not filled");
        assert_eq!(i, vec![MapIssue::NotAField("cor".into()), MapIssue::Unfilled("assunto".into())]);
        assert_eq!(MapIssue::parse("something else"), None);
        assert_eq!(issue_text("choose the flow"), tr("choose the flow"), "the service's own sentences are translated");
        assert_eq!(issue_text("something the screens do not know"), "something the screens do not know");
        assert_ne!(issue_text("input: the field \"a\" appears twice"), "input: the field \"a\" appears twice");
        assert!(issue_text("input: the field name \"\" must be a variable name").contains("\"\""));
        assert_eq!(issue_text("the required field \"a\" is not filled"), MapIssue::Unfilled("a".into()).text());
        let nodes = vec!["sub_1".to_string()];
        assert_eq!(
            split_issues("sub_1: \"x\" is not an input field; no blocks; sub_1: the required field \"y\" is not filled", &nodes),
            vec![("sub_1".into(), "\"x\" is not an input field; the required field \"y\" is not filled".into()), (String::new(), "no blocks".into())]
        );
    }

    #[test]
    fn a_runs_input_reads_as_its_values() {
        let fields = vec![json!({"name": "conta"}), json!({"name": "anexos"})];
        assert_eq!(input_summary("{\"anexos\": [\"a\", \"b\"], \"conta\": \"Conta de luz\"}", &fields), format!("\"Conta de luz\" · 2 {}", tr("items")));
        assert_eq!(input_summary("Fatura de novembro\nvence 28", &[]), "Fatura de novembro vence 28");
    }

    #[test]
    fn new_fields_get_free_names() {
        assert_eq!(new_field_name(&[]), "campo");
        assert_eq!(new_field_name(&[json!({"name": "campo"}), json!({"name": "campo_2"})]), "campo_3");
    }

    #[test]
    fn a_test_form_leaves_out_empty_boxes_and_keeps_text() {
        let v = form_object(&[("remetente".into(), " ana@exemplo.org ".into()), ("assunto".into(), "  ".into()), ("dias".into(), "30".into())]);
        assert_eq!(v, json!({"remetente": "ana@exemplo.org", "dias": "30"}));
    }

    #[test]
    fn the_call_example_lists_the_first_fields() {
        let f: Vec<Value> = ["a", "b", "c", "d"].iter().map(|n| json!({"name": n})).collect();
        assert_eq!(call_example(&f[..2]), format!("flows.run {} {{\"a\": …, \"b\": …}}", tr("with")));
        assert!(call_example(&f).ends_with("\"c\": …, …}"));
    }

    fn flow() -> FlowDoc {
        FlowDoc::from_json(&json!({
            "nodes": [
                {"id": "a", "position": {"x": 0, "y": 0}, "type": "agent", "data": {"agent_id": "org", "save_as": "classe"}, "title": "Classificar"},
                {"id": "c", "position": {"x": 228, "y": 0}, "type": "condition", "data": {"conditions": [{"id": "sim", "data": {"checks": [
                    {"field": "var:classe.tipo", "operator": "equals", "value": "fatura"}, {"field": "var:classe.valor", "operator": "greater_than", "value": 0}]}}]}},
                {"id": "e", "position": {"x": 456, "y": 0}, "type": "end"},
                {"id": "d", "position": {"x": 228, "y": 200}, "type": "decide", "data": {"route": "r", "questions": {"r": {"type": "choice", "instructions": "Para onde?", "criteria": {"fatura": "x", "agenda": null}}}}}
            ],
            "edges": [{"source": "a", "target": "c"}, {"source": "c", "target": "e", "sourceHandle": "bottom-handle"}],
            "limits": {"max_agent_runs": 3, "max_steps": 50}
        }))
    }

    #[test]
    fn saved_shape_round_trips_and_keeps_what_it_does_not_edit() {
        let f = flow();
        let back = FlowDoc::from_json(&f.to_json());
        assert_eq!(f, back);
        assert_eq!(back.rest["limits"]["max_steps"], 50);
        assert_eq!(f.to_json()["edges"][1]["sourceHandle"], "bottom-handle");
        assert_eq!(f.entry().unwrap().id, "a");
        assert_eq!(f.new_id("agent"), "agent_1");
    }

    #[test]
    fn ways_out_and_their_labels() {
        let f = flow();
        assert_eq!(ways(f.node("c").unwrap()), vec![Some("sim".into()), Some(ELSE_HANDLE.into())]);
        assert_eq!(ways(f.node("d").unwrap()), vec![Some("agenda".into()), Some("fatura".into()), Some(LOW_CONFIDENCE.into())]);
        assert_eq!(ways(f.node("e").unwrap()), Vec::<Option<String>>::new());
        assert_eq!(ways(f.node("a").unwrap()), vec![None]);
        assert_eq!(way_label("condition", Some(ELSE_HANDLE)), tr("no"));
        assert_eq!(way_label("decide", Some(LOW_CONFIDENCE)), tr("low confidence"));
        assert_eq!(way_label("decide", Some("true")), tr("yes"));
        assert_eq!(way_label("agent", None), "");
        assert_eq!(way_label("condition", Some("sim")), "sim");
        let noul = json!({"route": "q", "questions": {"q": {"type": "noul"}}});
        assert_eq!(decide_ways(&noul), vec!["true", "false", LOW_CONFIDENCE]);
        let score = json!({"route": "q", "questions": {"q": {"type": "score", "criteria": ["baixa", "média", "alta"]}}});
        assert_eq!(decide_ways(&score), vec!["0", "1", "2", LOW_CONFIDENCE]);
    }

    #[test]
    fn connecting_takes_the_next_free_way() {
        let mut f = flow();
        assert!(f.connect("c", "a"));
        assert_eq!(f.edges.last().unwrap().handle.as_deref(), Some("sim"));
        assert!(!f.connect("c", "a"), "the same connection twice");
        assert!(!f.connect("a", "a"), "a block to itself");
        assert!(!f.connect("c", "d"), "every way of the condition is taken");
        assert!(!f.connect("e", "a"), "the end has no way out");
        assert!(f.connect("d", "e"));
        assert_eq!(f.edges.last().unwrap().handle.as_deref(), Some("agenda"));
        f.set_way("d", Some("agenda"), Some("a"));
        assert!(f.edges.iter().any(|e| e.source == "d" && e.target == "a" && e.handle.as_deref() == Some("agenda")));
        assert!(!f.edges.iter().any(|e| e.source == "d" && e.target == "e"));
        f.set_way("d", Some("agenda"), None);
        assert!(!f.edges.iter().any(|e| e.source == "d"));
        f.remove_node("c");
        assert!(f.edges.iter().all(|e| e.source != "c" && e.target != "c"));
    }

    #[test]
    fn where_connections_leave_and_arrive() {
        let a = Rect { x: 0.0, y: 0.0, w: 172.0, h: 90.0 };
        let right = route(a, Rect { x: 228.0, y: 0.0, w: 172.0, h: 100.0 });
        assert_eq!((right.from, right.from_side, right.to, right.to_side, right.back), ((172.0, 45.0), Side::Right, (228.0, 50.0), Side::Left, false));
        let below = route(a, Rect { x: 0.0, y: 180.0, w: 172.0, h: 80.0 });
        assert_eq!((below.from, below.to), ((86.0, 90.0), (86.0, 180.0)));
        assert_eq!((below.from_side, below.to_side), (Side::Bottom, Side::Top));
        let back = route(Rect { x: 0.0, y: 300.0, w: 172.0, h: 90.0 }, a);
        assert!(back.back);
        assert_eq!((back.from_side, back.to_side), (Side::Left, Side::Left));
        let left = route(Rect { x: 300.0, y: 0.0, w: 172.0, h: 90.0 }, Rect { x: 0.0, y: 20.0, w: 172.0, h: 90.0 });
        assert_eq!((left.from_side, left.to_side), (Side::Left, Side::Right));
        let b = bezier(&right);
        assert_eq!((b[0], b[3]), (right.from, right.to));
        assert!(b[1].0 > b[0].0 && b[2].0 < b[3].0, "handles go out along the sides");
        let bb = bezier(&back);
        assert!(bb[1].0 < 0.0 && bb[2].0 < 0.0, "a loop bulges to the left");
        assert_eq!(point_at(&b, 0.0), b[0]);
        assert_eq!(point_at(&b, 1.0), b[3]);
        assert!(distance_to(&b, (200.0, 47.0)) < 3.0);
        assert!(distance_to(&b, (200.0, 200.0)) > 100.0);
    }

    #[test]
    fn hit_testing() {
        let rects = vec![("a".to_string(), Rect { x: 0.0, y: 0.0, w: 172.0, h: 90.0 }), ("b".to_string(), Rect { x: 100.0, y: 50.0, w: 172.0, h: 90.0 })];
        assert_eq!(hit_node(&rects, (120.0, 60.0)), Some("b"), "the top one");
        assert_eq!(hit_node(&rects, (10.0, 10.0)), Some("a"));
        assert_eq!(hit_node(&rects, (500.0, 10.0)), None);
        assert_eq!(hit_port(&rects, (173.0, 46.0), 7.0), Some("a"));
        assert_eq!(hit_port(&rects, (186.0, 141.0), 7.0), Some("b"), "the bottom port");
        assert_eq!(hit_port(&rects, (50.0, 50.0), 7.0), None);
    }

    #[test]
    fn view_zoom_and_fit() {
        let v = View { zoom: 1.0, pan: (10.0, 20.0) };
        assert_eq!(v.to_world(v.to_screen((5.0, 7.0))), (5.0, 7.0));
        let z = v.zoom_at((100.0, 100.0), 2.0);
        assert_eq!(z.zoom, 2.0);
        let w = v.to_world((100.0, 100.0));
        let s = z.to_screen(w);
        assert!((s.0 - 100.0).abs() < 1e-9 && (s.1 - 100.0).abs() < 1e-9, "the point under the pointer stays");
        assert_eq!(v.zoom_at((0.0, 0.0), 100.0).zoom, ZOOM_MAX);
        let f = View::fit(Rect { x: 0.0, y: 0.0, w: 2000.0, h: 500.0 }, 1000.0, 600.0, 20.0);
        assert!((f.zoom - 0.48).abs() < 1e-9);
        let small = View::fit(Rect { x: 50.0, y: 50.0, w: 100.0, h: 100.0 }, 1000.0, 600.0, 20.0);
        assert_eq!(small.zoom, 1.0, "never past 100%");
        assert_eq!(small.to_screen((50.0, 50.0)).1, 20.0);
        assert_eq!(bbox([Rect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 }, Rect { x: 20.0, y: -5.0, w: 10.0, h: 10.0 }]), Some(Rect { x: 0.0, y: -5.0, w: 30.0, h: 15.0 }));
    }

    #[test]
    fn free_spot_avoids_blocks() {
        let rects = vec![("a".to_string(), Rect { x: 0.0, y: 0.0, w: 172.0, h: 90.0 }), ("b".to_string(), Rect { x: 228.0, y: 0.0, w: 172.0, h: 90.0 })];
        let (x, y) = free_spot(&rects, Some(rects[0].1), 172.0, 90.0);
        let r = Rect { x, y, w: 172.0, h: 90.0 };
        assert!(!rects.iter().any(|(_, o)| o.x < r.x + r.w && r.x < o.x + o.w && o.y < r.y + r.h && r.y < o.y + o.h), "{x} {y}");
        assert_eq!(free_spot(&[], None, 172.0, 90.0), (40.0, 60.0));
    }

    #[test]
    fn words_on_blocks() {
        let f = flow();
        let agents: HashMap<String, String> = [("org".to_string(), "Organizador".to_string())].into();
        let (t, l) = node_text(f.node("a").unwrap(), &agents, None);
        assert_eq!(t, "Classificar");
        assert_eq!(l[0].text, "Organizador");
        assert_eq!(l[1].text, format!("{}: {{classe}}", tr("output")));
        let (_, l) = node_text(f.node("c").unwrap(), &agents, None);
        assert_eq!(l[0], Line { text: "{tipo} == \"fatura\" +1".into(), mono: true, wrap: false });
        let (t, l) = node_text(f.node("d").unwrap(), &agents, None);
        assert_eq!(t, "Para onde?");
        assert!(l[0].text.starts_with("d1-2B"));
        assert!(l[1].text.ends_with("80%"));
        let each = NodeDoc { id: "x".into(), x: 0.0, y: 0.0, kind: "for_each".into(), data: json!({"items": "var:anexos", "agent_id": "a"}), ..Default::default() };
        assert_eq!(node_text(&each, &agents, Some(2)).1[0].text, format!("{{anexos}} · 2 {}", tr("items")));
        assert_eq!(field_text("content"), format!("{{{}}}", tr("input")));
        assert_eq!(field_text("content.valor"), "{valor}");
        let leaves = rule_leaves(&f.node("c").unwrap().data["conditions"][0]["data"]);
        assert_eq!(leaves.len(), 2);
        let back = rule_from_leaves(&leaves, false);
        assert_eq!(rule_leaves(&back), leaves);
        assert_eq!(back["field"], "var:classe.tipo");
        assert_eq!(rule_leaves(&json!({"operator": "contains", "value": "x"})), vec![("content".to_string(), "contains".to_string(), json!("x"))]);
        assert_eq!(days_text(vec![5, 1, 2, 3, 4]), tr("weekdays"));
        assert_eq!(days_text(vec![]), tr("every day"));
        let (t, _) = trigger_text(&json!({"kind": "notification", "app": "Correio"}));
        assert!(t.ends_with("Correio"));
    }

    #[test]
    fn run_marks_on_the_canvas() {
        let f = flow();
        let st = |node: &str, status: &str| Step { node: node.into(), status: status.into(), ..Default::default() };
        let (states, travelled) = run_marks(&f, &[st("a", "done"), st("c", "done"), st("e", "failed")], None);
        assert_eq!(states["a"], "done");
        assert_eq!(states["e"], "failed");
        assert!(travelled.contains(&("a".into(), "c".into())) && travelled.contains(&("c".into(), "e".into())));
        let (states, travelled) = run_marks(&f, &[st("a", "done")], Some("c"));
        assert_eq!(states["c"], "waiting");
        assert!(travelled.contains(&("a".into(), "c".into())));
        assert_eq!(way_taken(&Step { kind: "condition".into(), detail: "→ sim".into(), ..Default::default() }).as_deref(), Some("sim"));
        assert_eq!(way_taken(&Step { kind: "approval".into(), detail: "deny".into(), ..Default::default() }).as_deref(), Some("não"));
    }

    #[test]
    fn decisions_read_back() {
        let s = Step {
            kind: "decide".into(),
            output: json!({"answers": {"rota": {"type": "choice", "choice": "fatura", "probabilities": {"agenda": 0.06, "fatura": 0.91, "ignorar": 0.03}, "confidence": 0.89}}}).to_string(),
            detail: "agenda 6%, fatura 91%, ignorar 3% · confiança 0.89 → fatura · 41 ms".into(),
            ..Default::default()
        };
        let d = parse_decision(&s, "rota").unwrap();
        assert_eq!(d.options[0], ("fatura".to_string(), 0.91));
        assert_eq!((d.confidence, d.chosen.as_str(), d.millis, d.cached), (0.89, "fatura", Some(41), false));
        let low = Step { kind: "decide".into(), output: json!({"answers": {"q": {"type": "noul", "noul": 0.6}}}).to_string(), detail: "noul 0.60 · confiança 0.20 → pouca_confianca · 37 ms (cache)".into(), ..Default::default() };
        let d2 = parse_decision(&low, "q").unwrap();
        assert!((d2.confidence - 0.2).abs() < 1e-9);
        assert_eq!((d2.chosen.as_str(), d2.millis, d2.cached), (LOW_CONFIDENCE, Some(37), true));
        let st = decision_stats(&[vec![s.clone()], vec![low.clone()], vec![s.clone()]]);
        assert_eq!((st.runs, st.alone, st.asked, st.cached), (3, 2, 1, 1));
        assert_eq!((st.median_ms, st.p95_ms), (Some(41), Some(41)));
        assert!(parse_decision(&Step { kind: "agent".into(), ..Default::default() }, "").is_none());
    }

    #[test]
    fn a_for_each_names_its_items_and_its_loop() {
        assert_eq!(item_word("var:anexos"), "anexo");
        assert_eq!(item_word("content.faturas"), "fatura");
        assert_eq!(item_word("content"), tr("item"));
        assert_eq!(item_word("var:lista.x"), tr("item"));
        assert_eq!(loop_label("anexo", None), kalango_i18n::trf("next {item}", &[("item", "anexo".into())]));
        assert!(loop_label("anexo", Some((2, 2))).ends_with(&format!("({})", kalango_i18n::trf("{i} of {n}", &[("i", "2".into()), ("n", "2".into())]))));
    }

    #[test]
    fn a_block_put_into_a_connection_pushes_what_follows() {
        let mut f = flow();
        f.nodes.push(NodeDoc { id: "z".into(), x: 0.0, y: 0.0, kind: "end".into(), data: Value::Null, ..Default::default() });
        f.edges.push(EdgeDoc { source: "d".into(), target: "a".into(), handle: None });
        let down = downstream(&f, "c", "a");
        assert!(down.contains(&"c".to_string()) && !down.contains(&"a".to_string()) && !down.contains(&"z".to_string()), "{down:?}");
        let a = Rect { x: 0.0, y: 0.0, w: 172.0, h: 90.0 };
        assert_eq!(insert_step(a, Rect { y: 186.0, ..a }, 172.0), Some((0.0, 186.0)));
        assert_eq!(insert_step(a, Rect { x: 228.0, ..a }, 172.0), Some((228.0, 0.0)));
        assert_eq!(insert_step(Rect { y: 300.0, ..a }, a, 172.0), None, "a connection going back");
    }

    #[test]
    fn service_sentences_read_in_the_persons_language() {
        assert_eq!(service_text("no connection reaches this block"), tr("no connection reaches this block"));
        let t = service_text("The model failed or was not running. Technical detail: entrada inválida: falta o campo \"pasta\".");
        assert!(t.starts_with(tr("The model failed or was not running.")) && t.ends_with("falta o campo \"pasta\"."), "{t}");
        assert_eq!(service_text("→ sim"), "→ sim");
        assert_eq!(issue_text("choose the flow; no connection reaches this block"), format!("{}; {}", tr("choose the flow"), tr("no connection reaches this block")));
    }

    #[test]
    fn blocks_without_a_name_are_called_by_what_they_do() {
        let names: HashMap<String, String> = [(tool_key("arquivos.mover"), "Move ou renomeia um arquivo ou pasta [nível usuario; escreve]".to_string()), (flow_key("arq"), "Arquivar fatura".to_string())].into();
        let n = |kind: &str, data: Value| NodeDoc { id: "x".into(), x: 0.0, y: 0.0, kind: kind.into(), data, ..Default::default() };
        assert_eq!(node_text(&n("system_action", json!({"tool": "arquivos.mover"})), &names, None).0, "Move ou renomeia um arquivo ou pasta");
        assert_eq!(node_text(&n("system_action", json!({"tool": "outra.coisa"})), &names, None).0, "outra.coisa");
        assert_eq!(tool_title("A data e a hora agora: o dia [nível observar]"), "A data e a hora agora");
        assert_eq!(node_text(&n("subflow", json!({"flow_id": "arq"})), &names, None).0, "Arquivar fatura");
        assert_eq!(node_text(&n("approval", json!({"question": ""})), &names, None).0, tr("Ask for approval"));
        assert_eq!(node_text(&n("notify", json!({"title": ""})), &names, None).0, tr("Notify"));
        assert_eq!(rule_text(&json!({"field": "var:tipo", "operator": "is_defined", "value": "fatura"})), "{tipo} ?");
    }

    #[test]
    fn undo_and_redo_go_back_and_forth_and_merge_quick_changes() {
        let f0 = flow();
        let mut h = History::new(&f0);
        let mut f1 = f0.clone();
        f1.remove_node("e");
        h.record(&f1, 10_000);
        // a word typed: three changes close together are one step
        let mut f2 = f1.clone();
        for (i, t) in ["a", "ab", "abc"].iter().enumerate() {
            f2.node_mut("a").unwrap().title = t.to_string();
            h.record(&f2, 20_000 + i as i64 * 200);
        }
        assert_eq!(h.undo(f2.clone()), Some(f1.clone()), "the whole word goes at once");
        assert_eq!(h.undo(f1.clone()), Some(f0.clone()));
        assert_eq!(h.undo(f0.clone()), None);
        assert_eq!(h.redo(f0.clone()), Some(f1.clone()));
        assert_eq!(h.redo(f1.clone()), Some(f2.clone()));
        assert_eq!(h.redo(f2.clone()), None);
        // a new change after an undo forgets what could be redone
        h.undo(f2.clone());
        let mut f3 = f1.clone();
        f3.node_mut("c").unwrap().x = 999.0;
        h.record(&f3, 20_100);
        assert_eq!(h.redo(f3.clone()), None);
        assert_eq!(h.undo(f3), Some(f1), "a change right after an undo is a step of its own");
    }

    #[test]
    fn numbers() {
        assert_eq!(duration_text(0.041), "41 ms");
        assert_eq!(duration_text(4.1), format!("{} s", decimal(4.1, 1)));
        assert_eq!(tokens_text(812), "812");
        assert_eq!(tokens_text(3100), format!("{} {}", decimal(3.1, 1), tr("k")));
        assert_eq!(tokens_text(38000), format!("38 {}", tr("k")));
        assert!(palette_matches("Decide", "Decidir", "deci"));
        assert!(palette_matches("Transcribe audio", "Transcrever áudio", "audio"));
        assert!(!palette_matches("Agent", "Agente", "zzz"));
    }

    fn chain(ids: &[&str]) -> FlowDoc {
        let mut f = FlowDoc::default();
        for (i, id) in ids.iter().enumerate() {
            f.nodes.push(NodeDoc { id: id.to_string(), x: 0.0, y: i as f64 * 100.0, kind: "message".into(), ..Default::default() });
        }
        for w in ids.windows(2) {
            f.edges.push(EdgeDoc { source: w[0].into(), target: w[1].into(), handle: None });
        }
        f
    }

    #[test]
    fn a_block_keeps_its_switch_and_its_sandbox_when_saved() {
        let v = json!({"id": "a", "position": {"x": 1, "y": 2}, "type": "end", "disabled": true, "parent": "s", "title": "Fim"});
        let n = NodeDoc::from_json(&v).unwrap();
        assert!(n.disabled);
        assert_eq!(n.parent.as_deref(), Some("s"));
        let back = n.to_json();
        assert_eq!(back["disabled"], json!(true));
        assert_eq!(back["parent"], json!("s"));
        assert_eq!(back["title"], json!("Fim"));
        let on = NodeDoc { disabled: false, parent: None, ..n }.to_json();
        assert!(on.get("disabled").is_none() && on.get("parent").is_none());
    }

    #[test]
    fn deleting_a_block_in_the_middle_connects_the_one_before_to_the_next() {
        let mut f = chain(&["a", "b", "c"]);
        f.edges[0].handle = Some("sim".into());
        assert!(f.remove_node_reconnecting("b"));
        assert_eq!(f.edges, vec![EdgeDoc { source: "a".into(), target: "c".into(), handle: Some("sim".into()) }]);
        // the last block has nothing after it: nothing to connect
        let mut g = chain(&["a", "b"]);
        assert!(!g.remove_node_reconnecting("b"));
        assert!(g.edges.is_empty());
        // a block that splits in two: no single block after it
        let mut h = chain(&["a", "b", "c"]);
        h.nodes.push(NodeDoc { id: "d".into(), kind: "end".into(), ..Default::default() });
        h.edges.push(EdgeDoc { source: "b".into(), target: "d".into(), handle: Some("x".into()) });
        assert!(!h.remove_node_reconnecting("b"));
        assert!(h.edges.is_empty());
    }

    #[test]
    fn a_sandbox_holds_its_blocks_and_only_it_connects_outside() {
        let mut f = chain(&["a", "b", "c"]);
        f.nodes.push(NodeDoc { id: "s".into(), kind: "sandbox".into(), data: default_data("sandbox", ""), ..Default::default() });
        f.set_parent("b", Some("s"));
        // (the connections a → b and b → c crossed its edge)
        assert!(f.edges.is_empty());
        assert!(!f.may_connect("a", "b"));
        assert!(f.may_connect("a", "s"));
        f.fix_sandboxes();
        assert_eq!(f.node("s").unwrap().data["entry"], json!("b"));
        // the entry is never a block inside
        assert_eq!(f.entry().map(|n| n.id.as_str()), Some("a"));
        // a sandbox never goes inside another block
        f.set_parent("s", Some("s"));
        assert!(f.node("s").unwrap().parent.is_none());
        // deleting the sandbox deletes what is inside it
        f.remove_node("s");
        assert!(f.node("b").is_none());
    }

    #[test]
    fn copies_get_new_ids_and_keep_their_connections() {
        let mut f = chain(&["a", "b", "c"]);
        let (nodes, edges) = f.copy_nodes(&["a".into(), "b".into()]);
        assert_eq!(edges.len(), 1);
        let new = f.paste_nodes(&nodes, &edges, 40.0, 40.0);
        assert_eq!(new.len(), 2);
        assert!(new.iter().all(|id| !["a", "b", "c"].contains(&id.as_str())));
        assert!(f.edges.iter().any(|e| e.source == new[0] && e.target == new[1]));
        assert_eq!(f.node(&new[0]).unwrap().x, 40.0);
    }

    #[test]
    fn organising_puts_each_block_one_row_below_what_leads_to_it() {
        let mut f = chain(&["a", "b", "c"]);
        f.nodes.push(NodeDoc { id: "d".into(), x: 900.0, y: -50.0, kind: "end".into(), ..Default::default() });
        f.edges.push(EdgeDoc { source: "a".into(), target: "d".into(), handle: Some("x".into()) });
        for n in f.nodes.iter_mut() {
            n.x += 333.0;
        }
        f.arrange(&HashMap::new());
        let (a, b, c, d) = (f.node("a").unwrap(), f.node("b").unwrap(), f.node("c").unwrap(), f.node("d").unwrap());
        assert!(b.y > a.y && c.y > b.y);
        assert_eq!(b.y, d.y);
        assert!((d.x - b.x).abs() > 1.0);
        // the row is centred under the block above it
        assert!(((b.x + d.x + 172.0) / 2.0 - (a.x + 86.0)).abs() < 1.0);
    }

    #[test]
    fn several_ways_at_once_without_a_join_warn() {
        let mut f = chain(&["p", "b"]);
        f.nodes[0].kind = "parallel".into();
        f.nodes.push(NodeDoc { id: "c".into(), kind: "message".into(), ..Default::default() });
        f.edges.push(EdgeDoc { source: "p".into(), target: "c".into(), handle: None });
        assert!(f.needs_join("p"));
        f.nodes.push(NodeDoc { id: "j".into(), kind: "join".into(), ..Default::default() });
        f.edges.push(EdgeDoc { source: "b".into(), target: "j".into(), handle: None });
        assert!(!f.needs_join("p"));
        // a condition takes one way only
        assert!(!chain(&["a", "b"]).needs_join("a"));
    }

    #[test]
    fn retry_boxes_read_their_numbers() {
        assert_eq!(retry_from("", "", ""), None);
        assert_eq!(retry_from("2", "3 s", "60 s"), Some(json!({"max_attempts": 2, "initial_delay_s": 3.0, "timeout_s": 60})));
        assert_eq!(retry_from("", "1,5 s", ""), Some(json!({"max_attempts": 3, "initial_delay_s": 1.5})));
    }

    #[test]
    fn crossings_read_and_write_one_per_line() {
        let v = parse_crossings("{arquivo} → /entrada/\n/saida/* -> ~/Downloads/\nsem seta");
        assert_eq!(v, json!([{"from": "{arquivo}", "to": "/entrada/"}, {"from": "/saida/*", "to": "~/Downloads/"}]));
        assert_eq!(crossings_text(&v), "{arquivo} → /entrada/\n/saida/* → ~/Downloads/");
    }

    #[test]
    fn a_test_run_numbers_its_steps_and_names_its_path() {
        let st = |n: &str| Step { node: n.into(), ..Default::default() };
        let steps = vec![st("a"), st("b"), st("b"), st("c")];
        let no = step_numbers(&steps);
        assert_eq!((no["a"], no["b"], no["c"]), (1, 2, 3));
        assert_eq!(path_text(&steps, |id| id.to_uppercase()), "A → B → C");
    }

    #[test]
    fn a_for_each_has_a_way_for_an_empty_list() {
        let n = NodeDoc { id: "f".into(), kind: "for_each".into(), ..Default::default() };
        assert_eq!(ways(&n), vec![None, Some(EMPTY_LIST.to_string())]);
        assert!(!way_label("for_each", Some(EMPTY_LIST)).is_empty());
    }

    #[test]
    fn sandbox_words_come_from_its_settings_and_its_run() {
        let c = sandbox_caption(&json!({"network": "nenhuma", "on_end": "apagar"}), "Ubuntu base");
        assert!(c.contains("Ubuntu base"));
        assert!(sandbox_note(&json!({"machine": "m1", "state": "ready", "millis": 800})).contains("m1"));
        assert_eq!(machine_list(&json!({"maquinas": [{"id": "x", "nome": "Alpine"}, {"id": "y"}]})), vec![("x".into(), "Alpine".into()), ("y".into(), "y".into())]);
    }

    #[test]
    fn the_variables_a_flow_has() {
        let mut f = chain(&["a"]);
        f.nodes[0].data = json!({"save_as": "classe"});
        f.nodes.push(NodeDoc { id: "v".into(), kind: "set_variables".into(), data: json!({"variables": [{"name": "anexos", "op": "set"}]}), ..Default::default() });
        assert_eq!(f.variable_names(), vec!["classe".to_string(), "anexos".to_string()]);
    }
}
