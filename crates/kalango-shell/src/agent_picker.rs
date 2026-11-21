//! Calling the person's own agents from the chat (designs "Chat · escolher agente" and "Chat · conversa com um
//! agente seu"): the picker (the KalangoOS agent, then the agents of kalango-fluxos, recent first, with a search),
//! "@nome" in the composer, the structured input an agent declares (typed and checked before it is sent), the
//! agent's icon and how a conversation with one is labelled in the history.
//!
//! The pure parts (mentions, typing the input, recency, icons, labels) are functions with tests; the widgets come
//! after them.

use crate::conversa::ui_icon;
use crate::fluxos_ui as ui;
use crate::superficies::{caixa, estrela, rotulo};
use gtk::prelude::*;
use gtk::{gdk, glib, Align, Orientation};
use kalango_i18n::{tr, trf};
use serde_json::{json, Map, Value};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// The name the system's own agent goes by in the picker and the chips.
pub const SYSTEM_AGENT: &str = "KalangoOS";

// ------------------------------------------------------------------------------------------------ mentions

/// The "@query" being typed at the end of the text (the picker opens on it): the text after the last "@" that starts
/// a word, while it is short and on one line. None when there is none.
pub fn mention_query(text: &str) -> Option<String> {
    let at = text.rfind('@')?;
    if at > 0 && !text[..at].ends_with(char::is_whitespace) {
        return None;
    }
    let q = &text[at + 1..];
    if q.contains('\n') || q.chars().count() > 40 || q.split_whitespace().count() > 4 || q.ends_with("  ") {
        return None;
    }
    Some(q.to_string())
}

/// The agent a message calls with "@Name" (the longest name that follows an "@" at the start of a word, accents and
/// case aside) and the message without the mention. `agents` are (id, name).
pub fn resolve_mention(text: &str, agents: &[(String, String)]) -> Option<(String, String)> {
    let mut best: Option<(usize, usize, &String)> = None; // (start of "@", end of the name, id)
    for (at, _) in text.match_indices('@') {
        if at > 0 && !text[..at].ends_with(char::is_whitespace) {
            continue;
        }
        let rest = &text[at + 1..];
        let folded = ui::fold(rest);
        for (id, name) in agents {
            let n = ui::fold(name);
            if n.is_empty() || !folded.starts_with(&n) {
                continue;
            }
            // the name's length in the original text (folding keeps one char per char)
            let len: usize = rest.chars().take(n.chars().count()).map(char::len_utf8).sum();
            let next = rest[len..].chars().next();
            if next.is_some_and(|c| c.is_alphanumeric()) {
                continue;
            }
            if best.is_none_or(|(_, e, _)| at + 1 + len - at > e - at) {
                best = Some((at, at + 1 + len, id));
            }
        }
        if best.is_some() {
            break;
        }
    }
    let (start, end, id) = best?;
    let rest = format!("{}{}", &text[..start], text[end..].trim_start_matches([',', ':']));
    Some((id.clone(), rest.split_whitespace().collect::<Vec<_>>().join(" ")))
}

/// Takes the "@query" off the end of the text (the agent was chosen in the picker).
pub fn strip_mention_query(text: &str) -> String {
    match (mention_query(text), text.rfind('@')) {
        (Some(_), Some(at)) => text[..at].trim_end().to_string(),
        _ => text.to_string(),
    }
}

// ------------------------------------------------------------------------------------------------ input fields

/// One field of an agent's structured input or output (kalango-fluxos `fields::Field`).
#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub name: String,
    /// "text", "number", "integer", "boolean", "list" or "object"
    pub kind: String,
    pub required: bool,
    pub description: String,
}

pub fn fields(v: &Value) -> Vec<Field> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|f| {
            Some(Field {
                name: f["name"].as_str()?.to_string(),
                kind: f["kind"].as_str().unwrap_or("text").to_string(),
                required: f["required"].as_bool().unwrap_or(false),
                description: f["description"].as_str().unwrap_or("").to_string(),
            })
        })
        .collect()
}

/// What the person typed in a field: a text, or a switch.
#[derive(Clone, Debug, PartialEq)]
pub enum Raw {
    Text(String),
    Switch(bool),
}

/// Why a field's value can't go: it is missing, or not of its kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldProblem {
    pub field: String,
    /// "missing" or "wrong_type"
    pub problem: &'static str,
    pub kind: String,
}

/// A typed value of one field: None when empty.
pub fn typed_value(kind: &str, raw: &Raw) -> Result<Option<Value>, &'static str> {
    let text = match raw {
        Raw::Switch(b) => return Ok(Some(json!(b))),
        Raw::Text(t) => t.trim(),
    };
    if text.is_empty() {
        return Ok(None);
    }
    match kind {
        "number" => text.replace(',', ".").parse::<f64>().ok().filter(|f| f.is_finite()).map(|f| Some(json!(f))).ok_or("wrong_type"),
        "integer" => text.parse::<i64>().map(|i| Some(json!(i))).map_err(|_| "wrong_type"),
        "boolean" => match ui::fold(text).as_str() {
            "sim" | "s" | "yes" | "y" | "true" | "1" => Ok(Some(json!(true))),
            "nao" | "n" | "no" | "false" | "0" => Ok(Some(json!(false))),
            _ => Err("wrong_type"),
        },
        "list" => {
            if text.starts_with('[') {
                return serde_json::from_str::<Value>(text).ok().filter(Value::is_array).map(Some).ok_or("wrong_type");
            }
            Ok(Some(json!(text.split([',', ';']).map(str::trim).filter(|s| !s.is_empty()).collect::<Vec<_>>())))
        }
        "object" => serde_json::from_str::<Value>(text).ok().filter(Value::is_object).map(Some).ok_or("wrong_type"),
        _ => Ok(Some(json!(text))),
    }
}

