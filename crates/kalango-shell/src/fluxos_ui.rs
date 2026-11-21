//! Pieces shared by the screens of the agents and automations platform (Meus agentes, Prontos para usar, Ferramentas,
//! the agent editor, the HTTP tool editor and the OpenAPI import): small widgets of the design (chips, segmented
//! choices, tabs with the short underline, buttons with a glyph, the icon tile of an agent) and the pure functions
//! behind what they show (an agent's kind, its variables, a run's day and time, the request as the test shows it).

use crate::conversa::ui_icon;
use crate::superficies::{caixa, estrela, rotulo};
use gtk::prelude::*;
use gtk::{glib, Align, Orientation};
use kalango_i18n::{tr, trf};
use serde_json::{json, Value};
use std::rc::Rc;

// ---------------------------------------------------------------- colours (dark tokens; the light theme swaps them)

pub const ACCENT: &str = "#A5A1E2";
pub const TEXT: &str = "#E2E2E6";
pub const MUTED: &str = "#8E8F99";
pub const ON_ACCENT: &str = "#1C1D24";

/// The glyph colour of a tone class.
pub fn tone_color(tone: &str) -> &'static str {
    match tone {
        "teal" => "#8ED2C3",
        "ouro" => "#E5C98C",
        "coral" => "#EFA2A6",
        "azul" => "#7CC3DF",
        "lilas" => "#C7A7DC",
        "cinza" => MUTED,
        _ => ACCENT,
    }
}

// ---------------------------------------------------------------- pure functions

/// An agent's kind as saved ("assistant", "sequential", "parallel", "loop", "remote", "flow").
pub fn kind_of(agent: &Value) -> &str {
    agent["kind"].as_str().unwrap_or("assistant")
}

/// What the library calls a kind.
pub fn kind_label(kind: &str) -> &'static str {
    match kind {
        "sequential" => tr("team in sequence"),
        "parallel" => tr("team in parallel"),
        "loop" => tr("repetition"),
        "remote" => tr("remote · A2A"),
        "flow" => tr("flow"),
        "tasks" => tr("task list"),
        _ => tr("assistant"),
    }
}

pub fn kind_glyph(kind: &str) -> &'static str {
    match kind {
        "sequential" => "lista",
        "parallel" => "enviar",
        "loop" => "reiniciar",
        "remote" => "agrupar",
        "flow" => "raio",
        "tasks" => "lista",
        _ => "spark",
    }
}

pub fn kind_tone(kind: &str) -> &'static str {
    match kind {
        "sequential" => "azul",
        "parallel" => "ouro",
        "loop" => "coral",
        "remote" => "azul",
        "flow" => "teal",
        "tasks" => "teal",
        _ => "acento",
    }
}

/// The library's filter: "all", "assistants", "teams", "flows", "remotes".
pub fn kind_in_filter(filter: &str, kind: &str) -> bool {
    match filter {
        "assistants" => kind == "assistant",
        "teams" => matches!(kind, "sequential" | "parallel" | "loop" | "tasks"),
        "flows" => kind == "flow",
        "remotes" => kind == "remote",
        _ => true,
    }
}

/// Whether an item was made by an agent (`created_by` = "agent:<id>").
pub fn made_by_agent(created_by: &str) -> bool {
    created_by.starts_with("agent:")
}

/// Lowercase without accents (searching "precos" finds "Preços").
pub fn fold(s: &str) -> String {
    s.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            c => c,
        })
        .collect()
}

/// Whether every word of the query is in one of these texts.
pub fn matches_query(query: &str, texts: &[&str]) -> bool {
    let all = fold(&texts.join(" "));
    fold(query).split_whitespace().all(|w| all.contains(w))
}

/// The `{variables}` of an instruction, in order, once each (`{cofre:…}` is a vault key, not a variable).
pub fn variables(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (i, _) in text.match_indices('{') {
        let rest = &text[i + 1..];
        let Some(end) = rest.find('}') else { continue };
        let name = &rest[..end];
        let ok = !name.is_empty() && name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        if ok && !out.iter().any(|n| n == name) {
            out.push(name.to_string());
        }
    }
    out
}

