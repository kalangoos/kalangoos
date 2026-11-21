//! The agent editor (designs "Agente · editor" and "Agente · equipe e memória"): one agent of kalango-fluxos, its
//! kind, instructions, where it thinks, tools, connectors, sub-agents, memory and the advanced settings, saved with
//! `agents.save`; on the right "Testar agora" (a real run of the saved agent through `chat.send`, with kalangod in
//! simulation in the test session) or, for teams, the preview of how the team works.
//!
//! The editor keeps a `Draft` (plain data, tested) and the widgets write into it; saving turns it back into the
//! service's agent. The "Avançado" tab ends with `sandbox_slot`, an empty box for the sandbox section.

use crate::central_agentes::{export_agent, now, Go, Nav};
use crate::fluxos;
use crate::fluxos_ui::{self as ui, Segmented, Tabs};
use crate::superficies::{caixa, rotulo};
use gtk::prelude::*;
use gtk::{glib, Align, Orientation};
use kalango_i18n::{tr, trf};
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

/// Below this window width the side panel is a page of its own.
const NARROW: i32 = 1000;

/// The token limits offered ("Limite por execução").
const TOKEN_LIMITS: [Option<u64>; 6] = [None, Some(5_000), Some(10_000), Some(20_000), Some(50_000), Some(100_000)];

/// The kinds, in the order of the "Que tipo de agente" choice.
const KINDS: [&str; 7] = ["assistant", "sequential", "parallel", "loop", "tasks", "remote", "flow"];

// ---------------------------------------------------------------- the draft (pure)

/// Everything the editor shows, as plain data.
#[derive(Clone, Debug, PartialEq)]
pub struct Draft {
    pub id: String,
    pub name: String,
    pub description: String,
    pub kind: String,
    /// the kind the agent had when opened (its config's unknown fields are kept only for that kind)
    pub original_kind: String,
    pub original_config: Value,
    pub folder_id: Value,
    pub instruction: String,
    /// "auto" (by input) or "fixed"
    pub model_mode: String,
    /// null = the system's local model
    pub model: Value,
    pub fallback: Vec<Value>,
    pub temperature: f64,
    pub max_tokens: Option<u64>,
    pub max_output_tokens: Option<u64>,
    pub reply_with_voice: bool,
    pub allow_cloud: bool,
    /// (tool name, "free" | "ask" | "never")
    pub tools: Vec<(String, String)>,
    /// sub-agents in order (for a coordinator: with their mode)
    pub subs: Vec<String>,
    pub sub_modes: Vec<String>,
    pub output_key: String,
    /// a team's word to every member, on top of its own instructions ("responda numa linha")
    pub member_instruction: String,
    pub conversation: bool,
    pub keep_exchanges: u64,
    pub long_term: bool,
    pub compact_every: u64,
    pub documents: Vec<String>,
    pub max_iterations: u64,
    pub card_url: String,
    pub key_secret: String,
    pub timeout_s: u64,
    pub flow_id: String,
    /// the agent's own icon ({name, color} or {svg, monochrome, color}); null = the default from its name and kind
    pub icon: Value,
    /// the structured input and output (fields {name, kind, required, description}); null = none
    pub input_fields: Value,
    pub output_fields: Value,
    /// a task list's tasks ([{agent, description, expected_output, tools}])
    pub tasks: Value,
    /// where an assistant runs: null = this computer; else its sandbox (`config.sandbox`)
    pub sandbox: Value,
}

impl Draft {
    pub fn new_assistant(id: &str) -> Draft {
        Draft::from_agent(&json!({"id": id, "name": "", "kind": "assistant", "config": {"instruction": ""}}))
    }

    pub fn from_agent(a: &Value) -> Draft {
        let c = &a["config"];
        let kind = ui::kind_of(a).to_string();
        let strs = |v: &Value| v.as_array().into_iter().flatten().filter_map(|s| s.as_str().map(str::to_string)).collect::<Vec<_>>();
        let (subs, sub_modes) = if kind == "assistant" {
            let l: Vec<&Value> = c["sub_agents"].as_array().into_iter().flatten().collect();
            (l.iter().map(|s| s["agent_id"].as_str().unwrap_or("").to_string()).collect(), l.iter().map(|s| s["mode"].as_str().unwrap_or("chat").to_string()).collect())
        } else {
            let s = strs(&c["sub_agents"]);
            let n = s.len();
            (s, vec!["chat".to_string(); n])
        };
        let m = &c["memory"];
        Draft {
            id: a["id"].as_str().unwrap_or("").to_string(),
            name: a["name"].as_str().unwrap_or("").to_string(),
            description: a["description"].as_str().unwrap_or("").to_string(),
            original_kind: kind.clone(),
            kind,
            original_config: c.clone(),
            folder_id: a["folder_id"].clone(),
            instruction: c["instruction"].as_str().unwrap_or("").to_string(),
            model_mode: c["model_mode"].as_str().unwrap_or("auto").to_string(),
            model: c["model"].clone(),
            fallback: c["fallback"].as_array().cloned().unwrap_or_default(),
            temperature: c["settings"]["temperature"].as_f64().unwrap_or(0.2),
            max_tokens: c["max_tokens_per_run"].as_u64(),
            max_output_tokens: c["settings"]["max_output_tokens"].as_u64(),
            reply_with_voice: c["reply_with_voice"].as_bool().unwrap_or(false),
            allow_cloud: c["allow_cloud"].as_bool().unwrap_or(false),
            tools: c["tools"].as_array().into_iter().flatten().map(|t| (t["name"].as_str().unwrap_or("").to_string(), t["permission"].as_str().unwrap_or("ask").to_string())).collect(),
            subs,
            sub_modes,
            output_key: c["output_key"].as_str().unwrap_or("").to_string(),
            member_instruction: c["member_instruction"].as_str().unwrap_or("").to_string(),
            conversation: m["conversation"].as_bool().unwrap_or(true),
            keep_exchanges: m["keep_exchanges"].as_u64().unwrap_or(20),
            long_term: m["long_term"].as_bool().unwrap_or(false),
            compact_every: m["compact_every"].as_u64().unwrap_or(30),
            documents: strs(&c["documents"]),
            max_iterations: c["max_iterations"].as_u64().unwrap_or(5),
            card_url: c["card_url"].as_str().unwrap_or("").to_string(),
            key_secret: c["key_secret"].as_str().unwrap_or("").to_string(),
            timeout_s: c["timeout_s"].as_u64().unwrap_or(300),
            flow_id: c["flow_id"].as_str().unwrap_or("").to_string(),
            icon: a["icon"].clone(),
            input_fields: a["input_fields"].clone(),
            output_fields: a["output_fields"].clone(),
            tasks: if c["tasks"].is_array() { c["tasks"].clone() } else { json!([]) },
            sandbox: if c["sandbox"].is_object() { c["sandbox"].clone() } else { Value::Null },
        }
    }

    /// The agent as its tile shows it (agent_picker::icon_tile).
    pub fn tile_agent(&self) -> Value {
        json!({"name": self.name, "description": self.description, "kind": self.kind, "icon": self.icon})
    }

    /// The agent as the service saves it.
    pub fn to_agent(&self) -> Value {
        let mut c = if self.kind == self.original_kind && self.original_config.is_object() { self.original_config.clone() } else { json!({}) };
        let key = if self.output_key.trim().is_empty() { Value::Null } else { json!(self.output_key.trim()) };
        match self.kind.as_str() {
            "assistant" => {
                let mut settings = c["settings"].as_object().cloned().unwrap_or_default();
                settings.insert("temperature".into(), json!((self.temperature * 10.0).round() / 10.0));
                match self.max_output_tokens {
                    Some(n) => settings.insert("max_output_tokens".into(), json!(n)),
                    None => settings.remove("max_output_tokens"),
                };
                c["instruction"] = json!(self.instruction);
                c["model_mode"] = json!(self.model_mode);
                c["model"] = if self.model_mode == "auto" { Value::Null } else { self.model.clone() };
                c["fallback"] = json!(self.fallback);
                c["settings"] = Value::Object(settings);
                c["tools"] = json!(self.tools.iter().map(|(n, p)| json!({"name": n, "permission": p})).collect::<Vec<_>>());
                c["sub_agents"] = json!(self.subs.iter().enumerate().map(|(i, s)| json!({"agent_id": s, "mode": self.sub_modes.get(i).map(String::as_str).unwrap_or("chat")})).collect::<Vec<_>>());
                c["output_key"] = key;
                c["memory"] = json!({"conversation": self.conversation, "keep_exchanges": self.keep_exchanges, "long_term": self.long_term, "compact_every": self.compact_every});
                c["documents"] = json!(self.documents);
                c["max_tokens_per_run"] = json!(self.max_tokens);
                c["allow_cloud"] = json!(self.allow_cloud);
                c["reply_with_voice"] = json!(self.reply_with_voice);
                if self.sandbox.is_object() {
                    c["sandbox"] = self.sandbox.clone();
                } else if let Some(o) = c.as_object_mut() {
                    o.remove("sandbox");
                }
            }
            "sequential" | "parallel" | "loop" => {
                c["sub_agents"] = json!(self.subs);
                c["output_key"] = key;
                match self.member_instruction.trim() {
                    "" => {
                        if let Some(o) = c.as_object_mut() {
                            o.remove("member_instruction");
                        }
                    }
                    t => c["member_instruction"] = json!(t),
                }
                if self.kind == "loop" {
                    c["max_iterations"] = json!(self.max_iterations);
                }
            }
            "tasks" => {
                c["tasks"] = self.tasks.clone();
                c["output_key"] = key;
            }
            "remote" => {
                c["card_url"] = json!(self.card_url.trim());
                c["key_secret"] = if self.key_secret.is_empty() { Value::Null } else { json!(self.key_secret) };
                c["timeout_s"] = json!(self.timeout_s);
            }
            _ => c["flow_id"] = json!(self.flow_id),
        }
        let mut a = json!({"id": self.id, "name": self.name.trim(), "description": self.description.trim(), "kind": self.kind, "config": c});
        if !self.folder_id.is_null() {
            a["folder_id"] = self.folder_id.clone();
        }
        // (the top-level fields this editor does not change go back as they came)
        let output = if output_allowed(&self.kind) { &self.output_fields } else { &Value::Null };
        for (k, v) in [("icon", &self.icon), ("input_fields", &self.input_fields), ("output_fields", output)] {
            if !v.is_null() && v.as_array().is_none_or(|a| !a.is_empty()) {
                a[k] = v.clone();
            }
        }
        a
    }

    /// Whether the chosen models leave this computer.
    pub fn uses_cloud(&self) -> bool {
        let cloud = |m: &Value| m.is_object() && m["provider"].as_str().unwrap_or("local") != "local";
        (self.model_mode == "fixed" && cloud(&self.model)) || self.fallback.iter().any(cloud)
    }
}

