//! The builder's inspector (design "Fluxo · construtor", "Gatilhos", "Bloco Decidir"): what the chosen block or
//! trigger is and does. Ajustes holds each kind's fields (a condition's rules, a decision's question and options in
//! the System One shape, a trigger's five kinds and their settings with the next firings), its ways out and what it
//! did in the last run; Dados the block's saved data as JSON; Execuções its steps in the recent runs. Every change
//! goes to the page, which redraws the canvas and saves.

use crate::conversa::ui_icon;
use crate::fluxo_construtor::FlowPage;
use crate::fluxo_modelo::{self as m, NodeDoc, Step};
use crate::superficies::{caixa, estrela, rotulo};
use gtk::prelude::*;
use gtk::{glib, Align, Orientation};
use kalango_i18n::{n_, tr, trf};
use serde_json::{json, Value};
use std::rc::Rc;

// ── small widgets ───────────────────────────────────────────────────────────────────────────────────────────────

fn label(t: &str) -> gtk::Label {
    let l = rotulo(t, "fluxo-campo-rotulo");
    l.set_xalign(0.0);
    // (a long field name wraps: the inspector has a fixed width and must not scroll sideways)
    l.set_wrap(true);
    l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    l
}

fn entry(text: &str, hint: &str, on_change: impl Fn(String) + 'static) -> gtk::Entry {
    let e = gtk::Entry::new();
    e.add_css_class("fluxo-campo");
    e.set_text(text);
    e.set_placeholder_text(Some(hint));
    e.connect_changed(move |e| on_change(e.text().to_string()));
    e
}

fn area(text: &str, mono: bool, on_change: impl Fn(String) + 'static) -> gtk::Box {
    let b = caixa(Orientation::Vertical, 0, "fluxo-area");
    let tv = gtk::TextView::new();
    tv.add_css_class("fluxo-area-texto");
    if mono {
        tv.add_css_class("mono");
    }
    tv.set_wrap_mode(gtk::WrapMode::WordChar);
    // (the text view fills its box: measured before its lines are laid out, a one-line text view asks for no height
    // at all, and then shows nothing and takes no click)
    tv.set_vexpand(true);
    tv.set_size_request(-1, 34);
    tv.buffer().set_text(text);
    tv.buffer().connect_changed(move |b| on_change(b.text(&b.start_iter(), &b.end_iter(), false).to_string()));
    b.append(&tv);
    b
}

fn dropdown(options: &[(String, String)], selected: &str, on_change: impl Fn(String) + 'static) -> gtk::DropDown {
    let names: Vec<&str> = options.iter().map(|(_, n)| n.as_str()).collect();
    let d = gtk::DropDown::from_strings(&names);
    d.add_css_class("central-escolha");
    d.add_css_class("fluxo-escolha");
    if let Some(i) = options.iter().position(|(id, _)| id == selected) {
        d.set_selected(i as u32);
    }
    let ids: Vec<String> = options.iter().map(|(id, _)| id.clone()).collect();
    d.connect_selected_notify(move |d| {
        if let Some(id) = ids.get(d.selected() as usize) {
            on_change(id.clone());
        }
    });
    d
}

/// A segmented control: the chosen one raised (design "Ajustes | Dados | Execuções", "Sim/não | Uma de N…").
pub(crate) fn segmented(options: &[(&str, String)], selected: &str, on_change: impl Fn(&str) + 'static) -> gtk::Box {
    let b = caixa(Orientation::Horizontal, 0, "fluxo-seg");
    b.set_halign(Align::Start);
    let on_change = Rc::new(on_change);
    let mut btns: Vec<(String, gtk::Button)> = Vec::new();
    for (id, name) in options {
        let x = gtk::Button::with_label(name);
        x.add_css_class("fluxo-seg-item");
        if *id == selected {
            x.add_css_class("ativo");
        }
        btns.push((id.to_string(), x.clone()));
        b.append(&x);
    }
    let all: Rc<Vec<(String, gtk::Button)>> = Rc::new(btns);
    for (id, x) in all.iter() {
        let (all, id, f) = (all.clone(), id.clone(), on_change.clone());
        x.connect_clicked(move |_| {
            for (i, y) in all.iter() {
                if *i == id { y.add_css_class("ativo") } else { y.remove_css_class("ativo") }
            }
            f(&id);
        });
    }
    b
}

fn switch_row(title: &str, sub: &str, on: bool, sensitive: bool, on_change: impl Fn(bool) + 'static) -> gtk::Box {
    let r = caixa(Orientation::Horizontal, 12, "fluxo-chave");
    let c = caixa(Orientation::Vertical, 2, "");
    c.set_hexpand(true);
    let t = rotulo(title, "fluxo-chave-titulo");
    t.set_xalign(0.0);
    c.append(&t);
    let s = rotulo(sub, "fluxo-chave-sub");
    s.set_xalign(0.0);
    s.set_wrap(true);
    s.set_max_width_chars(30);
    c.append(&s);
    r.append(&c);
    let sw = gtk::Switch::new();
    sw.add_css_class("central-switch");
    sw.set_active(on);
    sw.set_sensitive(sensitive);
    sw.set_valign(Align::Center);
    sw.connect_state_set(move |_, v| {
        on_change(v);
        glib::Propagation::Proceed
    });
    r.append(&sw);
    r
}

fn section(t: &str) -> gtk::Label {
    let l = rotulo(t, "fluxo-secao");
    l.set_xalign(0.0);
    l
}

fn tile(glyph: &str, tone: m::Tone, size: i32) -> gtk::Box {
    let t = caixa(Orientation::Horizontal, 0, "fluxo-no-tile");
    t.add_css_class(tone.class());
    t.add_css_class("grande");
    t.set_size_request(size, size);
    t.set_valign(Align::Start);
    t.set_hexpand(false);
    let g = if glyph == "spark" { estrela(14, tone.color()).upcast::<gtk::Widget>() } else { ui_icon(glyph, tone.color(), 16).upcast() };
    g.set_hexpand(true);
    t.append(&g);
    t
}

fn mono_box(lines: &[String]) -> gtk::Box {
    let b = caixa(Orientation::Vertical, 4, "fluxo-mono-caixa");
    for l in lines {
        let x = rotulo(l, "fluxo-mono");
        x.set_xalign(0.0);
        x.set_wrap(true);
        x.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        x.set_selectable(true);
        b.append(&x);
    }
    b
}

/// A field's text in a rule chip: `var:x` reads `{x}`.
fn field_to_chip(f: &str) -> String {
    match f.strip_prefix("var:") {
        Some(p) => format!("{{{p}}}"),
        None => f.to_string(),
    }
}

fn chip_to_field(t: &str) -> String {
    let t = t.trim();
    match t.strip_prefix('{').and_then(|x| x.strip_suffix('}')) {
        Some(p) => format!("var:{p}"),
        None if t.is_empty() => "content".into(),
        None => t.to_string(),
    }
}

/// A value typed in a chip: "texto" quoted, a number, true/false, else the text.
fn chip_to_value(t: &str) -> Value {
    let t = t.trim();
    if let Some(s) = t.strip_prefix('"').and_then(|x| x.strip_suffix('"')) {
        return json!(s);
    }
    if let Ok(n) = t.replace(',', ".").parse::<f64>() {
        return if n.fract() == 0.0 && n.abs() < 1e15 { json!(n as i64) } else { json!(n) };
    }
    match t {
        "true" => json!(true),
        "false" => json!(false),
        _ => json!(t),
    }
}

// ── the panel ───────────────────────────────────────────────────────────────────────────────────────────────────

/// Builds the inspector for what is chosen now into the page's panel.
pub fn build(page: &Rc<FlowPage>) {
    *page.issue_label.borrow_mut() = None;
    *page.insp_title.borrow_mut() = None;
    *page.on_issues.borrow_mut() = None;
    let content = &page.insp_content;
    while let Some(c) = content.first_child() {
        content.remove(&c);
    }
    let footer = &page.insp_footer;
    while let Some(c) = footer.first_child() {
        footer.remove(&c);
    }
    let sel = page.selected.borrow().clone();
    if page.chosen.borrow().len() > 1 {
        return selection_panel(page);
    }
    match sel {
        // a trigger: the flow's start (its input fields), and a click away the trigger's own settings
        Some(id) if id.starts_with(m::TRIGGER_PREFIX) && *page.insp_tab.borrow() == "gatilho" => trigger_panel(page, id.trim_start_matches(m::TRIGGER_PREFIX)),
        Some(id) if id.starts_with(m::TRIGGER_PREFIX) => start_panel(page, Some(id.trim_start_matches(m::TRIGGER_PREFIX))),
        Some(id) if page.doc.borrow().node(&id).is_some() => block_panel(page, &id),
        _ => empty_panel(page),
    }
}

fn empty_panel(page: &Rc<FlowPage>) {
    let c = &page.insp_content;
    let cab = caixa(Orientation::Horizontal, 8, "");
    cab.append(&ui_icon("info", "#8E8F99", 14));
    cab.append(&rotulo(tr("NO BLOCK SELECTED"), "fluxo-secao-cab"));
    c.append(&cab);
    let t = rotulo(tr("Click a block to adjust it, drag one from the palette, or connect a block's port to another."), "fluxo-chave-sub");
    t.set_xalign(0.0);
    t.set_wrap(true);
    c.append(&t);
    // (a block taken out of the middle of a path, until something else is chosen)
    if let Some(n) = page.note.borrow().as_ref() {
        c.append(&gold_note(n));
    }
    // (a flow without triggers starts by hand: its input fields are set here)
    if page.triggers.borrow().is_empty() {
        input_sections(page, None);
    }
    let issues = page.issues.borrow();
    if !issues.is_empty() {
        c.append(&section(tr("Before it can be saved")));
        let mut v: Vec<String> = issues.iter().map(|(k, v)| if k.is_empty() { m::issue_text(v) } else { format!("{} · {}", page.node_title(k), m::issue_text(v)) }).collect();
        v.sort();
        for x in v {
            let l = rotulo(&x, "fluxo-problema");
            l.set_xalign(0.0);
            l.set_wrap(true);
            c.append(&l);
        }
    }
}

fn header(page: &Rc<FlowPage>, glyph: &str, tone: m::Tone, title: &str, sub: &str, caps: &str) -> gtk::Label {
    let c = &page.insp_content;
    if !page.narrow.get() {
        let cab = caixa(Orientation::Horizontal, 8, "");
        // (the kind's own glyph: the agent's star, the sandbox's shield — designs "editor · inspetor", "em sandbox")
        let g = if caps == tr("TRIGGER") { "raio" } else { glyph };
        if g == "spark" {
            cab.append(&estrela(13, "#A5A1E2"));
        } else {
            cab.append(&ui_icon(g, "#A5A1E2", 14));
        }
        cab.append(&rotulo(caps, "fluxo-secao-cab"));
        c.append(&cab);
    }
    let row = caixa(Orientation::Horizontal, 12, "fluxo-insp-cab");
    row.append(&tile(glyph, tone, 36));
    let col = caixa(Orientation::Vertical, 2, "");
    col.set_hexpand(true);
    let t = rotulo(title, "fluxo-insp-titulo");
    t.set_xalign(0.0);
    t.set_ellipsize(gtk::pango::EllipsizeMode::End);
    t.set_max_width_chars(24);
    col.append(&t);
    let s = rotulo(sub, "fluxo-chave-sub");
    s.set_xalign(0.0);
    col.append(&s);
    row.append(&col);
    c.append(&row);
    t
}

fn block_panel(page: &Rc<FlowPage>, id: &str) {
    let n = page.doc.borrow().node(id).cloned().unwrap();
    let look = m::look(&n.kind);
    let (title, _) = m::node_text(&n, &page.agent_names(), None);
    let nways = m::ways(&n).len();
    let kind_name = tr(look.caps).to_lowercase();
    let inside = page.doc.borrow().nodes.iter().filter(|x| x.parent.as_deref() == Some(id)).count();
    let sub = if n.kind == "decide" {
        tr("one pass, without generating text").to_string()
    } else if n.kind == "sandbox" {
        trf("{n} blocks inside · disposable machine", &[("n", inside.to_string())])
    } else if nways > 1 {
        trf("{kind} · {n} ways out", &[("kind", kind_name), ("n", nways.to_string())])
    } else {
        kind_name
    };
    let title_label = header(page, look.glyph, look.tone, if n.kind == "decide" { tr("Decide") } else { &title }, &sub, if n.kind == "sandbox" { tr("SELECTED BLOCK") } else { tr("BLOCK") });
    // (the name follows the block's edits: a tool chosen, an agent, a question typed)
    if n.kind != "decide" {
        *page.insp_title.borrow_mut() = Some((id.to_string(), title_label, None));
    }
    let c = &page.insp_content;
    let tab = page.insp_tab.borrow().clone();
    let tabs_row = caixa(Orientation::Horizontal, 0, "");
    let w = Rc::downgrade(page);
    let tabs = segmented(&[("ajustes", tr("Block settings").into()), ("dados", tr("Data").into()), ("execucoes", tr("Runs").into())], &tab, move |t| {
        if let Some(p) = w.upgrade() {
            *p.insp_tab.borrow_mut() = t.to_string();
            let p2 = p.clone();
            glib::idle_add_local_once(move || build(&p2));
        }
    });
    tabs_row.append(&tabs);
    // (a decision's and a sandbox's panels go straight to their settings, as the design draws them)
    if n.kind != "decide" && n.kind != "sandbox" {
        c.append(&tabs_row);
    }
    // switched off: a run passes its input straight on
    if n.disabled {
        let row = caixa(Orientation::Horizontal, 8, "fluxo-desativado");
        row.append(&ui_icon("olho", "#8E8F99", 14));
        let l = rotulo(tr("Off: a run passes its input straight on"), "fluxo-chave-sub");
        l.set_xalign(0.0);
        l.set_wrap(true);
        l.set_hexpand(true);
        row.append(&l);
        let on = gtk::Button::with_label(tr("Turn on"));
        on.add_css_class("fluxo-adicionar");
        let w = Rc::downgrade(page);
        on.connect_clicked(move |_| {
            if let Some(p) = w.upgrade() {
                p.toggle_disabled();
            }
        });
        row.append(&on);
        c.append(&row);
    }
    // the service's word on this block (kept current as the flow is checked again)
    let l = rotulo(&m::issue_text(page.issues.borrow().get(id).map(String::as_str).unwrap_or("")), "fluxo-problema");
    l.set_xalign(0.0);
    l.set_wrap(true);
    l.set_visible(page.issues.borrow().contains_key(id));
    c.append(&l);
    *page.issue_label.borrow_mut() = Some((id.to_string(), l));
    match tab.as_str() {
        _ if n.kind == "sandbox" => return sandbox_settings(page, &n, inside),
        "dados" if n.kind != "decide" => data_tab(page, &n),
        "execucoes" if n.kind != "decide" => runs_tab(page, &n),
        _ => {
            if n.kind == "decide" {
                decide_settings(page, &n);
            } else {
                settings_tab(page, &n);
            }
        }
    }
    // Duplicar, Remover
    let f = &page.insp_footer;
    let dup = gtk::Button::new();
    dup.add_css_class("central-botao");
    dup.add_css_class("fluxo-rodape-botao");
    let dc = caixa(Orientation::Horizontal, 8, "");
    dc.set_halign(Align::Center);
    dc.append(&ui_icon("compartilhar", "#E2E2E6", 14));
    dc.append(&rotulo(tr("Duplicate it"), ""));
    dup.set_child(Some(&dc));
    dup.set_hexpand(true);
    let w = Rc::downgrade(page);
    dup.connect_clicked(move |_| {
        if let Some(p) = w.upgrade() {
            p.duplicate_selected();
        }
    });
    f.append(&dup);
    f.append(&remove_button(page));
}