/// Byte ranges of the `{variables}` in a text (to paint them as chips).
pub fn variable_spans(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for (i, _) in text.match_indices('{') {
        let rest = &text[i + 1..];
        let Some(end) = rest.find('}') else { continue };
        let name = &rest[..end];
        if !name.is_empty() && name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            out.push((i, i + end + 2));
        }
    }
    out
}

/// A tool level's tone ("observar" reads, "usuario" changes the person's files, "sistema"/"total" the system).
pub fn level_tone(level: &str) -> &'static str {
    match level {
        "observar" => "teal",
        "usuario" => "ouro",
        "sistema" | "total" => "coral",
        _ => "cinza",
    }
}

pub fn level_label(level: &str) -> &'static str {
    match level {
        "observar" => tr("observe"),
        "usuario" => tr("user"),
        "sistema" => tr("system"),
        "total" => tr("full"),
        _ => tr("unknown"),
    }
}

/// "free", "ask", "never" as the screens say them.
pub fn permission_label(p: &str) -> &'static str {
    match p {
        "free" => tr("free"),
        "never" => tr("never"),
        _ => tr("ask"),
    }
}

pub fn permission_tone(p: &str) -> &'static str {
    match p {
        "free" => "teal",
        "never" => "cinza",
        _ => "ouro",
    }
}

/// A run's status: (text, tone).
pub fn run_status(status: &str) -> (&'static str, &'static str) {
    match status {
        "done" => (tr("ok"), "teal"),
        "running" => (tr("running"), "acento"),
        "waiting_approval" | "waiting_input" => (tr("waits for you"), "ouro"),
        "stopped" => (tr("stopped"), "cinza"),
        _ => (tr("failed"), "coral"),
    }
}

/// When something happened, as the lists say it: "hoje 14:02", "ontem 14:00", "seg 14:00", "12/10 14:00".
pub fn day_and_time(now: i64, t: i64) -> String {
    let (Ok(n), Ok(d)) = (glib::DateTime::from_unix_local(now), glib::DateTime::from_unix_local(t)) else { return String::new() };
    let hm = d.format("%H:%M").map(|s| s.to_string()).unwrap_or_default();
    let days = day_number(&n) - day_number(&d);
    let day = match days {
        0 => tr("today").to_string(),
        1 => tr("yesterday").to_string(),
        2..=6 => d.format("%a").map(|s| s.to_string().to_lowercase()).unwrap_or_default(),
        _ => d.format("%d/%m").map(|s| s.to_string()).unwrap_or_default(),
    };
    format!("{day} {hm}")
}

fn day_number(d: &glib::DateTime) -> i64 {
    // days since the epoch in local time (the calendar day, not 24 h blocks)
    (d.to_unix() + d.utc_offset().as_seconds()).div_euclid(86_400)
}

/// "2 h", "3 dias", "agora": how long ago.
pub fn ago(now: i64, t: i64) -> String {
    let s = (now - t).max(0);
    if s < 60 {
        tr("now").to_string()
    } else if s < 3600 {
        trf("{n} min ago", &[("n", (s / 60).to_string())])
    } else if s < 86_400 {
        trf("{n} h ago", &[("n", (s / 3600).to_string())])
    } else {
        let d = s / 86_400;
        if d == 1 { tr("1 day ago").to_string() } else { trf("{n} days ago", &[("n", d.to_string())]) }
    }
}

/// A model reference as the screens show it ("LFM3 local", "OpenRouter · llama-3.3-70b").
pub fn model_label(m: &Value) -> String {
    if m.is_null() {
        return tr("LFM3 local").to_string();
    }
    let provider = m["provider"].as_str().unwrap_or("local");
    let model = m["model"].as_str().unwrap_or("");
    if provider == "local" {
        if model.is_empty() { tr("LFM3 local").to_string() } else { trf("{model} local", &[("model", model.to_string())]) }
    } else {
        format!("{provider} · {model}")
    }
}

/// "18 mil tokens" for 18000; "sem limite" for none.
pub fn tokens_label(n: Option<u64>) -> String {
    match n {
        None => tr("no limit").to_string(),
        Some(n) if n >= 1000 && n % 1000 == 0 => trf("{n} thousand tokens", &[("n", (n / 1000).to_string())]),
        Some(n) => trf("{n} tokens", &[("n", n.to_string())]),
    }
}