/// The input object of these fields from what was typed (by field name), or every problem found.
pub fn build_input(fields: &[Field], raws: &[(String, Raw)]) -> Result<Value, Vec<FieldProblem>> {
    let mut out = Map::new();
    let mut problems = Vec::new();
    for f in fields {
        let raw = raws.iter().find(|(n, _)| *n == f.name).map(|(_, r)| r.clone()).unwrap_or(Raw::Text(String::new()));
        match typed_value(&f.kind, &raw) {
            Ok(Some(v)) => {
                out.insert(f.name.clone(), v);
            }
            Ok(None) if f.required => problems.push(FieldProblem { field: f.name.clone(), problem: "missing", kind: f.kind.clone() }),
            Ok(None) => {}
            Err(p) => problems.push(FieldProblem { field: f.name.clone(), problem: p, kind: f.kind.clone() }),
        }
    }
    if problems.is_empty() { Ok(Value::Object(out)) } else { Err(problems) }
}

/// A kind as the person reads it ("texto", "inteiro"…).
pub fn kind_name(kind: &str) -> &'static str {
    match kind {
        "number" => tr("number"),
        "integer" => tr("integer"),
        "boolean" => tr("yes or no"),
        "list" => tr("list"),
        "object" => tr("object"),
        _ => tr("text"),
    }
}

/// The sentence for a field's problem.
pub fn problem_text(p: &FieldProblem) -> String {
    if p.problem == "missing" {
        trf("Fill in “{field}”.", &[("field", p.field.clone())])
    } else {
        trf("“{field}” must be {kind}.", &[("field", p.field.clone()), ("kind", kind_name(&p.kind).to_string())])
    }
}

/// A value as one line ("Imagens, Documentos", "sim", "30").
pub fn value_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Bool(b) => if *b { tr("yes") } else { tr("no") }.to_string(),
        Value::Array(a) => a.iter().map(value_text).collect::<Vec<_>>().join(", "),
        Value::Null => String::new(),
        Value::Number(n) => n.as_f64().filter(|f| f.fract() == 0.0 && f.abs() < 1e15).map(|f| format!("{}", f as i64)).unwrap_or_else(|| n.to_string()),
        o => o.to_string(),
    }
}

/// The input as the person's message shows it: "pasta: ~/Downloads · dias: 30 · apagar: sim".
pub fn input_summary(fields: &[Field], input: &Value) -> String {
    fields.iter().filter_map(|f| input.get(&f.name).map(|v| format!("{}: {}", f.name, value_text(v)))).collect::<Vec<_>>().join(" · ")
}

/// The kind of an output value, from its field or from the value ("inteiro", "lista", "texto"…).
pub fn output_kind(field: Option<&Field>, v: &Value) -> &'static str {
    if let Some(f) = field {
        return kind_name(&f.kind);
    }
    match v {
        Value::Bool(_) => kind_name("boolean"),
        Value::Number(n) if n.is_i64() || n.is_u64() => kind_name("integer"),
        Value::Number(_) => kind_name("number"),
        Value::Array(_) => kind_name("list"),
        Value::Object(_) => kind_name("object"),
        _ => kind_name("text"),
    }
}