fn remove_button(page: &Rc<FlowPage>) -> gtk::Button {
    let rem = gtk::Button::new();
    rem.add_css_class("central-botao");
    rem.add_css_class("perigo");
    rem.add_css_class("fluxo-rodape-botao");
    let rc = caixa(Orientation::Horizontal, 8, "");
    rc.set_halign(Align::Center);
    rc.append(&ui_icon("lixeira", "#EFA2A6", 14));
    rc.append(&rotulo(tr("Remove"), ""));
    rem.set_child(Some(&rc));
    rem.set_hexpand(true);
    let w = Rc::downgrade(page);
    rem.connect_clicked(move |_| {
        if let Some(p) = w.upgrade() {
            p.remove_selected();
        }
    });
    rem
}

/// Each kind's fields: (data key, name, how it is edited).
enum Field {
    Text(&'static str, &'static str, &'static str),
    Long(&'static str, &'static str),
    Number(&'static str, &'static str),
    Agent(&'static str),
    Flow(&'static str),
    Tool(&'static str),
    Json(&'static str, &'static str),
    Choice(&'static str, &'static str, &'static [(&'static str, &'static str)]),
    /// a For each's list: the previous output or one of the flow's variables
    Items(&'static str),
}

const EXPIRIES: &[(&str, &str)] = &[("3600", n_("1 hour")), ("7200", n_("2 hours")), ("28800", n_("8 hours")), ("86400", n_("1 day")), ("604800", n_("7 days"))];
const VOICES: &[(&str, &str)] = &[("female", n_("Female")), ("male", n_("Male"))];
/// What LFM3-Audio transcribes (saved by their English names, as the engine takes them).
const LANGUAGES: &[(&str, &str)] =
    &[("Portuguese", n_("Portuguese")), ("English", n_("English")), ("Spanish", n_("Spanish")), ("French", n_("French")), ("German", n_("German")), ("Italian", n_("Italian"))];

fn fields(kind: &str) -> Vec<Field> {
    use Field::*;
    match kind {
        "agent" => vec![Agent("agent_id"), Long("message", n_("Message (empty: the previous output)")), Text("save_as", n_("Save the answer as"), "classe")],
        "message" => vec![Long("content", n_("Text (with {variables})"))],
        "wait" => vec![Number("seconds", n_("Seconds"))],
        "ask" => vec![Long("message", n_("Question")), Text("save_as", n_("Save the answer as"), "resposta")],
        "for_each" => vec![
            Items("items"),
            Agent("agent_id"),
            Long("message", n_("Message per item ({item}, {indice}, {total})")),
            Number("max_items", n_("At most (items)")),
            Text("save_as", n_("Save the answers as"), "resultados"),
        ],
        "system_action" => vec![Tool("tool"), Json("arguments", n_("Arguments (JSON, with {variables})")), Text("save_as", n_("Save the result as"), "")],
        "approval" => vec![
            Long("question", n_("Question")),
            Long("details", n_("Details")),
            Choice("timeout_s", n_("Expires in"), EXPIRIES),
        ],
        "notify" => vec![Text("title", n_("Title"), ""), Long("body", n_("Text"))],
        "subflow" => vec![Flow("flow_id")],
        "transcribe" => vec![Text("file", n_("Audio file (a variable or a path)"), "{audio}"), Choice("language", n_("Language"), LANGUAGES), Text("save_as", n_("Save the text as"), "")],
        "describe_image" => vec![Text("file", n_("Image (empty: the screen)"), "{imagem}"), Long("question", n_("Question")), Text("save_as", n_("Save the answer as"), "")],
        "speak" => vec![Long("text", n_("Text")), Choice("voice", n_("Voice"), VOICES), Text("out", n_("Save the audio in"), "~/Música/fala.wav")],
        "set_variables" => vec![],
        _ => vec![],
    }
}

fn settings_tab(page: &Rc<FlowPage>, n: &NodeDoc) {
    let c = &page.insp_content;
    c.append(&label(tr("Name")));
    let (auto, _) = m::node_text(&NodeDoc { title: String::new(), ..n.clone() }, &page.agent_names(), None);
    let id = n.id.clone();
    let w = Rc::downgrade(page);
    let name = entry(&n.title, &auto, move |t| {
        if let Some(p) = w.upgrade() {
            p.edit_node(&id, |x| x.title = t.trim().to_string());
        }
    });
    if let Some(t) = page.insp_title.borrow_mut().as_mut().filter(|t| t.0 == n.id) {
        t.2 = Some(name.clone());
    }
    c.append(&name);
    if n.kind == "condition" {
        condition_rules(page, n);
    }
    for f in fields(&n.kind) {
        let id = n.id.clone();
        let w = Rc::downgrade(page);
        let set = move |k: &'static str, v: Value| {
            if let Some(p) = w.upgrade() {
                p.edit_node(&id, |x| {
                    if !x.data.is_object() {
                        x.data = json!({});
                    }
                    if v.is_null() { x.data.as_object_mut().unwrap().remove(k) } else { x.data.as_object_mut().unwrap().insert(k.into(), v) };
                });
            }
        };
        let d = &n.data;
        match f {
            Field::Text(k, name, hint) => {
                c.append(&label(tr(name)));
                c.append(&entry(d[k].as_str().unwrap_or(""), hint, move |t| set(k, if t.is_empty() { Value::Null } else { json!(t) })));
            }
            Field::Long(k, name) => {
                c.append(&label(tr(name)));
                c.append(&area(d[k].as_str().unwrap_or(""), false, move |t| set(k, json!(t))));
            }
            Field::Number(k, name) => {
                c.append(&label(tr(name)));
                let cur = d[k].as_u64().map(|x| x.to_string()).unwrap_or_default();
                c.append(&entry(&cur, "", move |t| set(k, t.trim().parse::<u64>().map(|x| json!(x)).unwrap_or(Value::Null))));
            }
            Field::Agent(k) => {
                c.append(&label(tr("Agent")));
                let mut opts: Vec<(String, String)> = vec![(String::new(), tr("— choose —").into())];
                opts.extend(page.agents.borrow().iter().cloned());
                c.append(&dropdown(&opts, d[k].as_str().unwrap_or(""), move |id| set(k, json!(id))));
            }
            Field::Flow(k) => {
                c.append(&label(tr("Flow")));
                let me = page.flow_id.borrow().clone();
                let mut opts: Vec<(String, String)> = vec![(String::new(), tr("— choose —").into())];
                opts.extend(page.flows.borrow().iter().filter(|(i, _)| *i != me).cloned());
                c.append(&dropdown(&opts, d[k].as_str().unwrap_or(""), move |id| set(k, json!(id))));
            }
            Field::Tool(k) => {
                c.append(&label(tr("System action")));
                let cur = d[k].as_str().unwrap_or("").to_string();
                if page.tool_items.borrow().is_empty() {
                    c.append(&entry(&cur, "arquivos.mover", move |t| set(k, json!(t.trim()))));
                } else {
                    // (the picker: the actions with what each does and its level, and the connectors' tools)
                    let b = gtk::Button::new();
                    b.add_css_class("fluxo-escolher-ferr");
                    let row = caixa(Orientation::Horizontal, 8, "");
                    let l = rotulo(if cur.is_empty() || cur == "mcp." { tr("— choose —") } else { &cur }, "fluxo-ferr-nome");
                    l.set_xalign(0.0);
                    l.set_hexpand(true);
                    l.set_ellipsize(gtk::pango::EllipsizeMode::End);
                    row.append(&l);
                    row.append(&ui_icon("chevron-baixo", "#8E8F99", 12));
                    b.set_child(Some(&row));
                    let (w, id) = (Rc::downgrade(page), n.id.clone());
                    b.connect_clicked(move |_| {
                        if let Some(p) = w.upgrade() {
                            tool_picker(&p, &id);
                        }
                    });
                    c.append(&b);
                }
            }
            Field::Items(k) => {
                c.append(&label(tr("Items")));
                let cur = d[k].as_str().unwrap_or("content").to_string();
                let mut opts: Vec<(String, String)> = vec![("content".into(), tr("the previous block's output").into())];
                opts.extend(page.doc.borrow().variable_names().into_iter().map(|v| (format!("var:{v}"), format!("{{{v}}}"))));
                if !opts.iter().any(|o| o.0 == cur) {
                    opts.push((cur.clone(), m::field_text(&cur)));
                }
                c.append(&dropdown(&opts, &cur, move |id| set(k, json!(id))));
            }
            Field::Json(k, name) => {
                c.append(&label(tr(name)));
                let text = serde_json::to_string_pretty(&d[k]).unwrap_or_default();
                c.append(&area(if d[k].is_null() { "" } else { &text }, true, move |t| {
                    if let Ok(v) = serde_json::from_str::<Value>(&t) {
                        set(k, v);
                    }
                }));
            }
            Field::Choice(k, name, opts) => {
                c.append(&label(tr(name)));
                let opts: Vec<(String, String)> = opts.iter().map(|(id, n)| (id.to_string(), tr(n).to_string())).collect();
                let cur = match &d[k] {
                    Value::Number(x) => x.to_string(),
                    Value::String(s) => s.clone(),
                    _ => opts.first().map(|o| o.0.clone()).unwrap_or_default(),
                };
                c.append(&dropdown(&opts, &cur, move |id| set(k, id.parse::<u64>().map(|x| json!(x)).unwrap_or(json!(id)))));
            }
        }
    }
    if n.kind == "subflow" {
        subflow_inputs(page, n);
    }
    if n.kind == "agent" {
        agent_sections(page, n);
    }
    if n.kind == "set_variables" {
        variables_rows(page, n);
    }
    if page.doc.borrow().needs_join(&n.id) {
        c.append(&gold_note(tr("These ways run at the same time and never meet again: add a Join where they should wait for each other.")));
    }
    ways_section(page, n);
    last_run_section(page, n);
}

/// A condition's rules: field, operator and value chips, joined by "e" (or "ou", a click away).
fn condition_rules(page: &Rc<FlowPage>, n: &NodeDoc) {
    let c = &page.insp_content;
    c.append(&label(tr("Rule")));
    let caixa_regra = caixa(Orientation::Vertical, 8, "fluxo-regra");
    let rule = n.data["conditions"].get(0).map(|x| x["data"].clone()).unwrap_or(Value::Null);
    let leaves = m::rule_leaves(&rule);
    let any = rule["match"] == "any";
    let id = n.id.clone();
    let w = Rc::downgrade(page);
    let save = Rc::new(move |leaves: Vec<(String, String, Value)>, any: bool| {
        if let Some(p) = w.upgrade() {
            p.edit_node(&id, |x| {
                let r = m::rule_from_leaves(&leaves, any);
                let mut conds = x.data["conditions"].as_array().cloned().unwrap_or_default();
                if conds.is_empty() {
                    conds.push(json!({"id": "sim", "type": "previous-output"}));
                }
                conds[0]["data"] = r;
                x.data["conditions"] = json!(conds);
            });
        }
    });
    let state = Rc::new(std::cell::RefCell::new((leaves, any)));
    for i in 0..state.borrow().0.len() {
        let row = caixa(Orientation::Horizontal, 6, "");
        if i > 0 {
            let j = gtk::Button::with_label(if any { tr("or") } else { tr("and") });
            j.add_css_class("fluxo-regra-e");
            let (st, sv) = (state.clone(), save.clone());
            j.connect_clicked(move |b| {
                let a = !st.borrow().1;
                st.borrow_mut().1 = a;
                b.set_label(if a { tr("or") } else { tr("and") });
                let s = st.borrow();
                sv(s.0.clone(), s.1);
            });
            row.append(&j);
        }
        let (f, o, v) = state.borrow().0[i].clone();
        let fe = gtk::Entry::new();
        fe.add_css_class("fluxo-chip-campo");
        fe.set_text(&field_to_chip(&f));
        // (as wide as what it holds: "{classe.tipo}" reads whole)
        let n = field_to_chip(&f).chars().count() as i32;
        fe.set_width_chars(n.clamp(4, 14));
        fe.set_max_width_chars(n.clamp(4, 14));
        let (st, sv) = (state.clone(), save.clone());
        fe.connect_changed(move |e| {
            st.borrow_mut().0[i].0 = chip_to_field(&e.text());
            let s = st.borrow();
            sv(s.0.clone(), s.1);
        });
        row.append(&fe);
        let opts: Vec<(String, String)> = m::OPERATORS.iter().map(|(id, name, _)| (id.to_string(), tr(name).to_string())).collect();
        let ve = gtk::Entry::new();
        // ("is filled" and "is empty" compare with nothing)
        ve.set_visible(m::takes_value(&o));
        let (st, sv, ve2) = (state.clone(), save.clone(), ve.clone());
        let od = dropdown(&opts, &o, move |id| {
            ve2.set_visible(m::takes_value(&id));
            st.borrow_mut().0[i].1 = id;
            let s = st.borrow();
            sv(s.0.clone(), s.1);
        });
        od.add_css_class("fluxo-chip-op");
        row.append(&od);
        ve.add_css_class("fluxo-chip-campo");
        ve.set_text(&m::value_text(&v));
        let n = m::value_text(&v).chars().count() as i32;
        ve.set_width_chars(n.clamp(3, 12));
        ve.set_max_width_chars(n.clamp(3, 12));
        let (st, sv) = (state.clone(), save.clone());
        ve.connect_changed(move |e| {
            st.borrow_mut().0[i].2 = chip_to_value(&e.text());
            let s = st.borrow();
            sv(s.0.clone(), s.1);
        });
        row.append(&ve);
        if state.borrow().0.len() > 1 {
            let x = gtk::Button::new();
            x.add_css_class("fluxo-x");
            x.set_child(Some(&ui_icon("fechar", "#8E8F99", 12)));
            x.set_tooltip_text(Some(tr("Remove this rule")));
            let (st, sv, w) = (state.clone(), save.clone(), Rc::downgrade(page));
            x.connect_clicked(move |_| {
                st.borrow_mut().0.remove(i);
                let s = st.borrow();
                sv(s.0.clone(), s.1);
                if let Some(p) = w.upgrade() {
                    glib::idle_add_local_once(move || build(&p));
                }
            });
            row.append(&x);
        }
        caixa_regra.append(&row);
    }
    let add = gtk::Button::new();
    add.add_css_class("fluxo-adicionar");
    let ac = caixa(Orientation::Horizontal, 6, "");
    ac.append(&ui_icon("mais", "#A5A1E2", 14));
    ac.append(&rotulo(tr("Add rule"), ""));
    add.set_child(Some(&ac));
    add.set_halign(Align::Start);
    let (st, sv, w) = (state.clone(), save.clone(), Rc::downgrade(page));
    add.connect_clicked(move |_| {
        st.borrow_mut().0.push(("content".into(), "equals".into(), json!("")));
        let s = st.borrow();
        sv(s.0.clone(), s.1);
        if let Some(p) = w.upgrade() {
            glib::idle_add_local_once(move || build(&p));
        }
    });
    caixa_regra.append(&add);
    c.append(&caixa_regra);
}

/// The ways out: each one's label and where it goes (a list of the other blocks).
fn ways_section(page: &Rc<FlowPage>, n: &NodeDoc) {
    let ways = m::ways(n);
    if ways.is_empty() {
        return;
    }
    let c = &page.insp_content;
    c.append(&label(tr("Ways out")));
    let doc = page.doc.borrow().clone();
    let mut targets: Vec<(String, String)> = vec![(String::new(), tr("— nowhere (the flow ends) —").into())];
    targets.extend(doc.nodes.iter().filter(|x| x.id != n.id).map(|x| (x.id.clone(), page.node_title(&x.id))));
    for way in ways {
        let row = caixa(Orientation::Horizontal, 8, "fluxo-saida");
        // (named as its port on the card: an agent's "when to follow it", an approval's "approved", a score's level)
        let name = m::short_label(&m::way_name(n, way.as_deref()), m::PORT_LABEL_CHARS);
        let chip = rotulo(if name.is_empty() { tr("next") } else { &name }, "fluxo-saida-chip");
        if matches!(way.as_deref(), Some("sim") | Some("true")) || (n.kind != "condition" && way.is_some() && way.as_deref() != Some(m::LOW_CONFIDENCE) && way.as_deref() != Some("não") && way.as_deref() != Some("false") && way.as_deref() != Some(m::ELSE_HANDLE)) {
            chip.add_css_class("sim");
        }
        if way.as_deref() == Some(m::LOW_CONFIDENCE) {
            chip.add_css_class("atencao");
        }
        chip.set_valign(Align::Center);
        row.append(&chip);
        row.append(&rotulo("→", "fluxo-saida-seta"));
        // (the default way may fan out to several blocks; its first one is shown)
        let cur = doc.edges.iter().find(|e| e.source == n.id && e.handle == way).map(|e| e.target.clone()).unwrap_or_default();
        let (w, src, way2) = (Rc::downgrade(page), n.id.clone(), way.clone());
        let d = dropdown(&targets, &cur, move |t| {
            if let Some(p) = w.upgrade() {
                p.edit_flow(|f| f.set_way(&src, way2.as_deref(), Some(t.as_str()).filter(|x| !x.is_empty())));
            }
        });
        d.add_css_class("fluxo-saida-alvo");
        d.set_hexpand(true);
        row.append(&d);
        c.append(&row);
    }
}

/// What the block did in the last run: its output's fields and the way it took.
fn last_run_section(page: &Rc<FlowPage>, n: &NodeDoc) {
    let Some(s) = page.last_step(&n.id) else { return };
    let c = &page.insp_content;
    c.append(&label(tr("In the last run")));
    let mut lines: Vec<String> = Vec::new();
    match serde_json::from_str::<Value>(&s.output) {
        Ok(Value::Object(o)) => {
            for (k, v) in o.iter().take(6) {
                let t = match v {
                    Value::Number(x) => m::decimal(x.as_f64().unwrap_or(0.0), if x.is_i64() { 0 } else { 2 }),
                    v => v.to_string(),
                };
                lines.push(format!("{{{k}}}: {t}"));
            }
        }
        _ if !s.output.trim().is_empty() => lines.push(s.output.chars().take(240).collect()),
        _ => {}
    }
    if let Some(way) = m::way_taken(&s) {
        lines.push(format!("→ {}: {}", tr("went by"), { let l = m::way_label(&n.kind, Some(&way)); if l.is_empty() { way } else { l } }));
    } else if !s.detail.is_empty() && s.detail != s.output {
        lines.push(s.detail.clone());
    }
    if s.status == "failed" {
        lines.push(format!("✕ {}", tr("failed")));
    }
    if !lines.is_empty() {
        c.append(&mono_box(&lines));
    }
}

fn data_tab(page: &Rc<FlowPage>, n: &NodeDoc) {
    let c = &page.insp_content;
    c.append(&label(tr("The block's data (JSON)")));
    let erro = rotulo("", "fluxo-problema");
    erro.set_xalign(0.0);
    erro.set_visible(false);
    let (id, w, e2) = (n.id.clone(), Rc::downgrade(page), erro.clone());
    let a = area(&serde_json::to_string_pretty(&n.data).unwrap_or_default(), true, move |t| match serde_json::from_str::<Value>(&t) {
        Ok(v) => {
            e2.set_visible(false);
            if let Some(p) = w.upgrade() {
                p.edit_node(&id, |x| x.data = v);
            }
        }
        Err(e) => {
            e2.set_text(&e.to_string());
            e2.set_visible(true);
        }
    });
    a.add_css_class("alta");
    c.append(&a);
    c.append(&erro);
    let idl = rotulo(&format!("id: {} · {}", n.id, n.kind), "fluxo-chave-sub");
    idl.set_xalign(0.0);
    idl.set_selectable(true);
    c.append(&idl);
}

fn runs_tab(page: &Rc<FlowPage>, n: &NodeDoc) {
    let c = &page.insp_content;
    let recent = page.recent.borrow();
    let mut any = false;
    for (run, steps) in recent.iter() {
        for s in steps.iter().filter(|s| s.node == n.id) {
            any = true;
            let row = caixa(Orientation::Vertical, 2, "fluxo-saida");
            let st = match s.status.as_str() {
                "done" => tr("done"),
                "failed" => tr("failed"),
                _ => tr("running"),
            };
            let t = rotulo(&format!("{} · {st}", crate::fluxo_construtor::when_text(run["created_at"].as_i64().unwrap_or(0))), "fluxo-chave-titulo");
            t.set_xalign(0.0);
            row.append(&t);
            let detail = if s.detail.is_empty() { s.output.clone() } else { s.detail.clone() };
            if !detail.is_empty() {
                let d = rotulo(&detail, "fluxo-chave-sub");
                d.set_xalign(0.0);
                d.set_wrap(true);
                d.set_wrap_mode(gtk::pango::WrapMode::WordChar);
                d.set_lines(3);
                d.set_ellipsize(gtk::pango::EllipsizeMode::End);
                row.append(&d);
            }
            c.append(&row);
        }
    }
    if !any {
        let l = rotulo(tr("This block has not run in the recent runs."), "fluxo-chave-sub");
        l.set_xalign(0.0);
        l.set_wrap(true);
        c.append(&l);
    }
}

// ── Decidir ─────────────────────────────────────────────────────────────────────────────────────────────────────

/// The decision's ready uses: (name, question, type, options).
const USES: &[(&str, &str, &str, &[(&str, &str)])] = &[
    (n_("Is it an invoice?"), "É uma fatura ou conta a pagar?", "noul", &[]),
    (n_("E-mail triage"), "O que fazer com este e-mail?", "choice", &[("responder", "precisa de resposta"), ("arquivar", "só guardar"), ("ignorar", "propaganda ou spam")]),
    (n_("Sort files"), "Que tipo de arquivo é este?", "choice", &[("documento", "PDF, texto, planilha"), ("imagem", "foto ou captura"), ("instalador", ".deb, .AppImage, .exe"), ("outro", "")]),
    (n_("Sensitive content before the cloud"), "Tem dado pessoal sensível (CPF, conta bancária, saúde)?", "noul", &[]),
    (n_("Which agent answers?"), "Para qual agente vai esta notificação?", "choice", &[]),
    (n_("Trigger filter"), "Vale a pena rodar o fluxo para este evento?", "noul", &[]),
];

fn decide_settings(page: &Rc<FlowPage>, n: &NodeDoc) {
    let c = &page.insp_content;
    let d = n.data.clone();
    let route = d["route"].as_str().unwrap_or("rota").to_string();
    let q = d["questions"][&route].clone();
    let kind = q["type"].as_str().unwrap_or("noul").to_string();
    let labels_mode = d["questions"].as_object().is_some_and(|o| o.len() > 1) && kind == "noul";
    let id = n.id.clone();
    let w = Rc::downgrade(page);
    let edit = Rc::new(move |f: &dyn Fn(&mut Value)| {
        if let Some(p) = w.upgrade() {
            p.edit_node(&id, |x| f(&mut x.data));
        }
    });
    let rebuild = {
        let w = Rc::downgrade(page);
        move || {
            if let Some(p) = w.upgrade() {
                glib::idle_add_local_once(move || build(&p));
            }
        }
    };
    c.append(&label(tr("Decision model")));
    let models = vec![("decisao".to_string(), format!("d1-2B · {}", tr("local (text and image)"))), ("decisao-omni".to_string(), format!("d1-Omni-2B · {}", tr("local (also audio)")))];
    let e = edit.clone();
    c.append(&dropdown(&models, d["model"].as_str().unwrap_or("decisao"), move |m| e(&|d: &mut Value| d["model"] = json!(m))));
    c.append(&label(tr("Question")));
    let (e, r) = (edit.clone(), route.clone());
    let qe = entry(q["instructions"].as_str().unwrap_or(""), tr("What should the model decide?"), move |t| {
        let r = r.clone();
        e(&move |d: &mut Value| d["questions"][&r]["instructions"] = json!(t))
    });
    qe.add_css_class("mono");
    c.append(&qe);
    c.append(&label(tr("Answer type")));
    let cur = if labels_mode { "labels" } else { kind.as_str() };
    let (e, r, rb) = (edit.clone(), route.clone(), rebuild.clone());
    let seg = segmented(&[("noul", tr("Yes/no").into()), ("choice", tr("One of N").into()), ("score", tr("Score").into()), ("labels", tr("Labels").into())], cur, move |t| {
        let (r, t) = (r.clone(), t.to_string());
        e(&move |d: &mut Value| {
            let instr = d["questions"][&r]["instructions"].clone();
            let mut qs = serde_json::Map::new();
            match t.as_str() {
                "choice" => {
                    qs.insert(r.clone(), json!({"type": "choice", "instructions": instr, "criteria": {"sim": null, "nao": null}}));
                }
                "score" => {
                    qs.insert(r.clone(), json!({"type": "score", "instructions": instr, "criteria": ["baixa", "média", "alta"]}));
                }
                "labels" => {
                    qs.insert(r.clone(), json!({"type": "noul", "instructions": instr}));
                    qs.insert("rotulo_2".into(), json!({"type": "noul", "instructions": ""}));
                }
                _ => {
                    qs.insert(r.clone(), json!({"type": "noul", "instructions": instr}));
                }
            }
            d["questions"] = Value::Object(qs);
        });
        rb();
    });
    c.append(&seg);
    // the options, each with the last run's probability
    let last = page.last_step(&n.id).and_then(|s| m::parse_decision(&s, &route));
    let prob = |k: &str| last.as_ref().and_then(|l| l.options.iter().find(|(o, _)| o == k).map(|x| x.1));
    let chosen = last.as_ref().map(|l| l.chosen.clone()).unwrap_or_default();
    c.append(&label(if last.is_some() { tr("Options and the last run") } else { tr("Options") }));
    let opts_box = caixa(Orientation::Vertical, 6, "fluxo-opcoes");
    match (kind.as_str(), labels_mode) {
        ("choice", _) => {
            let crit = q["criteria"].as_object().cloned().unwrap_or_default();
            let keys: Vec<String> = crit.keys().cloned().collect();
            for k in keys.iter() {
                let desc = crit[k].as_str().unwrap_or("").to_string();
                let row = option_row(k, &desc, prob(k), *k == chosen, {
                    let (e, r, old) = (edit.clone(), route.clone(), k.clone());
                    move |nk: String, nd: String| {
                        let (r, old) = (r.clone(), old.clone());
                        e(&move |d: &mut Value| {
                            let c = d["questions"][&r]["criteria"].as_object().cloned().unwrap_or_default();
                            let mut out = serde_json::Map::new();
                            for (kk, vv) in c {
                                if kk == old {
                                    out.insert(if nk.trim().is_empty() { kk } else { nk.trim().to_string() }, if nd.trim().is_empty() { Value::Null } else { json!(nd.trim()) });
                                } else {
                                    out.insert(kk, vv);
                                }
                            }
                            d["questions"][&r]["criteria"] = Value::Object(out);
                        })
                    }
                });
                let (e, r, old, rb) = (edit.clone(), route.clone(), k.clone(), rebuild.clone());
                let x = gtk::Button::new();
                x.add_css_class("fluxo-x");
                x.set_child(Some(&ui_icon("fechar", "#8E8F99", 12)));
                x.set_tooltip_text(Some(tr("Remove this option")));
                x.connect_clicked(move |_| {
                    let (r, old) = (r.clone(), old.clone());
                    e(&move |d: &mut Value| {
                        if let Some(c) = d["questions"][&r]["criteria"].as_object_mut() {
                            c.remove(&old);
                        }
                    });
                    rb();
                });
                row.append(&x);
                opts_box.append(&row);
            }
            let (e, r, rb) = (edit.clone(), route.clone(), rebuild.clone());
            opts_box.append(&add_button(tr("Add option"), move || {
                let r = r.clone();
                e(&move |d: &mut Value| {
                    let c = d["questions"][&r]["criteria"].as_object().cloned().unwrap_or_default();
                    let key = (1..).map(|i| format!("opcao_{i}")).find(|k| !c.contains_key(k)).unwrap();
                    d["questions"][&r]["criteria"][key] = Value::Null;
                });
                rb();
            }));
        }
        ("score", _) => {
            let levels: Vec<String> = q["criteria"].as_array().into_iter().flatten().map(|v| v.as_str().unwrap_or("").to_string()).collect();
            for (i, l) in levels.iter().enumerate() {
                let (e, r) = (edit.clone(), route.clone());
                let row = option_row(&i.to_string(), l, prob(&i.to_string()), i.to_string() == chosen, move |_, nd: String| {
                    let r = r.clone();
                    e(&move |d: &mut Value| d["questions"][&r]["criteria"][i] = json!(nd.trim()))
                });
                opts_box.append(&row);
            }
            let (e, r, rb) = (edit.clone(), route.clone(), rebuild.clone());
            opts_box.append(&add_button(tr("Add level"), move || {
                let r = r.clone();
                e(&move |d: &mut Value| {
                    if let Some(a) = d["questions"][&r]["criteria"].as_array_mut() {
                        if a.len() < 10 {
                            a.push(json!(""));
                        }
                    }
                });
                rb();
            }));
        }
        (_, true) => {
            // labels: one yes/no question each, asked in the same pass
            let qs = d["questions"].as_object().cloned().unwrap_or_default();
            for (k, v) in qs.iter() {
                let (e, old) = (edit.clone(), k.clone());
                // (each label is its own yes/no: its probability of "yes" in the last run)
                let p = page.last_step(&n.id).and_then(|s| serde_json::from_str::<Value>(&s.output).ok()).and_then(|o| o["answers"][k]["noul"].as_f64());
                let row = option_row(k, v["instructions"].as_str().unwrap_or(""), p, false, move |nk: String, nd: String| {
                    let old = old.clone();
                    e(&move |d: &mut Value| {
                        let mut qs = d["questions"].as_object().cloned().unwrap_or_default();
                        let mut q = qs.remove(&old).unwrap_or(json!({"type": "noul"}));
                        q["instructions"] = json!(nd.trim());
                        let key = if nk.trim().is_empty() { old.clone() } else { nk.trim().to_string() };
                        if d["route"] == json!(old) {
                            d["route"] = json!(key);
                        }
                        qs.insert(key, q);
                        d["questions"] = Value::Object(qs);
                    })
                });
                opts_box.append(&row);
            }
            let (e, rb) = (edit.clone(), rebuild.clone());
            opts_box.append(&add_button(tr("Add label"), move || {
                e(&|d: &mut Value| {
                    let qs = d["questions"].as_object().cloned().unwrap_or_default();
                    let key = (2..).map(|i| format!("rotulo_{i}")).find(|k| !qs.contains_key(k)).unwrap();
                    d["questions"][key] = json!({"type": "noul", "instructions": ""});
                });
                rb();
            }));
        }
        _ => {
            for (k, name) in [("true", tr("yes")), ("false", tr("no"))] {
                let row = option_row(name, "", prob(k), k == chosen, |_, _| {});
                opts_box.append(&row);
            }
        }
    }
    c.append(&opts_box);
    // the confidence threshold
    let th = d["threshold"].as_f64().unwrap_or(0.8);
    let lim = caixa(Orientation::Horizontal, 12, "fluxo-limite");
    let ll = rotulo(tr("Minimum confidence"), "fluxo-campo-rotulo");
    lim.append(&ll);
    let sc = gtk::Scale::with_range(Orientation::Horizontal, 0.5, 0.99, 0.01);
    sc.add_css_class("central-escala");
    sc.set_value(th);
    sc.set_hexpand(true);
    lim.append(&sc);
    let val = rotulo(&format!("{}%", (th * 100.0).round()), "fluxo-limite-valor");
    lim.append(&val);
    let e = edit.clone();
    sc.connect_value_changed(move |s| {
        let v = (s.value() * 100.0).round() / 100.0;
        val.set_text(&format!("{}%", (v * 100.0).round()));
        e(&move |d: &mut Value| d["threshold"] = json!(v));
    });
    c.append(&lim);
    // below the threshold: ask the person (an approval block), end, or another block
    c.append(&label(tr("Below the threshold")));
    let doc = page.doc.borrow().clone();
    let cur = doc.edges.iter().find(|e| e.source == n.id && e.handle.as_deref() == Some(m::LOW_CONFIDENCE)).map(|e| e.target.clone());
    let cur_is_approval = cur.as_ref().and_then(|t| doc.node(t)).is_some_and(|x| x.kind == "approval");
    let mut opts = vec![("ask".to_string(), tr("Ask you (with the suggestion)").to_string()), ("end".to_string(), tr("End the flow").to_string())];
    opts.extend(doc.nodes.iter().filter(|x| x.id != n.id && x.kind != "approval").map(|x| (x.id.clone(), format!("{} {}", tr("Go to"), page.node_title(&x.id)))));
    let sel = if cur_is_approval { "ask".to_string() } else { cur.clone().unwrap_or_else(|| "end".into()) };
    let (w, src) = (Rc::downgrade(page), n.id.clone());
    c.append(&dropdown(&opts, &sel, move |t| {
        if let Some(p) = w.upgrade() {
            match t.as_str() {
                "ask" => p.ask_below_threshold(&src),
                "end" => p.edit_flow(|f| f.set_way(&src, Some(m::LOW_CONFIDENCE), None)),
                other => p.edit_flow(|f| f.set_way(&src, Some(m::LOW_CONFIDENCE), Some(other))),
            }
        }
    }));
    // (plain, without a box: design "Bloco Decidir")
    let nota = caixa(Orientation::Horizontal, 8, "fluxo-nota-solta");
    let ic = ui_icon("escudo", "#8E8F99", 14);
    ic.set_valign(Align::Start);
    nota.append(&ic);
    let nt = rotulo(tr("Decide only chooses ways and filters. It never authorises an action: what goes past the user level keeps asking you and kalangod."), "fluxo-nota-texto");
    nt.set_xalign(0.0);
    nt.set_wrap(true);
    nt.set_max_width_chars(44);
    nt.set_hexpand(true);
    nota.append(&nt);
    c.append(&nota);
    c.append(&label(tr("Ready uses")));
    // chips that flow like words (a FlowBox gives every column the widest chip's width: one chip per line)
    let chips = caixa(Orientation::Vertical, 6, "fluxo-usos");
    let mut line = caixa(Orientation::Horizontal, 6, "");
    let mut used = 0;
    let room = if page.narrow.get() { 600 } else { 300 };
    for (name, question, kind, options) in USES {
        let b = gtk::Button::with_label(tr(name));
        b.add_css_class("fluxo-uso");
        let (e, r, rb, w) = (edit.clone(), route.clone(), rebuild.clone(), Rc::downgrade(page));
        b.connect_clicked(move |_| {
            let agents = w.upgrade().map(|p| p.agents.borrow().clone()).unwrap_or_default();
            let r = r.clone();
            e(&move |d: &mut Value| {
                let mut crit = serde_json::Map::new();
                if *kind == "choice" {
                    if options.is_empty() {
                        for (id, n) in agents.iter().take(8) {
                            crit.insert(id.clone(), json!(n));
                        }
                        crit.insert("ignorar".into(), json!("nenhum agente"));
                    } else {
                        for (k, v) in options.iter() {
                            crit.insert(k.to_string(), if v.is_empty() { Value::Null } else { json!(v) });
                        }
                    }
                }
                let q = if *kind == "choice" { json!({"type": "choice", "instructions": question, "criteria": crit}) } else { json!({"type": "noul", "instructions": question}) };
                d["questions"] = json!({ r.clone(): q });
                d["route"] = json!(r);
            });
            rb();
        });
        let w = b.measure(Orientation::Horizontal, -1).1;
        if used > 0 && used + 6 + w > room {
            chips.append(&line);
            line = caixa(Orientation::Horizontal, 6, "");
            used = 0;
        }
        used += if used > 0 { 6 + w } else { w };
        line.append(&b);
    }
    chips.append(&line);
    c.append(&chips);
}

fn add_button(text: &str, on: impl Fn() + 'static) -> gtk::Button {
    let add = gtk::Button::new();
    add.add_css_class("fluxo-adicionar");
    let ac = caixa(Orientation::Horizontal, 6, "");
    ac.append(&ui_icon("mais", "#A5A1E2", 14));
    ac.append(&rotulo(text, ""));
    add.set_child(Some(&ac));
    add.set_halign(Align::Start);
    add.connect_clicked(move |_| on());
    add
}

/// An option of a decision: its id and description (editable), and the last run's probability as a bar.
fn option_row(key: &str, desc: &str, p: Option<f64>, chosen: bool, on_change: impl Fn(String, String) + 'static) -> gtk::Box {
    let row = caixa(Orientation::Horizontal, 8, "fluxo-opcao");
    let k = gtk::Entry::new();
    k.add_css_class("fluxo-opcao-nome");
    k.set_text(key);
    k.set_width_chars(6);
    k.set_max_width_chars(10);
    let dsc = gtk::Entry::new();
    dsc.add_css_class("fluxo-opcao-desc");
    dsc.set_text(desc);
    dsc.set_placeholder_text(Some(tr("description")));
    dsc.set_hexpand(true);
    dsc.set_width_chars(6);
    let on = Rc::new(on_change);
    let (o, d2) = (on.clone(), dsc.clone());
    k.connect_changed(move |k| o(k.text().to_string(), d2.text().to_string()));
    let (o, k2) = (on.clone(), k.clone());
    dsc.connect_changed(move |d| o(k2.text().to_string(), d.text().to_string()));
    row.append(&k);
    row.append(&rotulo("·", "fluxo-saida-seta"));
    row.append(&dsc);
    if let Some(p) = p {
        let bar = gtk::LevelBar::for_interval(0.0, 1.0);
        bar.add_css_class("fluxo-barra");
        if chosen {
            bar.add_css_class("escolhida");
        }
        bar.set_value(p);
        bar.set_size_request(48, -1);
        bar.set_valign(Align::Center);
        row.append(&bar);
        let pct = rotulo(&format!("{}%", (p * 100.0).round()), "fluxo-pct");
        pct.set_width_chars(4);
        pct.set_xalign(1.0);
        row.append(&pct);
    }
    row
}

// ── triggers ────────────────────────────────────────────────────────────────────────────────────────────────────

const TRIGGER_KINDS: &[(&str, &str, &str)] = &[
    ("schedule", n_("Schedule"), n_("every day, weekdays, every N min or cron")),
    ("file", n_("File or folder"), n_("created, changed or removed in a folder")),
    ("notification", n_("Notification"), n_("from an app: Mail, Calendar, Messages…")),
    ("webhook", n_("Local webhook"), n_("an address only on this computer, with a key")),
    ("system", n_("System event"), n_("USB, network, battery, entering a space, login")),
];

fn trigger_panel(page: &Rc<FlowPage>, tid: &str) {
    let Some(t) = page.triggers.borrow().iter().find(|t| t.id == tid).cloned() else { return empty_panel(page) };
    let c = &page.insp_content;
    let kind = t.config["kind"].as_str().unwrap_or("manual").to_string();
    let cab = caixa(Orientation::Horizontal, 8, "");
    cab.append(&ui_icon("raio", "#A5A1E2", 14));
    let caps = rotulo(tr("TRIGGER"), "fluxo-secao-cab");
    caps.set_hexpand(true);
    caps.set_xalign(0.0);
    cab.append(&caps);
    // back to the flow's start (its input fields)
    let back = gtk::Button::new();
    back.add_css_class("fluxo-link");
    let bc = caixa(Orientation::Horizontal, 4, "");
    bc.append(&ui_icon("seta-esquerda", "#A5A1E2", 12));
    bc.append(&rotulo(tr("Flow start"), ""));
    back.set_child(Some(&bc));
    let w = Rc::downgrade(page);
    back.connect_clicked(move |_| {
        if let Some(p) = w.upgrade() {
            *p.insp_tab.borrow_mut() = "ajustes".into();
            glib::idle_add_local_once(move || build(&p));
        }
    });
    cab.append(&back);
    c.append(&cab);
    let tid_s = tid.to_string();
    for (k, name, desc) in TRIGGER_KINDS {
        let b = gtk::Button::new();
        b.add_css_class("fluxo-gatilho-tipo");
        if *k == kind {
            b.add_css_class("ativo");
        }
        let row = caixa(Orientation::Horizontal, 12, "");
        let tl = caixa(Orientation::Horizontal, 0, "fluxo-gatilho-tile");
        tl.set_size_request(28, 28);
        tl.set_valign(Align::Start);
        let g = ui_icon(m::trigger_glyph(k), "#C9CAD1", 16);
        g.set_hexpand(true);
        tl.append(&g);
        tl.set_hexpand(false);
        row.append(&tl);
        let col = caixa(Orientation::Vertical, 2, "");
        let a = rotulo(tr(name), "fluxo-chave-titulo");
        a.set_xalign(0.0);
        col.append(&a);
        let s = rotulo(tr(desc), "fluxo-chave-sub");
        s.set_xalign(0.0);
        s.set_wrap(true);
        s.set_max_width_chars(30);
        col.append(&s);
        row.append(&col);
        b.set_child(Some(&row));
        let (w, id, k) = (Rc::downgrade(page), tid_s.clone(), k.to_string());
        b.connect_clicked(move |_| {
            if let Some(p) = w.upgrade() {
                let fid = p.flow_id.borrow().clone();
                p.edit_trigger(&id, |cfg| *cfg = m::default_trigger(&k, &fid));
                let p2 = p.clone();
                glib::idle_add_local_once(move || build(&p2));
            }
        });
        c.append(&b);
    }
    let name = TRIGGER_KINDS.iter().find(|x| x.0 == kind).map(|x| tr(x.1)).unwrap_or(tr("By hand"));
    c.append(&section(&format!("{} · {}", name.to_uppercase(), tr("SETTINGS"))));
    let w = Rc::downgrade(page);
    let id = tid.to_string();
    let set = Rc::new(move |k: &'static str, v: Value| {
        if let Some(p) = w.upgrade() {
            p.edit_trigger(&id, |cfg| cfg[k] = v);
        }
    });
    let cfg = &t.config;
    match kind.as_str() {
        "schedule" => {
            c.append(&label(tr("Days")));
            let days: Vec<u64> = cfg["days"].as_array().into_iter().flatten().filter_map(|d| d.as_u64()).collect();
            let row = caixa(Orientation::Horizontal, 4, "");
            let names = [n_("Mon"), n_("Tue"), n_("Wed"), n_("Thu"), n_("Fri"), n_("Sat"), n_("Sun")];
            let state = Rc::new(std::cell::RefCell::new(days));
            for (i, nm) in names.iter().enumerate() {
                let b = gtk::ToggleButton::with_label(tr(nm));
                b.add_css_class("fluxo-dia");
                b.set_active(state.borrow().contains(&(i as u64 + 1)));
                let (st, set) = (state.clone(), set.clone());
                b.connect_toggled(move |b| {
                    let d = i as u64 + 1;
                    let mut s = st.borrow_mut();
                    s.retain(|x| *x != d);
                    if b.is_active() {
                        s.push(d);
                        s.sort_unstable();
                    }
                    set("days", json!(*s));
                });
                row.append(&b);
            }
            c.append(&row);
            c.append(&label(tr("Time")));
            let s2 = set.clone();
            c.append(&entry(&format!("{:02}:{:02}", cfg["hour"].as_u64().unwrap_or(9), cfg["minute"].as_u64().unwrap_or(0)), "09:00", move |t| {
                if let Some((h, mi)) = t.trim().split_once(':').and_then(|(h, mi)| Some((h.parse::<u8>().ok()?, mi.parse::<u8>().ok()?))) {
                    s2("hour", json!(h));
                    s2("minute", json!(mi));
                }
            }));
            c.append(&label(tr("Or a cron expression (empty: the days and time above)")));
            let s2 = set.clone();
            c.append(&entry(cfg["cron"].as_str().unwrap_or(""), "*/30 * * * *", move |t| s2("cron", json!(t.trim()))));
        }
        "file" => {
            c.append(&label(tr("Folder")));
            let s2 = set.clone();
            c.append(&entry(cfg["path"].as_str().unwrap_or(""), "~/Downloads", move |t| s2("path", json!(t.trim()))));
            c.append(&label(tr("When a file is")));
            let evs: Vec<String> = cfg["events"].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(str::to_string)).collect();
            let row = caixa(Orientation::Horizontal, 4, "");
            let state = Rc::new(std::cell::RefCell::new(evs));
            for (k, nm) in [("created", n_("created")), ("changed", n_("changed")), ("removed", n_("removed"))] {
                let b = gtk::ToggleButton::with_label(tr(nm));
                b.add_css_class("fluxo-dia");
                b.set_active(state.borrow().iter().any(|x| x == k));
                let (st, s2) = (state.clone(), set.clone());
                b.connect_toggled(move |b| {
                    let mut s = st.borrow_mut();
                    s.retain(|x| x != k);
                    if b.is_active() {
                        s.push(k.to_string());
                    }
                    s2("events", json!(*s));
                });
                row.append(&b);
            }
            c.append(&row);
            c.append(&label(tr("Names (e.g. *.pdf, *.zip)")));
            let s2 = set.clone();
            c.append(&entry(cfg["pattern"].as_str().unwrap_or(""), "*.pdf", move |t| s2("pattern", json!(t.trim()))));
        }
        "notification" => {
            c.append(&label(tr("App")));
            let mut apps = vec![(String::new(), tr("Any app").to_string())];
            for a in ["Correio", "Agenda", "Mensagens", "Arquivos", "Terminal"] {
                apps.push((a.to_string(), a.to_string()));
            }
            let cur = cfg["app"].as_str().unwrap_or("").to_string();
            if !cur.is_empty() && !apps.iter().any(|x| x.0 == cur) {
                apps.push((cur.clone(), cur.clone()));
            }
            let s2 = set.clone();
            c.append(&dropdown(&apps, &cur, move |a| s2("app", json!(a))));
            c.append(&label(tr("Only when")));
            let rb = caixa(Orientation::Vertical, 8, "fluxo-regra");
            let leaves = cfg.get("rule").filter(|r| !r.is_null()).map(m::rule_leaves).unwrap_or_default();
            let state = Rc::new(std::cell::RefCell::new(leaves));
            let s_rule = {
                let s2 = set.clone();
                Rc::new(move |l: &Vec<(String, String, Value)>| s2("rule", if l.is_empty() { Value::Null } else { m::rule_from_leaves(l, false) }))
            };
            for i in 0..state.borrow().len() {
                let row = caixa(Orientation::Horizontal, 6, "");
                let (f, o, v) = state.borrow()[i].clone();
                let fe = gtk::Entry::new();
                fe.add_css_class("fluxo-chip-campo");
                fe.set_text(&field_to_chip(&f));
                fe.set_width_chars(7);
                let (st, sr) = (state.clone(), s_rule.clone());
                fe.connect_changed(move |e| {
                    st.borrow_mut()[i].0 = chip_to_field(&e.text());
                    sr(&st.borrow());
                });
                row.append(&fe);
                let opts: Vec<(String, String)> = m::OPERATORS.iter().map(|(id, name, _)| (id.to_string(), tr(name).to_string())).collect();
                let (st, sr) = (state.clone(), s_rule.clone());
                let od = dropdown(&opts, &o, move |id| {
                    st.borrow_mut()[i].1 = id;
                    sr(&st.borrow());
                });
                od.add_css_class("fluxo-chip-op");
                row.append(&od);
                let ve = gtk::Entry::new();
                ve.add_css_class("fluxo-chip-campo");
                ve.set_text(&match &v {
                    Value::String(s) => s.clone(),
                    v => m::value_text(v),
                });
                ve.set_width_chars(5);
                let (st, sr) = (state.clone(), s_rule.clone());
                ve.connect_changed(move |e| {
                    st.borrow_mut()[i].2 = json!(e.text().to_string());
                    sr(&st.borrow());
                });
                row.append(&ve);
                rb.append(&row);
            }
            let (st, sr, w) = (state.clone(), s_rule.clone(), Rc::downgrade(page));
            rb.append(&add_button(tr("Add rule"), move || {
                st.borrow_mut().push(("var:titulo".into(), "contains".into(), json!("")));
                sr(&st.borrow());
                if let Some(p) = w.upgrade() {
                    glib::idle_add_local_once(move || build(&p));
                }
            }));
            c.append(&rb);
            let s2 = set.clone();
            c.append(&switch_row(tr("One conversation per sender"), tr("messages from the same sender follow the same thread"), cfg["group_by_sender"].as_bool().unwrap_or(false), true, move |v| s2("group_by_sender", json!(v))));
            // (the service always waits for focus mode to end: shown, not changeable)
            c.append(&switch_row(tr("Wait for focus mode to end"), tr("like the routines, it never interrupts focus"), true, false, |_| {}));
        }
        "webhook" => {
            c.append(&label(tr("Key in the vault")));
            let s2 = set.clone();
            c.append(&entry(cfg["key_secret"].as_str().unwrap_or(""), "webhook-faturas", move |t| s2("key_secret", json!(t.trim()))));
            let l = rotulo(tr("Only programs on this computer reach the address, and only with this key."), "fluxo-chave-sub");
            l.set_xalign(0.0);
            l.set_wrap(true);
            c.append(&l);
        }
        "system" => {
            c.append(&label(tr("Event")));
            let evs: Vec<(String, String)> = [("usb", n_("USB device connected")), ("network", n_("Network changed")), ("battery", n_("Battery")), ("space", n_("Entering a space")), ("login", n_("Login"))]
                .iter()
                .map(|(k, n)| (k.to_string(), tr(n).to_string()))
                .collect();
            let s2 = set.clone();
            c.append(&dropdown(&evs, cfg["event"].as_str().unwrap_or("usb"), move |e| s2("event", json!(e))));
            c.append(&label(tr("Only for (a name: the space, the network…)")));
            let s2 = set.clone();
            c.append(&entry(cfg["filter"].as_str().unwrap_or(""), "", move |t| s2("filter", json!(t.trim()))));
        }
        _ => {}
    }
    // the next firings (simulated): a schedule's next times, or the recent events that fired the flow
    let prox = caixa(Orientation::Vertical, 4, "fluxo-proximos");
    prox.set_vexpand(true);
    prox.set_valign(Align::End);
    let cab = rotulo(tr("NEXT FIRINGS (SIMULATED)"), "fluxo-proximos-cab");
    cab.set_xalign(0.0);
    prox.append(&cab);
    let corpo = caixa(Orientation::Vertical, 4, "");
    prox.append(&corpo);
    c.append(&prox);
    if kind == "schedule" {
        let corpo2 = corpo.clone();
        crate::fluxos::call("triggers.preview", json!({"trigger": cfg, "count": 3}), move |r| {
            let lines: Vec<String> = match r {
                Ok(v) => {
                    let issues: Vec<String> = v["issues"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_string)).collect();
                    if issues.is_empty() {
                        v["next"].as_array().into_iter().flatten().filter_map(|t| t.as_i64()).map(crate::fluxo_construtor::when_text).collect()
                    } else {
                        issues
                    }
                }
                Err(e) => vec![e],
            };
            for l in lines {
                let x = rotulo(&l, "fluxo-mono");
                x.set_xalign(0.0);
                corpo2.append(&x);
            }
        });
    } else {
        let recent = page.recent.borrow();
        let mut n = 0;
        for (run, steps) in recent.iter().take(3) {
            let what = steps.first().map(|s| s.input.clone()).unwrap_or_default();
            let what: String = what.chars().take(40).collect();
            let x = rotulo(&format!("{} · {what}", crate::fluxo_construtor::when_text(run["created_at"].as_i64().unwrap_or(0))), "fluxo-mono");
            x.set_xalign(0.0);
            x.set_ellipsize(gtk::pango::EllipsizeMode::End);
            corpo.append(&x);
            n += 1;
        }
        if n == 0 {
            let x = rotulo(tr("nothing has fired this flow yet"), "fluxo-mono");
            x.set_xalign(0.0);
            corpo.append(&x);
        }
    }
    page.insp_footer.append(&remove_button(page));
}

// ── the flow's start: its structured input ──────────────────────────────────────────────────────────────────────

/// "Início do fluxo" (design "Entrada do fluxo"): the trigger that starts it (a click opens its settings), the input
/// fields every run must bring, how the trigger's event fills them, and who calls the flow with them.
fn start_panel(page: &Rc<FlowPage>, tid: Option<&str>) {
    let c = &page.insp_content;
    let cab = caixa(Orientation::Horizontal, 8, "");
    cab.append(&ui_icon("raio", "#A5A1E2", 14));
    cab.append(&rotulo(tr("FLOW START"), "fluxo-secao-cab"));
    c.append(&cab);
    if let Some(t) = tid.and_then(|id| page.triggers.borrow().iter().find(|t| t.id == id).cloned()) {
        let b = gtk::Button::new();
        b.add_css_class("fluxo-inicio-gatilho");
        b.set_tooltip_text(Some(tr("Trigger settings")));
        let row = caixa(Orientation::Horizontal, 12, "");
        row.append(&tile(m::trigger_glyph(t.config["kind"].as_str().unwrap_or("")), m::Tone::Gold, 36));
        let col = caixa(Orientation::Vertical, 2, "");
        col.set_hexpand(true);
        col.set_valign(Align::Center);
        let name = rotulo(&m::trigger_text(&t.config).0, "fluxo-insp-titulo");
        name.set_xalign(0.0);
        name.set_ellipsize(gtk::pango::EllipsizeMode::End);
        name.set_max_width_chars(24);
        col.append(&name);
        let sub = rotulo(&format!("{} · {}", tr("trigger"), m::trigger_summary(&t.config)), "fluxo-chave-sub");
        sub.set_xalign(0.0);
        sub.set_ellipsize(gtk::pango::EllipsizeMode::End);
        col.append(&sub);
        row.append(&col);
        let ch = ui_icon("chevron", "#8E8F99", 14);
        ch.set_valign(Align::Center);
        row.append(&ch);
        b.set_child(Some(&row));
        let w = Rc::downgrade(page);
        b.connect_clicked(move |_| {
            if let Some(p) = w.upgrade() {
                *p.insp_tab.borrow_mut() = "gatilho".into();
                glib::idle_add_local_once(move || build(&p));
            }
        });
        c.append(&b);
    }
    input_sections(page, tid);
}

/// A heading of the start panel ("Entrada do fluxo", "Quem chama este fluxo").
fn heading(t: &str) -> gtk::Label {
    let l = rotulo(t, "fluxo-titulo-secao");
    l.set_xalign(0.0);
    l
}

/// Changes the flow's input fields (the canvas' start line follows; the flow is checked and saved).
fn edit_inputs(page: &Rc<FlowPage>, f: impl FnOnce(&mut Vec<Value>)) {
    page.edit_flow(|d| {
        let mut v = d.inputs();
        f(&mut v);
        d.set_inputs(v);
    });
}

fn input_sections(page: &Rc<FlowPage>, tid: Option<&str>) {
    let c = &page.insp_content;
    let fields = page.doc.borrow().inputs();
    c.append(&heading(tr("Flow input")));
    let s = rotulo(
        if fields.is_empty() {
            tr("No fields: a run receives free text. With fields, every run brings them, checked before it starts: the trigger, another flow, a test or an HTTP/MCP call.")
        } else {
            tr("The fields every run brings: the trigger, another flow, a test or an HTTP/MCP call.")
        },
        "fluxo-chave-sub",
    );
    s.set_xalign(0.0);
    s.set_wrap(true);
    s.set_max_width_chars(36);
    c.append(&s);
    let list = caixa(Orientation::Vertical, 0, "fluxo-campos");
    for (i, f) in fields.iter().enumerate() {
        list.append(&field_row(page, i, f));
    }
    let w = Rc::downgrade(page);
    let add = add_button(tr("Add field"), move || {
        if let Some(p) = w.upgrade() {
            edit_inputs(&p, |v| {
                let name = m::new_field_name(v);
                v.push(json!({"name": name, "kind": "text", "required": false, "description": ""}));
            });
            glib::idle_add_local_once(move || build(&p));
        }
    });
    add.add_css_class("fluxo-campos-mais");
    list.append(&add);
    c.append(&list);
    // the service's word on the fields (a name that is not a variable, a field twice…)
    // (with nothing chosen, the panel lists the flow's problems already)
    let rest: Option<String> = page.issues.borrow().get("").filter(|_| tid.is_some()).map(|e| e.split("; ").filter(|x| !x.starts_with("trigger mapping: ")).collect::<Vec<_>>().join("; ")).filter(|e| !e.is_empty());
    if let Some(e) = rest {
        let l = rotulo(&m::issue_text(&e), "fluxo-problema");
        l.set_xalign(0.0);
        l.set_wrap(true);
        c.append(&l);
    }
    if fields.is_empty() {
        return;
    }
    let names: Vec<String> = fields.iter().filter_map(|f| f["name"].as_str().map(str::to_string)).collect();
    // how the trigger's event fills the fields (`input_map`); none mapped: the event itself is the input
    if tid.is_some() {
        c.append(&heading(tr("From the trigger to the fields")));
        let map = page.doc.borrow().input_map();
        let b = caixa(Orientation::Vertical, 6, "fluxo-mono-caixa");
        let mut marks: Vec<(String, gtk::Label)> = Vec::new();
        // the fields, then mappings left over from a field that is gone
        let mut rows: Vec<String> = names.clone();
        rows.extend(map.keys().filter(|k| !names.contains(k)).cloned());
        for n in &rows {
            let row = caixa(Orientation::Horizontal, 6, "");
            let l = rotulo(&format!("{n} ←"), "fluxo-mono");
            l.set_xalign(0.0);
            row.append(&l);
            let w = Rc::downgrade(page);
            let field = n.clone();
            let e = entry(map.get(n).and_then(Value::as_str).unwrap_or(""), &format!("{{evento.{n}}}"), move |t| {
                if let Some(p) = w.upgrade() {
                    p.edit_flow(|d| d.set_mapping(&field, &t));
                }
            });
            e.remove_css_class("fluxo-campo");
            e.add_css_class("fluxo-mapa");
            e.set_tooltip_text(Some(tr("All empty: the event itself is the input, its keys named as the fields. A single reference such as {evento.de} keeps its type; anything else becomes text.")));
            e.set_hexpand(true);
            gtk::prelude::EntryExt::set_alignment(&e, 1.0);
            row.append(&e);
            b.append(&row);
            let x = rotulo("", "fluxo-problema");
            x.set_xalign(0.0);
            x.set_wrap(true);
            x.set_margin_top(0);
            b.append(&x);
            marks.push((n.clone(), x));
        }
        c.append(&b);
        // the service's word on each row, kept current as the flow is checked
        let w = Rc::downgrade(page);
        let show: Rc<dyn Fn()> = Rc::new(move || {
            let Some(p) = w.upgrade() else { return };
            let issues = m::trigger_map_issues(p.issues.borrow().get("").map(String::as_str).unwrap_or(""));
            for (n, l) in &marks {
                let t: Vec<String> = issues.iter().filter(|i| i.field() == n).map(m::MapIssue::text).collect();
                l.set_text(&t.join("; "));
                l.set_visible(!t.is_empty());
            }
        });
        show();
        *page.on_issues.borrow_mut() = Some(show);

    }
    c.append(&heading(tr("Who calls this flow")));
    let example = m::call_example(&fields);
    for (t, sub) in [
        (tr("Another flow"), tr("the Subflow block shows these fields to fill in").to_string()),
        (tr("Test with data"), tr("opens a form with these fields").to_string()),
        (tr("HTTP and MCP"), example),
    ] {
        let row = caixa(Orientation::Horizontal, 10, "fluxo-quem");
        let ic = ui_icon("check", "#8ED2C3", 14);
        ic.set_valign(Align::Start);
        row.append(&ic);
        let col = caixa(Orientation::Vertical, 2, "");
        col.set_hexpand(true);
        let a = rotulo(t, "fluxo-chave-titulo");
        a.set_xalign(0.0);
        col.append(&a);
        let b = rotulo(&sub, "fluxo-chave-sub");
        b.set_xalign(0.0);
        b.set_wrap(true);
        b.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        b.set_max_width_chars(44);
        col.append(&b);
        row.append(&col);
        c.append(&row);
    }
    // what a call without a required field gets: the service's last refusal, else what it would say
    let note = page.run_refusal.borrow().clone().or_else(|| {
        fields.iter().find(|f| f["required"].as_bool().unwrap_or(false)).and_then(|f| f["name"].as_str()).map(|n| trf("A call without \"{f}\" does not start: invalid input: the field \"{f}\" is missing.", &[("f", n.to_string())]))
    });
    if let Some(n) = note {
        let b = caixa(Orientation::Horizontal, 10, "fluxo-aviso");
        let ic = ui_icon("alerta", "#EFA2A6", 14);
        ic.set_valign(Align::Start);
        b.append(&ic);
        let l = rotulo(&n, "fluxo-aviso-texto");
        l.set_xalign(0.0);
        l.set_wrap(true);
        l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        l.set_max_width_chars(34);
        l.set_hexpand(true);
        b.append(&l);
        c.append(&b);
    }
}

/// A field of the flow's input: name, kind, "obrigatório", and ⋮ to change or remove it.
fn field_row(page: &Rc<FlowPage>, i: usize, f: &Value) -> gtk::Box {
    let row = caixa(Orientation::Horizontal, 6, "fluxo-campo-linha");
    let name = rotulo(f["name"].as_str().unwrap_or(""), "fluxo-campo-nome");
    name.set_xalign(0.0);
    name.set_hexpand(true);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    if let Some(d) = f["description"].as_str().filter(|d| !d.is_empty()) {
        row.set_tooltip_text(Some(d));
    }
    row.append(&name);
    let kind = rotulo(m::field_kind_name(f["kind"].as_str().unwrap_or("text")), "fluxo-saida-chip");
    kind.set_valign(Align::Center);
    row.append(&kind);
    if f["required"].as_bool().unwrap_or(false) {
        let r = rotulo(tr("required"), "fluxo-obrigatorio");
        r.set_valign(Align::Center);
        row.append(&r);
    }
    let more = gtk::MenuButton::new();
    more.add_css_class("fluxo-x");
    more.set_child(Some(&ui_icon("pontos-v", "#8E8F99", 14)));
    more.set_tooltip_text(Some(tr("Change the field")));
    more.set_valign(Align::Center);
    let pop = gtk::Popover::new();
    pop.add_css_class("fluxo-pop");
    let c = caixa(Orientation::Vertical, 0, "fluxo-pop-corpo");
    c.set_size_request(260, -1);
    let set = {
        let w = Rc::downgrade(page);
        Rc::new(move |k: &'static str, v: Value| {
            if let Some(p) = w.upgrade() {
                edit_inputs(&p, |fs| {
                    if let Some(x) = fs.get_mut(i) {
                        x[k] = v;
                    }
                });
            }
        })
    };
    let l = label(tr("Name (a variable: letters, digits, _)"));
    l.set_margin_top(0);
    c.append(&l);
    // (a rename carries the field's trigger mapping along)
    let w = Rc::downgrade(page);
    let ne = entry(f["name"].as_str().unwrap_or(""), "remetente", move |t| {
        if let Some(p) = w.upgrade() {
            p.edit_flow(|d| {
                let mut fs = d.inputs();
                if let Some(x) = fs.get_mut(i) {
                    let old = x["name"].as_str().unwrap_or("").to_string();
                    x["name"] = json!(t.trim());
                    d.set_inputs(fs);
                    d.rename_input(&old, t.trim());
                }
            });
        }
    });
    ne.add_css_class("mono");
    c.append(&ne);
    c.append(&label(tr("Kind")));
    let kinds: Vec<(String, String)> = m::FIELD_KINDS.iter().map(|(k, n)| (k.to_string(), tr(n).to_string())).collect();
    let s2 = set.clone();
    c.append(&dropdown(&kinds, f["kind"].as_str().unwrap_or("text"), move |k| s2("kind", json!(k))));
    c.append(&label(tr("Description")));
    let s2 = set.clone();
    c.append(&entry(f["description"].as_str().unwrap_or(""), "", move |t| s2("description", json!(t.trim()))));
    let s2 = set.clone();
    c.append(&switch_row(tr("Required"), tr("a run without it does not start"), f["required"].as_bool().unwrap_or(false), true, move |v| s2("required", json!(v))));
    let rem = gtk::Button::with_label(tr("Remove field"));
    rem.add_css_class("fluxo-botao");
    rem.add_css_class("perigo");
    rem.set_margin_top(6);
    let (w, pw) = (Rc::downgrade(page), pop.downgrade());
    rem.connect_clicked(move |_| {
        if let Some(pop) = pw.upgrade() {
            pop.popdown();
        }
        if let Some(p) = w.upgrade() {
            p.edit_flow(|d| {
                let mut fs = d.inputs();
                if i < fs.len() {
                    let old = fs.remove(i);
                    d.set_inputs(fs);
                    d.forget_mapping(old["name"].as_str().unwrap_or(""));
                }
            });
        }
    });
    c.append(&rem);
    pop.set_child(Some(&c));
    // (the row shows the changes once the popover closes)
    let w = Rc::downgrade(page);
    pop.connect_closed(move |_| {
        if let Some(p) = w.upgrade() {
            glib::idle_add_local_once(move || build(&p));
        }
    });
    more.set_popover(Some(&pop));
    row.append(&more);
    row
}

/// A Subflow block: the fields the chosen flow expects, each filled from a template over this flow's variables
/// (`input`); none filled: the previous block's output, as a JSON object with those fields.
fn subflow_inputs(page: &Rc<FlowPage>, n: &NodeDoc) {
    let id = n.data["flow_id"].as_str().unwrap_or("").to_string();
    if id.is_empty() {
        return;
    }
    let c = &page.insp_content;
    let holder = caixa(Orientation::Vertical, 0, "");
    c.append(&holder);
    let (w, pw) = (holder.downgrade(), Rc::downgrade(page));
    let node = n.id.clone();
    let input = n.data["input"].as_object().cloned().unwrap_or_default();
    crate::fluxos::call("flows.get", json!({"id": id}), move |r| {
        let (Some(holder), Some(page)) = (w.upgrade(), pw.upgrade()) else { return };
        let Ok(v) = r else { return };
        let fields: Vec<Value> = v["flow"]["inputs"].as_array().cloned().unwrap_or_default();
        holder.append(&section(tr("Subflow input")));
        if fields.is_empty() {
            let l = rotulo(tr("This flow has no input fields: it receives the previous output as text."), "fluxo-chave-sub");
            l.set_xalign(0.0);
            l.set_wrap(true);
            holder.append(&l);
            return;
        }
        let mut marks: Vec<(String, gtk::Label)> = Vec::new();
        let list = caixa(Orientation::Vertical, 0, "fluxo-campos");
        for f in &fields {
            let fname = f["name"].as_str().unwrap_or("").to_string();
            let row = caixa(Orientation::Horizontal, 6, "fluxo-campo-linha");
            let name = rotulo(&fname, "fluxo-campo-nome");
            name.set_xalign(0.0);
            name.set_hexpand(true);
            row.append(&name);
            row.append(&rotulo(m::field_kind_name(f["kind"].as_str().unwrap_or("text")), "fluxo-saida-chip"));
            if f["required"].as_bool().unwrap_or(false) {
                row.append(&rotulo(tr("required"), "fluxo-obrigatorio"));
            }
            list.append(&row);
            let (w, nd, fd) = (Rc::downgrade(&page), node.clone(), fname.clone());
            let e = entry(input.get(&fname).and_then(Value::as_str).unwrap_or(""), &format!("{{{fname}}}"), move |t| {
                if let Some(p) = w.upgrade() {
                    p.edit_node(&nd, |x| {
                        let mut m = x.data["input"].as_object().cloned().unwrap_or_default();
                        m::set_template(&mut m, &fd, &t);
                        if m.is_empty() {
                            x.data.as_object_mut().map(|o| o.remove("input"));
                        } else {
                            x.data["input"] = Value::Object(m);
                        }
                    });
                }
            });
            e.add_css_class("mono");
            e.set_margin_start(10);
            e.set_margin_end(10);
            e.set_margin_bottom(6);
            list.append(&e);
            let x = rotulo("", "fluxo-problema");
            x.set_xalign(0.0);
            x.set_wrap(true);
            x.set_margin_start(10);
            x.set_margin_top(0);
            x.set_margin_bottom(6);
            list.append(&x);
            marks.push((fname, x));
        }
        holder.append(&list);
        let (w, nd) = (Rc::downgrade(&page), node.clone());
        let show: Rc<dyn Fn()> = Rc::new(move || {
            let Some(p) = w.upgrade() else { return };
            let issues: Vec<m::MapIssue> = p.issues.borrow().get(&nd).map(|e| e.split("; ").filter_map(m::MapIssue::parse).collect()).unwrap_or_default();
            for (n, l) in &marks {
                let t: Vec<String> = issues.iter().filter(|i| i.field() == n).map(m::MapIssue::text).collect();
                l.set_text(&t.join("; "));
                l.set_visible(!t.is_empty());
            }
        });
        show();
        // (only while this block is still the one shown)
        if page.selected.borrow().as_deref() == Some(node.as_str()) && page.on_issues.borrow().is_none() {
            *page.on_issues.borrow_mut() = Some(show);
        }
        let l = rotulo(tr("Fill each field from this flow's variables, such as {saida}. All empty: the previous block's output, as a JSON object with these fields."), "fluxo-chave-sub");
        l.set_xalign(0.0);
        l.set_wrap(true);
        l.set_max_width_chars(36);
        l.set_margin_top(8);
        holder.append(&l);
    });
}

// ── several blocks, menus ───────────────────────────────────────────────────────────────────────────────────────

/// The gold note (design "seleção e menus": "Apagar um bloco do meio liga o anterior ao seguinte; Ctrl+Z desfaz.").
fn gold_note(text: &str) -> gtk::Box {
    let b = caixa(Orientation::Horizontal, 8, "fluxo-nota-ouro");
    let i = ui_icon("alerta", "#E5C98C", 14);
    i.set_valign(Align::Start);
    b.append(&i);
    let l = rotulo(text, "fluxo-nota-ouro-texto");
    l.set_xalign(0.0);
    l.set_wrap(true);
    l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    l.set_hexpand(true);
    b.append(&l);
    b
}

/// A row of the selection panel and of the menus: its glyph, what it does and its key.
fn action_row(class: &str, icon: &str, text: &str, keys: &str, danger: bool, on: impl Fn() + 'static) -> gtk::Button {
    let b = gtk::Button::new();
    b.add_css_class(class);
    if danger {
        b.add_css_class("perigo");
    }
    let row = caixa(Orientation::Horizontal, 10, "");
    row.append(&ui_icon(icon, if danger { "#EFA2A6" } else { "#B9BAC4" }, 14));
    let l = rotulo(text, "fluxo-acao-nome");
    l.set_xalign(0.0);
    l.set_hexpand(true);
    row.append(&l);
    if !keys.is_empty() {
        row.append(&rotulo(keys, "fluxo-acao-atalho"));
    }
    b.set_child(Some(&row));
    b.connect_clicked(move |_| on());
    b
}

/// Several blocks chosen (design "seleção e menus"): how many and of which kinds, and what can be done with all of
/// them at once.
fn selection_panel(page: &Rc<FlowPage>) {
    let c = &page.insp_content;
    let ids = page.chosen.borrow().clone();
    let n = ids.len();
    let cab = caixa(Orientation::Horizontal, 8, "");
    cab.append(&ui_icon("lista", "#A5A1E2", 14));
    cab.append(&rotulo(tr("SELECTION"), "fluxo-secao-cab"));
    c.append(&cab);
    let t = rotulo(&trf("{n} blocks selected", &[("n", n.to_string())]), "fluxo-selecao-titulo");
    t.set_xalign(0.0);
    c.append(&t);
    let mut kinds: Vec<String> = Vec::new();
    for id in &ids {
        if let Some(k) = page.doc.borrow().node(id).map(|x| m::kind_name(&x.kind)) {
            if !kinds.contains(&k) {
                kinds.push(k);
            }
        }
    }
    let s = rotulo(&kinds.join(" · "), "fluxo-chave-sub");
    s.set_xalign(0.0);
    s.set_wrap(true);
    c.append(&s);
    let all_off = ids.iter().all(|i| page.doc.borrow().node(i).is_some_and(|x| x.disabled));
    let list = caixa(Orientation::Vertical, 8, "fluxo-selecao-acoes");
    let act = |f: fn(&Rc<FlowPage>)| {
        let w = Rc::downgrade(page);
        move || {
            if let Some(p) = w.upgrade() {
                f(&p);
            }
        }
    };
    list.append(&action_row("fluxo-selecao-acao", "compartilhar", &trf("Duplicate the {n}", &[("n", n.to_string())]), "Ctrl+D", false, act(|p| p.duplicate_selected())));
    list.append(&action_row("fluxo-selecao-acao", "lista", tr("Copy"), "Ctrl+C", false, act(|p| p.copy_selected())));
    list.append(&action_row("fluxo-selecao-acao", "olho", if all_off { tr("Turn on") } else { tr("Turn off") }, "", false, act(|p| p.toggle_disabled())));
    list.append(&action_row("fluxo-selecao-acao", "apps-pontos", tr("Align vertically"), "", false, act(|p| p.align_selected())));
    list.append(&action_row("fluxo-selecao-acao", "lixeira", &trf("Delete the {n}", &[("n", n.to_string())]), "Del", true, act(|p| p.remove_selected())));
    c.append(&list);
    let h = rotulo(tr("Drag to move them together. Shift+click adds or removes a block; dragging on the empty canvas draws the band."), "fluxo-chave-sub");
    h.set_xalign(0.0);
    h.set_wrap(true);
    h.set_margin_top(12);
    c.append(&h);
    let note = gold_note(tr("Deleting a block in the middle connects the one before it to the next one; Ctrl+Z undoes."));
    note.set_margin_top(12);
    c.append(&note);
}

/// The right button's menu (design "seleção e menus"): on a block, what to do with it (or with the blocks chosen
/// with it); on the empty canvas, adding, pasting, organising, choosing all, undoing.
pub fn context_menu(page: &Rc<FlowPage>, id: Option<String>, at: (f64, f64), world: (f64, f64)) {
    // (a block outside the ones chosen becomes the one chosen)
    if let Some(i) = &id {
        if !page.selection().contains(i) {
            page.select(Some(i.clone()));
        }
    }
    let pop = gtk::Popover::new();
    pop.add_css_class("fluxo-pop");
    pop.add_css_class("fluxo-menu");
    pop.set_has_arrow(false);
    pop.set_autohide(true);
    pop.set_position(gtk::PositionType::Bottom);
    pop.set_halign(Align::Start);
    pop.set_parent(&page.canvas.root);
    pop.set_pointing_to(Some(&gtk::gdk::Rectangle::new(at.0 as i32, at.1 as i32, 1, 1)));
    let list = caixa(Orientation::Vertical, 0, "fluxo-menu-lista");
    let pw = pop.downgrade();
    let item = |icon: &str, text: &str, keys: &str, danger: bool, f: Box<dyn Fn(&Rc<FlowPage>)>| {
        let (w, pw) = (Rc::downgrade(page), pw.clone());
        action_row("fluxo-menu-item", icon, text, keys, danger, move || {
            if let Some(p) = pw.upgrade() {
                p.popdown();
            }
            if let Some(page) = w.upgrade() {
                f(&page);
            }
        })
    };
    let sep = || {
        let s = gtk::Separator::new(Orientation::Horizontal);
        s.add_css_class("fluxo-menu-sep");
        s
    };
    if id.is_some() {
        let all_off = page.selection().iter().all(|i| page.doc.borrow().node(i).is_some_and(|x| x.disabled));
        list.append(&item("doc", tr("Rename"), "F2", false, Box::new(|p| p.rename_selected())));
        list.append(&item("compartilhar", tr("Duplicate it"), "Ctrl+D", false, Box::new(|p| p.duplicate_selected())));
        list.append(&item("lista", tr("Copy"), "Ctrl+C", false, Box::new(|p| p.copy_selected())));
        list.append(&item("olho", if all_off { tr("Turn on") } else { tr("Turn off") }, "", false, Box::new(|p| p.toggle_disabled())));
        list.append(&sep());
        list.append(&item("lixeira", tr("Delete"), "Del", true, Box::new(|p| p.remove_selected())));
    } else {
        list.append(&item("mais", tr("Add a block here…"), "/", false, Box::new(move |p| p.add_here(world))));
        list.append(&item("baixar", tr("Paste"), "Ctrl+V", false, Box::new(move |p| p.paste(Some(world)))));
        list.append(&item("apps-pontos", tr("Organise the flow"), "", false, Box::new(|p| p.arrange())));
        list.append(&item("check", tr("Select all"), "Ctrl+A", false, Box::new(|p| p.select_all())));
        list.append(&sep());
        list.append(&item("seta-esquerda", tr("Undo"), "Ctrl+Z", false, Box::new(|p| p.undo())));
        list.append(&item("seta-direita", tr("Redo"), "Ctrl+Shift+Z", false, Box::new(|p| p.redo())));
    }
    pop.set_child(Some(&list));
    pop.connect_closed(|p| {
        let p = p.clone();
        glib::idle_add_local_once(move || p.unparent());
    });
    pop.popup();
}

// ── the tool picker ─────────────────────────────────────────────────────────────────────────────────────────────

/// A level as its badge says it.
fn level_word(level: &str) -> String {
    match level {
        "observar" => tr("observe").to_string(),
        "usuario" => tr("user").to_string(),
        "sistema" => tr("system").to_string(),
        "total" => tr("full").to_string(),
        l => l.to_string(),
    }
}

/// The system action of a block, picked (design "editor · inspetor", N1/N2): a search, the system's actions with
/// what each does and its level ("AÇÃO DO SISTEMA · 3 DE 82"), and the connectors' tools (a connector opens its own
/// list). Opens over the block on the canvas.
pub fn tool_picker(page: &Rc<FlowPage>, node: &str) {
    let items = page.tool_items.borrow().clone();
    if items.is_empty() {
        return;
    }
    let pop = gtk::Popover::new();
    pop.add_css_class("fluxo-pop");
    pop.add_css_class("fluxo-ferr-pop");
    pop.set_has_arrow(false);
    pop.set_autohide(true);
    pop.set_position(gtk::PositionType::Bottom);
    pop.set_parent(&page.canvas.root);
    page.canvas.reveal(node);
    let r = page.canvas.screen_rect(node).unwrap_or(gtk::gdk::Rectangle::new(page.canvas.root.width() / 2, 80, 1, 1));
    pop.set_pointing_to(Some(&gtk::gdk::Rectangle::new(r.x() + r.width() / 2, r.y() + 40, 1, 1)));
    let body = caixa(Orientation::Vertical, 0, "fluxo-ferr-corpo");
    body.set_size_request(370, -1);
    let busca = caixa(Orientation::Horizontal, 7, "fluxo-busca");
    busca.append(&ui_icon("busca", "#8E8F99", 14));
    let search = gtk::Entry::new();
    search.add_css_class("fluxo-busca-campo");
    search.set_placeholder_text(Some(tr("Search actions…")));
    search.set_hexpand(true);
    busca.append(&search);
    body.append(&busca);
    let list = caixa(Orientation::Vertical, 0, "fluxo-ferr-lista");
    let sc = gtk::ScrolledWindow::new();
    sc.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    sc.set_propagate_natural_height(true);
    sc.set_max_content_height(380);
    sc.set_child(Some(&list));
    body.append(&sc);
    pop.set_child(Some(&body));
    // (the connector open, when one is)
    let open: Rc<std::cell::RefCell<Option<String>>> = Rc::default();
    let render: Rc<std::cell::RefCell<Option<Box<dyn Fn()>>>> = Rc::default();
    let pick = {
        let (w, pw, node) = (Rc::downgrade(page), pop.downgrade(), node.to_string());
        Rc::new(move |tool: String| {
            if let Some(p) = pw.upgrade() {
                p.popdown();
            }
            if let Some(page) = w.upgrade() {
                page.edit_node(&node, |x| {
                    if !x.data.is_object() {
                        x.data = json!({});
                    }
                    x.data["tool"] = json!(tool);
                });
                page.refresh_inspector();
            }
        })
    };
    let tool_row = {
        let pick = pick.clone();
        move |name: &str, shown: &str, desc: &str, level: &str| -> gtk::Button {
            let b = gtk::Button::new();
            b.add_css_class("fluxo-ferr-item");
            let row = caixa(Orientation::Horizontal, 10, "");
            let col = caixa(Orientation::Vertical, 2, "");
            col.set_hexpand(true);
            let n = rotulo(shown, "fluxo-ferr-nome");
            n.set_xalign(0.0);
            n.set_ellipsize(gtk::pango::EllipsizeMode::End);
            col.append(&n);
            let d = rotulo(&m::tool_title(desc), "fluxo-ferr-desc");
            d.set_xalign(0.0);
            d.set_ellipsize(gtk::pango::EllipsizeMode::End);
            d.set_max_width_chars(40);
            col.append(&d);
            row.append(&col);
            if !level.is_empty() {
                let l = rotulo(&level_word(level), "fluxo-nivel");
                l.add_css_class(level);
                l.set_valign(Align::Center);
                row.append(&l);
            }
            b.set_child(Some(&row));
            let (pick, name) = (pick.clone(), name.to_string());
            b.connect_clicked(move |_| pick(name.clone()));
            b
        }
    };
    let group = |t: &str| {
        let g = rotulo(t, "fluxo-paleta-grupo");
        g.set_xalign(0.0);
        g
    };
    let connectors: Vec<String> = {
        let mut v: Vec<String> = items.iter().filter(|i| i.origin == "connector").map(|i| crate::fluxos_ui::tool_prefix(&i.name).to_string()).collect();
        v.dedup();
        v
    };
    {
        let (list, search, open, render2) = (list.clone(), search.clone(), open.clone(), render.clone());
        *render.borrow_mut() = Some(Box::new(move || {
            while let Some(x) = list.first_child() {
                list.remove(&x);
            }
            let q = search.text().to_string();
            let hit = |i: &crate::fluxos_ferramentas::ToolItem| m::palette_matches(&i.name, &i.description, &q);
            // (the first row found stands out: Enter picks it — design "editor · inspetor")
            let mark_first = |list: &gtk::Box| {
                let mut c = list.first_child();
                while let Some(w) = c {
                    if w.has_css_class("fluxo-ferr-item") && !w.has_css_class("voltar") {
                        w.add_css_class("atual");
                        break;
                    }
                    c = w.next_sibling();
                }
            };
            match open.borrow().clone() {
                Some(server) => {
                    let mine: Vec<_> = items.iter().filter(|i| i.origin == "connector" && crate::fluxos_ui::tool_prefix(&i.name) == server).collect();
                    let back = gtk::Button::new();
                    back.add_css_class("fluxo-ferr-item");
                    back.add_css_class("voltar");
                    let row = caixa(Orientation::Horizontal, 8, "");
                    row.append(&ui_icon("seta-esquerda", "#B9BAC4", 12));
                    row.append(&rotulo(tr("All actions"), "fluxo-ferr-desc"));
                    back.set_child(Some(&row));
                    let (open, r) = (open.clone(), render2.clone());
                    back.connect_clicked(move |_| {
                        *open.borrow_mut() = None;
                        let r = r.clone();
                        glib::idle_add_local_once(move || {
                            if let Some(f) = r.borrow().as_ref() {
                                f();
                            }
                        });
                    });
                    list.append(&back);
                    let shown: Vec<_> = mine.iter().filter(|i| hit(i)).collect();
                    list.append(&group(&format!("{} {server} · {}", tr("CONNECTOR"), trf("{x} OF {n}", &[("x", shown.len().to_string()), ("n", mine.len().to_string())]))));
                    for i in shown {
                        let short = i.name.split_once('.').map(|x| x.1).unwrap_or(&i.name);
                        list.append(&tool_row(&i.name, short, &i.description, &i.level));
                    }
                }
                None => {
                    let sys: Vec<_> = items.iter().filter(|i| i.origin != "connector").collect();
                    let shown: Vec<_> = sys.iter().filter(|i| hit(i)).collect();
                    list.append(&group(&format!("{} · {}", tr("SYSTEM ACTION"), trf("{x} OF {n}", &[("x", shown.len().to_string()), ("n", sys.len().to_string())]))));
                    for i in shown.iter().take(60) {
                        list.append(&tool_row(&i.name, &i.name, &i.description, &i.level));
                    }
                    let mut any = false;
                    for server in &connectors {
                        let tools: Vec<_> = items.iter().filter(|i| i.origin == "connector" && crate::fluxos_ui::tool_prefix(&i.name) == server).collect();
                        let Some(first) = tools.iter().find(|i| hit(i)).or(if q.trim().is_empty() || m::palette_matches(server, server, &q) { tools.first() } else { None }) else { continue };
                        if !any {
                            list.append(&group(tr("CONNECTORS (MCP)")));
                            any = true;
                        }
                        let b = gtk::Button::new();
                        b.add_css_class("fluxo-ferr-item");
                        let row = caixa(Orientation::Horizontal, 10, "");
                        let tile = caixa(Orientation::Horizontal, 0, "fluxo-paleta-tile");
                        tile.add_css_class(m::Tone::Accent.class());
                        tile.set_size_request(22, 22);
                        tile.set_valign(Align::Center);
                        let g = ui_icon("ramo", m::Tone::Accent.color(), 13);
                        g.set_hexpand(true);
                        tile.append(&g);
                        row.append(&tile);
                        let col = caixa(Orientation::Vertical, 2, "");
                        col.set_hexpand(true);
                        let short = first.name.split_once('.').map(|x| x.1).unwrap_or(&first.name);
                        let n = rotulo(&format!("{server} · {short}"), "fluxo-ferr-titulo");
                        n.set_xalign(0.0);
                        n.set_ellipsize(gtk::pango::EllipsizeMode::End);
                        col.append(&n);
                        let d = rotulo(&format!("{} {server} · {}", tr("connector"), trf("{n} tools", &[("n", tools.len().to_string())])), "fluxo-ferr-desc");
                        d.set_xalign(0.0);
                        col.append(&d);
                        row.append(&col);
                        row.append(&ui_icon("chevron", "#B9BAC4", 12));
                        b.set_child(Some(&row));
                        let (open, r, server) = (open.clone(), render2.clone(), server.clone());
                        b.connect_clicked(move |_| {
                            *open.borrow_mut() = Some(server.clone());
                            let r = r.clone();
                            glib::idle_add_local_once(move || {
                                if let Some(f) = r.borrow().as_ref() {
                                    f();
                                }
                            });
                        });
                        list.append(&b);
                    }
                    if shown.is_empty() && !any {
                        let l = rotulo(tr("No action with these words."), "fluxo-chave-sub");
                        l.set_xalign(0.0);
                        l.set_margin_top(8);
                        list.append(&l);
                    }
                }
            }
            mark_first(&list);
        }));
    }
    let r = render.clone();
    search.connect_changed(move |_| {
        if let Some(f) = r.borrow().as_ref() {
            f();
        }
    });
    // Enter: the first action found
    let l = list.downgrade();
    search.connect_activate(move |_| {
        let first = l.upgrade().and_then(|l| {
            let mut c = l.first_child();
            while let Some(w) = c {
                if let Some(b) = w.downcast_ref::<gtk::Button>().filter(|b| b.has_css_class("fluxo-ferr-item") && !b.has_css_class("voltar")) {
                    return Some(b.clone());
                }
                c = w.next_sibling();
            }
            None
        });
        if let Some(b) = first {
            b.emit_clicked();
        }
    });
    if let Some(f) = render.borrow().as_ref() {
        f();
    }
    let r = render.clone();
    pop.connect_closed(move |p| {
        // (the closure holds the list that holds it: let it go)
        r.borrow_mut().take();
        let p = p.clone();
        glib::idle_add_local_once(move || p.unparent());
    });
    pop.popup();
    // (once the popover is on screen: the search takes the keys)
    glib::idle_add_local_once(move || {
        search.grab_focus();
    });
}

// ── the Agente block (design "editor · inspetor", N3) ───────────────────────────────────────────────────────────

/// A section's title with its muted words beside it ("Entrada do agente  os campos que o Organizador recebe").
fn titled(t: &str, sub: &str) -> gtk::Box {
    let row = caixa(Orientation::Horizontal, 8, "fluxo-secao-linha");
    let l = rotulo(t, "fluxo-secao-titulo");
    l.set_xalign(0.0);
    row.append(&l);
    if !sub.is_empty() {
        let s = rotulo(sub, "fluxo-chave-sub");
        s.set_ellipsize(gtk::pango::EllipsizeMode::End);
        s.set_xalign(0.0);
        s.set_valign(Align::Baseline);
        row.append(&s);
    }
    row
}

/// "Entrada do agente", "Caminhos", "Seguir todos os caminhos que valerem" and "Se falhar".
fn agent_sections(page: &Rc<FlowPage>, n: &NodeDoc) {
    let c = &page.insp_content;
    let node = n.id.clone();
    let agent = n.data["agent_id"].as_str().unwrap_or("").to_string();
    // the agent's input fields, each filled from a template over the flow's variables (`input`)
    let fields = page.agent_fields.borrow().get(&agent).cloned().unwrap_or_default();
    if !fields.is_empty() {
        let name = page.agents.borrow().iter().find(|a| a.0 == agent).map(|a| a.1.clone()).unwrap_or_default();
        c.append(&titled(tr("Agent input"), &trf("the fields {agent} receives", &[("agent", name)])));
        let input = n.data["input"].as_object().cloned().unwrap_or_default();
        let list = caixa(Orientation::Vertical, 0, "fluxo-campos");
        for f in &fields {
            let fname = f["name"].as_str().unwrap_or("").to_string();
            let kind = f["kind"].as_str().unwrap_or("text");
            let required = f["required"].as_bool().unwrap_or(false);
            let row = caixa(Orientation::Horizontal, 8, "fluxo-entrada-linha");
            let left = caixa(Orientation::Horizontal, 6, "");
            left.set_hexpand(true);
            left.set_size_request(120, -1);
            let nm = rotulo(&fname, "fluxo-campo-nome");
            nm.set_xalign(0.0);
            nm.set_ellipsize(gtk::pango::EllipsizeMode::End);
            left.append(&nm);
            if required {
                let b = rotulo(tr("required"), "fluxo-obrigatorio");
                b.set_valign(Align::Center);
                left.append(&b);
            }
            row.append(&left);
            let cur = input.get(&fname).and_then(Value::as_str).unwrap_or("").to_string();
            let hint = if matches!(kind, "list" | "object") { "{…}".to_string() } else { format!("{{{fname}}}") };
            let (w, nd, fd) = (Rc::downgrade(page), node.clone(), fname.clone());
            let e = gtk::Entry::new();
            e.add_css_class("fluxo-campo");
            e.add_css_class("mono");
            e.set_text(&cur);
            e.set_placeholder_text(Some(&hint));
            e.set_width_chars(12);
            e.set_hexpand(true);
            // (a required field left empty: red, before the service says so)
            if required && cur.trim().is_empty() {
                e.add_css_class("erro");
            }
            e.connect_changed(move |e| {
                let t = e.text().to_string();
                if required && t.trim().is_empty() { e.add_css_class("erro") } else { e.remove_css_class("erro") }
                if let Some(p) = w.upgrade() {
                    p.edit_node(&nd, |x| {
                        let mut m = x.data["input"].as_object().cloned().unwrap_or_default();
                        m::set_template(&mut m, &fd, &t);
                        if m.is_empty() {
                            x.data.as_object_mut().map(|o| o.remove("input"));
                        } else {
                            x.data["input"] = Value::Object(m);
                        }
                    });
                }
            });
            row.append(&e);
            list.append(&row);
        }
        c.append(&list);
    }
    // the ways the agent may choose (it gets a tool to pick one), up to 8
    c.append(&titled(tr("Ways"), tr("when to follow each way out")));
    let routes: Vec<Value> = n.data["routes"].as_array().cloned().unwrap_or_default();
    let list = caixa(Orientation::Vertical, 4, "fluxo-caminhos");
    let save = {
        let (w, nd) = (Rc::downgrade(page), node.clone());
        Rc::new(move |routes: Vec<Value>, rebuild: bool| {
            if let Some(p) = w.upgrade() {
                p.edit_node(&nd, |x| {
                    if routes.is_empty() {
                        x.data.as_object_mut().map(|o| o.remove("routes"));
                    } else {
                        x.data["routes"] = json!(routes);
                    }
                });
                if rebuild {
                    glib::idle_add_local_once(move || build(&p));
                }
            }
        })
    };
    let state = Rc::new(std::cell::RefCell::new(routes.clone()));
    for (i, r) in routes.iter().enumerate() {
        let row = caixa(Orientation::Horizontal, 8, "fluxo-caminho");
        let dot = caixa(Orientation::Horizontal, 0, "fluxo-caminho-ponto");
        dot.set_size_request(6, 6);
        dot.set_valign(Align::Center);
        row.append(&dot);
        let id = gtk::Entry::new();
        id.add_css_class("fluxo-caminho-id");
        id.set_text(r["id"].as_str().unwrap_or(""));
        id.set_width_chars(6);
        id.set_max_width_chars(10);
        let (st, sv) = (state.clone(), save.clone());
        id.connect_changed(move |e| {
            let t: String = e.text().chars().filter(|c| c.is_alphanumeric() || *c == '_').collect();
            st.borrow_mut()[i]["id"] = json!(t);
            st.borrow_mut()[i]["label"] = json!(t);
            sv(st.borrow().clone(), false);
        });
        row.append(&id);
        let d = gtk::Entry::new();
        d.add_css_class("fluxo-caminho-desc");
        d.set_text(r["description"].as_str().unwrap_or(""));
        d.set_placeholder_text(Some(tr("when to follow it")));
        d.set_hexpand(true);
        d.set_width_chars(8);
        let (st, sv) = (state.clone(), save.clone());
        d.connect_changed(move |e| {
            st.borrow_mut()[i]["description"] = json!(e.text().to_string());
            sv(st.borrow().clone(), false);
        });
        row.append(&d);
        let x = gtk::Button::new();
        x.add_css_class("fluxo-x");
        x.set_child(Some(&ui_icon("fechar", "#8E8F99", 12)));
        x.set_tooltip_text(Some(tr("Remove this way")));
        let (st, sv) = (state.clone(), save.clone());
        x.connect_clicked(move |_| {
            st.borrow_mut().remove(i);
            sv(st.borrow().clone(), true);
        });
        row.append(&x);
        list.append(&row);
    }
    if routes.len() < 8 {
        let (st, sv) = (state.clone(), save.clone());
        list.append(&add_button(tr("Add a way (up to 8)"), move || {
            let taken: Vec<String> = st.borrow().iter().filter_map(|r| r["id"].as_str().map(str::to_string)).collect();
            let id = (1..).map(|i| format!("caminho_{i}")).find(|x| !taken.contains(x)).unwrap();
            st.borrow_mut().push(json!({"id": id, "label": id, "description": ""}));
            sv(st.borrow().clone(), true);
        }));
    }
    c.append(&list);
    // (no "follow every way" here: the engine takes one way of an agent's, the one it chose — only a condition
    // follows several)
    // Se falhar: attempts, first wait (doubles each time) and the limit of each attempt
    c.append(&titled(tr("If it fails"), ""));
    let retry = n.data["retry"].clone();
    let grid = caixa(Orientation::Horizontal, 8, "fluxo-tentativas");
    grid.set_homogeneous(true);
    let boxes: Rc<std::cell::RefCell<Vec<gtk::Entry>>> = Rc::default();
    let cols = [
        (tr("Attempts"), retry["max_attempts"].as_u64().map(|x| x.to_string()), "3"),
        (tr("First wait"), retry["initial_delay_s"].as_f64().map(|x| format!("{} s", m::decimal(x, if x.fract() == 0.0 { 0 } else { 1 }))), "2 s"),
        (tr("Limit"), retry["timeout_s"].as_u64().map(|x| format!("{x} s")), "—"),
    ];
    for (name, cur, hint) in cols {
        let col = caixa(Orientation::Vertical, 4, "");
        col.append(&label(name));
        let e = gtk::Entry::new();
        e.add_css_class("fluxo-campo");
        e.set_width_chars(4);
        e.set_text(cur.as_deref().unwrap_or(""));
        e.set_placeholder_text(Some(hint));
        col.append(&e);
        boxes.borrow_mut().push(e);
        grid.append(&col);
    }
    for e in boxes.borrow().iter() {
        let (w, nd, bs) = (Rc::downgrade(page), node.clone(), boxes.clone());
        e.connect_changed(move |_| {
            let v: Vec<String> = bs.borrow().iter().map(|b| b.text().to_string()).collect();
            if let Some(p) = w.upgrade() {
                p.edit_node(&nd, |x| match m::retry_from(&v[0], &v[1], &v[2]) {
                    Some(r) => x.data["retry"] = r,
                    None => {
                        x.data.as_object_mut().map(|o| o.remove("retry"));
                    }
                });
            }
        });
    }
    c.append(&grid);
}

/// A Variables block (N4): one row per variable, "nome = valor" (the value takes {variables}).
fn variables_rows(page: &Rc<FlowPage>, n: &NodeDoc) {
    let c = &page.insp_content;
    c.append(&titled(tr("Variables"), ""));
    let vars: Vec<Value> = n.data["variables"].as_array().cloned().unwrap_or_default();
    let state = Rc::new(std::cell::RefCell::new(vars.clone()));
    let save = {
        let (w, nd) = (Rc::downgrade(page), n.id.clone());
        Rc::new(move |vars: Vec<Value>, rebuild: bool| {
            if let Some(p) = w.upgrade() {
                p.edit_node(&nd, |x| x.data["variables"] = json!(vars));
                if rebuild {
                    glib::idle_add_local_once(move || build(&p));
                }
            }
        })
    };
    let list = caixa(Orientation::Vertical, 6, "fluxo-variaveis");
    for (i, v) in vars.iter().enumerate() {
        let row = caixa(Orientation::Horizontal, 6, "");
        let name = gtk::Entry::new();
        name.add_css_class("fluxo-campo");
        name.add_css_class("mono");
        name.set_text(v["name"].as_str().unwrap_or(""));
        name.set_placeholder_text(Some("nome"));
        name.set_width_chars(8);
        name.set_hexpand(true);
        let (st, sv) = (state.clone(), save.clone());
        name.connect_changed(move |e| {
            st.borrow_mut()[i]["name"] = json!(e.text().to_string());
            sv(st.borrow().clone(), false);
        });
        row.append(&name);
        let op = v["op"].as_str().unwrap_or("set");
        // (the other operations say what they do in place of "=")
        let sign = match op {
            "increment" => "+=",
            "previous" => "←",
            "clear" => "∅",
            _ => "=",
        };
        let eq = rotulo(sign, "fluxo-saida-seta");
        eq.set_tooltip_text(Some(match op {
            "increment" => tr("adds to it"),
            "previous" => tr("the previous block's output"),
            "clear" => tr("clears it"),
            _ => tr("sets it"),
        }));
        row.append(&eq);
        let val = gtk::Entry::new();
        val.add_css_class("fluxo-campo");
        val.add_css_class("mono");
        val.set_text(&match &v["value"] {
            Value::Null => String::new(),
            Value::String(s) => s.clone(),
            x => x.to_string(),
        });
        val.set_width_chars(8);
        val.set_hexpand(true);
        val.set_sensitive(op == "set" || op == "increment");
        let (st, sv) = (state.clone(), save.clone());
        val.connect_changed(move |e| {
            let t = e.text().to_string();
            // (a list or an object typed as JSON stays one; anything else is text with {variables})
            let v = if st.borrow()[i]["op"] == json!("increment") {
                chip_to_value(&t)
            } else {
                serde_json::from_str::<Value>(t.trim()).ok().filter(|v| v.is_array() || v.is_object()).unwrap_or(json!(t))
            };
            st.borrow_mut()[i]["value"] = v;
            sv(st.borrow().clone(), false);
        });
        row.append(&val);
        let x = gtk::Button::new();
        x.add_css_class("fluxo-x");
        x.set_child(Some(&ui_icon("fechar", "#8E8F99", 12)));
        x.set_tooltip_text(Some(tr("Remove this variable")));
        let (st, sv) = (state.clone(), save.clone());
        x.connect_clicked(move |_| {
            st.borrow_mut().remove(i);
            sv(st.borrow().clone(), true);
        });
        row.append(&x);
        list.append(&row);
    }
    let (st, sv) = (state.clone(), save.clone());
    list.append(&add_button(tr("Add a variable"), move || {
        let taken: Vec<String> = st.borrow().iter().filter_map(|r| r["name"].as_str().map(str::to_string)).collect();
        let name = std::iter::once("valor".to_string()).chain((2..).map(|i| format!("valor_{i}"))).find(|x| !taken.contains(x)).unwrap();
        st.borrow_mut().push(json!({"name": name, "op": "set", "value": ""}));
        sv(st.borrow().clone(), true);
    }));
    c.append(&list);
}

// ── Em sandbox (design "Bloco em sandbox") ──────────────────────────────────────────────────────────────────────

fn sandbox_settings(page: &Rc<FlowPage>, n: &NodeDoc, _inside: usize) {
    let c = &page.insp_content;
    let id = n.id.clone();
    let set = {
        let (w, id) = (Rc::downgrade(page), id.clone());
        Rc::new(move |k: &'static str, v: Value| {
            if let Some(p) = w.upgrade() {
                p.edit_node(&id, |x| {
                    if !x.data.is_object() {
                        x.data = json!({});
                    }
                    x.data[k] = v;
                });
            }
        })
    };
    c.append(&label(tr("Machine template")));
    let cur = n.data["template"].as_str().unwrap_or("").to_string();
    let mut opts: Vec<(String, String)> = vec![(String::new(), tr("— choose —").into())];
    opts.extend(page.templates.borrow().iter().cloned());
    if !cur.is_empty() && !opts.iter().any(|o| o.0 == cur) {
        opts.push((cur.clone(), cur.clone()));
    }
    let s = set.clone();
    c.append(&dropdown(&opts, &cur, move |v| s("template", json!(v))));
    if page.templates.borrow().is_empty() {
        let l = rotulo(tr("No VirtualDesk machine found: make one in VirtualDesk to use as the template."), "fluxo-chave-sub");
        l.set_xalign(0.0);
        l.set_wrap(true);
        c.append(&l);
    }
    c.append(&label(tr("Network")));
    let nets: Vec<(&str, String)> = m::SANDBOX_NETWORKS.iter().map(|x| (x.0, tr(x.1).to_string())).collect();
    let s = set.clone();
    c.append(&segmented(&nets, n.data["network"].as_str().unwrap_or("nenhuma"), move |v| s("network", json!(v))));
    c.append(&label(tr("At the end")));
    let ends: Vec<(&str, String)> = m::SANDBOX_ENDS.iter().map(|x| (x.0, tr(x.1).to_string())).collect();
    let s = set.clone();
    c.append(&segmented(&ends, n.data["on_end"].as_str().unwrap_or("apagar"), move |v| s("on_end", json!(v))));
    c.append(&label(tr("Goes into the machine")));
    let s = set.clone();
    let a = area(&m::crossings_text(&n.data["inputs"]), true, move |t| s("inputs", m::parse_crossings(&t)));
    a.set_vexpand(false);
    a.set_tooltip_text(Some(tr("One per line: <variable or path> → /folder/in/the/machine/")));
    c.append(&a);
    c.append(&label(tr("Comes back to the computer")));
    let s = set.clone();
    let a = area(&m::crossings_text(&n.data["outputs"]), true, move |t| s("outputs", m::parse_crossings(&t)));
    a.set_vexpand(false);
    a.set_tooltip_text(Some(tr("One per line: /folder/in/the/machine/* → ~/folder/")));
    c.append(&a);
    // (no shared folders: only the declared files cross)
    let share = switch_row(tr("Share folders"), tr("none (default)"), false, false, |_| {});
    share.set_tooltip_text(Some(tr("Only the files declared above go in and come back.")));
    c.append(&share);
    last_run_section(page, n);
    let f = &page.insp_footer;
    let info = caixa(Orientation::Horizontal, 8, "fluxo-sandbox-dica");
    let i = ui_icon("info", "#8E8F99", 14);
    i.set_valign(Align::Start);
    info.append(&i);
    let l = rotulo(tr("Drag blocks into the dashed area. Blocks outside do not see the machine; only the declared outputs pass. Each file that goes in or comes back asks you every time — an approval on your own machines does not count for a copy — unless you chose “never” for sandbox copies in VirtualDesk › Settings › What the agents can do (copy files)."), "fluxo-chave-sub");
    l.set_xalign(0.0);
    l.set_wrap(true);
    l.set_hexpand(true);
    info.append(&l);
    f.append(&info);
}