/// The creativity slider's words: temperature → "0,2 · preciso".
pub fn creativity_label(t: f64) -> String {
    let word = if t <= 0.3 { tr("precise") } else if t <= 0.7 { tr("balanced") } else { tr("creative") };
    format!("{} · {word}", format!("{t:.1}").replace('.', ","))
}

/// The host and the path with its query of a URL ("https://a.b/v1/x?y=1" → ("a.b", "/v1/x?y=1")).
pub fn split_url(url: &str) -> (String, String) {
    let rest = url.split_once("://").map(|x| x.1).unwrap_or(url);
    match rest.find('/') {
        Some(i) => (rest[..i].to_string(), rest[i..].to_string()),
        None => (rest.to_string(), "/".to_string()),
    }
}

/// A header's value as the test shows it: vault references and tokens become dots.
/// What a secret shows instead of its value.
const MASK: &str = "••••••••";

pub fn mask_secret(name: &str, value: &str) -> String {
    let mut out = String::new();
    let mut rest = value;
    while let Some(i) = rest.find("{cofre:") {
        out.push_str(&rest[..i]);
        match rest[i..].find('}') {
            Some(j) => {
                out.push_str(MASK);
                rest = &rest[i + j + 1..];
            }
            None => {
                rest = "";
            }
        }
    }
    out.push_str(rest);
    let lower = name.to_ascii_lowercase();
    if (lower == "authorization" || lower.contains("key") || lower.contains("token")) && !out.contains('•') {
        // a literal secret typed into the header: only its scheme stays
        return match out.split_once(' ') {
            Some((scheme, _)) => format!("{scheme} {MASK}"),
            None => MASK.to_string(),
        };
    }
    out
}

/// The request as the "Testar" panel shows it, from the service's exchange (method, url, headers, body).
pub fn request_text(exchange: &Value) -> String {
    let (host, path) = split_url(exchange["url"].as_str().unwrap_or(""));
    let mut s = format!("{} {path}\nHost: {host}", exchange["method"].as_str().unwrap_or("GET"));
    for h in exchange["headers"].as_array().into_iter().flatten() {
        let (k, v) = (h[0].as_str().unwrap_or(""), h[1].as_str().unwrap_or(""));
        if k == "X-Requested-With" {
            continue;
        }
        s.push_str(&format!("\n{k}: {}", mask_secret(k, v)));
    }
    if let Some(b) = exchange.get("body").filter(|b| !b.is_null()) {
        s.push_str(&format!("\n\n{}", serde_json::to_string_pretty(b).unwrap_or_default()));
    }
    s
}

/// What kind of value an output is ("número", "lista de números"…).
pub fn json_type_label(v: &Value) -> &'static str {
    match v {
        Value::Number(_) => tr("number"),
        Value::String(_) => tr("text"),
        Value::Bool(_) => tr("yes/no"),
        Value::Object(_) => tr("object"),
        Value::Null => "—",
        Value::Array(a) => {
            if !a.is_empty() && a.iter().all(Value::is_number) {
                tr("list of numbers")
            } else if !a.is_empty() && a.iter().all(Value::is_string) {
                tr("list of texts")
            } else {
                tr("list")
            }
        }
    }
}

/// A parameter type as the table says it.
pub fn param_type_label(kind: &str, fixed: bool) -> &'static str {
    if fixed {
        return tr("fixed list");
    }
    match kind {
        "number" | "integer" => tr("number"),
        "boolean" => tr("yes/no"),
        "array" => tr("list"),
        "object" => tr("object"),
        _ => tr("text"),
    }
}

pub fn location_label(loc: &str) -> &'static str {
    match loc {
        "path" => tr("path"),
        "body" => tr("body"),
        _ => tr("query"),
    }
}

/// "2 vezes · espera 1 s → 2 s".
pub fn retry_label(count: u32, delay: f64) -> String {
    if count == 0 {
        return tr("no").to_string();
    }
    let waits: Vec<String> = (0..count).map(|i| format!("{} s", fmt_num(delay * 2f64.powi(i as i32)))).collect();
    let times = if count == 1 { tr("1 time").to_string() } else { trf("{n} times", &[("n", count.to_string())]) };
    format!("{times} · {} {}", tr("waits"), waits.join(" → "))
}