/// The rows of a structured output: (name, kind, value), in the order of the agent's output fields, then the rest.
pub fn output_rows(fields: &[Field], output: &Value) -> Vec<(String, &'static str, String)> {
    let Some(o) = output.as_object() else { return vec![] };
    let mut rows: Vec<(String, &'static str, String)> = fields.iter().filter_map(|f| o.get(&f.name).map(|v| (f.name.clone(), output_kind(Some(f), v), value_text(v)))).collect();
    for (k, v) in o {
        if !fields.iter().any(|f| f.name == *k) {
            rows.push((k.clone(), output_kind(None, v), value_text(v)));
        }
    }
    rows
}

// ------------------------------------------------------------------------------------------------ the agents

/// What a conversation keeps of the agent it talks to (its meta's "agent").
pub fn agent_ref(agent: &Value) -> Value {
    json!({"id": agent["id"], "name": agent["name"], "kind": agent["kind"], "icon": icon_spec(agent)})
}

/// Whether the agent runs as a flow (it is run, not chatted with).
pub fn is_flow(agent: &Value) -> bool {
    agent["kind"].as_str() == Some("flow")
}

/// The history's line under a conversation's title: the agent it talks to, or None for the system's own.
pub fn history_label(meta: &Value) -> Option<String> {
    meta["agent"]["name"].as_str().filter(|n| !n.is_empty()).map(str::to_string)
}

/// The agents, the ones used last first (by the conversations' last update), the others in the service's order.
pub fn order_recent(agents: Vec<Value>, conversations: &[(String, Value)]) -> Vec<Value> {
    let last = |id: &str| conversations.iter().filter(|(_, m)| m["agent"]["id"].as_str() == Some(id)).filter_map(|(_, m)| m["atualizado"].as_i64()).max().unwrap_or(0);
    let mut v: Vec<(i64, usize, Value)> = agents.into_iter().enumerate().map(|(i, a)| (last(a["id"].as_str().unwrap_or("")), i, a)).collect();
    v.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    v.into_iter().map(|(_, _, a)| a).collect()
}

/// The line under an agent's name in the picker: its kind and what it asks for or holds.
pub fn agent_subtitle(agent: &Value) -> String {
    let kind = ui::kind_of(agent);
    let c = &agent["config"];
    let more = match kind {
        "tasks" => {
            let n = c["tasks"].as_array().map(|t| t.iter().filter_map(|x| x["agent"].as_str()).collect::<std::collections::BTreeSet<_>>().len()).unwrap_or(0);
            (n > 0).then(|| if n == 1 { tr("1 assistant").to_string() } else { trf("{n} assistants", &[("n", n.to_string())]) })
        }
        "flow" => Some(tr("opens the run").to_string()),
        _ => {
            // (the fields it needs; the optional ones only when it has nothing else)
            let all = fields(&agent["input_fields"]);
            let needed: Vec<String> = all.iter().filter(|f| f.required).map(|f| f.name.clone()).collect();
            let names = if needed.is_empty() { all.iter().map(|f| f.name.clone()).collect() } else { needed };
            (!names.is_empty()).then(|| trf("asks for {fields}", &[("fields", join_and(&names))]))
        }
    };
    match more {
        Some(m) => format!("{} · {m}", ui::kind_label(kind)),
        None => ui::kind_label(kind).to_string(),
    }
}

/// "a", "a e b", "a, b e c".
fn join_and(v: &[String]) -> String {
    match v {
        [] => String::new(),
        [a] => a.clone(),
        [rest @ .., last] => format!("{} {} {last}", rest.join(", "), tr("and")),
    }
}

// ------------------------------------------------------------------------------------------------ icons

/// An agent's icon: one of the shell's glyphs on a tile of a token's tone, or an SVG the person imported.
#[derive(Clone, Debug, PartialEq)]
pub enum AgentIcon {
    Glyph { glyph: String, tone: &'static str },
    Svg { svg: String, monochrome: bool, tone: &'static str },
}

/// A token colour of kalango-fluxos (`icon::COLORS`) as the library's tone classes.
pub fn tone_of(color: &str) -> &'static str {
    match color {
        "teal" => "teal",
        "blue" => "azul",
        "gold" => "ouro",
        "coral" => "coral",
        "purple" => "lilas",
        "grey" => "cinza",
        _ => "acento",
    }
}

/// The shell's glyph for an icon name of kalango-fluxos (its names are English; a shell name passes as is).
pub fn glyph_of(name: &str) -> String {
    let g = match name {
        "search" => "busca",
        "folder" => "pasta",
        "image" => "imagem",
        "send" => "enviar",
        "code" => "codigo",
        "clock" => "relogio",
        "shield" => "escudo",
        "play" => "tocar",
        "bell" => "sino",
        "list" => "lista",
        "apps" => "grade",
        "ring" => "anel",
        "network" => "rede-cabo",
        "bolt" => "raio",
        "" => "spark",
        n => n,
    };
    if g == "spark" || crate::conversa::has_icon(g) { g.to_string() } else { "spark".to_string() }
}

/// The icon an agent gets when the person did not choose one (the service's `icon::default_icon`): from words in its
/// name and description, then from its kind. Returns (name, colour) in the service's terms.
pub fn default_icon(kind: &str, name: &str, description: &str) -> (&'static str, &'static str) {
    let text = format!("{name} {description}").to_lowercase();
    let by_words: [(&[&str], &str, &str); 12] = [
        (&["busca", "pesquis", "procura", "search"], "search", "blue"),
        (&["arquivo", "pasta", "download", "organiz"], "folder", "gold"),
        (&["foto", "imagem", "image"], "image", "teal"),
        (&["e-mail", "email", "correio", "caixa"], "send", "coral"),
        (&["texto", "revis", "resum", "redat", "escrev", "traduz"], "doc", "teal"),
        (&["código", "codigo", "teste", "program", "corrig"], "code", "purple"),
        (&["agenda", "plano", "lembrete", "calend"], "clock", "gold"),
        (&["backup", "cópia", "copia"], "download", "teal"),
        (&["segur", "senha", "proteg"], "shield", "coral"),
        (&["sistema", "disco", "memória", "cpu"], "cpu", "blue"),
        (&["jogo", "game"], "play", "purple"),
        (&["notific", "aviso"], "bell", "gold"),
    ];
    for (words, icon, color) in by_words {
        if words.iter().any(|w| text.contains(w)) {
            return (icon, color);
        }
    }
    match kind {
        "sequential" | "tasks" => ("list", "teal"),
        "parallel" => ("apps", "blue"),
        "loop" => ("ring", "purple"),
        "remote" => ("network", "blue"),
        "flow" => ("bolt", "gold"),
        _ => ("spark", "accent"),
    }
}

/// The agent's "icon" as kept (its own, or the default by its kind and words).
pub fn icon_spec(agent: &Value) -> Value {
    let i = &agent["icon"];
    if i.is_object() && (i["svg"].is_string() || i["name"].as_str().is_some_and(|n| !n.is_empty())) {
        return i.clone();
    }
    let (name, color) = default_icon(ui::kind_of(agent), agent["name"].as_str().unwrap_or(""), agent["description"].as_str().unwrap_or(""));
    json!({"name": name, "color": color})
}

pub fn agent_icon(agent: &Value) -> AgentIcon {
    let i = icon_spec(agent);
    let tone = tone_of(i["color"].as_str().unwrap_or("accent"));
    match i["svg"].as_str() {
        Some(svg) => AgentIcon::Svg { svg: svg.to_string(), monochrome: i["monochrome"].as_bool().unwrap_or(false), tone },
        None => AgentIcon::Glyph { glyph: glyph_of(i["name"].as_str().unwrap_or("")), tone },
    }
}

/// The agent's icon on its tile (`side` px), as the library's cards draw it.
pub fn icon_tile(agent: &Value, side: i32) -> gtk::Box {
    let px = if side >= 40 { 18 } else if side >= 26 { 15 } else { 12 };
    match agent_icon(agent) {
        AgentIcon::Glyph { glyph, tone } => {
            let t = ui::icon_tile(&glyph, tone, side);
            if let Some(g) = t.first_child() {
                // (the tile's own glyph size is for 32 px and more)
                if let Ok(img) = g.clone().downcast::<gtk::Image>() {
                    img.set_pixel_size(px);
                    img.set_size_request(px, px);
                }
            }
            t.set_valign(Align::Center);
            t
        }
        AgentIcon::Svg { svg, monochrome, tone } => {
            let t = caixa(Orientation::Horizontal, 0, "fx-icone");
            t.add_css_class(tone);
            t.set_size_request(side, side);
            t.set_hexpand(false);
            t.set_valign(Align::Center);
            let paint = move |svg: &str| {
                let s = if monochrome { crate::conversa::recolor_svg(svg, &crate::tema::cor(ui::tone_color(tone))) } else { svg.to_string() };
                gdk::Texture::from_bytes(&glib::Bytes::from_owned(s.into_bytes())).ok()
            };
            let img = gtk::Image::from_paintable(paint(&svg).as_ref());
            img.set_pixel_size(px);
            img.set_size_request(px, px);
            img.set_hexpand(true);
            img.set_halign(Align::Center);
            img.set_valign(Align::Center);
            if monochrome {
                crate::tema::registrar(&img, move |w| {
                    if let Some(i) = w.downcast_ref::<gtk::Image>() {
                        i.set_paintable(paint(&svg).as_ref());
                    }
                });
            }
            t.append(&img);
            t
        }
    }
}

/// The system agent's tile (the star).
pub fn system_tile(side: i32) -> gtk::Box {
    let t = caixa(Orientation::Horizontal, 0, "fx-icone");
    t.set_size_request(side, side);
    t.set_hexpand(false);
    t.set_valign(Align::Center);
    let s = estrela(if side >= 26 { 13 } else { 11 }, ui::ACCENT);
    s.set_hexpand(true);
    s.set_halign(Align::Center);
    s.set_valign(Align::Center);
    t.append(&s);
    t
}

/// The tile of an agent, or of the system's own (None).
pub fn tile_for(agent: Option<&Value>, side: i32) -> gtk::Box {
    match agent {
        Some(a) => icon_tile(a, side),
        None => system_tile(side),
    }
}

// ------------------------------------------------------------------------------------------------ the picker

/// What the picker hands back.
pub enum Picked {
    /// the system's agent (None) or one of the person's
    Agent(Option<Value>),
    /// a flow: run it
    Run(Value),
    Create,
    Manage,
}

type OnPicked = Rc<dyn Fn(Picked)>;

// The agents of kalango-fluxos, kept for a moment (every picker reads the same list).
thread_local! {
    static AGENTS: RefCell<(i64, Vec<Value>)> = const { RefCell::new((0, Vec::new())) };
}

fn now() -> i64 {
    glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0)
}

/// The person's agents (enabled ones), from the service; `done` gets them recent first. A list read in the last
/// 20 s is reused.
pub fn load_agents(done: impl FnOnce(Result<Vec<Value>, String>) + 'static) {
    let fresh = AGENTS.with(|a| {
        let a = a.borrow();
        (now() - a.0 < 20 && !a.1.is_empty()).then(|| a.1.clone())
    });
    if let Some(v) = fresh {
        return done(Ok(order_recent(v, &crate::conversa::conversas())));
    }
    crate::fluxos::call("agents.list", json!({}), move |r| match r {
        Ok(v) => {
            let list: Vec<Value> = v.as_array().into_iter().flatten().map(|x| x["agent"].clone()).filter(|a| a["enabled"].as_bool() != Some(false) && a["id"].is_string()).collect();
            AGENTS.with(|a| *a.borrow_mut() = (now(), list.clone()));
            done(Ok(order_recent(list, &crate::conversa::conversas())));
        }
        Err(e) => done(Err(e)),
    });
}

/// The agents read last (no call): for "@nome" at send time.
pub fn cached_agents() -> Vec<Value> {
    AGENTS.with(|a| a.borrow().1.clone())
}

/// The picker (design "Chat · escolher agente"): a search, the system's agent, the person's agents recent first
/// (an "entrada" mark on those that ask for fields, "Executar" on flows), "Criar agente" and "Gerenciar agentes".
pub struct Picker {
    pub popover: gtk::Popover,
    search: gtk::Entry,
    list: gtk::Box,
    agents: RefCell<Vec<Value>>,
    current: RefCell<Option<String>>,
    on_picked: OnPicked,
    /// the first row shown (Enter picks it)
    first: RefCell<Option<Picked>>,
    /// the search field is filled from code ("@query"): no reload
    filling: Cell<bool>,
}

impl Picker {
    pub fn new(on_picked: impl Fn(Picked) + 'static) -> Rc<Picker> {
        let popover = gtk::Popover::new();
        popover.add_css_class("agentes-escolha");
        popover.set_has_arrow(false);
        popover.set_autohide(true);
        let b = caixa(Orientation::Vertical, 0, "");
        b.set_size_request(368, -1);
        let (sb, search) = ui::search(tr("Search agents…"));
        b.append(&sb);
        let list = caixa(Orientation::Vertical, 2, "");
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroll.set_propagate_natural_height(true);
        scroll.set_max_content_height(420);
        scroll.set_child(Some(&list));
        b.append(&scroll);
        let foot = caixa(Orientation::Horizontal, 8, "agentes-escolha-rodape");
        let create = gtk::Button::new();
        create.add_css_class("agentes-escolha-criar");
        let cc = caixa(Orientation::Horizontal, 6, "");
        cc.append(&ui_icon("mais", ui::ACCENT, 13));
        cc.append(&rotulo(tr("Create agent"), ""));
        create.set_child(Some(&cc));
        foot.append(&create);
        let sp = caixa(Orientation::Horizontal, 0, "");
        sp.set_hexpand(true);
        foot.append(&sp);
        let manage = gtk::Button::new();
        manage.set_child(Some(&rotulo(tr("Manage agents"), "")));
        manage.add_css_class("agentes-escolha-gerenciar");
        foot.append(&manage);
        b.append(&foot);
        popover.set_child(Some(&b));
        let p = Rc::new(Picker { popover, search, list, agents: RefCell::default(), current: RefCell::default(), on_picked: Rc::new(on_picked), first: RefCell::default(), filling: Cell::new(false) });
        let w = Rc::downgrade(&p);
        p.search.connect_changed(move |_| {
            if let Some(p) = w.upgrade() {
                p.render();
            }
        });
        let w = Rc::downgrade(&p);
        p.search.connect_activate(move |_| {
            let Some(p) = w.upgrade() else { return };
            let x = p.first.borrow_mut().take();
            if let Some(x) = x {
                p.pick(x);
            }
        });
        for (btn, what) in [(create, 0), (manage, 1)] {
            let w = Rc::downgrade(&p);
            btn.connect_clicked(move |_| {
                if let Some(p) = w.upgrade() {
                    p.pick(if what == 0 { Picked::Create } else { Picked::Manage });
                }
            });
        }
        p
    }

    fn pick(&self, x: Picked) {
        self.popover.popdown();
        (self.on_picked)(x);
    }

    /// Opens under `anchor` with the agent in use marked (None = the system's) and the search filled.
    pub fn open(self: &Rc<Self>, anchor: &impl IsA<gtk::Widget>, current: Option<&str>, query: &str, focus_search: bool) {
        if self.popover.parent().as_ref() != Some(anchor.upcast_ref()) {
            if self.popover.parent().is_some() {
                self.popover.unparent();
            }
            self.popover.set_parent(anchor);
        }
        *self.current.borrow_mut() = current.map(str::to_string);
        self.filling.set(true);
        self.search.set_text(query);
        self.filling.set(false);
        self.render();
        self.popover.set_can_focus(focus_search);
        self.popover.popup();
        if focus_search {
            self.search.grab_focus();
        }
        let w = Rc::downgrade(self);
        load_agents(move |r| {
            let Some(p) = w.upgrade() else { return };
            match r {
                Ok(v) => *p.agents.borrow_mut() = v,
                Err(e) => {
                    p.agents.borrow_mut().clear();
                    if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                        eprintln!("debug conversa: agentes indisponíveis ({e})");
                    }
                }
            }
            p.render();
        });
    }

    /// The search follows "@query" while the picker is open.
    pub fn set_query(&self, q: &str) {
        if self.search.text() != q {
            self.search.set_text(q);
        }
    }

    pub fn is_open(&self) -> bool {
        self.popover.is_visible()
    }

    pub fn close(&self) {
        self.popover.popdown();
    }

    /// Whether the search finds anything (with "@query" a picker with nothing to show closes).
    pub fn has_matches(&self, q: &str) -> bool {
        let mine = self.agents.borrow();
        // (before its first opening the picker knows the list read last)
        let all = if mine.is_empty() { cached_agents() } else { mine.clone() };
        ui::matches_query(q, &[SYSTEM_AGENT]) || all.iter().any(|a| ui::matches_query(q, &[a["name"].as_str().unwrap_or("")]))
    }

    /// Picks the first row (Enter on "@query" in the composer).
    pub fn pick_first(&self) -> bool {
        let x = self.first.borrow_mut().take();
        match x {
            Some(x) => {
                self.pick(x);
                true
            }
            None => false,
        }
    }

    fn render(self: &Rc<Self>) {
        ui::clear_box(&self.list);
        let q = self.search.text().to_string();
        let current = self.current.borrow().clone();
        *self.first.borrow_mut() = None;
        if ui::matches_query(&q, &[SYSTEM_AGENT, tr("the system agent")]) {
            self.list.append(&section(tr("SYSTEM")));
            let r = self.row(None, current.is_none());
            self.list.append(&r);
            *self.first.borrow_mut() = Some(Picked::Agent(None));
        }
        let agents: Vec<Value> = self.agents.borrow().iter().filter(|a| ui::matches_query(&q, &[a["name"].as_str().unwrap_or(""), a["description"].as_str().unwrap_or("")])).cloned().collect();
        if !agents.is_empty() {
            let recent = crate::conversa::conversas().iter().any(|(_, m)| m["agent"]["id"].is_string());
            self.list.append(&section(if recent { tr("MY AGENTS · USED RECENTLY") } else { tr("MY AGENTS") }));
        }
        for a in &agents {
            let chosen = current.as_deref() == a["id"].as_str();
            self.list.append(&self.row(Some(a), chosen));
            let mut f = self.first.borrow_mut();
            if f.is_none() || (!q.is_empty() && matches!(*f, Some(Picked::Agent(None))) && !ui::matches_query(&q, &[SYSTEM_AGENT])) {
                *f = Some(if is_flow(a) { Picked::Run(a.clone()) } else { Picked::Agent(Some(a.clone())) });
            }
        }
        if self.list.first_child().is_none() {
            let l = ui::wrapped(if self.agents.borrow().is_empty() && q.is_empty() { tr("No agent of yours yet.") } else { tr("No agent matches.") }, "central-agente-sub");
            l.set_margin_top(10);
            l.set_margin_bottom(10);
            l.set_margin_start(8);
            self.list.append(&l);
        }
    }

    fn row(self: &Rc<Self>, agent: Option<&Value>, chosen: bool) -> gtk::Button {
        let b = gtk::Button::new();
        b.add_css_class("agentes-escolha-item");
        if chosen {
            b.add_css_class("ativo");
        }
        let r = caixa(Orientation::Horizontal, 10, "");
        r.append(&tile_for(agent, 26));
        let t = caixa(Orientation::Vertical, 1, "");
        t.set_hexpand(true);
        t.set_valign(Align::Center);
        let (name, sub) = match agent {
            Some(a) => (a["name"].as_str().unwrap_or("").to_string(), agent_subtitle(a)),
            None => (SYSTEM_AGENT.to_string(), format!("{} · {}", tr("the system agent"), tr("default"))),
        };
        t.append(&ui::cut(&name, "agentes-escolha-nome"));
        t.append(&ui::cut(&sub, "agentes-escolha-sub"));
        r.append(&t);
        if chosen {
            r.append(&ui_icon("check", ui::ACCENT, 14));
        } else if let Some(a) = agent {
            if is_flow(a) {
                r.append(&ui::chip(tr("Run"), ""));
            } else if !fields(&a["input_fields"]).is_empty() {
                r.append(&ui::chip(tr("input"), ""));
            }
        }
        b.set_child(Some(&r));
        let pick = match agent {
            Some(a) if is_flow(a) => Picked::Run(a.clone()),
            Some(a) => Picked::Agent(Some(a.clone())),
            None => Picked::Agent(None),
        };
        let cell = RefCell::new(Some(pick));
        let w = Rc::downgrade(self);
        let pick = Rc::new(cell);
        b.connect_clicked(move |_| {
            let Some(p) = w.upgrade() else { return };
            let x = pick.borrow_mut().take();
            if let Some(x) = x {
                // (a row is drawn again on the next open: the choice is taken once)
                let again = match &x {
                    Picked::Agent(a) => Picked::Agent(a.clone()),
                    Picked::Run(a) => Picked::Run(a.clone()),
                    Picked::Create => Picked::Create,
                    Picked::Manage => Picked::Manage,
                };
                *pick.borrow_mut() = Some(again);
                p.pick(x);
            }
        });
        if let Some(a) = agent {
            crate::central::posicao_debug(&format!("escolha agente {}", a["name"].as_str().unwrap_or("")), b.upcast_ref());
        } else {
            crate::central::posicao_debug("escolha agente KalangoOS", b.upcast_ref());
        }
        b
    }
}