/// A kind in one word, for the live log ("agente", "condição", "ação").
pub fn kind_word(kind: &str) -> String {
    tr(m::look(kind).caps).to_lowercase()
}

/// The step of a block in the last run (for "Na última execução").
pub fn last_of(steps: &[Step], node: &str) -> Option<Step> {
    steps.iter().rev().find(|s| s.node == node).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chips_read_and_write_fields_and_values() {
        assert_eq!(field_to_chip("var:classe.tipo"), "{classe.tipo}");
        assert_eq!(chip_to_field("{classe.tipo}"), "var:classe.tipo");
        assert_eq!(chip_to_field(""), "content");
        assert_eq!(chip_to_field("content.valor"), "content.valor");
        assert_eq!(chip_to_value("\"fatura\""), json!("fatura"));
        assert_eq!(chip_to_value("0"), json!(0));
        assert_eq!(chip_to_value("189,90"), json!(189.9));
        assert_eq!(chip_to_value("pdf"), json!("pdf"));
        assert_eq!(chip_to_value("true"), json!(true));
    }

    #[test]
    fn every_kind_with_fields_has_its_data_keys() {
        for k in ["agent", "message", "wait", "ask", "for_each", "system_action", "approval", "notify", "subflow", "transcribe", "describe_image", "speak"] {
            let d = m::default_data(k, "");
            for f in fields(k) {
                let key = match f {
                    Field::Text(k, ..) | Field::Long(k, _) | Field::Number(k, _) | Field::Json(k, _) | Field::Choice(k, ..) => k,
                    Field::Agent(k) | Field::Flow(k) | Field::Tool(k) | Field::Items(k) => k,
                };
                assert!(d.get(key).is_some() || ["save_as", "max_items"].contains(&key), "{k}.{key}");
            }
        }
    }
}