fn fmt_num(x: f64) -> String {
    if x.fract() == 0.0 { format!("{x:.0}") } else { format!("{x:.1}").replace('.', ",") }
}

/// The section of an operation in the OpenAPI list: its path's first word ("/pedidos/{id}" → "PEDIDOS").
pub fn path_group(path: &str) -> String {
    path.trim_start_matches('/').split('/').next().filter(|s| !s.is_empty() && !s.starts_with('{')).unwrap_or("API").to_uppercase()
}

/// A method's tone (reads teal, writes gold, deletes coral).
pub fn method_tone(method: &str) -> &'static str {
    match method.to_uppercase().as_str() {
        "GET" | "HEAD" => "teal",
        "DELETE" => "coral",
        _ => "ouro",
    }
}

/// A new id for things saved from this screen.
pub fn new_id(prefix: &str) -> String {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{prefix}-{:x}{:04x}", t / 1_000_000, (t % 65_536) as u32 ^ std::process::id() & 0xffff)
}

/// A ready-made template's agents with fresh ids: the template names its agents "$main", "$a1"…; every string equal
/// to such a name (ids, sub-agent lists, flow blocks) becomes the new id. Returns the agents in saving order
/// (sub-agents first) and the flow, if the template has one.
pub fn instantiate_template(template: &Value, mut fresh: impl FnMut(&str) -> String) -> (Vec<Value>, Option<Value>) {
    let mut ids: Vec<(String, String)> = Vec::new();
    for a in template["agents"].as_array().into_iter().flatten() {
        if let Some(id) = a["id"].as_str() {
            ids.push((id.to_string(), fresh(id)));
        }
    }
    fn swap(v: &Value, ids: &[(String, String)]) -> Value {
        match v {
            Value::String(s) => ids.iter().find(|(k, _)| k == s).map(|(_, n)| json!(n)).unwrap_or_else(|| v.clone()),
            Value::Array(a) => Value::Array(a.iter().map(|x| swap(x, ids)).collect()),
            Value::Object(o) => Value::Object(o.iter().map(|(k, x)| (k.clone(), swap(x, ids))).collect()),
            _ => v.clone(),
        }
    }
    let mut agents: Vec<Value> = template["agents"].as_array().into_iter().flatten().map(|a| swap(a, &ids)).collect();
    // the main one ("$main") last: the others are its sub-agents
    agents.sort_by_key(|a| template["agents"].as_array().and_then(|l| l.iter().find(|x| swap(&x["id"], &ids) == a["id"])).map(|x| x["id"] == "$main").unwrap_or(false));
    let flow = template.get("flow").filter(|f| !f.is_null()).map(|f| swap(f, &ids));
    (agents, flow)
}

/// A team as a flow (for "Abrir como fluxo"): the sub-agents one after the other, each with the previous output.
pub fn team_as_flow(sub_agents: &[String]) -> Value {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    for (i, a) in sub_agents.iter().enumerate() {
        nodes.push(json!({"id": format!("n{}", i + 1), "position": {"x": 0.0, "y": i as f64 * 120.0}, "type": "agent", "data": {"agent_id": a}}));
        if i > 0 {
            edges.push(json!({"source": format!("n{i}"), "target": format!("n{}", i + 1)}));
        }
    }
    json!({"nodes": nodes, "edges": edges})
}

/// A kalangod tool's description without the level note it ends with ("… [nível observar; só ler]").
pub fn tool_description(d: &str) -> &str {
    match d.rfind(" [") {
        Some(i) if d.ends_with(']') => &d[..i],
        _ => d,
    }
}

/// The prefix of a tool name ("arquivos.listar" → "arquivos").
pub fn tool_prefix(name: &str) -> &str {
    name.split('.').next().unwrap_or(name)
}

/// The connectors installed for the agents (kalangod manifests): (name, on).
pub fn connectors() -> Vec<(String, bool)> {
    let dir = crate::central_ajustes::pasta();
    let state: Value = std::fs::read_to_string(dir.with_extension("json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(json!({}));
    let mut v: Vec<(String, bool)> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            (p.extension().is_some_and(|x| x == "toml")).then(|| p.file_stem().map(|s| s.to_string_lossy().into_owned())).flatten()
        })
        .map(|n| {
            let on = state[&n]["ligado"].as_bool().unwrap_or(true);
            (n, on)
        })
        .collect();
    v.sort();
    v
}