fn section(t: &str) -> gtk::Label {
    let l = ui::section(t);
    l.add_css_class("agentes-escolha-secao");
    l
}

// ------------------------------------------------------------------------------------------------ the input form

/// The composer's fields for an agent's structured input (design: compact labelled entries and switches).
pub struct InputForm {
    pub root: gtk::Box,
    fields: RefCell<Vec<(Field, gtk::Widget)>>,
}

impl InputForm {
    pub fn new() -> Rc<InputForm> {
        let root = caixa(Orientation::Horizontal, 8, "");
        root.set_halign(Align::Start);
        Rc::new(InputForm { root, fields: RefCell::default() })
    }

    pub fn set_fields(&self, fs: Vec<Field>) {
        ui::clear_box(&self.root);
        let mut v = Vec::new();
        for f in fs {
            let w: gtk::Widget = if f.kind == "boolean" {
                let b = caixa(Orientation::Horizontal, 8, "entrada-campo");
                b.add_css_class("chave");
                b.append(&rotulo(&f.name, "entrada-campo-nome"));
                let s = gtk::Switch::new();
                s.set_valign(Align::Center);
                b.append(&s);
                self.root.append(&b);
                s.upcast()
            } else {
                let b = caixa(Orientation::Horizontal, 8, "entrada-campo");
                b.append(&rotulo(&f.name, "entrada-campo-nome"));
                let e = gtk::Entry::new();
                e.add_css_class("entrada-campo-valor");
                // (the natural width is the longest: numbers are short, texts a path)
                let (min, max) = match f.kind.as_str() {
                    "number" | "integer" => (3, 6),
                    "boolean" => (3, 6),
                    _ => (10, 20),
                };
                e.set_width_chars(min);
                e.set_max_width_chars(max);
                if f.kind == "number" || f.kind == "integer" {
                    e.set_input_purpose(gtk::InputPurpose::Number);
                }
                let hint = if f.description.is_empty() { kind_name(&f.kind).to_string() } else { f.description.clone() };
                e.set_tooltip_text(Some(&format!("{}{} · {hint}", f.name, if f.required { " *" } else { "" })));
                b.append(&e);
                self.root.append(&b);
                e.upcast()
            };
            v.push((f, w));
        }
        self.root.set_visible(!v.is_empty());
        *self.fields.borrow_mut() = v;
    }