/// What keeps the draft from being saved, by field ("name", "instruction", "subs", "card_url", "flow", "times",
/// "output_key", "model"), before asking the service (which checks the same and more).
pub fn draft_issues(d: &Draft) -> Vec<(&'static str, String)> {
    let mut v: Vec<(&'static str, String)> = Vec::new();
    if d.name.trim().is_empty() || d.name.chars().count() > 80 {
        v.push(("name", tr("Give the agent a name (up to 80 characters).").into()));
    }
    let key_ok = d.output_key.trim().is_empty() || ui::variables(&format!("{{{}}}", d.output_key.trim())).len() == 1;
    if !key_ok {
        v.push(("output_key", tr("The result variable takes letters, digits and _.").into()));
    }
    match d.kind.as_str() {
        "assistant" => {
            if d.instruction.trim().is_empty() {
                v.push(("instruction", tr("Write what the agent does and how.").into()));
            }
            if d.model_mode == "fixed" && d.model.is_object() && d.model["provider"].as_str() != Some("local") && d.model["model"].as_str().unwrap_or("").trim().is_empty() {
                v.push(("model", tr("Write the model's name at the provider.").into()));
            }
        }
        "sequential" | "parallel" | "loop" => {
            if d.subs.is_empty() {
                v.push(("subs", tr("Add at least one sub-agent.").into()));
            }
            if d.kind == "loop" && !(1..=20).contains(&d.max_iterations) {
                v.push(("times", tr("Repeat from 1 to 20 times.").into()));
            }
        }
        "tasks" => {
            if let Some(t) = crate::agent_structured::tasks_issues(&d.tasks).into_iter().next() {
                v.push(("tasks", t));
            }
        }
        "remote" => {
            if !(d.card_url.trim().starts_with("http://") || d.card_url.trim().starts_with("https://")) {
                v.push(("card_url", tr("The address starts with http:// or https://.").into()));
            }
        }
        _ => {
            if d.flow_id.is_empty() {
                v.push(("flow", tr("Choose the flow.").into()));
            }
        }
    }
    if d.subs.contains(&d.id) {
        v.push(("subs", tr("An agent cannot use itself.").into()));
    }
    for f in [&d.input_fields, &d.output_fields] {
        if let Some(t) = crate::agent_structured::fields_issues(f).into_iter().next() {
            v.push(("fields", t));
        }
    }
    v
}

/// The service's reasons (stable English texts) as the person reads them.
pub fn issue_text(msg: &str) -> String {
    msg.split("; ")
        .map(|m| match m {
            "the name has 1 to 80 characters" => tr("Give the agent a name (up to 80 characters).").to_string(),
            "write the instructions" => tr("Write what the agent does and how.").to_string(),
            "this agent may not use cloud models" => tr("This agent may not use cloud models: turn on “Allow cloud models” in Advanced.").to_string(),
            "add at least one sub-agent" => tr("Add at least one sub-agent.").to_string(),
            "the result variable must be a variable name" => tr("The result variable takes letters, digits and _.").to_string(),
            "an agent cannot use itself" => tr("An agent cannot use itself.").to_string(),
            "choose the flow" => tr("Choose the flow.").to_string(),
            "the remote agent's address starts with http:// or https://" => tr("The address starts with http:// or https://.").to_string(),
            "the time limit goes from 1 to 600 s" => tr("The time limit goes from 1 to 600 s.").to_string(),
            "structured output is for assistants and task lists" => tr("Only assistants and task lists have a structured output.").to_string(),
            m if m.starts_with("repeat from 1 to") => tr("Repeat from 1 to 20 times.").to_string(),
            m => m.to_string(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether an agent of this kind can have a structured output (kalango-fluxos: assistants and task lists).
fn output_allowed(kind: &str) -> bool {
    matches!(kind, "assistant" | "tasks")
}

fn kind_note(kind: &str) -> &'static str {
    match kind {
        "remote" => tr("An agent of another machine, through the A2A protocol: it gets the request and answers."),
        "flow" => tr("The agent is a flow: blocks, conditions and approvals in the visual builder."),
        // (design 41: the tasks' own words)
        "tasks" => tr("An assistant talks and uses tools. Sequence, parallel and repetition combine other agents; tasks give each assistant a part of the work; flow opens the visual builder."),
        _ => tr("An assistant talks and uses tools. Sequence, parallel and repetition combine other agents; flow opens the visual builder."),
    }
}

fn mode_note_text(mode: &str, times: u64) -> String {
    match mode {
        "parallel" => tr("Every sub-agent gets the request at the same time; the answers are put together.").to_string(),
        "loop" => trf("The team runs again until one of them says it is done, at most {n} times.", &[("n", times.to_string())]),
        "assistant" => tr("This assistant talks and hands the task to the right sub-agent.").to_string(),
        _ => tr("Each sub-agent gets the previous one's result. The final answer is the last one's.").to_string(),
    }
}

/// The model an agent of this draft thinks with, as its header's chip ("LFM3 local").
fn draft_models(d: &Draft) -> Vec<String> {
    if d.kind != "assistant" {
        return Vec::new();
    }
    vec![if d.model_mode == "auto" { tr("LFM3 local").to_string() } else { ui::model_label(&d.model) }]
}

/// A tool that would have asked for approval in a real run (design 02: "arquivos.apagar pediria aprovação
/// (simulado)"): the engine's `would_ask` entry `{tool, arguments}` → the tool and what it would work on.
pub fn would_ask_step(entry: &Value) -> (String, String) {
    let tool = entry["tool"].as_str().unwrap_or("");
    let args = &entry["arguments"];
    // (what it would work on: the usual argument, or else the first text one — a search's terms, a page's address)
    let mut detail = crate::chat_blocks::main_argument(args);
    if detail.is_empty() {
        detail = args.as_object().and_then(|o| o.values().find_map(|v| v.as_str().map(str::to_string))).unwrap_or_default();
    }
    (tool.to_string(), detail)
}

/// Why a test run failed, as the person reads it. The service, the engine and ADK answer in English (or, for the
/// agent's own fields, already in pt-BR); nothing of theirs reaches the panel raw except that.
pub fn run_error_text(msg: &str) -> String {
    let m = msg.trim();
    let low = m.to_lowercase();
    if let Some(i) = m.find("State variable '") {
        // (a `{variable}` without value is the common one: outside a flow nothing fills it)
        let rest = &m[i + 16..];
        let name = &rest[..rest.find('\'').unwrap_or(rest.len())];
        return trf("The instructions use {name}, which only has a value inside a flow (a block keeps it). Test without it, or run the agent in a flow.", &[("name", format!("{{{name}}}"))]);
    }
    // the agent's input and output fields: the engine's text is pt-BR already
    if m.starts_with("entrada inválida") || m.starts_with("a resposta não seguiu") {
        return crate::chat_blocks::first_upper(m);
    }
    if low.contains("local model not reachable") || low.contains("model.local.unavailable") {
        return tr("The local model is not answering. Start it in Agents › Models and test again.").to_string();
    }
    if low.starts_with("local model") || low.contains("local model (") || low.contains("local model answer") {
        return tr("The local model could not answer this time. Test again in a moment.").to_string();
    }
    if low.contains("no model is available") {
        return tr("No model is available: choose one in Where it thinks, or start the local model.").to_string();
    }
    if low.contains("agent not found") || (low.starts_with("agent ") && low.ends_with("not found")) {
        return tr("This agent (or one it uses) no longer exists. Save it and check its sub-agents.").to_string();
    }
    if low.contains("no tokens to spend") {
        return tr("The limit per run is zero tokens: raise it in Where it thinks.").to_string();
    }
    if low.contains("circular reference") || low.contains("nested too deep") {
        return tr("The agents use each other in a circle (or too deep): check the sub-agents.").to_string();
    }
    if low.contains("is not an assistant") {
        return tr("Only assistants can do a task or use tools: check who does each one.").to_string();
    }
    // a remote (A2A) agent: the other side refused the key, or is not there
    if low.contains("401") || low.contains("403") || low.contains("unauthorized") || low.contains("forbidden") {
        return tr("The remote agent refused the key: choose its key in Key (from the vault).").to_string();
    }
    if low.contains("connection refused") || low.contains("error sending request") || low.contains("dns error") {
        return tr("The remote agent is not answering at this address.").to_string();
    }
    if low.contains("remote (a2a) agents are not available") {
        return tr("Remote (A2A) agents cannot be tested here yet.").to_string();
    }
    if low.contains("flow agent runs through the flow runner") {
        return tr("A flow agent is tested in the flow builder.").to_string();
    }
    if low.contains("kalangod") {
        return tr("The system service is not answering, so the system tools are unavailable. Test again in a moment.").to_string();
    }
    if low.starts_with("kalango-fluxos:") {
        return tr("The agents service is not running. Try again in a moment.").to_string();
    }
    if low.contains("timed out") || low.contains("os error 11") || low.contains("temporarily unavailable") {
        return tr("The test took too long and was stopped.").to_string();
    }
    if low.contains("tool confirmation required") {
        // (an older service: ADK's own confirmation text)
        return tr("A tool would ask for approval; a test does not run it.").to_string();
    }
    tr("The test did not run. Check the agent's settings and test again.").to_string()
}

/// Why the engine stopped a test run (its defence), as the person reads it.
pub fn stop_text(why: &str) -> String {
    let quoted = |s: &str| s.split('"').nth(1).unwrap_or("").to_string();
    if let Some((tool, _)) = why.split_once(" was asked with \"") {
        return trf("Stopped: {tool} would use “{value}”, which came from content read during the task.", &[("tool", tool.to_string()), ("value", quoted(why))]);
    }
    if let Some(rest) = why.strip_prefix("content returned by ") {
        let tool = rest.split(' ').next().unwrap_or("");
        return trf("Stopped: what {tool} returned tried to give orders (“{order}”).", &[("tool", tool.to_string()), ("order", quoted(why))]);
    }
    tr("Stopped by the defence: content read during the task tried to steer it.").to_string()
}


// ---------------------------------------------------------------- the page

pub struct AgentEditor {
    pub root: gtk::Overlay,
    /// the sandbox section's place, first in "Avançado"
    pub sandbox_slot: gtk::Box,
    sandbox: Rc<crate::agent_sandbox::SandboxSection>,
    pub nav: RefCell<Option<Nav>>,
    /// tells the window the agent's name (the breadcrumb "Agentes › …")
    pub on_title: RefCell<Option<Rc<dyn Fn(&str)>>>,
    draft: RefCell<Draft>,
    is_new: Cell<bool>,
    dirty: Cell<bool>,
    filling: Cell<bool>,
    version: Cell<i64>,
    saved_at: Cell<i64>,
    narrow: Cell<bool>,
    // header
    /// the structured parts (agent_structured.rs): the task list, the input and output tables, the test form
    tasks_editor: Rc<crate::agent_structured::TasksEditor>,
    tasks_err: gtk::Label,
    input_table: Rc<crate::agent_structured::FieldsTable>,
    output_table: Rc<crate::agent_structured::FieldsTable>,
    fields_err: gtk::Label,
    test_form: Rc<crate::agent_structured::TestForm>,
    test_note: gtk::Label,
    chat_input: gtk::Box,
    /// the agent's icon on the header's tile, and its picker (design 44)
    icon_picker: Rc<crate::agent_icon_picker::IconPicker>,
    name: gtk::Entry,
    sub_kind: gtk::Label,
    sub_state: gtk::Label,
    sub_version: gtk::Label,
    error: gtk::Label,
    history_box: gtk::Box,
    test_btn: gtk::Button,
    tabs: Rc<Tabs>,
    pages: gtk::Stack,
    // Geral
    kind_seg: Segmented,
    kind_note: gtk::Label,
    assistant_box: gtk::Box,
    team_box: gtk::Box,
    team_summary: gtk::Label,
    remote_box: gtk::Box,
    flow_box: gtk::Box,
    instr: gtk::TextView,
    instr_err: gtk::Label,
    vars: gtk::Box,
    model_dd: gtk::DropDown,
    model_name: gtk::Entry,
    model_err: gtk::Label,
    fallback_dd: gtk::DropDown,
    fallback_name: gtk::Entry,
    routing: gtk::Box,
    voice: gtk::Switch,
    creativity: gtk::Scale,
    creativity_value: gtk::Label,
    limit_dd: gtk::DropDown,
    tools_summary: gtk::Label,
    tools_list: gtk::Box,
    remote_url: gtk::Entry,
    remote_err: gtk::Label,
    remote_key: gtk::DropDown,
    remote_timeout: gtk::SpinButton,
    flow_dd: gtk::DropDown,
    flow_err: gtk::Label,
    // Ferramentas
    catalog_query: gtk::Entry,
    catalog: gtk::Box,
    // Conectores
    connectors_list: gtk::Box,
    // Subagentes
    mode_seg: Segmented,
    mode_note: gtk::Label,
    times_row: gtk::Box,
    times: gtk::SpinButton,
    member_box: gtk::Box,
    member_entry: gtk::Entry,
    output_box: gtk::Box,
    subs_count: gtk::Label,
    subs_list: gtk::Box,
    subs_err: gtk::Label,
    add_sub_box: gtk::Box,
    add_sub: gtk::MenuButton,
    // Memória
    mem_box: gtk::Box,
    mem_note: gtk::Label,
    mem_conv: gtk::Switch,
    mem_conv_sub: gtk::Label,
    mem_long: gtk::Switch,
    mem_compact: gtk::Switch,
    docs: gtk::Box,
    // Avançado
    desc: gtk::Entry,
    output_key: gtk::Entry,
    output_err: gtk::Label,
    max_out: gtk::SpinButton,
    cloud: gtk::Switch,
    // the side
    side: gtk::Box,
    side_stack: gtk::Stack,
    chat_list: gtk::Box,
    chat_scroll: gtk::ScrolledWindow,
    chat_entry: gtk::Entry,
    chat_send: gtk::Button,
    chat_back: gtk::Button,
    session: RefCell<String>,
    preview: gtk::Box,
    main: gtk::Stack,
    /// the test panel's width limit and the form's (see `set_width`)
    side_limit: crate::limite::Limite,
    form: crate::limite::Limite,
    row: gtk::Box,
    holder: gtk::Box,
    // what the service knows
    agents: RefCell<Vec<Value>>,
    system_tools: RefCell<Vec<Value>>,
    custom_tools: RefCell<Vec<Value>>,
    providers: RefCell<Vec<Value>>,
    flows: RefCell<Vec<Value>>,
    secrets: RefCell<Vec<String>>,
    /// the providers behind the model choices (after "automatic" and "local")
    provider_ids: RefCell<Vec<String>>,
}

fn section_title(t: &str) -> gtk::Label {
    ui::left(t, "fx-secao-titulo")
}

fn row_title(title: &str, sub: &str) -> gtk::Box {
    let b = caixa(Orientation::Horizontal, 8, "");
    let t = section_title(title);
    t.set_valign(Align::Baseline);
    b.append(&t);
    if !sub.is_empty() {
        let s = ui::left(sub, "fx-secao-sub");
        s.set_valign(Align::Baseline);
        b.append(&s);
    }
    b
}

fn switch_row(title: &str, sub: &str) -> (gtk::Box, gtk::Switch, gtk::Label) {
    let r = caixa(Orientation::Horizontal, 12, "fx-chave-linha");
    let c = caixa(Orientation::Vertical, 2, "");
    c.set_hexpand(true);
    c.append(&ui::left(title, "fx-chave-titulo"));
    let s = ui::wrapped(sub, "fx-chave-sub");
    c.append(&s);
    r.append(&c);
    let sw = ui::switch();
    sw.set_valign(Align::Center);
    r.append(&sw);
    (r, sw, s)
}

fn spin(min: f64, max: f64, step: f64) -> gtk::SpinButton {
    let s = gtk::SpinButton::with_range(min, max, step);
    s.add_css_class("fx-numero");
    s.set_valign(Align::Center);
    s
}

/// The widest the form gets (frame 02: about 900 px of fields beside the test panel); on wider windows it stays this
/// wide, centred (crate::limite: the limit gives way, so it never holds the window wide).
const FORM_MAX: i32 = 920;

/// The narrowest the test panel is beside the form.
const PANEL_MIN: i32 = 360;

/// Whether the editor shows the test panel as a page of its own (behind "Testar") at this width, with a form that
/// cannot be narrower than `form_min`.
fn is_narrow(width: i32, form_min: i32) -> bool {
    width < NARROW.max(form_min + PANEL_MIN)
}

/// The test panel's width beside the form: frame 02's on usual windows, a little more on wide ones (it holds a
/// conversation), never so much that the form is squeezed.
fn side_width(width: i32) -> i32 {
    ((width as f64 * 0.3) as i32).clamp(ui::panel_width(width), 600)
}

impl AgentEditor {
    pub fn new(window_width: i32) -> Rc<AgentEditor> {
        let pw = ui::panel_width(window_width);
        let root = gtk::Overlay::new();
        let row = caixa(Orientation::Horizontal, 0, "");
        row.set_vexpand(true);
        let main = gtk::Stack::new();
        main.set_hexpand(true);
        main.set_hhomogeneous(false);
        main.set_transition_type(gtk::StackTransitionType::SlideLeftRight);
        main.set_transition_duration(180);
        let page = caixa(Orientation::Vertical, 0, "fx-pagina");
        page.set_hexpand(true);

        // ---- header: tile, name, what and how saved; Histórico, Exportar, Salvar
        let head = caixa(Orientation::Horizontal, 14, "fx-cabecalho");
        let tile = caixa(Orientation::Horizontal, 0, "");
        tile.set_valign(Align::Start);
        head.append(&tile);
        let hc = caixa(Orientation::Vertical, 2, "");
        hc.set_hexpand(true);
        let name = gtk::Entry::new();
        name.add_css_class("fx-titulo-campo");
        name.set_placeholder_text(Some(tr("Agent's name")));
        name.set_hexpand(true);
        hc.append(&name);
        let sub = caixa(Orientation::Horizontal, 0, "");
        let sub_kind = rotulo("", "fx-cabecalho-sub");
        let sub_state = rotulo("", "fx-cabecalho-estado");
        let sub_version = rotulo("", "fx-cabecalho-sub");
        sub.append(&sub_kind);
        sub.append(&rotulo("  ·  ", "fx-cabecalho-sub"));
        sub.append(&sub_state);
        sub.append(&sub_version);
        hc.append(&sub);
        head.append(&hc);
        let history = gtk::MenuButton::new();
        history.add_css_class("fx-menu-botao");
        let hb = caixa(Orientation::Horizontal, 7, "");
        hb.append(&ui::glyph("relogio", ui::TEXT, 14));
        hb.append(&rotulo(tr("History"), ""));
        history.set_child(Some(&hb));
        history.set_valign(Align::Start);
        let hpop = gtk::Popover::new();
        let history_box = caixa(Orientation::Vertical, 2, "");
        hpop.set_child(Some(&history_box));
        history.set_popover(Some(&hpop));
        head.append(&history);
        let export = ui::button(Some("compartilhar"), tr("Export"), false);
        export.set_valign(Align::Start);
        head.append(&export);
        let test_btn = ui::button(Some("tocar"), tr("Test"), false);
        test_btn.set_valign(Align::Start);
        test_btn.set_visible(false);
        head.append(&test_btn);
        let save = ui::button(Some("check"), tr("Save"), true);
        save.set_valign(Align::Start);
        head.append(&save);
        page.append(&head);
        let error = ui::error_label();
        error.set_margin_start(24);
        error.set_margin_end(24);
        page.append(&error);

        let pages = gtk::Stack::new();
        pages.set_vexpand(true);
        pages.set_hhomogeneous(false);
        pages.set_transition_type(gtk::StackTransitionType::Crossfade);
        pages.set_transition_duration(120);
        let ed: Rc<RefCell<Weak<AgentEditor>>> = Rc::default();
        // the header's tile is the icon picker (design 44)
        let e2 = ed.clone();
        let icon_picker = crate::agent_icon_picker::IconPicker::new(move |icon| {
            if let Some(e) = e2.borrow().upgrade() {
                e.draft.borrow_mut().icon = icon;
                e.changed();
            }
        });
        tile.append(&icon_picker.button);
        let e2 = ed.clone();
        let tabs = Tabs::new(
            &[("geral", tr("General")), ("ferramentas", tr("Tools")), ("conectores", tr("Connectors")), ("subagentes", tr("Sub-agents")), ("memoria", tr("Memory")), ("avancado", tr("Advanced"))],
            move |id| {
                if let Some(e) = e2.borrow().upgrade() {
                    e.show_tab(id);
                }
            },
        );
        let tabs_row = caixa(Orientation::Vertical, 0, "fx-abas-linha");
        tabs_row.append(&tabs.root);
        page.append(&tabs_row);
        page.append(&pages);
        let new_page = |id: &str| {
            let b = caixa(Orientation::Vertical, 0, "fx-editor-corpo");
            pages.add_named(&ui::scroll(&b), Some(id));
            b
        };

        // ---- Geral
        let g = new_page("geral");
        g.append(&section_title(tr("What kind of agent")));
        let e2 = ed.clone();
        let kind_seg = Segmented::new(
            &[
                ("assistant".into(), tr("Assistant").into()),
                ("sequential".into(), tr("Sequence").into()),
                ("parallel".into(), tr("Parallel").into()),
                ("loop".into(), tr("Repetition").into()),
                ("tasks".into(), tr("Tasks").into()),
                ("remote".into(), tr("Remote (A2A)").into()),
                ("flow".into(), tr("Flow").into()),
            ],
            "assistant",
            move |id| {
                if let Some(e) = e2.borrow().upgrade().filter(|e| !e.filling.get()) {
                    e.set_kind(id);
                }
            },
        );
        kind_seg.root.set_margin_top(12);
        g.append(&kind_seg.root);
        let kind_note = ui::wrapped("", "fx-nota");
        kind_note.set_margin_top(10);
        g.append(&kind_note);

        // assistant: instructions, variables, where it thinks, tools
        let assistant_box = caixa(Orientation::Vertical, 0, "");
        let ih = row_title(tr("Instructions"), tr("what it is and how it should act"));
        ih.set_margin_top(24);
        assistant_box.append(&ih);
        let instr = gtk::TextView::new();
        instr.add_css_class("fx-instrucoes");
        instr.set_wrap_mode(gtk::WrapMode::WordChar);
        instr.set_top_margin(12);
        instr.set_bottom_margin(12);
        instr.set_left_margin(12);
        instr.set_right_margin(12);
        instr.set_pixels_inside_wrap(2);
        instr.set_pixels_below_lines(6);
        instr.set_size_request(-1, 128);
        instr.buffer().create_tag(Some("var"), &[("foreground", &"#A5A1E2"), ("background", &"#2E2C52"), ("family", &"JetBrains Mono")]);
        let iframe = caixa(Orientation::Vertical, 0, "fx-instrucoes-moldura");
        iframe.set_margin_top(12);
        iframe.append(&instr);
        assistant_box.append(&iframe);
        let instr_err = ui::error_label();
        assistant_box.append(&instr_err);
        let vars = caixa(Orientation::Horizontal, 6, "");
        vars.set_margin_top(10);
        assistant_box.append(&vars);
        let wh = section_title(tr("Where it thinks"));
        wh.set_margin_top(24);
        assistant_box.append(&wh);
        let models_row = caixa(Orientation::Horizontal, 12, "");
        models_row.set_homogeneous(true);
        models_row.set_margin_top(12);
        let mcol = caixa(Orientation::Vertical, 6, "");
        let model_dd = ui::dropdown(&[tr("Automatic by input (local)").to_string()], 0);
        mcol.append(&ui::labelled(tr("Model"), &model_dd));
        let model_name = ui::entry("", tr("model name at the provider"));
        model_name.set_visible(false);
        mcol.append(&model_name);
        let model_err = ui::error_label();
        mcol.append(&model_err);
        models_row.append(&mcol);
        let fcol = caixa(Orientation::Vertical, 6, "");
        let fallback_dd = ui::dropdown(&[tr("none — stay on this computer").to_string()], 0);
        fcol.append(&ui::labelled(tr("If it fails, use"), &fallback_dd));
        let fallback_name = ui::entry("", tr("model name at the provider"));
        fallback_name.set_visible(false);
        fcol.append(&fallback_name);
        models_row.append(&fcol);
        assistant_box.append(&models_row);
        let route_row = caixa(Orientation::Horizontal, 8, "");
        route_row.set_margin_top(12);
        let routing = caixa(Orientation::Horizontal, 8, "");
        for (what, model) in [(tr("Text →"), "LFM3 1.7B"), (tr("Image or screen →"), "LFM3-VL"), (tr("Audio →"), "LFM3-Audio")] {
            routing.append(&rotulo(what, "fx-rotulo-linha"));
            routing.append(&ui::mono_chip(model, "teal"));
        }
        route_row.append(&routing);
        route_row.append(&rotulo(tr("Reply with voice"), "fx-rotulo-linha"));
        let voice = ui::switch();
        route_row.append(&voice);
        assistant_box.append(&route_row);
        let cr = caixa(Orientation::Horizontal, 12, "");
        cr.set_margin_top(12);
        cr.append(&rotulo(tr("Creativity"), "fx-rotulo-linha"));
        let creativity = gtk::Scale::with_range(Orientation::Horizontal, 0.0, 1.0, 0.1);
        creativity.add_css_class("central-escala");
        creativity.set_size_request(300, -1);
        creativity.set_draw_value(false);
        cr.append(&creativity);
        let creativity_value = rotulo("", "fx-ficha-valor");
        cr.append(&creativity_value);
        let sp = caixa(Orientation::Horizontal, 0, "");
        sp.set_hexpand(true);
        cr.append(&sp);
        cr.append(&rotulo(tr("Limit per run"), "fx-rotulo-linha"));
        let limit_dd = ui::dropdown(&TOKEN_LIMITS.iter().map(|n| ui::tokens_label(*n)).collect::<Vec<_>>(), 0);
        limit_dd.add_css_class("fx-escolha-chip");
        limit_dd.set_hexpand(false);
        cr.append(&limit_dd);
        assistant_box.append(&cr);
        let th = caixa(Orientation::Horizontal, 8, "");
        th.set_margin_top(24);
        let tt = section_title(tr("Tools"));
        th.append(&tt);
        let tools_summary = ui::left("", "fx-secao-sub");
        tools_summary.set_hexpand(true);
        th.append(&tools_summary);
        let add_tool = ui::button(Some("mais"), tr("Add"), false);
        th.append(&add_tool);
        assistant_box.append(&th);
        let tools_list = caixa(Orientation::Vertical, 0, "fx-lista");
        tools_list.set_margin_top(12);
        assistant_box.append(&tools_list);
        g.append(&assistant_box);

        // teams: what is in the team
        let team_box = caixa(Orientation::Vertical, 8, "");
        team_box.set_margin_top(24);
        team_box.append(&section_title(tr("Sub-agents")));
        let team_summary = ui::wrapped("", "fx-descricao");
        team_box.append(&team_summary);
        let to_subs = ui::link(tr("Choose and order them in “Sub-agents” ›"));
        to_subs.set_halign(Align::Start);
        team_box.append(&to_subs);
        g.append(&team_box);

        // tasks: who does what, in order (design 41)
        let e2 = ed.clone();
        let tasks_editor = crate::agent_structured::TasksEditor::new(move || {
            if let Some(e) = e2.borrow().upgrade().filter(|e| !e.filling.get()) {
                let v = e.tasks_editor.value();
                e.draft.borrow_mut().tasks = v;
                e.changed();
            }
        });
        tasks_editor.root.set_margin_top(24);
        g.append(&tasks_editor.root);
        let tasks_err = ui::error_label();
        g.append(&tasks_err);

        // remote
        let remote_box = caixa(Orientation::Vertical, 12, "");
        remote_box.set_margin_top(24);
        remote_box.append(&section_title(tr("Where the agent is")));
        let remote_url = ui::mono_entry("", "https://maquina.local/.well-known/agent.json");
        remote_box.append(&ui::labelled(tr("Address of the agent card"), &remote_url));
        let remote_err = ui::error_label();
        remote_box.append(&remote_err);
        let rr = caixa(Orientation::Horizontal, 12, "");
        rr.set_homogeneous(true);
        let remote_key = ui::dropdown(&[tr("no key").to_string()], 0);
        rr.append(&ui::labelled(tr("Key (from the vault)"), &remote_key));
        let remote_timeout = spin(1.0, 600.0, 10.0);
        rr.append(&ui::labelled(tr("Time limit (s)"), &remote_timeout));
        remote_box.append(&rr);
        g.append(&remote_box);

        // flow
        let flow_box = caixa(Orientation::Vertical, 12, "");
        flow_box.set_margin_top(24);
        flow_box.append(&section_title(tr("The flow")));
        let flow_dd = ui::dropdown(&[tr("choose a flow").to_string()], 0);
        flow_box.append(&ui::labelled(tr("Flow"), &flow_dd));
        let flow_err = ui::error_label();
        flow_box.append(&flow_err);
        g.append(&flow_box);

        // ---- Ferramentas: everything the agent may use
        let f = new_page("ferramentas");
        f.append(&row_title(tr("Tools it may use"), tr("the level says what a tool can do; you decide whether it asks first")));
        let (sb, catalog_query) = ui::search(tr("Search tools…"));
        sb.set_margin_top(14);
        sb.set_size_request(320, -1);
        sb.set_halign(Align::Start);
        f.append(&sb);
        let catalog = caixa(Orientation::Vertical, 0, "");
        catalog.set_margin_top(12);
        f.append(&catalog);

        // ---- Conectores
        let k = new_page("conectores");
        k.append(&row_title(tr("Connectors"), tr("app tools through MCP")));
        let connectors_list = caixa(Orientation::Vertical, 0, "fx-lista");
        connectors_list.set_margin_top(12);
        k.append(&connectors_list);
        let manage = ui::link(tr("Manage the connectors in Settings ›"));
        manage.set_halign(Align::Start);
        manage.set_margin_top(12);
        k.append(&manage);

        // ---- Subagentes
        let s = new_page("subagentes");
        s.append(&section_title(tr("How the team works")));
        let e2 = ed.clone();
        let mode_seg = Segmented::new(
            &[("sequential".into(), tr("One after the other").into()), ("parallel".into(), tr("All at the same time").into()), ("loop".into(), tr("Repeat until done").into()), ("assistant".into(), tr("A coordinator decides").into())],
            "sequential",
            move |id| {
                if let Some(e) = e2.borrow().upgrade().filter(|e| !e.filling.get()) {
                    e.set_kind(id);
                }
            },
        );
        mode_seg.root.set_margin_top(12);
        s.append(&mode_seg.root);
        let mode_note = ui::wrapped("", "fx-nota");
        mode_note.set_margin_top(10);
        s.append(&mode_note);
        let times_row = caixa(Orientation::Horizontal, 10, "");
        times_row.set_margin_top(12);
        times_row.append(&rotulo(tr("Repeat at most"), "fx-rotulo-linha"));
        let times = spin(1.0, 20.0, 1.0);
        times_row.append(&times);
        times_row.append(&rotulo(tr("times"), "fx-rotulo-linha"));
        s.append(&times_row);
        // what every member hears in this team, on top of its own instructions (kalango-fluxos' member_instruction)
        let member_box = caixa(Orientation::Vertical, 6, "");
        member_box.set_margin_top(18);
        member_box.append(&ui::left(tr("Every member hears"), "fx-rotulo-linha"));
        let member_entry = ui::entry("", tr("on top of its own instructions, e.g. “answer in one line”"));
        member_box.append(&member_entry);
        member_box.append(&ui::wrapped(tr("Keeps the members' answers short and in one shape (most useful when they work at the same time)."), "fx-nota"));
        s.append(&member_box);
        let sh = caixa(Orientation::Horizontal, 8, "");
        sh.set_margin_top(24);
        let st = section_title(tr("Sub-agents"));
        st.set_valign(Align::Center);
        sh.append(&st);
        let subs_count = ui::left("", "fx-secao-sub");
        subs_count.set_hexpand(true);
        subs_count.set_valign(Align::Center);
        sh.append(&subs_count);
        let add_sub = gtk::MenuButton::new();
        add_sub.add_css_class("fx-menu-botao");
        let ab = caixa(Orientation::Horizontal, 7, "");
        ab.append(&ui::glyph("mais", ui::TEXT, 14));
        ab.append(&rotulo(tr("Add sub-agent"), ""));
        add_sub.set_child(Some(&ab));
        let spop = gtk::Popover::new();
        let add_sub_box = caixa(Orientation::Vertical, 2, "");
        let sscroll = gtk::ScrolledWindow::new();
        sscroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        sscroll.set_propagate_natural_height(true);
        sscroll.set_max_content_height(360);
        sscroll.set_child(Some(&add_sub_box));
        spop.set_child(Some(&sscroll));
        add_sub.set_popover(Some(&spop));
        sh.append(&add_sub);
        s.append(&sh);
        let subs_list = caixa(Orientation::Vertical, 8, "");
        subs_list.set_margin_top(12);
        s.append(&subs_list);
        let subs_err = ui::error_label();
        s.append(&subs_err);

        // ---- Memória
        let m = new_page("memoria");
        m.append(&section_title(tr("Memory and knowledge")));
        let mem_note = ui::wrapped(tr("Memory belongs to each assistant of the team: adjust it in the sub-agents."), "fx-nota");
        mem_note.set_margin_top(10);
        m.append(&mem_note);
        let mem_box = caixa(Orientation::Vertical, 14, "");
        mem_box.set_margin_top(14);
        let (r1, mem_conv, mem_conv_sub) = switch_row(tr("Remember this conversation"), "");
        mem_box.append(&r1);
        let (r2, mem_long, _) = switch_row(tr("Long-term memory"), tr("uses what the system's Memory knows about you"));
        mem_box.append(&r2);
        let (r3, mem_compact, _) = switch_row(tr("Summarise long conversations"), tr("compacts the history every 30 exchanges"));
        mem_box.append(&r3);
        let dbox = caixa(Orientation::Vertical, 8, "fx-ficha");
        let dh = caixa(Orientation::Horizontal, 8, "");
        let dt = ui::left(tr("Reference documents"), "fx-chave-titulo");
        dt.set_hexpand(true);
        dh.append(&dt);
        dh.append(&rotulo(tr("local search"), "fx-teal"));
        dbox.append(&dh);
        let docs = caixa(Orientation::Vertical, 4, "");
        dbox.append(&docs);
        let add_docs = ui::link(tr("+ Add files"));
        add_docs.set_halign(Align::Start);
        dbox.append(&add_docs);
        mem_box.append(&dbox);
        m.append(&mem_box);

        // ---- Avançado
        let a = new_page("avancado");
        // structured input and output (design 42)
        let fields_changed = |e2: Rc<RefCell<Weak<AgentEditor>>>| {
            move || {
                if let Some(e) = e2.borrow().upgrade().filter(|e| !e.filling.get()) {
                    e.read_fields();
                }
            }
        };
        a.append(&row_title(tr("Structured input"), tr("the fields every run must bring")));
        let input_table = crate::agent_structured::FieldsTable::new(fields_changed(ed.clone()));
        input_table.root.set_margin_top(12);
        a.append(&input_table.root);
        let inote = ui::wrapped(tr("The fields fill the variables of the same name in the instructions. Flows, the Chat, HTTP and MCP pass these fields; a run only starts when they check."), "fx-nota");
        inote.set_margin_top(8);
        a.append(&inote);
        // (structured output: assistants and task lists only — kalango-fluxos refuses it on the others)
        let output_box = caixa(Orientation::Vertical, 0, "");
        let oh = caixa(Orientation::Horizontal, 8, "");
        oh.set_margin_top(24);
        let ot = row_title(tr("Structured output"), tr("what the answer must have"));
        ot.set_hexpand(true);
        oh.append(&ot);
        let schema_btn = gtk::MenuButton::new();
        schema_btn.add_css_class("fx-link-botao");
        let sb = caixa(Orientation::Horizontal, 6, "");
        sb.append(&ui::glyph("codigo", ui::ACCENT, 14));
        sb.append(&rotulo(tr("See JSON Schema"), "fx-link"));
        schema_btn.set_child(Some(&sb));
        let schema_pop = gtk::Popover::new();
        let schema_text = rotulo("", "cartao-codigo");
        schema_text.set_selectable(true);
        schema_text.set_xalign(0.0);
        schema_pop.set_child(Some(&schema_text));
        schema_btn.set_popover(Some(&schema_pop));
        oh.append(&schema_btn);
        output_box.append(&oh);
        let output_table = crate::agent_structured::FieldsTable::new(fields_changed(ed.clone()));
        output_table.root.set_margin_top(12);
        output_box.append(&output_table.root);
        let onote = ui::wrapped(tr("The model answers in this format; if it does not, it is corrected and answers again. Flows read the fields as <result>.<field>."), "fx-nota");
        onote.set_margin_top(8);
        output_box.append(&onote);
        a.append(&output_box);
        let fields_err = ui::error_label();
        a.append(&fields_err);
        {
            let (t, ot) = (schema_text.clone(), output_table.clone());
            schema_pop.connect_show(move |_| {
                t.set_text(&serde_json::to_string_pretty(&crate::agent_structured::json_schema(&ot.value())).unwrap_or_default());
            });
        }
        let nt = section_title(tr("Name and result"));
        nt.set_margin_top(24);
        a.append(&nt);
        let desc = ui::entry("", tr("what it does, in one line (the library shows it)"));
        let dl = ui::labelled(tr("Description"), &desc);
        dl.set_margin_top(12);
        a.append(&dl);
        let output_key = ui::mono_entry("", "resumo");
        let ol = ui::labelled(tr("Keep the result in the variable"), &output_key);
        ol.set_margin_top(12);
        a.append(&ol);
        let output_err = ui::error_label();
        a.append(&output_err);
        let at = section_title(tr("Model"));
        at.set_margin_top(24);
        a.append(&at);
        let mo = caixa(Orientation::Horizontal, 10, "");
        mo.set_margin_top(12);
        mo.append(&rotulo(tr("Longest answer (tokens, 0 = the model's)"), "fx-rotulo-linha"));
        let max_out = spin(0.0, 8192.0, 256.0);
        mo.append(&max_out);
        a.append(&mo);
        let (cl, cloud, _) = switch_row(tr("Allow cloud models"), tr("without it, the agent never leaves this computer, even when the local model fails"));
        cl.set_margin_top(14);
        a.append(&cl);
        // where it runs (design "Agente · sandbox"): first in Avançado
        let sandbox_slot = caixa(Orientation::Vertical, 0, "");
        sandbox_slot.set_margin_bottom(24);
        a.prepend(&sandbox_slot);
        let e2 = ed.clone();
        let sandbox = crate::agent_sandbox::SandboxSection::new(move |v| {
            if let Some(e) = e2.borrow().upgrade().filter(|e| !e.filling.get()) {
                e.draft.borrow_mut().sandbox = v;
                e.changed();
                e.update_side();
            }
        });
        sandbox_slot.append(&sandbox.root);

        let form = crate::limite::Limite::new(&page, FORM_MAX);
        form.add_css_class("fx-pagina");
        main.add_named(&form, Some("editor"));
        // ---- the side: Testar agora, or the team's preview
        let side = ui::side_panel(pw);
        let side_stack = gtk::Stack::new();
        side_stack.set_vexpand(true);
        side_stack.set_transition_type(gtk::StackTransitionType::Crossfade);
        let chat = caixa(Orientation::Vertical, 0, "");
        let ch = caixa(Orientation::Horizontal, 8, "");
        ch.append(&ui::panel_title("tocar", tr("TEST NOW")));
        let sp2 = caixa(Orientation::Horizontal, 0, "");
        sp2.set_hexpand(true);
        ch.append(&sp2);
        let chat_back = ui::link(tr("‹ Preview"));
        chat_back.set_visible(false);
        ch.append(&chat_back);
        chat.append(&ch);
        let cn = ui::wrapped(tr("Runs for real, with the chosen model, in simulation: nothing changes in the system."), "fx-nota");
        cn.set_margin_top(10);
        chat.append(&cn);
        let chat_frame = caixa(Orientation::Vertical, 0, "fx-conversa");
        chat_frame.set_margin_top(14);
        chat_frame.set_vexpand(true);
        let chat_list = caixa(Orientation::Vertical, 12, "");
        let chat_scroll = ui::scroll(&chat_list);
        chat_frame.append(&chat_scroll);
        let ib = caixa(Orientation::Horizontal, 8, "fx-conversa-campo");
        let chat_entry = gtk::Entry::new();
        chat_entry.add_css_class("fx-busca-campo");
        chat_entry.set_placeholder_text(Some(tr("Ask or give a test task…")));
        chat_entry.set_hexpand(true);
        ib.append(&chat_entry);
        let chat_send = gtk::Button::new();
        chat_send.add_css_class("fx-enviar");
        chat_send.set_child(Some(&ui::glyph("enviar", ui::ON_ACCENT, 14)));
        ib.append(&chat_send);
        chat_frame.append(&ib);
        // an agent with a structured input is tested from a form of its fields (design 42)
        let test_form = crate::agent_structured::TestForm::new();
        test_form.root.set_margin_top(14);
        test_form.root.set_visible(false);
        chat.append(&test_form.root);
        chat.append(&chat_frame);
        let shield = ui::note("escudo", tr("What the agent reads from files and pages is data, never orders."));
        shield.set_margin_top(12);
        chat.append(&shield);
        side_stack.add_named(&chat, Some("teste"));
        let team = caixa(Orientation::Vertical, 0, "");
        team.append(&ui::panel_title("olho", tr("PREVIEW")));
        let preview = caixa(Orientation::Vertical, 0, "fx-previa");
        preview.set_margin_top(14);
        team.append(&preview);
        let tsp = caixa(Orientation::Vertical, 0, "");
        tsp.set_vexpand(true);
        team.append(&tsp);
        let tb = caixa(Orientation::Horizontal, 8, "fx-botoes-painel");
        tb.set_homogeneous(true);
        let test_team = ui::button(Some("tocar"), tr("Test the team"), false);
        let as_flow = ui::button(Some("compartilhar"), tr("Open as flow"), false);
        tb.append(&test_team);
        tb.append(&as_flow);
        team.append(&tb);
        side_stack.add_named(&team, Some("equipe"));
        side_stack.add_named(&sandbox.side, Some("sandbox"));
        side.append(&side_stack);
        // the side as a page on narrow windows
        let detail = caixa(Orientation::Vertical, 0, "fx-detalhe");
        let back = ui::button(Some("seta-esquerda"), tr("Back"), false);
        back.add_css_class("plano");
        back.set_halign(Align::Start);
        back.set_margin_start(16);
        back.set_margin_top(12);
        detail.append(&back);
        let holder = caixa(Orientation::Vertical, 0, "");
        holder.set_vexpand(true);
        detail.append(&holder);
        main.add_named(&detail, Some("lado"));
        row.append(&main);
        // the panel's width is a limit, not a minimum: a narrowing window squeezes it (until the editor turns narrow
        // and the panel becomes a page) instead of being held wide by it, which pushed the panel out of sight
        side.set_size_request(-1, -1);
        let side_limit = crate::limite::Limite::new(&side, pw);
        side_limit.set_hexpand(false);
        side_limit.set_fill(true);
        row.append(&side_limit);
        root.set_child(Some(&row));
        let meter = gtk::DrawingArea::new();
        meter.set_can_target(false);
        meter.set_hexpand(true);
        meter.set_vexpand(true);
        root.add_overlay(&meter);
        root.set_measure_overlay(&meter, false);

        let e = Rc::new(AgentEditor {
            root, sandbox_slot, sandbox, nav: RefCell::default(), on_title: RefCell::default(), draft: RefCell::new(Draft::new_assistant("")), is_new: Cell::new(true), dirty: Cell::new(false),
            side_limit, form: form.clone(), filling: Cell::new(false), version: Cell::new(0), saved_at: Cell::new(0), narrow: Cell::new(false), icon_picker, tasks_editor, tasks_err, input_table, output_table, fields_err, test_form: test_form.clone(), test_note: cn.clone(), chat_input: ib.clone(), name: name.clone(), sub_kind, sub_state, sub_version, error,
            history_box, test_btn: test_btn.clone(), tabs, pages, kind_seg, kind_note, assistant_box, team_box, team_summary, remote_box, flow_box, instr: instr.clone(), instr_err, vars,
            model_dd: model_dd.clone(), model_name: model_name.clone(), model_err, fallback_dd: fallback_dd.clone(), fallback_name: fallback_name.clone(), routing, voice: voice.clone(),
            creativity: creativity.clone(), creativity_value, limit_dd: limit_dd.clone(), tools_summary, tools_list, remote_url: remote_url.clone(), remote_err, remote_key: remote_key.clone(),
            remote_timeout: remote_timeout.clone(), flow_dd: flow_dd.clone(), flow_err, catalog_query: catalog_query.clone(), catalog, connectors_list, mode_seg, mode_note, times_row, member_box: member_box.clone(), member_entry: member_entry.clone(), output_box: output_box.clone(),
            times: times.clone(), subs_count, subs_list, subs_err, add_sub_box, add_sub: add_sub.clone(), mem_box, mem_note, mem_conv: mem_conv.clone(), mem_conv_sub, mem_long: mem_long.clone(),
            mem_compact: mem_compact.clone(), docs, desc: desc.clone(), output_key: output_key.clone(), output_err, max_out: max_out.clone(), cloud: cloud.clone(), side, side_stack,
            chat_list, chat_scroll, chat_entry: chat_entry.clone(), chat_send: chat_send.clone(), chat_back: chat_back.clone(), session: RefCell::default(), preview, main, row, holder,
            agents: RefCell::default(), system_tools: RefCell::default(), custom_tools: RefCell::default(), providers: RefCell::default(), flows: RefCell::default(), secrets: RefCell::default(),
            provider_ids: RefCell::default(),
        });
        *ed.borrow_mut() = Rc::downgrade(&e);

        // ---- what the widgets change
        let w = Rc::downgrade(&e);
        meter.connect_resize(move |_, width, _| {
            if let Some(e) = w.upgrade() {
                glib::idle_add_local_once(move || e.set_width(width));
            }
        });
        let edit = |e: &Weak<AgentEditor>, f: Box<dyn Fn(&Rc<AgentEditor>)>| {
            let w = e.clone();
            move || {
                if let Some(e) = w.upgrade().filter(|e| !e.filling.get()) {
                    f(&e);
                    e.changed();
                }
            }
        };
        let w = Rc::downgrade(&e);
        let f = edit(&w, Box::new(|e| {
            let n = e.name.text().to_string();
            e.draft.borrow_mut().name = n.clone();
            e.tell_title(&n);
        }));
        name.connect_changed(move |_| f());
        let f = edit(&w, Box::new(|e| {
            let b = e.instr.buffer();
            e.draft.borrow_mut().instruction = b.text(&b.start_iter(), &b.end_iter(), false).to_string();
            e.paint_variables();
        }));
        instr.buffer().connect_changed(move |_| f());
        let f = edit(&w, Box::new(|e| e.read_models()));
        model_dd.connect_selected_notify(move |_| f());
        let f = edit(&w, Box::new(|e| e.read_models()));
        fallback_dd.connect_selected_notify(move |_| f());
        let f = edit(&w, Box::new(|e| e.read_models()));
        model_name.connect_changed(move |_| f());
        let f = edit(&w, Box::new(|e| e.read_models()));
        fallback_name.connect_changed(move |_| f());
        let f = edit(&w, Box::new(|e| e.draft.borrow_mut().reply_with_voice = e.voice.is_active()));
        voice.connect_active_notify(move |_| f());
        let f = edit(&w, Box::new(|e| {
            let v = (e.creativity.value() * 10.0).round() / 10.0;
            e.draft.borrow_mut().temperature = v;
            e.creativity_value.set_text(&ui::creativity_label(v));
        }));
        creativity.connect_value_changed(move |_| f());
        let f = edit(&w, Box::new(|e| e.draft.borrow_mut().max_tokens = TOKEN_LIMITS.get(e.limit_dd.selected() as usize).copied().flatten()));
        limit_dd.connect_selected_notify(move |_| f());
        let f = edit(&w, Box::new(|e| e.draft.borrow_mut().card_url = e.remote_url.text().to_string()));
        remote_url.connect_changed(move |_| f());
        let f = edit(&w, Box::new(|e| {
            let i = e.remote_key.selected() as usize;
            e.draft.borrow_mut().key_secret = if i == 0 { String::new() } else { e.secrets.borrow().get(i - 1).cloned().unwrap_or_default() };
        }));
        remote_key.connect_selected_notify(move |_| f());
        let f = edit(&w, Box::new(|e| e.draft.borrow_mut().timeout_s = e.remote_timeout.value() as u64));
        remote_timeout.connect_value_changed(move |_| f());
        let f = edit(&w, Box::new(|e| {
            let i = e.flow_dd.selected() as usize;
            e.draft.borrow_mut().flow_id = if i == 0 { String::new() } else { e.flows.borrow().get(i - 1).and_then(|f| f["id"].as_str()).unwrap_or("").to_string() };
        }));
        flow_dd.connect_selected_notify(move |_| f());
        let f = edit(&w, Box::new(|e| {
            e.draft.borrow_mut().max_iterations = e.times.value() as u64;
            e.mode_note.set_text(&mode_note_text("loop", e.times.value() as u64));
            e.render_preview();
        }));
        times.connect_value_changed(move |_| f());
        let f = edit(&w, Box::new(|e| e.draft.borrow_mut().member_instruction = e.member_entry.text().to_string()));
        member_entry.connect_changed(move |_| f());
        let f = edit(&w, Box::new(|e| e.draft.borrow_mut().conversation = e.mem_conv.is_active()));
        mem_conv.connect_active_notify(move |_| f());
        let f = edit(&w, Box::new(|e| e.draft.borrow_mut().long_term = e.mem_long.is_active()));
        mem_long.connect_active_notify(move |_| f());
        let f = edit(&w, Box::new(|e| e.draft.borrow_mut().compact_every = if e.mem_compact.is_active() { 30 } else { 0 }));
        mem_compact.connect_active_notify(move |_| f());
        let f = edit(&w, Box::new(|e| e.draft.borrow_mut().description = e.desc.text().to_string()));
        desc.connect_changed(move |_| f());
        let f = edit(&w, Box::new(|e| e.draft.borrow_mut().output_key = e.output_key.text().to_string()));
        output_key.connect_changed(move |_| f());
        let f = edit(&w, Box::new(|e| {
            let v = e.max_out.value() as u64;
            e.draft.borrow_mut().max_output_tokens = (v > 0).then_some(v);
        }));
        max_out.connect_value_changed(move |_| f());
        let f = edit(&w, Box::new(|e| e.draft.borrow_mut().allow_cloud = e.cloud.is_active()));
        cloud.connect_active_notify(move |_| f());
        let w2 = w.clone();
        catalog_query.connect_changed(move |_| {
            if let Some(e) = w2.upgrade() {
                e.render_catalog();
            }
        });
        let w2 = w.clone();
        add_tool.connect_clicked(move |_| {
            if let Some(e) = w2.upgrade() {
                e.show_tab("ferramentas");
            }
        });
        let w2 = w.clone();
        to_subs.connect_clicked(move |_| {
            if let Some(e) = w2.upgrade() {
                e.show_tab("subagentes");
            }
        });
        let w2 = w.clone();
        manage.connect_clicked(move |_| {
            if let Some(e) = w2.upgrade() {
                e.go(Go::Page("ajustes"));
            }
        });
        let w2 = w.clone();
        add_docs.connect_clicked(move |b| {
            if let Some(e) = w2.upgrade() {
                e.pick_documents(b);
            }
        });
        let w2 = w.clone();
        save.connect_clicked(move |_| {
            if let Some(e) = w2.upgrade() {
                e.save(None);
            }
        });
        let w2 = w.clone();
        export.connect_clicked(move |b| {
            if let Some(e) = w2.upgrade() {
                export_agent(b, &e.draft.borrow().to_agent());
            }
        });
        let w2 = w.clone();
        hpop.connect_show(move |_| {
            if let Some(e) = w2.upgrade() {
                e.load_history();
            }
        });
        let w2 = w.clone();
        let send = move || {
            if let Some(e) = w2.upgrade() {
                e.send_test();
            }
        };
        let s2 = send.clone();
        chat_send.connect_clicked(move |_| s2());
        let s3 = send.clone();
        test_form.run.connect_clicked(move |_| s3());
        chat_entry.connect_activate(move |_| send());
        let w2 = w.clone();
        test_team.connect_clicked(move |_| {
            if let Some(e) = w2.upgrade() {
                e.side_stack.set_visible_child_name("teste");
                e.chat_back.set_visible(true);
            }
        });
        let w2 = w.clone();
        chat_back.connect_clicked(move |b| {
            if let Some(e) = w2.upgrade() {
                e.side_stack.set_visible_child_name("equipe");
                b.set_visible(false);
            }
        });
        let w2 = w.clone();
        as_flow.connect_clicked(move |_| {
            if let Some(e) = w2.upgrade() {
                e.team_to_flow();
            }
        });
        let w2 = w.clone();
        test_btn.connect_clicked(move |_| {
            if let Some(e) = w2.upgrade() {
                e.main.set_visible_child_name("lado");
            }
        });
        let w2 = w.clone();
        back.connect_clicked(move |_| {
            if let Some(e) = w2.upgrade() {
                e.main.set_visible_child_name("editor");
            }
        });
        // "salvo há 2 min" stays true
        let w2 = w.clone();
        glib::timeout_add_seconds_local(30, move || {
            let Some(e) = w2.upgrade() else { return glib::ControlFlow::Break };
            if e.root.is_mapped() {
                e.render_state();
            }
            glib::ControlFlow::Continue
        });
        e
    }

    pub fn set_nav(&self, nav: Nav) {
        *self.nav.borrow_mut() = Some(nav);
    }

    fn go(&self, g: Go) {
        if let Some(n) = self.nav.borrow().clone() {
            n(g);
        }
    }

    fn tell_title(&self, name: &str) {
        if let Some(f) = self.on_title.borrow().clone() {
            f(if name.trim().is_empty() { tr("New agent") } else { name });
        }
    }

    /// Opens an agent (`new` = never saved); `chat` puts the cursor in "Testar agora".
    pub fn open(self: &Rc<Self>, agent: &Value, new: bool, chat: bool) {
        let mut d = if agent.is_null() { Draft::new_assistant(&ui::new_id("agente")) } else { Draft::from_agent(agent) };
        if d.id.is_empty() {
            d.id = ui::new_id("agente");
        }
        *self.draft.borrow_mut() = d;
        self.is_new.set(new);
        self.dirty.set(new);
        self.version.set(0);
        self.saved_at.set(0);
        *self.session.borrow_mut() = ui::new_id("teste");
        ui::clear_box(&self.chat_list);
        self.chat_entry.set_text("");
        ui::show_error(&self.error, "");
        self.chat_back.set_visible(false);
        self.fill();
        self.show_tab("geral");
        self.main.set_visible_child_name("editor");
        self.load_service();
        if !new {
            self.find_version();
        }
        if chat {
            let k = self.draft.borrow().kind.clone();
            if matches!(k.as_str(), "sequential" | "parallel" | "loop") {
                self.side_stack.set_visible_child_name("teste");
                self.chat_back.set_visible(true);
            }
            if self.narrow.get() {
                self.main.set_visible_child_name("lado");
            }
            self.chat_entry.grab_focus();
        }
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug central: editor de agente {} ({})", self.draft.borrow().name, self.draft.borrow().kind);
        }
    }

    /// Agents, tools, providers, flows and vault names, for the lists of the editor.
    fn load_service(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        fluxos::call("agents.list", json!({}), move |r| {
            let Some(e) = w.upgrade() else { return };
            *e.agents.borrow_mut() = r.ok().and_then(|v| v.as_array().cloned()).unwrap_or_default().into_iter().map(|x| x["agent"].clone()).collect();
            e.render_subs();
            e.render_tasks();
            e.render_preview();
        });
        let w = Rc::downgrade(self);
        fluxos::call("tools.list", json!({}), move |r| {
            let Some(e) = w.upgrade() else { return };
            let v = r.unwrap_or(json!({}));
            *e.system_tools.borrow_mut() = v["system"].as_array().cloned().unwrap_or_default();
            *e.custom_tools.borrow_mut() = v["custom"].as_array().cloned().unwrap_or_default();
            e.render_tools();
            e.render_catalog();
            e.render_connectors();
        });
        let w = Rc::downgrade(self);
        fluxos::call("models.providers", json!({}), move |r| {
            let Some(e) = w.upgrade() else { return };
            let all = r.ok().and_then(|v| v["providers"].as_array().cloned()).unwrap_or_default();
            *e.providers.borrow_mut() = all.into_iter().filter(|p| p["kind"].as_str() != Some("local") && p["enabled"].as_bool().unwrap_or(true)).collect();
            e.fill_models();
        });
        let w = Rc::downgrade(self);
        fluxos::call("flows.list", json!({}), move |r| {
            let Some(e) = w.upgrade() else { return };
            *e.flows.borrow_mut() = r.ok().and_then(|v| v.as_array().cloned()).unwrap_or_default();
            e.fill_flows();
        });
        let w = Rc::downgrade(self);
        fluxos::call("secrets.list", json!({}), move |r| {
            let Some(e) = w.upgrade() else { return };
            *e.secrets.borrow_mut() = r.ok().and_then(|v| v.as_array().cloned()).unwrap_or_default().into_iter().filter_map(|s| s.as_str().map(str::to_string)).collect();
            e.fill_secrets();
        });
    }

    fn set_width(&self, width: i32) {
        // narrow: under NARROW, or when the form at its narrowest and the panel at its narrowest do not fit side by
        // side (then the window could not be as narrow as the compositor wants, and the panel would be cut off)
        let narrow = is_narrow(width, self.form.measure(Orientation::Horizontal, -1).0);
        // wide: the test panel grows a little (the form keeps frame 02's width: `FORM_MAX`, in `new`)
        self.side_limit.set_max(if narrow { i32::MAX / 2 } else { side_width(width) });
        self.side_limit.set_fill(!narrow);
        if narrow == self.narrow.get() {
            return;
        }
        self.narrow.set(narrow);
        if narrow {
            self.row.remove(&self.side_limit);
            self.holder.append(&self.side_limit);
            self.side_limit.set_hexpand(true);
            self.side.add_css_class("estreito");
        } else {
            self.holder.remove(&self.side_limit);
            self.row.append(&self.side_limit);
            self.side_limit.set_hexpand(false);
            self.side.remove_css_class("estreito");
            self.main.set_visible_child_name("editor");
        }
        self.test_btn.set_visible(narrow);
    }

    fn show_tab(&self, id: &str) {
        self.tabs.show(id);
        self.pages.set_visible_child_name(id);
        self.update_side();
    }

    /// The side: "Como fica" beside Avançado while the agent runs in a sandbox; else the test (or the team).
    fn update_side(&self) {
        let on_adv = self.pages.visible_child_name().as_deref() == Some("avancado");
        let kind = self.draft.borrow().kind.clone();
        if on_adv && kind == "assistant" && self.sandbox.is_on() {
            self.side_stack.set_visible_child_name("sandbox");
        } else if self.side_stack.visible_child_name().as_deref() == Some("sandbox") {
            self.side_stack.set_visible_child_name(if matches!(kind.as_str(), "sequential" | "parallel" | "loop") { "equipe" } else { "teste" });
        }
    }

    fn changed(&self) {
        self.dirty.set(true);
        self.render_state();
    }

    // ---------------------------------------------------------------- filling the widgets from the draft

    fn fill(self: &Rc<Self>) {
        self.filling.set(true);
        let d = self.draft.borrow().clone();
        self.name.set_text(&d.name);
        self.tell_title(&d.name);
        self.instr.buffer().set_text(&d.instruction);
        self.voice.set_active(d.reply_with_voice);
        self.creativity.set_value(d.temperature.clamp(0.0, 1.0));
        self.creativity_value.set_text(&ui::creativity_label(d.temperature));
        self.limit_dd.set_selected(TOKEN_LIMITS.iter().position(|n| *n == d.max_tokens).unwrap_or(0) as u32);
        self.remote_url.set_text(&d.card_url);
        self.remote_timeout.set_value(d.timeout_s as f64);
        self.times.set_value(d.max_iterations as f64);
        self.member_entry.set_text(&d.member_instruction);
        self.mem_conv.set_active(d.conversation);
        self.mem_conv_sub.set_text(&trf("the history goes into the context (the last {n} exchanges)", &[("n", d.keep_exchanges.to_string())]));
        self.mem_long.set_active(d.long_term);
        self.mem_compact.set_active(d.compact_every > 0);
        self.desc.set_text(&d.description);
        self.output_key.set_text(&d.output_key);
        self.max_out.set_value(d.max_output_tokens.unwrap_or(0) as f64);
        self.cloud.set_active(d.allow_cloud);
        self.filling.set(false);
        self.fill_models();
        self.fill_flows();
        self.fill_secrets();
        self.paint_variables();
        self.input_table.set(&d.input_fields);
        self.output_table.set(&d.output_fields);
        self.test_form.set_fields(&d.input_fields);
        self.render_test_mode();
        self.render_tasks();
        self.sandbox.set(&d.sandbox);
        self.render_kind();
        self.render_tools();
        self.render_catalog();
        self.render_connectors();
        self.render_subs();
        self.render_docs();
        self.render_state();
        for l in [&self.instr_err, &self.model_err, &self.remote_err, &self.flow_err, &self.subs_err, &self.output_err, &self.tasks_err, &self.fields_err] {
            ui::show_error(l, "");
        }
    }

    /// The model choices: automatic, the local model, then each cloud provider.
    fn fill_models(&self) {
        self.filling.set(true);
        let d = self.draft.borrow().clone();
        let providers = self.providers.borrow().clone();
        let pname = |p: &Value| p["name"].as_str().filter(|n| !n.is_empty()).or(p["id"].as_str()).unwrap_or("").to_string();
        let mut opts = vec![tr("Automatic by input (local)").to_string(), tr("LFM3 1.7B (local)").to_string()];
        opts.extend(providers.iter().map(|p| trf("{name} (cloud)", &[("name", pname(p))])));
        let mut fopts = vec![tr("none — stay on this computer").to_string()];
        fopts.extend(providers.iter().map(|p| trf("{name} (cloud)", &[("name", pname(p))])));
        // a provider the agent names that is not configured (still shown, so saving keeps it)
        let mut extra: Vec<String> = Vec::new();
        for m in std::iter::once(&d.model).chain(d.fallback.iter()) {
            if let Some(pid) = m["provider"].as_str().filter(|p| *p != "local") {
                if !providers.iter().any(|p| p["id"].as_str() == Some(pid)) && !extra.iter().any(|x| x == pid) {
                    extra.push(pid.to_string());
                }
            }
        }
        for x in &extra {
            opts.push(trf("{name} (cloud)", &[("name", x.clone())]));
            fopts.push(trf("{name} (cloud)", &[("name", x.clone())]));
        }
        let ids: Vec<String> = providers.iter().map(|p| p["id"].as_str().unwrap_or("").to_string()).chain(extra).collect();
        let pos = |m: &Value| m["provider"].as_str().and_then(|p| ids.iter().position(|x| x == p));
        let sel = if d.model_mode == "auto" { 0 } else { pos(&d.model).map(|i| i + 2).unwrap_or(1) };
        self.model_dd.set_model(Some(&gtk::StringList::new(&opts.iter().map(String::as_str).collect::<Vec<_>>())));
        self.model_dd.set_selected(sel as u32);
        self.model_name.set_text(d.model["model"].as_str().unwrap_or(""));
        self.model_name.set_visible(sel >= 2);
        let fb = d.fallback.first().cloned().unwrap_or(Value::Null);
        let fsel = pos(&fb).map(|i| i + 1).unwrap_or(0);
        self.fallback_dd.set_model(Some(&gtk::StringList::new(&fopts.iter().map(String::as_str).collect::<Vec<_>>())));
        self.fallback_dd.set_selected(fsel as u32);
        self.fallback_name.set_text(fb["model"].as_str().unwrap_or(""));
        self.fallback_name.set_visible(fsel >= 1);
        self.routing.set_visible(sel == 0);
        *self.provider_ids.borrow_mut() = ids;
        self.filling.set(false);
    }

    fn read_models(&self) {
        let ids = self.provider_ids.borrow().clone();
        let sel = self.model_dd.selected() as usize;
        let fsel = self.fallback_dd.selected() as usize;
        {
            let mut d = self.draft.borrow_mut();
            d.model_mode = if sel == 0 { "auto" } else { "fixed" }.into();
            d.model = if sel >= 2 { json!({"provider": ids.get(sel - 2).cloned().unwrap_or_default(), "model": self.model_name.text().trim()}) } else { Value::Null };
            d.fallback = if fsel >= 1 { vec![json!({"provider": ids.get(fsel - 1).cloned().unwrap_or_default(), "model": self.fallback_name.text().trim()})] } else { vec![] };
            // choosing a cloud model is the person allowing it
            if d.uses_cloud() && !d.allow_cloud {
                d.allow_cloud = true;
                self.filling.set(true);
                self.cloud.set_active(true);
                self.filling.set(false);
            }
        }
        self.model_name.set_visible(sel >= 2);
        self.fallback_name.set_visible(fsel >= 1);
        self.routing.set_visible(sel == 0);
    }

    fn fill_flows(&self) {
        self.filling.set(true);
        let flows = self.flows.borrow().clone();
        let mut opts = vec![tr("choose a flow").to_string()];
        opts.extend(flows.iter().map(|f| f["name"].as_str().unwrap_or("").to_string()));
        self.flow_dd.set_model(Some(&gtk::StringList::new(&opts.iter().map(String::as_str).collect::<Vec<_>>())));
        let id = self.draft.borrow().flow_id.clone();
        self.flow_dd.set_selected(flows.iter().position(|f| f["id"].as_str() == Some(&id)).map(|i| i + 1).unwrap_or(0) as u32);
        self.filling.set(false);
    }

    fn fill_secrets(&self) {
        self.filling.set(true);
        let s = self.secrets.borrow().clone();
        let mut opts = vec![tr("no key").to_string()];
        opts.extend(s.iter().map(|n| format!("{{cofre:{n}}}")));
        self.remote_key.set_model(Some(&gtk::StringList::new(&opts.iter().map(String::as_str).collect::<Vec<_>>())));
        let k = self.draft.borrow().key_secret.clone();
        self.remote_key.set_selected(s.iter().position(|n| *n == k).map(|i| i + 1).unwrap_or(0) as u32);
        self.filling.set(false);
    }

    /// The `{variables}` painted in the instructions and listed under them.
    fn paint_variables(self: &Rc<Self>) {
        let b = self.instr.buffer();
        let text = b.text(&b.start_iter(), &b.end_iter(), false).to_string();
        b.remove_tag_by_name("var", &b.start_iter(), &b.end_iter());
        for (s, e) in ui::variable_spans(&text) {
            let (cs, ce) = (text[..s].chars().count() as i32, text[..e].chars().count() as i32);
            b.apply_tag_by_name("var", &b.iter_at_offset(cs), &b.iter_at_offset(ce));
        }
        ui::clear_box(&self.vars);
        self.vars.append(&rotulo(tr("Variables:"), "fx-rotulo-linha"));
        for v in ui::variables(&text) {
            self.vars.append(&ui::mono_chip(&format!("{{{v}}}"), "acento"));
        }
        let add = gtk::Button::with_label(tr("+ variable"));
        add.add_css_class("fx-chip-botao");
        let w = Rc::downgrade(self);
        add.connect_clicked(move |_| {
            if let Some(e) = w.upgrade() {
                let b = e.instr.buffer();
                b.insert_at_cursor("{nova_variavel}");
                e.instr.grab_focus();
            }
        });
        self.vars.append(&add);
    }

    fn render_kind(self: &Rc<Self>) {
        // (only an assistant runs in a sandbox: kalango-fluxos reads `sandbox` from an assistant's config)
        self.sandbox_slot.set_visible(self.draft.borrow().kind == "assistant");
        self.update_side();
        let d = self.draft.borrow().clone();
        let k = d.kind.as_str();
        self.filling.set(true);
        self.kind_seg.set(if k == "assistant" { "assistant" } else { k });
        let team = matches!(k, "sequential" | "parallel" | "loop");
        self.mode_seg.set(if team { k } else { "assistant" });
        self.filling.set(false);
        self.kind_note.set_text(kind_note(k));
        self.mode_note.set_text(&mode_note_text(k, d.max_iterations));
        self.times_row.set_visible(k == "loop");
        self.member_box.set_visible(team);
        self.output_box.set_visible(output_allowed(k));
        self.icon_picker.set(&d.tile_agent());
        self.sub_kind.set_text(ui::kind_label(k));
        let assistant = k == "assistant";
        self.assistant_box.set_visible(assistant);
        self.team_box.set_visible(team);
        self.tasks_editor.root.set_visible(k == "tasks");
        self.remote_box.set_visible(k == "remote");
        self.flow_box.set_visible(k == "flow");
        for (tab, on) in [("ferramentas", assistant), ("conectores", assistant), ("subagentes", assistant || team), ("memoria", true)] {
            self.tabs.set_visible(tab, on);
        }
        self.mem_box.set_visible(assistant);
        self.mem_note.set_visible(!assistant);
        let names: Vec<String> = d.subs.iter().map(|s| self.name_of(s)).collect();
        self.team_summary.set_text(&if names.is_empty() { tr("No sub-agent yet.").to_string() } else { names.join(if k == "parallel" { " · " } else { " → " }) });
        self.side_stack.set_visible_child_name(if team && !self.chat_back.is_visible() { "equipe" } else { "teste" });
        self.render_preview();
    }

    /// The task list from the draft (with the assistants that can take tasks).
    fn render_tasks(&self) {
        let d = self.draft.borrow().clone();
        self.filling.set(true);
        self.tasks_editor.set(&d.tasks, &self.agents.borrow());
        self.filling.set(false);
    }

    /// The structured input and output from their tables; the test form follows the input.
    fn read_fields(&self) {
        let none_if_empty = |v: Value| if v.as_array().is_some_and(|a| a.is_empty()) { Value::Null } else { v };
        let (i, o) = (none_if_empty(self.input_table.value()), none_if_empty(self.output_table.value()));
        {
            let mut d = self.draft.borrow_mut();
            d.input_fields = i.clone();
            d.output_fields = o;
        }
        ui::show_error(&self.fields_err, "");
        self.test_form.set_fields(&i);
        self.render_test_mode();
        self.changed();
    }

    /// "Testar agora" with a form (an agent with a structured input) or with the message field.
    fn render_test_mode(&self) {
        let form = !self.test_form.is_empty();
        self.test_form.root.set_visible(form);
        self.chat_input.set_visible(!form);
        self.test_note.set_text(if form { tr("The form comes from the structured input.") } else { tr("Runs for real, with the chosen model, in simulation: nothing changes in the system.") });
    }

    fn render_state(&self) {
        let (text, class) = if self.dirty.get() {
            (if self.is_new.get() { tr("not saved yet").to_string() } else { tr("unsaved changes").to_string() }, "naosalvo")
        } else if self.saved_at.get() > 0 {
            (trf("saved {when}", &[("when", ui::ago(now(), self.saved_at.get()))]), "salvo")
        } else {
            (tr("saved").to_string(), "salvo")
        };
        self.sub_state.set_text(&text);
        self.sub_state.set_css_classes(&["fx-cabecalho-estado", class]);
        self.sub_version.set_text(&if self.version.get() > 0 { format!("  ·  {}", trf("version {n}", &[("n", self.version.get().to_string())])) } else { String::new() });
    }

    fn name_of(&self, id: &str) -> String {
        self.agents.borrow().iter().find(|a| a["id"].as_str() == Some(id)).and_then(|a| a["name"].as_str().map(str::to_string)).unwrap_or_else(|| id.to_string())
    }

    fn tool_info(&self, name: &str) -> (String, Option<String>, bool) {
        // (description, level, custom)
        if let Some(t) = self.system_tools.borrow().iter().find(|t| t["name"].as_str() == Some(name)) {
            return (ui::tool_description(t["description"].as_str().unwrap_or("")).to_string(), t["level"].as_str().map(str::to_string), false);
        }
        if let Some(t) = self.custom_tools.borrow().iter().find(|t| t["name"].as_str() == Some(name)) {
            return (t["description"].as_str().or(t["tool"]["description"].as_str()).unwrap_or("").to_string(), None, true);
        }
        (String::new(), None, false)
    }

    /// The agent's tools (Geral): name, what it does, level, permission; the chevron chooses or removes.
    fn render_tools(self: &Rc<Self>) {
        ui::clear_box(&self.tools_list);
        let d = self.draft.borrow().clone();
        let connectors: Vec<String> = ui::connectors().into_iter().map(|c| c.0).collect();
        let custom: Vec<String> = self.custom_tools.borrow().iter().filter_map(|t| t["name"].as_str().map(str::to_string)).collect();
        let names: Vec<&str> = d.tools.iter().map(|t| t.0.as_str()).collect();
        self.tools_summary.set_text(&crate::central_agentes::tools_summary(crate::central_agentes::tool_counts(&names, &connectors, &custom)));
        if d.tools.is_empty() {
            let l = ui::wrapped(tr("No tool: the agent only talks. Add tools to let it act."), "fx-nota");
            l.set_margin_top(10);
            l.set_margin_bottom(10);
            l.set_margin_start(16);
            self.tools_list.append(&l);
        }
        for (i, (n, perm)) in d.tools.iter().enumerate() {
            let (desc, level, own) = self.tool_info(n);
            let r = caixa(Orientation::Horizontal, 8, "fx-ferramenta-linha");
            let c = caixa(Orientation::Vertical, 2, "");
            c.set_hexpand(true);
            c.append(&ui::left(n, "fx-mono-nome"));
            if !desc.is_empty() {
                c.append(&ui::cut(&desc, "fx-cartao-desc"));
            }
            r.append(&c);
            if let Some(l) = &level {
                r.append(&ui::chip(ui::level_label(l), ui::level_tone(l)));
            } else if own {
                r.append(&ui::chip(tr("yours"), "acento"));
            } else if connectors.iter().any(|x| x == ui::tool_prefix(n)) {
                r.append(&ui::chip(tr("connector"), "lilas"));
            }
            r.append(&ui::chip(ui::permission_label(perm), ui::permission_tone(perm)));
            let more = gtk::MenuButton::new();
            more.add_css_class("fx-mais");
            more.set_child(Some(&crate::conversa::ui_icon("chevron-baixo", ui::MUTED, 14)));
            more.set_valign(Align::Center);
            let pop = gtk::Popover::new();
            let pc = caixa(Orientation::Vertical, 2, "");
            for (p, label) in [("free", tr("Free: never asks")), ("ask", tr("Ask before using")), ("never", tr("Never use")), ("remover", tr("Remove from the agent"))] {
                let b = gtk::Button::with_label(label);
                b.add_css_class("central-menu-item");
                let (w, pop2, p) = (Rc::downgrade(self), pop.clone(), p.to_string());
                b.connect_clicked(move |_| {
                    pop2.popdown();
                    let Some(e) = w.upgrade() else { return };
                    {
                        let mut d = e.draft.borrow_mut();
                        if p == "remover" {
                            if i < d.tools.len() {
                                d.tools.remove(i);
                            }
                        } else if let Some(t) = d.tools.get_mut(i) {
                            t.1 = p.clone();
                        }
                    }
                    e.changed();
                    e.render_tools();
                    e.render_catalog();
                });
                pc.append(&b);
            }
            pop.set_child(Some(&pc));
            more.set_popover(Some(&pop));
            r.append(&more);
            self.tools_list.append(&r);
        }
    }

    /// Ferramentas: every tool (the system's, the person's, the connectors'), each on or off for this agent.
    fn render_catalog(self: &Rc<Self>) {
        ui::clear_box(&self.catalog);
        let q = self.catalog_query.text().to_string();
        let chosen: Vec<String> = self.draft.borrow().tools.iter().map(|t| t.0.clone()).collect();
        let connectors: Vec<String> = ui::connectors().into_iter().map(|c| c.0).collect();
        let system: Vec<Value> = self.system_tools.borrow().clone();
        let custom: Vec<Value> = self.custom_tools.borrow().clone();
        let (from_connectors, from_system): (Vec<Value>, Vec<Value>) = system.into_iter().partition(|t| connectors.iter().any(|c| c == ui::tool_prefix(t["name"].as_str().unwrap_or(""))));
        let groups = [(tr("FROM THE SYSTEM"), from_system), (tr("CREATED BY YOU"), custom), (tr("FROM CONNECTORS"), from_connectors)];
        let mut any = false;
        for (title, tools) in groups {
            let shown: Vec<&Value> = tools.iter().filter(|t| ui::matches_query(&q, &[t["name"].as_str().unwrap_or(""), t["description"].as_str().unwrap_or("")])).collect();
            if shown.is_empty() {
                continue;
            }
            any = true;
            let h = ui::section(title);
            h.set_margin_top(14);
            self.catalog.append(&h);
            let list = caixa(Orientation::Vertical, 0, "fx-lista");
            list.set_margin_top(8);
            for t in shown {
                let n = t["name"].as_str().unwrap_or("").to_string();
                let r = caixa(Orientation::Horizontal, 8, "fx-ferramenta-linha");
                let c = caixa(Orientation::Vertical, 2, "");
                c.set_hexpand(true);
                c.append(&ui::left(&n, "fx-mono-nome"));
                let desc = ui::tool_description(t["description"].as_str().unwrap_or(""));
                if !desc.is_empty() {
                    c.append(&ui::cut(desc, "fx-cartao-desc"));
                }
                r.append(&c);
                if let Some(l) = t["level"].as_str() {
                    r.append(&ui::chip(ui::level_label(l), ui::level_tone(l)));
                }
                let sw = ui::switch();
                sw.set_active(chosen.contains(&n));
                let w = Rc::downgrade(self);
                let level = t["level"].as_str().unwrap_or("").to_string();
                sw.connect_active_notify(move |s| {
                    let Some(e) = w.upgrade() else { return };
                    {
                        let mut d = e.draft.borrow_mut();
                        d.tools.retain(|x| x.0 != n);
                        if s.is_active() {
                            // reading is free; what changes things asks first
                            let p = if level == "observar" { "free" } else { "ask" };
                            d.tools.push((n.clone(), p.into()));
                        }
                    }
                    e.changed();
                    e.render_tools();
                });
                r.append(&sw);
                list.append(&r);
            }
            self.catalog.append(&list);
        }
        if !any {
            let l = ui::wrapped(if q.is_empty() { tr("No tool yet: kalangod's appear when it runs; create your own in Agents › Tools.") } else { tr("No tool matches.") }, "fx-nota");
            l.set_margin_top(12);
            self.catalog.append(&l);
        }
    }

    fn render_connectors(self: &Rc<Self>) {
        ui::clear_box(&self.connectors_list);
        let list = ui::connectors();
        let tools: Vec<String> = self.draft.borrow().tools.iter().map(|t| t.0.clone()).collect();
        if list.is_empty() {
            let l = ui::wrapped(tr("No connector installed. Connectors bring the tools of apps (mail, browser, calendar) through MCP."), "fx-nota");
            l.set_margin_top(10);
            l.set_margin_bottom(10);
            l.set_margin_start(16);
            self.connectors_list.append(&l);
        }
        for (name, on) in list {
            let r = caixa(Orientation::Horizontal, 10, "fx-ferramenta-linha");
            r.append(&ui::icon_tile("agrupar", "lilas", 32));
            let c = caixa(Orientation::Vertical, 2, "");
            c.set_hexpand(true);
            c.set_valign(Align::Center);
            c.append(&ui::left(&name, "fx-cartao-nome"));
            let used = tools.iter().filter(|t| ui::tool_prefix(t) == name).count();
            c.append(&ui::left(&if used == 0 { tr("the agent uses none of its tools").to_string() } else if used == 1 { tr("the agent uses 1 of its tools").to_string() } else { trf("the agent uses {n} of its tools", &[("n", used.to_string())]) }, "fx-cartao-desc"));
            r.append(&c);
            r.append(&ui::chip(if on { tr("on") } else { tr("off") }, if on { "teal" } else { "cinza" }));
            let pick = ui::button(None, tr("Choose tools"), false);
            let w = Rc::downgrade(self);
            let n2 = name.clone();
            pick.connect_clicked(move |_| {
                if let Some(e) = w.upgrade() {
                    e.catalog_query.set_text(&n2);
                    e.show_tab("ferramentas");
                }
            });
            r.append(&pick);
            self.connectors_list.append(&r);
        }
    }

    /// Subagentes: numbered rows (drag the handle or use ⋮ to reorder) and the agents to add.
    fn render_subs(self: &Rc<Self>) {
        ui::clear_box(&self.subs_list);
        let d = self.draft.borrow().clone();
        let n = d.subs.len();
        self.subs_count.set_text(&if n == 0 { tr("none yet").to_string() } else { trf("{n} · drag to change the order", &[("n", n.to_string())]) });
        for (i, id) in d.subs.iter().enumerate() {
            let a = self.agents.borrow().iter().find(|a| a["id"].as_str() == Some(id)).cloned().unwrap_or(json!({"id": id, "name": id}));
            let k = ui::kind_of(&a);
            let r = caixa(Orientation::Horizontal, 12, "fx-sub-linha");
            let handle = caixa(Orientation::Horizontal, 8, "fx-alca");
            handle.append(&ui::glyph("menu", ui::MUTED, 14));
            handle.append(&rotulo(&(i + 1).to_string(), "fx-passo-numero"));
            handle.set_valign(Align::Start);
            handle.set_cursor_from_name(Some("grab"));
            r.append(&handle);
            r.append(&crate::agent_picker::icon_tile(&a, 36));
            let c = caixa(Orientation::Vertical, 2, "");
            c.set_hexpand(true);
            c.append(&ui::left(a["name"].as_str().unwrap_or(id), "fx-cartao-nome"));
            let desc = a["description"].as_str().unwrap_or("");
            if !desc.is_empty() {
                c.append(&ui::cut(desc, "fx-cartao-desc"));
            }
            let cfg = &a["config"];
            let mut facts: Vec<String> = Vec::new();
            if k == "assistant" {
                facts.push(if cfg["model_mode"].as_str().unwrap_or("auto") == "auto" { tr("LFM3 local").to_string() } else { ui::model_label(&cfg["model"]) });
                let tools: Vec<&str> = cfg["tools"].as_array().into_iter().flatten().filter_map(|t| t["name"].as_str()).take(3).collect();
                if !tools.is_empty() {
                    facts.push(tools.join(", "));
                }
            } else {
                facts.push(ui::kind_label(k).to_string());
            }
            c.append(&ui::cut(&facts.join("  ·  "), "fx-cartao-tipo"));
            r.append(&c);
            if let Some(key) = cfg["output_key"].as_str().filter(|k| !k.is_empty()) {
                let g = caixa(Orientation::Vertical, 4, "");
                g.set_valign(Align::Start);
                let l = rotulo(tr("keeps in"), "fx-cartao-tipo");
                l.set_halign(Align::End);
                g.append(&l);
                let ch = ui::mono_chip(&format!("{{{key}}}"), "acento");
                ch.set_halign(Align::End);
                g.append(&ch);
                r.append(&g);
            }
            let more = gtk::MenuButton::new();
            more.add_css_class("fx-mais");
            more.set_child(Some(&crate::conversa::ui_icon("pontos-v", ui::MUTED, 14)));
            more.set_valign(Align::Start);
            let pop = gtk::Popover::new();
            let pc = caixa(Orientation::Vertical, 2, "");
            let mut acts: Vec<(&str, &str)> = Vec::new();
            if i > 0 {
                acts.push(("cima", tr("Move up")));
            }
            if i + 1 < n {
                acts.push(("baixo", tr("Move down")));
            }
            acts.push(("abrir", tr("Open this agent")));
            acts.push(("remover", tr("Remove from the team")));
            for (act, label) in acts {
                let b = gtk::Button::with_label(label);
                b.add_css_class("central-menu-item");
                let (w, pop2, act, a2) = (Rc::downgrade(self), pop.clone(), act.to_string(), a.clone());
                b.connect_clicked(move |_| {
                    pop2.popdown();
                    let Some(e) = w.upgrade() else { return };
                    match act.as_str() {
                        "cima" => e.move_sub(i, i - 1),
                        "baixo" => e.move_sub(i, i + 1),
                        "abrir" => e.go(Go::Agent { agent: a2.clone(), new: false, chat: false }),
                        _ => {
                            {
                                let mut d = e.draft.borrow_mut();
                                d.subs.remove(i);
                                if i < d.sub_modes.len() {
                                    d.sub_modes.remove(i);
                                }
                            }
                            e.subs_changed();
                        }
                    }
                });
                pc.append(&b);
            }
            pop.set_child(Some(&pc));
            more.set_popover(Some(&pop));
            r.append(&more);
            // drag the handle onto another row
            let ds = gtk::DragSource::new();
            ds.set_actions(gtk::gdk::DragAction::MOVE);
            ds.connect_prepare(move |_, _, _| Some(gtk::gdk::ContentProvider::for_value(&(i as i32).to_value())));
            handle.add_controller(ds);
            let dt = gtk::DropTarget::new(i32::static_type(), gtk::gdk::DragAction::MOVE);
            let w = Rc::downgrade(self);
            dt.connect_drop(move |_, v, _, _| {
                let (Some(e), Ok(from)) = (w.upgrade(), v.get::<i32>()) else { return false };
                e.move_sub(from as usize, i);
                true
            });
            r.add_controller(dt);
            self.subs_list.append(&r);
        }
        // the agents that may join (not this one, not those already in)
        ui::clear_box(&self.add_sub_box);
        let mut any = false;
        for a in self.agents.borrow().iter() {
            let id = a["id"].as_str().unwrap_or("");
            if id == d.id || d.subs.iter().any(|s| s == id) {
                continue;
            }
            any = true;
            let b = gtk::Button::new();
            b.add_css_class("central-menu-item");
            let bc = caixa(Orientation::Horizontal, 8, "");
            bc.append(&crate::agent_picker::icon_tile(a, 22));
            bc.append(&rotulo(a["name"].as_str().unwrap_or(id), ""));
            b.set_child(Some(&bc));
            let (w, id) = (Rc::downgrade(self), id.to_string());
            b.connect_clicked(move |_| {
                let Some(e) = w.upgrade() else { return };
                e.add_sub.popdown();
                {
                    let mut d = e.draft.borrow_mut();
                    d.subs.push(id.clone());
                    d.sub_modes.push("chat".into());
                }
                e.subs_changed();
            });
            self.add_sub_box.append(&b);
        }
        if !any {
            let l = ui::wrapped(tr("No other agent to add. Create one in Agents."), "fx-nota");
            l.set_margin_start(8);
            l.set_margin_end(8);
            self.add_sub_box.append(&l);
        }
    }

    fn move_sub(self: &Rc<Self>, from: usize, to: usize) {
        {
            let mut d = self.draft.borrow_mut();
            if from >= d.subs.len() || to >= d.subs.len() || from == to {
                return;
            }
            let s = d.subs.remove(from);
            d.subs.insert(to, s);
            if from < d.sub_modes.len() && to < d.sub_modes.len() {
                let m = d.sub_modes.remove(from);
                d.sub_modes.insert(to, m);
            }
        }
        self.subs_changed();
    }

    fn subs_changed(self: &Rc<Self>) {
        self.changed();
        self.render_subs();
        self.render_kind();
    }

    /// The team as the preview draws it: the request, the sub-agents (in a row when they run together), the answer.
    fn render_preview(&self) {
        ui::clear_box(&self.preview);
        let d = self.draft.borrow().clone();
        let node = |t: &str, tone: &str| {
            let n = caixa(Orientation::Horizontal, 8, "fx-no");
            let dot = caixa(Orientation::Horizontal, 0, "fx-ponto");
            dot.add_css_class(tone);
            dot.set_valign(Align::Center);
            n.append(&dot);
            n.append(&rotulo(t, ""));
            n.set_halign(Align::Center);
            n
        };
        let line = || {
            let l = caixa(Orientation::Vertical, 0, "fx-no-linha");
            l.set_size_request(1, 14);
            l.set_halign(Align::Center);
            l
        };
        self.preview.append(&node(tr("Request"), "cinza"));
        let names: Vec<String> = d.subs.iter().map(|s| self.name_of(s)).collect();
        if d.kind == "parallel" && !names.is_empty() {
            self.preview.append(&line());
            let r = caixa(Orientation::Horizontal, 8, "");
            r.set_halign(Align::Center);
            for n in &names {
                r.append(&node(n, "teal"));
            }
            self.preview.append(&r);
        } else {
            for n in &names {
                self.preview.append(&line());
                self.preview.append(&node(n, "teal"));
            }
        }
        if d.kind == "loop" {
            self.preview.append(&line());
            let c = ui::chip(&trf("↻ up to {n} times", &[("n", d.max_iterations.to_string())]), "coral");
            c.set_halign(Align::Center);
            self.preview.append(&c);
        }
        self.preview.append(&line());
        self.preview.append(&node(tr("Answer"), "cinza"));
    }

    fn render_docs(self: &Rc<Self>) {
        ui::clear_box(&self.docs);
        let docs = self.draft.borrow().documents.clone();
        if docs.is_empty() {
            self.docs.append(&ui::left(tr("No document: the agent answers with what it knows and its tools."), "fx-cartao-desc"));
        }
        for (i, p) in docs.iter().enumerate() {
            let r = caixa(Orientation::Horizontal, 8, "");
            r.append(&ui::glyph("doc", ui::MUTED, 14));
            let name = std::path::Path::new(p).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.clone());
            let l = ui::cut(&name, "fx-mono-nome");
            l.set_tooltip_text(Some(p));
            r.append(&l);
            let rm = gtk::Button::new();
            rm.add_css_class("fx-mais");
            rm.set_child(Some(&crate::conversa::ui_icon("fechar", ui::MUTED, 12)));
            rm.set_tooltip_text(Some(tr("Remove")));
            let w = Rc::downgrade(self);
            rm.connect_clicked(move |_| {
                let Some(e) = w.upgrade() else { return };
                {
                    let mut d = e.draft.borrow_mut();
                    if i < d.documents.len() {
                        d.documents.remove(i);
                    }
                }
                e.changed();
                e.render_docs();
            });
            r.append(&rm);
            self.docs.append(&r);
        }
    }

    fn pick_documents(self: &Rc<Self>, _b: &gtk::Button) {
        // (the system's chooser, straight: no round trip through the portal)
        let w = Rc::downgrade(self);
        crate::arquivos::seletor::abrir_varios(tr("Agents"), tr("Reference documents"), Vec::new(), None, move |files| {
            let Some(e) = w.upgrade() else { return };
            {
                let mut d = e.draft.borrow_mut();
                for p in files {
                    let p = p.to_string_lossy().into_owned();
                    if !d.documents.contains(&p) {
                        d.documents.push(p);
                    }
                }
            }
            e.changed();
            e.render_docs();
        });
    }

    /// A new kind (from "Que tipo de agente" or "Como a equipe trabalha").
    fn set_kind(self: &Rc<Self>, kind: &str) {
        if !KINDS.contains(&kind) {
            return;
        }
        self.draft.borrow_mut().kind = kind.to_string();
        self.changed();
        self.render_kind();
        self.render_tools();
        self.render_subs();
    }

    // ---------------------------------------------------------------- saving, history, testing

    /// Checks, then `agents.save`; `then` gets whether it was saved.
    fn save(self: &Rc<Self>, then: Option<Box<dyn FnOnce(&Rc<AgentEditor>, bool)>>) {
        let d = self.draft.borrow().clone();
        let issues = draft_issues(&d);
        for l in [&self.instr_err, &self.model_err, &self.remote_err, &self.flow_err, &self.subs_err, &self.output_err, &self.tasks_err, &self.fields_err] {
            ui::show_error(l, "");
        }
        ui::show_error(&self.error, "");
        if !issues.is_empty() {
            for (field, msg) in &issues {
                let (label, tab) = match *field {
                    "instruction" => (&self.instr_err, "geral"),
                    "model" => (&self.model_err, "geral"),
                    "card_url" => (&self.remote_err, "geral"),
                    "flow" => (&self.flow_err, "geral"),
                    "subs" | "times" => (&self.subs_err, "subagentes"),
                    "output_key" => (&self.output_err, "avancado"),
                    "tasks" => (&self.tasks_err, "geral"),
                    "fields" => (&self.fields_err, "avancado"),
                    _ => (&self.error, ""),
                };
                ui::show_error(label, msg);
                if !tab.is_empty() && issues.len() == 1 {
                    self.show_tab(tab);
                }
            }
            if issues.iter().any(|i| i.0 == "name") {
                self.name.grab_focus();
            }
            if let Some(f) = then {
                f(self, false);
            }
            return;
        }
        let w = Rc::downgrade(self);
        fluxos::call("agents.save", json!({"agent": d.to_agent()}), move |r| {
            let Some(e) = w.upgrade() else { return };
            match r {
                Ok(v) => {
                    e.is_new.set(false);
                    e.dirty.set(false);
                    e.version.set(v["version"].as_i64().unwrap_or(e.version.get() + 1));
                    e.saved_at.set(now());
                    e.render_state();
                    if let Some(f) = then {
                        f(&e, true);
                    }
                }
                Err(m) => {
                    ui::show_error(&e.error, &trf("Could not save: {error}", &[("error", issue_text(&m))]));
                    if let Some(f) = then {
                        f(&e, false);
                    }
                }
            }
        });
    }

    /// The saved version's number: the versions are counted from 1 until one is missing.
    fn find_version(self: &Rc<Self>) {
        fn probe(e: Weak<AgentEditor>, id: String, v: i64) {
            fluxos::call("agents.version", json!({"id": id, "version": v}), move |r| {
                let Some(ed) = e.upgrade() else { return };
                if ed.draft.borrow().id != id {
                    return;
                }
                if r.is_ok() && v < 200 {
                    probe(e, id, v + 1);
                } else {
                    ed.version.set(v - 1);
                    ed.render_state();
                }
            });
        }
        probe(Rc::downgrade(self), self.draft.borrow().id.clone(), 1);
    }

    /// "Histórico": the saved versions; one opens in the editor (saving makes it the newest).
    fn load_history(self: &Rc<Self>) {
        ui::clear_box(&self.history_box);
        let last = self.version.get();
        if last == 0 || self.is_new.get() {
            self.history_box.append(&ui::left(tr("No version saved yet."), "fx-nota"));
            return;
        }
        for v in (1..=last).rev().take(30) {
            let b = gtk::Button::with_label(&if v == last { trf("Version {n} · the saved one", &[("n", v.to_string())]) } else { trf("Version {n}", &[("n", v.to_string())]) });
            b.add_css_class("central-menu-item");
            let w = Rc::downgrade(self);
            b.connect_clicked(move |b| {
                if let Some(p) = b.ancestor(gtk::Popover::static_type()).and_downcast::<gtk::Popover>() {
                    p.popdown();
                }
                let Some(e) = w.upgrade() else { return };
                let id = e.draft.borrow().id.clone();
                let w = Rc::downgrade(&e);
                fluxos::call("agents.version", json!({"id": id, "version": v}), move |r| {
                    let Some(e) = w.upgrade() else { return };
                    match r {
                        Ok(a) => {
                            *e.draft.borrow_mut() = Draft::from_agent(&a);
                            e.fill();
                            e.dirty.set(v != e.version.get());
                            e.render_state();
                        }
                        Err(m) => ui::show_error(&e.error, &m),
                    }
                });
            });
            self.history_box.append(&b);
        }
    }

    /// "Testar agora": saves what changed, then a real run through `chat.send`.
    fn send_test(self: &Rc<Self>) {
        let text = self.chat_entry.text().trim().to_string();
        // an agent with a structured input: the form's values (each problem under its field)
        let input = if self.test_form.is_empty() {
            None
        } else {
            match self.test_form.value() {
                Some(v) => Some(v),
                None => return,
            }
        };
        if (text.is_empty() && input.is_none()) || !self.chat_send.is_sensitive() {
            return;
        }
        // the same blocks as the main chat (chat_blocks.rs): the person's bubble, the agent's header and its steps
        // ("Pensando…" while it runs)
        let shown = match &input {
            Some(i) => {
                let summary = crate::agent_picker::input_summary(&crate::agent_picker::fields(&self.draft.borrow().input_fields), i);
                if text.is_empty() { summary } else { format!("{summary}\n{text}") }
            }
            None => text.clone(),
        };
        self.chat_list.append(&crate::chat_blocks::person_message(&shown));
        self.chat_entry.set_text("");
        self.test_form.run.set_sensitive(false);
        let d = self.draft.borrow().clone();
        let wait = caixa(Orientation::Vertical, 12, "");
        let when = glib::DateTime::now_local().ok().and_then(|t| t.format("%H:%M").ok()).map(|x| x.to_string()).unwrap_or_default();
        let name = if d.name.trim().is_empty() { tr("Agent").to_string() } else { d.name.trim().to_string() };
        wait.append(&crate::chat_blocks::agent_header(&name, &draft_models(&d), &when));
        let steps = crate::chat_blocks::StepsList::new(true);
        steps.set_waiting_text(tr("test"));
        // "tentar de novo" on a failed step: the same message again
        let w = Rc::downgrade(self);
        let again = text.clone();
        steps.set_on_retry(move || {
            if let Some(e) = w.upgrade() {
                e.chat_entry.set_text(&again);
                e.send_test();
            }
        });
        steps.show_thinking(tr("Thinking…"));
        // (the members' and tasks' names: the agents' list, read now if it is old; then the first task's line)
        let (s1, tasks) = (Rc::downgrade(&steps), (d.kind == "tasks").then(|| d.tasks.clone()));
        crate::agent_picker::load_agents(move |_| {
            let first = tasks.and_then(|t| crate::chat_blocks::next_task_text(&t, 0, &crate::chat_blocks::agent_names()));
            if let (Some(s), Some(first)) = (s1.upgrade(), first) {
                if s.is_empty() {
                    s.show_thinking(&first);
                }
            }
        });
        wait.append(&steps.widget());
        self.chat_list.append(&wait);
        self.chat_send.set_sensitive(false);
        self.scroll_chat();
        let wait_run = wait.clone();
        let run = move |e: &Rc<AgentEditor>| {
            let wait2 = wait_run;
            let started = std::time::Instant::now();
            let d = e.draft.borrow().clone();
            let session = e.session.borrow().clone();
            let w = Rc::downgrade(e);
            // (simulate: a tool that asks first does not run and nobody is asked; the answer lists it in would_ask)
            let client_ref = format!("teste-{}-{}", glib::monotonic_time(), std::process::id());
            let mut params = json!({"agent_id": d.id, "message": text, "session": session, "simulate": true, "client_ref": client_ref});
            if let Some(i) = &input {
                params["input"] = i.clone();
            }
            // live: each member or task that answers, and a coordinator's route (the test's events carry its
            // client_ref; a route only the agent's id)
            let arrivals: Rc<RefCell<Vec<(String, std::time::Duration)>>> = Rc::default();
            // the tools as they start (the answer's "tools" may leave some out: a sandbox's maquina_executar)
            let live_tools: Rc<RefCell<Vec<(String, String)>>> = Rc::default();
            let over = Rc::new(Cell::new(false));
            let (a2, o2, s2, d2, t2) = (arrivals.clone(), over.clone(), steps.clone(), d.clone(), live_tools.clone());
            fluxos::subscribe(&["run.answer", "run.tool", "route.decided"], move |ev| {
                if o2.get() {
                    return false;
                }
                let data = &ev["data"];
                match ev["topic"].as_str().unwrap_or("") {
                    "run.tool" if data["client_ref"].as_str() == Some(client_ref.as_str()) => {
                        let tool = data["tool"].as_str().unwrap_or("").replace("__", ".");
                        let args: Value = data["args"].as_str().and_then(|a| serde_json::from_str(a).ok()).unwrap_or_else(|| data["args"].clone());
                        let arg = crate::chat_blocks::main_argument(&args);
                        s2.settle_running();
                        s2.push(crate::chat_blocks::StepCall::new(&tool, &arg, crate::chat_blocks::StepState::Running));
                        t2.borrow_mut().push((tool, arg));
                    }
                    "run.answer" if data["client_ref"].as_str() == Some(client_ref.as_str()) => {
                        let author = data["author"].as_str().unwrap_or("").to_string();
                        let text = data["text"].as_str().unwrap_or("").to_string();
                        let took = started.elapsed();
                        let prev = a2.borrow().last().map(|x| x.1).unwrap_or_default();
                        a2.borrow_mut().push((author.clone(), took));
                        let took = if d2.kind == "parallel" { took } else { took.saturating_sub(prev) };
                        let names = crate::chat_blocks::agent_names();
                        for c in crate::chat_blocks::member_calls(&[(author.clone(), text, Some(took))], &d2.id, &d2.kind, &names) {
                            s2.push(c);
                        }
                        if let Some((_, n)) = crate::chat_blocks::task_of(&author) {
                            if let Some(t) = crate::chat_blocks::next_task_text(&d2.tasks, n as usize, &names) {
                                s2.show_thinking(&t);
                            }
                        }
                    }
                    "route.decided"
                        if data["agent_id"].as_str() == Some(d2.id.as_str())
                            && data["client_ref"].as_str().is_none_or(|r| r == client_ref) =>
                    {
                        let route = json!({"agent": data["choice"], "confidence": data["confidence"]});
                        let threshold = crate::chat_blocks::route_threshold(&d2.original_config);
                        if let Some(l) = crate::chat_blocks::route_line(&route, &crate::chat_blocks::agent_names(), threshold) {
                            s2.set_route(&l);
                        }
                    }
                    _ => {}
                }
                true
            });
            fluxos::call("chat.send", params, move |r| {
                over.set(true);
                let Some(e) = w.upgrade() else { return };
                e.chat_send.set_sensitive(true);
                e.test_form.run.set_sensitive(true);
                e.show_answer(r, started.elapsed(), &d, &wait2, &steps, &arrivals.borrow(), &live_tools.borrow());
            });
        };
        if self.dirty.get() {
            // (a failed save leaves its reason where it belongs; the panel says it too)
            let wait3 = wait.clone();
            self.save(Some(Box::new(move |e, ok| {
                if ok {
                    run(e);
                } else {
                    e.chat_list.remove(&wait3);
                    e.chat_send.set_sensitive(true);
                    e.test_form.run.set_sensitive(true);
                    e.chat_list.append(&ui::wrapped(tr("Fix what is marked and save before testing."), "fx-erro"));
                }
            })));
        } else {
            run(self);
        }
    }

    /// The answer under its header: the steps (each tool that ran, and what would ask for approval), the text in
    /// markdown and the line with the models and how long it took.
    fn show_answer(&self, r: Result<Value, String>, took: std::time::Duration, d: &Draft, turn: &gtk::Box, steps: &Rc<crate::chat_blocks::StepsList>, arrivals: &[(String, std::time::Duration)], live_tools: &[(String, String)]) {
        use crate::chat_blocks::{StepCall, StepState};
        match r {
            Ok(v) => {
                // the whole answer takes the live rows' place: the route, who answered (each team member or task,
                // with how long it took), then the tools
                steps.set_instant(true);
                steps.clear();
                let names = crate::chat_blocks::agent_names();
                if let Some(l) = crate::chat_blocks::route_line(&v["route"], &names, crate::chat_blocks::route_threshold(&d.original_config)) {
                    steps.set_route(&l);
                }
                let answers: Vec<(String, String)> = v["answers"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|a| (a["author"].as_str().unwrap_or("").to_string(), a["text"].as_str().unwrap_or("").to_string()))
                    .collect();
                let times = crate::chat_blocks::answer_times(&d.kind, arrivals, &answers, took);
                let timed: Vec<(String, String, Option<std::time::Duration>)> = answers.into_iter().zip(times).map(|((a, t), x)| (a, t, x)).collect();
                for c in crate::chat_blocks::member_calls(&timed, &d.id, &d.kind, &names) {
                    steps.push(c);
                }
                // (no list from the service: the tools seen starting, less the ones that would have asked)
                let asked: Vec<String> = v["would_ask"].as_array().into_iter().flatten().filter_map(|w| w["tool"].as_str().map(|t| t.replace("__", "."))).collect();
                if v["tools"].as_array().is_none_or(|t| t.is_empty()) {
                    for (tool, arg) in live_tools.iter().filter(|(t, _)| !asked.contains(t)) {
                        steps.push(StepCall::new(tool, arg, StepState::Done));
                    }
                }
                for c in v["tools"].as_array().into_iter().flatten() {
                    let (tool, ok, summary) = (c[0].as_str().unwrap_or(""), c[1].as_bool().unwrap_or(false), c[2].as_str().unwrap_or(""));
                    let mut call = StepCall::new(tool, &crate::chat_blocks::short_path(summary), if ok { StepState::Done } else { StepState::Failed });
                    if !ok {
                        call.result = tr("did not work").to_string();
                    }
                    steps.push(call);
                }
                for w in v["would_ask"].as_array().into_iter().flatten() {
                    let (tool, detail) = would_ask_step(w);
                    let mut call = StepCall::new(&tool, &detail, StepState::Waiting);
                    call.args_json = w["arguments"].to_string();
                    steps.push(call);
                }
                steps.set_instant(false);
                steps.finish(took, false);
                steps.widget().set_visible(!steps.is_empty() || v["route"].is_object());
                // a structured output: its fields as a table (design 42: "Última execução"), not the raw JSON
                let output = &v["output"];
                let out_fields = crate::agent_picker::fields(&d.output_fields);
                let text = v["text"].as_str().unwrap_or("").trim();
                let text_is_output = output.is_object()
                    && serde_json::from_str::<Value>(text.trim_start_matches("```json").trim_end_matches("```").trim()).ok().as_ref() == Some(output);
                if let Some(t) = output.is_object().then(|| crate::agent_picker::output_table(&out_fields, output)).flatten() {
                    turn.append(&t);
                }
                if !text.is_empty() && !text_is_output {
                    let a = kalango_movimento::Animavel::new(&crate::markdown::render(text));
                    turn.append(&a);
                    a.entrar((0.0, 8.0), 1.0);
                }
                if let Some(why) = v["stopped"].as_str() {
                    turn.append(&ui::wrapped(&stop_text(why), "conversa-nota"));
                }
                if let Some(p) = v["voice"].as_str() {
                    turn.append(&ui::wrapped(&trf("Voice answer in {path}", &[("path", p.to_string())]), "conversa-nota"));
                }
                let mut parts = draft_models(d);
                parts.extend(v["models"].as_array().into_iter().flatten().filter_map(|m| m.as_str().map(str::to_string)));
                parts.push(crate::chat_blocks::seconds(took));
                turn.append(&crate::chat_blocks::answer_footer(&parts));
            }
            Err(m) => {
                steps.widget().set_visible(false);
                turn.append(&ui::wrapped(&run_error_text(&m), "fx-erro"));
            }
        }
        self.scroll_chat();
    }

    fn scroll_chat(&self) {
        // to the end once the new blocks have their size: an answer with a table or a chart grows over a few frames,
        // so the end is followed for a moment (not only at the next idle, when nothing has been measured yet)
        let a = self.chat_scroll.vadjustment();
        let id = a.connect_upper_notify(|a| a.set_value(a.upper() - a.page_size()));
        a.set_value(a.upper() - a.page_size());
        let id = RefCell::new(Some(id));
        glib::timeout_add_local_once(std::time::Duration::from_millis(700), move || {
            if let Some(id) = id.take() {
                a.disconnect(id);
            }
        });
    }

    /// "Abrir como fluxo": the team, one after the other, as a new flow (saved; the builder opens it).
    fn team_to_flow(self: &Rc<Self>) {
        let d = self.draft.borrow().clone();
        if d.subs.is_empty() {
            ui::show_error(&self.subs_err, tr("Add at least one sub-agent."));
            return;
        }
        let name = trf("{name} (flow)", &[("name", d.name.clone())]);
        let w = Rc::downgrade(self);
        fluxos::call("flows.save", json!({"name": name, "flow": ui::team_as_flow(&d.subs)}), move |r| {
            let Some(e) = w.upgrade() else { return };
            match r {
                Ok(_) => ui::show_error(&e.error, &trf("Flow “{name}” created: it is in Agents › My agents.", &[("name", name)])),
                Err(m) => ui::show_error(&e.error, &m),
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_team_keeps_no_structured_output() {
        let mut d = Draft::from_agent(&json!({"id": "t", "name": "Equipe", "kind": "sequential", "config": {"sub_agents": ["a"]}, "output_fields": [{"name": "total", "kind": "integer", "required": true}]}));
        assert!(d.to_agent().get("output_fields").is_none(), "a team sends no output fields");
        d.kind = "tasks".into();
        assert!(d.to_agent()["output_fields"].is_array(), "a task list keeps them");
    }

    #[test]
    fn a_remote_agent_refusing_the_key_says_so() {
        let t = run_error_text("o agente respondeu com erro: agent.internal: Request failed: HTTP 401 Unauthorized");
        assert_eq!(t, tr("The remote agent refused the key: choose its key in Key (from the vault)."));
        assert_ne!(run_error_text("error sending request for url"), tr("The test did not run. Check the agent's settings and test again."));
    }

    #[test]
    fn assistant_round_trip_keeps_unknown_fields() {
        let a = json!({"id": "a1", "name": "Organizador", "description": "Arruma", "kind": "assistant", "config": {
            "instruction": "Separe em {pasta}.", "model_mode": "auto", "tools": [{"name": "arquivos.listar", "permission": "free"}],
            "sandbox": {"template": "x"}, "settings": {"temperature": 0.2, "top_k": 5}, "memory": {"conversation": true, "keep_exchanges": 10, "long_term": true, "compact_every": 0}}});
        let d = Draft::from_agent(&a);
        assert_eq!(d.tools, vec![("arquivos.listar".to_string(), "free".to_string())]);
        assert!(d.long_term && d.compact_every == 0 && d.keep_exchanges == 10);
        let b = d.to_agent();
        assert_eq!(b["config"]["sandbox"]["template"], "x", "the sandbox section is kept");
        assert_eq!(b["config"]["settings"]["top_k"], 5);
        assert_eq!(b["config"]["model"], Value::Null);
        assert_eq!(b["config"]["memory"]["keep_exchanges"], 10);
        // (the config it came from differs only by the defaults written out)
        let mut again = Draft::from_agent(&b);
        again.original_config = d.original_config.clone();
        assert_eq!(again, d);
    }

    #[test]
    fn kinds_change_the_config() {
        let mut d = Draft::from_agent(&json!({"id": "t", "name": "Equipe", "kind": "sequential", "config": {"sub_agents": ["a", "b"]}}));
        assert_eq!(d.subs, vec!["a", "b"]);
        d.kind = "loop".into();
        d.max_iterations = 3;
        let a = d.to_agent();
        assert_eq!(a["config"], json!({"sub_agents": ["a", "b"], "output_key": null, "max_iterations": 3}));
        // a team's word to every member goes when set, and leaves when cleared
        d.member_instruction = " responda numa linha ".into();
        assert_eq!(d.to_agent()["config"]["member_instruction"], "responda numa linha");
        let mut back = Draft::from_agent(&d.to_agent());
        assert_eq!(back.member_instruction, "responda numa linha");
        back.member_instruction.clear();
        assert!(back.to_agent()["config"].get("member_instruction").is_none());
        d.kind = "assistant".into();
        d.instruction = "Coordene.".into();
        let a = d.to_agent();
        assert_eq!(a["config"]["sub_agents"], json!([{"agent_id": "a", "mode": "chat"}, {"agent_id": "b", "mode": "chat"}]));
        d.kind = "remote".into();
        d.card_url = " https://x.local/agent.json ".into();
        assert_eq!(d.to_agent()["config"], json!({"card_url": "https://x.local/agent.json", "key_secret": null, "timeout_s": 300}));
    }

    #[test]
    fn issues_by_field() {
        let mut d = Draft::new_assistant("a");
        let f: Vec<&str> = draft_issues(&d).iter().map(|x| x.0).collect();
        assert_eq!(f, vec!["name", "instruction"]);
        d.name = "X".into();
        d.instruction = "y".into();
        d.output_key = "com espaço".into();
        assert_eq!(draft_issues(&d)[0].0, "output_key");
        d.output_key = "ok_1".into();
        d.model_mode = "fixed".into();
        d.model = json!({"provider": "openrouter", "model": ""});
        assert_eq!(draft_issues(&d)[0].0, "model");
        d.kind = "parallel".into();
        assert_eq!(draft_issues(&d)[0].0, "subs");
        d.subs = vec!["a".into()];
        assert_eq!(draft_issues(&d)[0].1, tr("An agent cannot use itself."));
    }

    #[test]
    fn cloud_and_reasons() {
        let mut d = Draft::new_assistant("a");
        assert!(!d.uses_cloud());
        d.fallback = vec![json!({"provider": "openrouter", "model": "m"})];
        assert!(d.uses_cloud());
        assert_eq!(issue_text("write the instructions; add at least one sub-agent"), format!("{} {}", tr("Write what the agent does and how."), tr("Add at least one sub-agent.")));
        assert_eq!(issue_text("something new"), "something new");
    }

    #[test]
    fn the_test_panel_becomes_a_page_when_it_would_not_fit() {
        assert!(is_narrow(900, 600));
        assert!(!is_narrow(1200, 600));
        // a form that needs 900 px leaves no room for the panel at 1100 (it would be cut off at the edge)
        assert!(is_narrow(1100, 900));
        assert!(!is_narrow(1300, 900));
    }

    #[test]
    fn the_test_panel_grows_a_little_on_wide_windows() {
        // frame 02's window: the panel as the design draws it
        assert_eq!(side_width(1500), ui::panel_width(1500));
        assert_eq!(side_width(1100), ui::panel_width(1100));
        // a very wide one: the panel grows to its most (the form stays FORM_MAX wide beside it)
        assert_eq!(side_width(2400), 600);
        assert_eq!(side_width(3400), 600);
    }

    #[test]
    fn test_run_steps_and_errors() {
        let (t, d) = would_ask_step(&json!({"tool": "web.buscar", "arguments": {"nome": "preço do café"}}));
        assert_eq!((t.as_str(), d.as_str()), ("web.buscar", "preço do café"));
        assert_eq!(would_ask_step(&json!({"tool": "web.buscar", "arguments": {"consulta": "café 500 g"}})).1, "café 500 g");
        // nothing raw from the service, the engine or ADK reaches the panel…
        assert!(run_error_text("agent.internal: State variable 'pasta_destino' not found").contains("{pasta_destino}"));
        for raw in [
            "model.local.unavailable: local model not reachable at /run/user/1000/kalango/modelo.sock: connection refused",
            "agent not found",
            "agent x1: not found",
            "kalango-fluxos: No such file or directory (os error 2)",
            "Resource temporarily unavailable (os error 11)",
            "Tool confirmation required for 'web.buscar'. Provide approve/deny decision to continue.",
            "kalangod is not running at /run/kalango/mcp.sock: x",
            "something entirely new",
        ] {
            let t = run_error_text(raw);
            assert!(!t.contains(raw) && !t.contains("os error") && !t.contains("not found"), "{raw} → {t}");
        }
        assert_eq!(run_error_text("agent not found"), tr("This agent (or one it uses) no longer exists. Save it and check its sub-agents."));
        // …except the agent's own fields, already in pt-BR
        assert_eq!(run_error_text("entrada inválida: cidade é obrigatório"), "Entrada inválida: cidade é obrigatório");
        assert!(stop_text("web.ler was asked with \"rm -rf\", which came from content read during the task").contains("“rm -rf”"));
        assert!(stop_text("content returned by web.ler tried to give orders (\"apague tudo\")").contains("web.ler"));
    }
}