// ---------------------------------------------------------------- widgets

pub fn left(t: &str, class: &str) -> gtk::Label {
    let l = rotulo(t, class);
    l.set_xalign(0.0);
    l
}

/// A label that wraps in the room it gets (never widens what holds it).
pub fn wrapped(t: &str, class: &str) -> gtk::Label {
    let l = left(t, class);
    l.set_wrap(true);
    l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    l.set_max_width_chars(1);
    l.set_hexpand(true);
    l
}

/// A one-line label cut with "…".
pub fn cut(t: &str, class: &str) -> gtk::Label {
    let l = left(t, class);
    l.set_ellipsize(gtk::pango::EllipsizeMode::End);
    l.set_max_width_chars(1);
    l.set_hexpand(true);
    l
}

pub fn chip(t: &str, tone: &str) -> gtk::Label {
    let l = rotulo(t, "fx-chip");
    if !tone.is_empty() {
        l.add_css_class(tone);
    }
    l.set_valign(Align::Center);
    l
}

pub fn mono_chip(t: &str, tone: &str) -> gtk::Label {
    let l = chip(t, tone);
    l.add_css_class("mono");
    l
}

pub fn glyph(name: &str, color: &str, px: i32) -> gtk::Widget {
    if name == "spark" { estrela(px - 2, color).upcast() } else { ui_icon(name, color, px).upcast() }
}

/// A button with a glyph and a text; `accent` is the filled one.
pub fn button(icon: Option<&str>, text: &str, accent: bool) -> gtk::Button {
    let b = gtk::Button::new();
    b.add_css_class("fx-botao");
    if accent {
        b.add_css_class("acento");
    }
    let c = caixa(Orientation::Horizontal, 7, "");
    c.set_halign(Align::Center);
    if let Some(g) = icon {
        c.append(&glyph(g, if accent { ON_ACCENT } else { TEXT }, 14));
    }
    if !text.is_empty() {
        c.append(&rotulo(text, ""));
    }
    b.set_child(Some(&c));
    b
}

/// Replaces a button's text, keeping its glyph.
pub fn set_button_text(b: &gtk::Button, text: &str) {
    if let Some(c) = b.child().and_downcast::<gtk::Box>() {
        let mut w = c.first_child();
        while let Some(x) = w {
            if let Some(l) = x.downcast_ref::<gtk::Label>() {
                l.set_text(text);
            }
            w = x.next_sibling();
        }
    }
}

/// A text-only button of the design (links: "Usar", "Marcar todas de leitura").
pub fn link(text: &str) -> gtk::Button {
    let b = gtk::Button::with_label(text);
    b.add_css_class("fx-link");
    b
}

/// The tile of an agent, a template or a tool: a glyph on its tone, 6 px corners.
pub fn icon_tile(icon: &str, tone: &str, side: i32) -> gtk::Box {
    let t = caixa(Orientation::Horizontal, 0, "fx-icone");
    t.add_css_class(tone);
    t.set_size_request(side, side);
    t.set_valign(Align::Start);
    t.set_hexpand(false);
    let g = glyph(icon, tone_color(tone), if side >= 40 { 18 } else { 16 });
    g.set_hexpand(true);
    t.append(&g);
    t
}

/// A section's title in capitals, muted.
pub fn section(t: &str) -> gtk::Label {
    left(t, "central-painel-secao")
}

/// The side panel's title: its glyph and the capitals.
pub fn panel_title(icon: &str, t: &str) -> gtk::Box {
    let b = caixa(Orientation::Horizontal, 8, "");
    b.append(&glyph(icon, ACCENT, 14));
    b.append(&rotulo(t, "central-painel-agente"));
    b
}

/// Empties a flow box. (Through the box: unparenting its children by hand leaves the box's own list pointing at
/// them, and the shell crashed on the next layout.)
pub fn clear(b: &gtk::FlowBox) {
    b.remove_all();
}

pub fn clear_box(b: &gtk::Box) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
}