    pub fn is_empty(&self) -> bool {
        self.fields.borrow().is_empty()
    }

    /// The typed input, or the first problem's sentence (the field with it is marked).
    pub fn value(&self) -> Result<Value, String> {
        let fs = self.fields.borrow();
        let raws: Vec<(String, Raw)> = fs
            .iter()
            .map(|(f, w)| {
                let r = match w.downcast_ref::<gtk::Switch>() {
                    Some(s) => Raw::Switch(s.is_active()),
                    None => Raw::Text(w.downcast_ref::<gtk::Entry>().map(|e| e.text().to_string()).unwrap_or_default()),
                };
                (f.name.clone(), r)
            })
            .collect();
        let list: Vec<Field> = fs.iter().map(|(f, _)| f.clone()).collect();
        for (_, w) in fs.iter() {
            if let Some(p) = w.parent() {
                p.remove_css_class("erro");
            }
        }
        build_input(&list, &raws).map_err(|problems| {
            for p in &problems {
                if let Some((_, w)) = fs.iter().find(|(f, _)| f.name == p.field) {
                    if let Some(b) = w.parent() {
                        b.add_css_class("erro");
                    }
                }
            }
            if let Some((_, w)) = problems.first().and_then(|p| fs.iter().find(|(f, _)| f.name == p.field)) {
                w.grab_focus();
            }
            problems.first().map(problem_text).unwrap_or_default()
        })
    }