/// An entry of the forms.
pub fn entry(text: &str, placeholder: &str) -> gtk::Entry {
    let e = gtk::Entry::new();
    e.add_css_class("fx-campo");
    e.set_text(text);
    e.set_placeholder_text(Some(placeholder));
    e.set_hexpand(true);
    e
}

pub fn mono_entry(text: &str, placeholder: &str) -> gtk::Entry {
    let e = entry(text, placeholder);
    e.add_css_class("mono");
    e
}

/// A field's title above it.
pub fn labelled(title: &str, w: &impl IsA<gtk::Widget>) -> gtk::Box {
    let b = caixa(Orientation::Vertical, 6, "");
    b.append(&left(title, "fx-rotulo-campo"));
    b.append(w);
    b.set_hexpand(true);
    b
}

pub fn dropdown(options: &[String], selected: usize) -> gtk::DropDown {
    let d = gtk::DropDown::from_strings(&options.iter().map(String::as_str).collect::<Vec<_>>());
    d.add_css_class("central-escolha");
    d.set_selected(selected.min(options.len().saturating_sub(1)) as u32);
    d.set_hexpand(true);
    d
}

pub fn switch() -> gtk::Switch {
    crate::configuracoes::interruptor()
}

/// A search field: the magnifier and an entry.
pub fn search(placeholder: &str) -> (gtk::Box, gtk::Entry) {
    let b = caixa(Orientation::Horizontal, 8, "fx-busca");
    b.append(&ui_icon("busca", MUTED, 14));
    let e = gtk::Entry::new();
    e.add_css_class("fx-busca-campo");
    e.set_placeholder_text(Some(placeholder));
    e.set_hexpand(true);
    b.append(&e);
    (b, e)
}

/// A segmented choice. `on_pick` gets the chosen id. Returns the box and a way to choose one from code.
pub struct Segmented {
    pub root: gtk::Box,
    buttons: Vec<(String, gtk::ToggleButton)>,
}

impl Segmented {
    pub fn new(options: &[(String, String)], current: &str, on_pick: impl Fn(&str) + 'static) -> Segmented {
        let root = caixa(Orientation::Horizontal, 2, "fx-seg");
        root.set_halign(Align::Start);
        root.set_valign(Align::Center);
        let on_pick = Rc::new(on_pick);
        let mut buttons: Vec<(String, gtk::ToggleButton)> = Vec::new();
        for (id, name) in options {
            let b = gtk::ToggleButton::with_label(name);
            b.add_css_class("fx-seg-item");
            if let Some((_, f)) = buttons.first() {
                b.set_group(Some(f));
            }
            b.set_active(id == current);
            let (id2, f) = (id.clone(), on_pick.clone());
            b.connect_toggled(move |b| {
                if b.is_active() {
                    f(&id2);
                }
            });
            root.append(&b);
            buttons.push((id.clone(), b));
        }
        Segmented { root, buttons }
    }

    /// Chooses without telling (`on_pick` does not run).
    pub fn set(&self, id: &str) {
        for (i, b) in &self.buttons {
            if i == id && !b.is_active() {
                b.set_active(true);
            }
        }
    }

    pub fn set_label(&self, id: &str, text: &str) {
        for (i, b) in &self.buttons {
            if i == id {
                b.set_label(text);
            }
        }
    }

    pub fn button(&self, id: &str) -> Option<gtk::ToggleButton> {
        self.buttons.iter().find(|(i, _)| i == id).map(|(_, b)| b.clone())
    }
}

/// Tabs of a page with the design's short underline. `on_pick` gets the chosen id.
pub struct Tabs {
    pub root: gtk::Box,
    items: Vec<(String, gtk::Button, gtk::Box)>,
}

impl Tabs {
    pub fn new(options: &[(&str, &str)], on_pick: impl Fn(&str) + 'static) -> Rc<Tabs> {
        let root = caixa(Orientation::Horizontal, 26, "fx-abas");
        let mut items = Vec::new();
        let on_pick = Rc::new(on_pick);
        for (id, name) in options {
            let b = gtk::Button::new();
            b.add_css_class("fx-aba");
            let col = caixa(Orientation::Vertical, 6, "");
            col.append(&rotulo(name, ""));
            let bar = caixa(Orientation::Horizontal, 0, "fx-aba-traco");
            bar.set_size_request(24, 2);
            bar.set_halign(Align::Center);
            col.append(&bar);
            b.set_child(Some(&col));
            let (id2, f) = (id.to_string(), on_pick.clone());
            b.connect_clicked(move |_| f(&id2));
            crate::central::posicao_debug_sempre(&format!("aba {id}"), b.upcast_ref());
            root.append(&b);
            items.push((id.to_string(), b, bar));
        }
        Rc::new(Tabs { root, items })
    }

    /// Shows or hides a tab (the agent editor's tabs follow the agent's kind).
    pub fn set_visible(&self, id: &str, on: bool) {
        for (i, b, _) in &self.items {
            if i == id {
                b.set_visible(on);
            }
        }
    }

    pub fn show(&self, id: &str) {
        for (i, b, bar) in &self.items {
            if i == id {
                b.add_css_class("ativa");
                bar.add_css_class("ativa");
            } else {
                b.remove_css_class("ativa");
                bar.remove_css_class("ativa");
            }
        }
    }
}

/// A vertical scroll that never scrolls sideways.
pub fn scroll(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    let s = gtk::ScrolledWindow::new();
    s.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    s.set_vexpand(true);
    s.set_child(Some(child));
    s
}

/// The side panel of a page (fixed width; it gives way on narrow windows).
pub fn side_panel(width: i32) -> gtk::Box {
    let p = caixa(Orientation::Vertical, 0, "central-painel");
    p.add_css_class("fx-painel");
    p.set_size_request(width, -1);
    p.set_hexpand(false);
    p
}

/// A note with the shield (what the agent reads is data, never orders).
pub fn note(icon: &str, t: &str) -> gtk::Box {
    let b = caixa(Orientation::Horizontal, 8, "");
    let g = ui_icon(icon, MUTED, 14);
    g.set_valign(Align::Start);
    g.set_margin_top(1);
    b.append(&g);
    b.append(&wrapped(t, "fx-nota"));
    b
}

/// An error line under a form (hidden while empty).
pub fn error_label() -> gtk::Label {
    let l = wrapped("", "fx-erro");
    l.set_visible(false);
    l
}

pub fn show_error(l: &gtk::Label, t: &str) {
    l.set_text(t);
    l.set_visible(!t.is_empty());
}