    /// Fills the fields with a value (a kept conversation's last input).
    pub fn fill(&self, input: &Value) {
        for (f, w) in self.fields.borrow().iter() {
            let Some(v) = input.get(&f.name) else { continue };
            if let Some(s) = w.downcast_ref::<gtk::Switch>() {
                s.set_active(v.as_bool().unwrap_or(false));
            } else if let Some(e) = w.downcast_ref::<gtk::Entry>() {
                e.set_text(&match v {
                    Value::Object(_) => v.to_string(),
                    _ => value_text(v),
                });
            }
        }
    }
}

// ------------------------------------------------------------------------------------------------ result blocks

/// A structured output as a name / type / value table (design "Chat · conversa com um agente seu").
pub fn output_table(fields: &[Field], output: &Value) -> Option<gtk::Box> {
    let rows = output_rows(fields, output);
    if rows.is_empty() {
        return None;
    }
    let t = caixa(Orientation::Vertical, 0, "saida-tabela");
    for (name, kind, value) in rows {
        let r = caixa(Orientation::Horizontal, 12, "saida-linha");
        let n = rotulo(&name, "saida-nome");
        n.set_xalign(0.0);
        n.set_width_chars(10);
        n.set_max_width_chars(16);
        n.set_ellipsize(gtk::pango::EllipsizeMode::End);
        r.append(&n);
        let k = ui::chip(kind, "");
        k.set_valign(Align::Center);
        r.append(&k);
        let v = ui::wrapped(&value, "saida-valor");
        v.set_selectable(true);
        r.append(&v);
        t.append(&r);
    }
    Some(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agents() -> Vec<(String, String)> {
        vec![("org".into(), "Organizador".into()), ("rel".into(), "Relatório da semana".into()), ("rel2".into(), "Relatório".into())]
    }

    #[test]
    fn a_mention_being_typed() {
        assert_eq!(mention_query("@"), Some(String::new()));
        assert_eq!(mention_query("arrume @org"), Some("org".into()));
        assert_eq!(mention_query("@Relatório da"), Some("Relatório da".into()));
        // an e-mail is not a mention, nor a finished line
        assert_eq!(mention_query("me@exemplo.com"), None);
        assert_eq!(mention_query("@org\nmais"), None);
        assert_eq!(mention_query("sem nada"), None);
        assert_eq!(strip_mention_query("arrume isto @org"), "arrume isto");
    }

    #[test]
    fn a_mention_calls_the_longest_name_and_leaves_the_message() {
        assert_eq!(resolve_mention("@organizador arrume Downloads", &agents()), Some(("org".into(), "arrume Downloads".into())));
        assert_eq!(resolve_mention("@relatorio da semana: faça o de hoje", &agents()), Some(("rel".into(), "faça o de hoje".into())));
        assert_eq!(resolve_mention("faça @Relatório agora", &agents()), Some(("rel2".into(), "faça agora".into())));
        // part of a word is not the name; an e-mail is not a mention
        assert_eq!(resolve_mention("@Organizadores", &agents()), None);
        assert_eq!(resolve_mention("fale com eu@organizador", &agents()), None);
        assert_eq!(resolve_mention("@ninguem oi", &agents()), None);
    }

    fn f(name: &str, kind: &str, required: bool) -> Field {
        Field { name: name.into(), kind: kind.into(), required, description: String::new() }
    }

    #[test]
    fn input_is_typed_by_kind() {
        assert_eq!(typed_value("integer", &Raw::Text(" 30 ".into())), Ok(Some(json!(30))));
        assert_eq!(typed_value("integer", &Raw::Text("3,5".into())), Err("wrong_type"));
        assert_eq!(typed_value("number", &Raw::Text("3,5".into())), Ok(Some(json!(3.5))));
        assert_eq!(typed_value("boolean", &Raw::Text("Não".into())), Ok(Some(json!(false))));
        assert_eq!(typed_value("boolean", &Raw::Switch(true)), Ok(Some(json!(true))));
        assert_eq!(typed_value("list", &Raw::Text("Imagens, Documentos;".into())), Ok(Some(json!(["Imagens", "Documentos"]))));
        assert_eq!(typed_value("object", &Raw::Text("{\"a\": 1}".into())), Ok(Some(json!({"a": 1}))));
        assert_eq!(typed_value("object", &Raw::Text("[1]".into())), Err("wrong_type"));
        assert_eq!(typed_value("text", &Raw::Text("  ".into())), Ok(None));
    }

    #[test]
    fn input_is_checked_before_it_goes() {
        let fs = vec![f("pasta", "text", true), f("dias", "integer", false), f("apagar", "boolean", false)];
        let ok = build_input(&fs, &[("pasta".into(), Raw::Text("~/Downloads".into())), ("dias".into(), Raw::Text("30".into())), ("apagar".into(), Raw::Switch(true))]);
        assert_eq!(ok, Ok(json!({"pasta": "~/Downloads", "dias": 30, "apagar": true})));
        // (the words follow the locale the test runs in)
        assert_eq!(input_summary(&fs, ok.as_ref().unwrap()), format!("pasta: ~/Downloads · dias: 30 · apagar: {}", tr("yes")));
        let bad = build_input(&fs, &[("dias".into(), Raw::Text("trinta".into())), ("apagar".into(), Raw::Switch(false))]).unwrap_err();
        assert_eq!(bad.iter().map(|p| (p.field.as_str(), p.problem)).collect::<Vec<_>>(), vec![("pasta", "missing"), ("dias", "wrong_type")]);
        // an empty optional field stays out
        assert_eq!(build_input(&fs, &[("pasta".into(), Raw::Text("x".into())), ("apagar".into(), Raw::Switch(false))]), Ok(json!({"pasta": "x", "apagar": false})));
    }

    #[test]
    fn output_rows_follow_the_fields() {
        let fs = vec![f("movidos", "integer", true), f("pastas", "list", false)];
        let rows = output_rows(&fs, &json!({"resumo": "ok", "pastas": ["Imagens", "Documentos"], "movidos": 22}));
        assert_eq!(rows, vec![("movidos".into(), tr("integer"), "22".into()), ("pastas".into(), tr("list"), "Imagens, Documentos".into()), ("resumo".into(), tr("text"), "ok".into())]);
        assert!(output_rows(&fs, &json!("texto")).is_empty());
    }

    #[test]
    fn conversations_are_labelled_with_their_agent() {
        assert_eq!(history_label(&json!({"titulo": "x", "agent": {"id": "org", "name": "Organizador"}})), Some("Organizador".into()));
        assert_eq!(history_label(&json!({"titulo": "x"})), None);
        let a = json!({"id": "org", "name": "Organizador", "kind": "assistant", "description": "Arruma Downloads"});
        assert_eq!(agent_ref(&a), json!({"id": "org", "name": "Organizador", "kind": "assistant", "icon": {"name": "folder", "color": "gold"}}));
    }

    #[test]
    fn recent_agents_come_first() {
        let list = vec![json!({"id": "a"}), json!({"id": "b"}), json!({"id": "c"})];
        let convs = vec![("1".to_string(), json!({"agent": {"id": "c"}, "atualizado": 50})), ("2".to_string(), json!({"agent": {"id": "b"}, "atualizado": 90})), ("3".to_string(), json!({"atualizado": 99}))];
        let ids: Vec<String> = order_recent(list, &convs).iter().map(|a| a["id"].as_str().unwrap().to_string()).collect();
        assert_eq!(ids, ["b", "c", "a"]);
    }

    #[test]
    fn icons_mirror_the_service() {
        assert_eq!(default_icon("assistant", "Organizador", "Arruma Downloads"), ("folder", "gold"));
        assert_eq!(default_icon("tasks", "Relatório da semana", "Junta os números"), ("list", "teal"));
        assert_eq!(default_icon("flow", "Backup de fotos", ""), ("image", "teal"));
        assert_eq!(default_icon("assistant", "Ajudante", ""), ("spark", "accent"));
        assert_eq!(glyph_of("folder"), "pasta");
        assert_eq!(glyph_of("pasta"), "pasta");
        assert_eq!(glyph_of("nao-existe"), "spark");
        assert_eq!(tone_of("purple"), "lilas");
        let own = json!({"name": "X", "kind": "assistant", "icon": {"name": "bell", "color": "coral"}});
        assert_eq!(agent_icon(&own), AgentIcon::Glyph { glyph: "sino".into(), tone: "coral" });
        let svg = json!({"name": "X", "icon": {"svg": "<svg/>", "monochrome": true, "color": "teal"}});
        assert_eq!(agent_icon(&svg), AgentIcon::Svg { svg: "<svg/>".into(), monochrome: true, tone: "teal" });
    }
}