/// The width of the side panels for a window of this width (the design's 535 at full size).
pub fn panel_width(window: i32) -> i32 {
    if window >= 1480 { 492 } else if window >= 1200 { 420 } else { 360 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variables_and_spans() {
        let t = "Separe em {pasta_destino} e por mês. {total_movidos} arquivos; {cofre:x} e {1a} não; {pasta_destino} de novo";
        assert_eq!(variables(t), vec!["pasta_destino", "total_movidos"]);
        let spans = variable_spans(t);
        assert_eq!(spans.len(), 3);
        assert_eq!(&t[spans[0].0..spans[0].1], "{pasta_destino}");
        assert!(variables("sem chaves").is_empty());
        assert!(variables("{aberta").is_empty());
    }

    #[test]
    fn kinds_and_filters() {
        assert!(kind_in_filter("teams", "loop") && kind_in_filter("teams", "parallel") && !kind_in_filter("teams", "assistant"));
        assert!(kind_in_filter("all", "remote") && kind_in_filter("remotes", "remote") && !kind_in_filter("flows", "remote"));
        assert_eq!(kind_of(&json!({"kind": "flow"})), "flow");
        assert_eq!(kind_of(&json!({})), "assistant");
        assert!(made_by_agent("agent:principal") && !made_by_agent("person"));
    }

    #[test]
    fn search_ignores_case_and_accents() {
        assert!(matches_query("precos", &["Pesquisa de preços", "Busca"]));
        assert!(matches_query("PESQ busca", &["Pesquisa de preços", "Busca"]));
        assert!(!matches_query("fotos", &["Pesquisa de preços"]));
        assert!(matches_query("", &["x"]));
    }

    #[test]
    fn urls_and_secrets() {
        assert_eq!(split_url("https://api.clima.exemplo/v1/previsao/Recife?dias=3"), ("api.clima.exemplo".into(), "/v1/previsao/Recife?dias=3".into()));
        assert_eq!(split_url("http://host"), ("host".into(), "/".into()));
        assert_eq!(mask_secret("Authorization", "Bearer {cofre:clima.token}"), "Bearer ••••••••");
        assert_eq!(mask_secret("Authorization", "Bearer abc123"), "Bearer ••••••••");
        assert_eq!(mask_secret("X-API-Key", "segredo"), "••••••••");
        assert_eq!(mask_secret("Accept", "application/json"), "application/json");
        let ex = json!({"method": "GET", "url": "https://api.x/v1/a?b=1", "headers": [["Authorization", "Bearer {cofre:t}"], ["X-Requested-With", "XMLHttpRequest"]], "body": null});
        assert_eq!(request_text(&ex), "GET /v1/a?b=1\nHost: api.x\nAuthorization: Bearer ••••••••");
    }

    #[test]
    fn labels() {
        assert_eq!(json_type_label(&json!([0, 3.2])), tr("list of numbers"));
        assert_eq!(json_type_label(&json!(29.5)), tr("number"));
        assert_eq!(json_type_label(&json!("x")), tr("text"));
        assert_eq!(path_group("/pedidos/{id}"), "PEDIDOS");
        assert_eq!(path_group("/{id}"), "API");
        assert_eq!(method_tone("delete"), "coral");
        assert_eq!(tool_prefix("correio.enviar"), "correio");
        assert_eq!(tool_description("Lê pastas [nível observar; só ler]"), "Lê pastas");
        assert_eq!(tool_description("Sem nota"), "Sem nota");
        assert!(retry_label(2, 1.0).ends_with("1 s → 2 s"), "{}", retry_label(2, 1.0));
        assert_eq!(retry_label(0, 1.0), tr("no"));
        assert!(creativity_label(0.2).starts_with("0,2"));
        assert_eq!(model_label(&json!({"provider": "openrouter", "model": "llama"})), "openrouter · llama");
    }

    #[test]
    fn days() {
        let now = glib::DateTime::from_local(2026, 11, 15, 14, 30, 0.0).unwrap().to_unix();
        let at = |d: i32, h: i32| glib::DateTime::from_local(2026, 11, d, h, 2, 0.0).unwrap().to_unix();
        assert_eq!(day_and_time(now, at(15, 9)), format!("{} 09:02", tr("today")));
        assert_eq!(day_and_time(now, at(14, 23)), format!("{} 23:02", tr("yesterday")));
        assert!(day_and_time(now, at(1, 8)).starts_with("01/11"));
        assert_eq!(ago(now, now - 7200), trf("{n} h ago", &[("n", "2".into())]));
    }

    #[test]
    fn templates_get_fresh_ids() {
        let t = json!({
            "agents": [
                {"id": "$main", "name": "Equipe", "kind": "sequential", "config": {"sub_agents": ["$a1", "$a2"]}},
                {"id": "$a1", "name": "Buscador", "kind": "assistant", "config": {"instruction": "x"}},
                {"id": "$a2", "name": "Redator", "kind": "assistant", "config": {"instruction": "y"}}
            ],
            "flow": {"nodes": [{"id": "n1", "type": "agent", "data": {"agent_id": "$a1"}}], "edges": []}
        });
        let mut n = 0;
        let (agents, flow) = instantiate_template(&t, |_| {
            n += 1;
            format!("id{n}")
        });
        assert_eq!(agents.len(), 3);
        assert_eq!(agents[2]["id"], "id1", "the main agent is saved last");
        assert_eq!(agents[2]["config"]["sub_agents"], json!(["id2", "id3"]));
        assert_eq!(flow.unwrap()["nodes"][0]["data"]["agent_id"], "id2");
    }

    #[test]
    fn team_flow_chains_the_agents() {
        let f = team_as_flow(&["a".into(), "b".into(), "c".into()]);
        assert_eq!(f["nodes"].as_array().unwrap().len(), 3);
        assert_eq!(f["edges"], json!([{"source": "n1", "target": "n2"}, {"source": "n2", "target": "n3"}]));
        assert_eq!(f["nodes"][1]["data"]["agent_id"], "b");
    }
}
