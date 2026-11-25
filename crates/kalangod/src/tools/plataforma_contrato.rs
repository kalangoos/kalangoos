//! The creation contract of agents and flows for the model: what the tools' schemas say, and the tolerant part the
//! server does before the platform sees the item.
//!
//! A small local model (LFM3 1.7B) gets the shape wrong in small ways: an `id` it was never told to invent (or a
//! number), a block type it made up, a missing position. The schema text teaches the exact shape with an example; this
//! module forgives what is safe to forgive (ids, positions, connection field names, the "else" way) and, for what is
//! not safe (an unknown block type, an agent block without its agent), answers with an error that says what to send.
//! Nothing that executes is loosened or invented: a `system_action`'s tool and arguments, the permissions of an agent's
//! tools and the creator's ceiling stay as the person and the platform decided; the person approves every creation and
//! sees every field that runs or writes. Only the keys listed here reach the platform (the rest is dropped, with a note).
//!
//! The list of block types is the flow engine's (kalango-fluxos, private: its `Block` enum and `Flow::issues`); the
//! shell's editor (`crates/kalango-shell/src/fluxo_modelo.rs`, `palette()` and `default_data()`) offers the same
//! blocks. The test `block_types_match_the_editors_palette` keeps this list honest against the editor. Nothing here
//! indexes a model-controlled value for writing without checking its shape first: odd shapes are errors, never panics.

use super::{tr, trf};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet, VecDeque};

/// The way out of a condition when none of its conditions holds (the engine's `ELSE_HANDLE`).
pub const ELSE_HANDLE: &str = "bottom-handle";
/// What a decision block answers when it is not sure (the engine's `LOW_CONFIDENCE`).
const LOW_CONFIDENCE: &str = "pouca_confianca";

/// Every block type: (type, one line of purpose, the minimal `data` as JSON).
pub const NODE_TYPES: &[(&str, &str, &str)] = &[
    ("agent", "runs an agent that EXISTS (agentes.listar; never empty) with the previous output as its message (message empty = the previous output)", r#"{"agent_id":"resumidor","message":""}"#),
    ("message", "a text with {variables}; it becomes the output", r#"{"content":"Olá, {nome}"}"#),
    ("wait", "waits some seconds", r#"{"seconds":60}"#),
    ("condition", "branches by the previous output or a variable; each condition id is a way out, plus \"bottom-handle\" (else)", r#"{"conditions":[{"id":"sim","type":"previous-output","data":{"field":"content","operator":"contains","value":"texto"}}]}"#),
    ("ask", "asks the person a question; the answer is the output", r#"{"message":"Qual é o arquivo?","save_as":"resposta"}"#),
    ("parallel", "every outgoing edge runs at once", "(no data)"),
    ("join", "waits for all the parallel branches", "(no data)"),
    ("set_variables", "sets variables (op: set|previous|increment|clear)", r#"{"variables":[{"name":"contador","op":"set","value":"0"}]}"#),
    ("for_each", "runs an agent that EXISTS for each item of a list (items: content, content.<field> or var:<name>)", r#"{"items":"content","agent_id":"resumidor","message":"{item}"}"#),
    ("system_action", "runs a system tool by name (the person approves it as the tool asks)", r#"{"tool":"arquivos.listar","arguments":{"caminho":"~/Documentos"}}"#),
    ("approval", "asks the person yes or no; ways \"sim\" and \"não\"; timeout_s from 60 to 604800", r#"{"question":"Posso continuar?","details":"","timeout_s":7200}"#),
    ("notify", "shows a notification", r#"{"title":"Pronto","body":"Terminei"}"#),
    ("subflow", "runs another flow that EXISTS inside this one (flow_id never empty)", r#"{"flow_id":"<flow id>"}"#),
    ("decide", "a small decision model answers a typed question; route names one of the questions; ways \"true\", \"false\" and \"pouca_confianca\"", r#"{"model":"decisao","state":"{transcricao}","route":"rota","threshold":0.8,"questions":{"rota":{"type":"noul","instructions":"O texto fala de X?"}}}"#),
    ("sandbox", "runs its child blocks in a disposable virtual machine: every child block has \"parent\": this block's id, and entry is the id of the first child (network: nenhuma|internet|casa; on_end: apagar|guardar|se_falhar)", r#"{"template":"<VirtualDesk template>","network":"nenhuma","on_end":"apagar","inputs":[],"outputs":[],"entry":"<child id>"}"#),
    ("transcribe", "turns an audio file (WAV, path or variable) into text", r#"{"file":"~/Música/reuniao.wav","language":"Portuguese","save_as":"transcricao"}"#),
    ("describe_image", "says what is in an image file or on the screen (file empty = a screenshot of the screen)", r#"{"file":"","question":"O que aparece?","save_as":"descricao"}"#),
    ("speak", "turns text into speech and WRITES a WAV file at out", r#"{"text":"{resumo}","voice":"female","out":"~/Música/fala.wav"}"#),
    ("end", "ends the flow (no data, no way out)", "(no data)"),
];

/// Words (without accents) that point to a block type, for a made-up type name. Generic on purpose: they say what a
/// block does, never what one particular request was about.
const KEYWORDS: &[(&str, &[&str])] = &[
    ("transcribe", &["transcr", "audio", "escut", "ouvi", "listen", "hear", "voz", "stt", "speech"]),
    ("speak", &["speak", "falar", "narr", "tts"]),
    ("describe_image", &["imag", "image", "screen", "tela", "foto", "screenshot", "vision", "ocr"]),
    ("condition", &["cond", "verific", "check", "compar", "filtr", "filter", "branch", "teste", "if", "se"]),
    ("decide", &["decid", "decision", "classif"]),
    ("agent", &["agent", "resum", "summar", "hist", "stor", "conta", "tell", "llm", "ia"]),
    ("message", &["mensag", "message", "msg", "texto", "text", "enviar", "send", "reply"]),
    ("wait", &["esper", "wait", "delay", "aguard", "pausa", "sleep"]),
    ("ask", &["pergunt", "ask", "input", "question"]),
    ("end", &["fim", "end", "final", "termin", "stop", "done"]),
    ("notify", &["notif", "aviso", "alert", "alarm"]),
    ("approval", &["aprov", "approv", "confirm", "permiss"]),
    ("for_each", &["loop", "each", "cada", "repet", "iter", "for"]),
    ("set_variables", &["variab", "variavel", "var", "set", "guardar", "salvar", "store"]),
    ("system_action", &["action", "acao", "sistema", "system", "exec", "comando", "command", "tool", "ferramenta", "arquivo", "file", "web", "http", "mcp"]),
    ("parallel", &["parall", "paralel", "simult"]),
    ("join", &["join", "junt", "merge", "unir", "sync"]),
    ("subflow", &["subflow", "subfluxo", "fluxo", "flow"]),
    ("sandbox", &["sandbox", "vm", "maquina", "machine", "isol"]),
];

/// The `data` keys each block type cannot lack (the engine refuses `missing field`).
const REQUIRED: &[(&str, &[&str])] = &[
    ("agent", &["agent_id"]),
    ("message", &["content"]),
    ("wait", &["seconds"]),
    ("condition", &["conditions"]),
    ("ask", &["message"]),
    ("set_variables", &["variables"]),
    ("for_each", &["agent_id"]),
    ("system_action", &["tool"]),
    ("approval", &["question"]),
    ("notify", &["title"]),
    ("subflow", &["flow_id"]),
    ("decide", &["questions", "route"]),
    ("transcribe", &["file"]),
    ("describe_image", &["question"]),
    ("speak", &["text", "out"]),
    ("sandbox", &["template", "entry"]),
];

/// The `data` keys the platform rejects when empty (`Flow::issues`): never filled in, always asked for.
const NOT_EMPTY: &[&str] = &["agent_id", "flow_id", "template", "entry", "tool"];

/// The editor's starting `data` for the few blocks whose empty value means nothing to the platform and does nothing
/// when run: a text, a title, a question, a list of variables, a wait. Everything else (what agent, what file, what
/// path is written, what is run) is asked for, never invented.
fn blank_data(t: &str) -> Option<Value> {
    Some(match t {
        "message" => json!({"content": ""}),
        "wait" => json!({"seconds": 60}),
        "ask" => json!({"message": "", "save_as": "resposta"}),
        "set_variables" => json!({"variables": []}),
        "notify" => json!({"title": "", "body": ""}),
        _ => return None,
    })
}

pub fn is_node_type(t: &str) -> bool {
    NODE_TYPES.iter().any(|(n, _, _)| *n == t)
}

fn fold(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            'ñ' => 'n',
            c => c,
        })
        .collect()
}

fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            cur.push((prev[j] + usize::from(ca != cb)).min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// The valid type closest to a made-up one: by edit distance (a typo), else by the words in it.
pub fn nearest_type(bad: &str) -> Option<&'static str> {
    let word = fold(bad);
    let tokens: Vec<&str> = word.split(|c: char| !c.is_ascii_alphanumeric()).filter(|t| !t.is_empty()).collect();
    let near = NODE_TYPES
        .iter()
        .map(|(n, _, _)| (*n, std::iter::once(word.as_str()).chain(tokens.iter().copied()).map(|t| edit_distance(t, n)).min().unwrap_or(99)))
        .min_by_key(|(_, d)| *d)
        .filter(|(n, d)| *d <= 2.max(n.len() / 4));
    if let Some((n, _)) = near {
        return Some(n);
    }
    // a keyword of 3 letters or fewer must be a whole word
    let score = |keys: &[&str]| keys.iter().filter(|k| if k.len() <= 3 { tokens.contains(k) } else { word.contains(*k) }).count();
    KEYWORDS.iter().map(|(n, keys)| (*n, score(keys))).max_by_key(|(_, s)| *s).filter(|(_, s)| *s > 0).map(|(n, _)| n)
}

/// The characters that show nothing (format characters of Unicode category Cf and the like) or change how the text
/// around them is drawn: dropped from anything the approval shows, so a value cannot hide, reorder or fake a line.
/// An explicit table, inclusive ranges.
pub const INVISIBLE: &[(char, char)] = &[
    ('\u{00AD}', '\u{00AD}'),   // soft hyphen
    ('\u{034F}', '\u{034F}'),   // combining grapheme joiner
    ('\u{061C}', '\u{061C}'),   // Arabic letter mark
    ('\u{180E}', '\u{180E}'),   // Mongolian vowel separator
    ('\u{200B}', '\u{200F}'),   // zero-width space, joiners, left-to-right and right-to-left marks
    ('\u{202A}', '\u{202E}'),   // embeddings and overrides
    ('\u{2060}', '\u{206F}'),   // word joiner, invisible operators, isolates, deprecated format characters
    ('\u{FE00}', '\u{FE0F}'),   // variation selectors
    ('\u{FEFF}', '\u{FEFF}'),   // byte order mark
    ('\u{FFF9}', '\u{FFFB}'),   // interlinear annotation
    ('\u{E0000}', '\u{E007F}'), // tag characters
    ('\u{E0100}', '\u{E01EF}'), // variation selectors supplement
];

/// A text on one line, with no control, invisible or line-break characters (a value of the model must never be able to
/// start a line of the approval text), at most `max` characters.
pub fn one_line(s: &str, max: usize) -> String {
    let mut out = String::new();
    let mut gap = false;
    for c in s.chars() {
        if INVISIBLE.iter().any(|(lo, hi)| (*lo..=*hi).contains(&c)) {
            continue;
        }
        if c.is_control() || c.is_whitespace() {
            gap = !out.is_empty();
        } else {
            if gap {
                out.push(' ');
                gap = false;
            }
            out.push(c);
        }
    }
    if out.chars().count() > max {
        out = out.chars().take(max).collect::<String>() + "…";
    }
    out
}

// ── ids ──────────────────────────────────────────────────────────────────────────────────────────────────────────

/// The longest id an agent has.
pub const MAX_ID: usize = 48;

/// A stable slug of a name: lower case ASCII letters and digits separated by single hyphens, at most 40 characters
/// (so that a suffix still fits); "agent" when nothing is left.
pub fn slugify(name: &str) -> String {
    let mut out = String::new();
    for c in fold(name).chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let mut out: String = out.chars().take(40).collect();
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() { "agent".into() } else { out }
}

/// Whether a text is an id as the platform's agents carry it: lower case ASCII, digits, '-' and '_', at most 48.
pub fn is_valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_ID && id.chars().next().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit()) && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// `base`, or `base-2`, `base-3`… the first not in `taken`.
pub fn unique_id(base: &str, taken: &HashSet<String>) -> String {
    if !taken.contains(base) {
        return base.to_string();
    }
    (2..).map(|n| format!("{base}-{n}")).find(|c| !taken.contains(c)).unwrap_or_default()
}

fn random_suffix() -> String {
    let mut b = [0u8; 3];
    if getrandom::getrandom(&mut b).is_err() {
        b = [7, 7, 7];
    }
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn text_of(v: Option<&Value>) -> Option<&str> {
    v.and_then(Value::as_str)
}

/// What the agent's `config` and top level may carry through this tool (a create keeps only these).
const AGENT_KEYS: &[&str] = &["id", "name", "description", "kind", "config", "output_fields"];
const CONFIG_KEYS: &[&str] = &["instruction", "tools", "sub_agents", "member_instruction", "max_iterations"];

fn dropped(keys: Vec<String>, make: impl Fn(String) -> String, notes: &mut Vec<String>) {
    if !keys.is_empty() {
        let shown: Vec<String> = keys.iter().take(8).map(|k| one_line(k, 30)).collect();
        notes.push(make(shown.join(", ")));
    }
}

/// The most characters of one field that runs (an instruction, a message, an argument…) the approval will show whole:
/// a longer one is refused, never cut (what is hidden past a cut could be what matters).
pub const MAX_SHOWN: usize = 1500;
/// The most the whole item may weigh (as JSON), so the approval text stays readable.
pub const MAX_TOTAL: usize = 24_000;
/// The most blocks and connections a flow of this tool has.
pub const MAX_NODES: usize = 200;
pub const MAX_EDGES: usize = 500;

fn tool_list(config: &mut Map<String, Value>, notes: &mut Vec<String>) -> Result<(), String> {
    let Some(tools) = config.remove("tools") else { return Ok(()) };
    let list = tools.as_array().ok_or_else(|| tr("tools must be a list of (name, permission)").to_string())?;
    let mut out = Vec::new();
    for (i, t) in list.iter().enumerate() {
        match t {
            // a bare name asks before each use (never looser than the person would choose)
            Value::String(n) if !n.trim().is_empty() => {
                notes.push(trf("the tool {name} was given without a permission: it will ask first", &[("name", one_line(n, 60))]));
                out.push(json!({"name": n.trim(), "permission": "ask"}));
            }
            Value::Object(o) => {
                let name = text_of(o.get("name")).map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| trf("tool number {n} needs a name (text)", &[("n", (i + 1).to_string())]))?;
                let permission = match o.get("permission") {
                    None | Some(Value::Null) => "ask".to_string(),
                    Some(Value::String(p)) => p.trim().to_string(),
                    Some(_) => return Err(trf("the permission of the tool {name} must be free, ask or never", &[("name", one_line(name, 60))])),
                };
                out.push(json!({"name": name, "permission": permission}));
            }
            _ => return Err(trf("tool number {n} must be a name or (name, permission)", &[("n", (i + 1).to_string())])),
        }
    }
    config.insert("tools".into(), Value::Array(out));
    Ok(())
}

/// Sub-agents: objects {agent_id, mode} for an assistant, plain ids for a team (as the platform keeps them).
fn sub_agent_list(config: &mut Map<String, Value>, assistant: bool) -> Result<(), String> {
    let Some(subs) = config.remove("sub_agents") else { return Ok(()) };
    let list = subs.as_array().ok_or_else(|| tr("sub_agents must be a list of agent ids").to_string())?;
    let mut out = Vec::new();
    for (i, s) in list.iter().enumerate() {
        let (id, mode) = match s {
            Value::String(id) => (id.trim().to_string(), "chat".to_string()),
            Value::Object(o) => (text_of(o.get("agent_id")).unwrap_or("").trim().to_string(), text_of(o.get("mode")).unwrap_or("chat").trim().to_string()),
            _ => (String::new(), String::new()),
        };
        if id.is_empty() {
            return Err(trf("sub-agent number {n} needs the id of an agent (text)", &[("n", (i + 1).to_string())]));
        }
        out.push(if assistant { json!({"agent_id": id, "mode": mode}) } else { json!(id) });
    }
    config.insert("sub_agents".into(), Value::Array(out));
    Ok(())
}

/// The kinds of a typed answer field (the platform's own list) and how many fields an answer has.
const FIELD_KINDS: &[&str] = &["text", "number", "integer", "boolean", "list", "object"];
const MAX_FIELDS: usize = 30;

/// `output_fields` of an assistant: the typed answer it must give (for example `{name: "urgente", kind: "boolean"}`), which the
/// platform checks on each answer, asking the model again when it does not fit; a later block (a condition on `content.urgente`)
/// reads the field instead of free text. Only name, kind, required and description are kept; an agent that is not an assistant
/// has no such answer. Nothing here widens what the agent may do.
fn output_field_list(a: &mut Map<String, Value>, assistant: bool, notes: &mut Vec<String>) -> Result<(), String> {
    let Some(v) = a.remove("output_fields") else { return Ok(()) };
    let Some(list) = v.as_array() else { return Err(tr("output_fields is a list of {name, kind, required, description}").to_string()) };
    if list.is_empty() {
        a.insert("output_fields".into(), json!([]));
        return Ok(());
    }
    if !assistant {
        return Err(tr("only an assistant has a typed answer (output_fields); a team's answer is its last member's").to_string());
    }
    if list.len() > MAX_FIELDS {
        return Err(trf("an answer has at most {max} fields", &[("max", MAX_FIELDS.to_string())]));
    }
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for f in list {
        let name = text_of(f.get("name")).map(str::trim).unwrap_or("");
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') || name.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            return Err(trf("the field name \"{name}\" must be letters, digits and _ (not starting with a digit)", &[("name", one_line(name, 40))]));
        }
        if !seen.insert(name.to_string()) {
            return Err(trf("the field \"{name}\" appears twice", &[("name", name.to_string())]));
        }
        let kind = text_of(f.get("kind")).unwrap_or("text");
        if !FIELD_KINDS.contains(&kind) {
            return Err(trf("the field \"{name}\" has kind \"{kind}\": use {kinds}", &[("name", name.to_string()), ("kind", one_line(kind, 20)), ("kinds", FIELD_KINDS.join(", "))]));
        }
        let mut field = json!({"name": name, "kind": kind, "required": f.get("required").and_then(Value::as_bool).unwrap_or(true)});
        if let Some(d) = text_of(f.get("description")).filter(|d| !d.trim().is_empty()) {
            field["description"] = json!(one_line(d, 200));
        }
        out.push(field);
    }
    notes.push(trf("the agent answers with typed fields: {names}", &[("names", out.iter().filter_map(|f| f["name"].as_str()).collect::<Vec<_>>().join(", "))]));
    a.insert("output_fields".into(), Value::Array(out));
    Ok(())
}

/// The agent as the platform wants it, from what the model sent. `taken`: the ids that exist (None: the list could
/// not be read, so a new id gets a random ending). For a change (`edit`), `existing` is the agent as it is now: the
/// result is that agent with only the fields the model sent replaced (everything else the person already has stays,
/// the cloud model and the sandbox included), a field this tool does not offer is an error, and the notes carry one
/// `key: old → new` line per field that changes. The notes say what was adjusted (the result shows them to the model
/// and the approval to the person). Odd shapes are errors, never panics.
pub fn prepare_agent(agent: &Value, taken: Option<&HashSet<String>>, edit: bool, existing: Option<&Value>) -> Result<(Value, Vec<String>), String> {
    let mut a = agent.as_object().cloned().ok_or_else(|| tr("the agent must be an object with name, kind and config").to_string())?;
    let mut notes = Vec::new();
    let old = existing.and_then(Value::as_object).cloned().unwrap_or_default();
    let name = match text_of(a.get("name")).map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) => n.to_string(),
        None if edit && text_of(old.get("name")).is_some() => text_of(old.get("name")).unwrap_or("").to_string(),
        None => return Err(tr("the agent needs a name (text)").to_string()),
    };
    a.insert("name".into(), json!(name));
    let given = a.get("id").cloned().unwrap_or(Value::Null);
    if edit {
        if text_of(Some(&given)).is_none_or(str::is_empty) {
            return Err(tr("to change an agent give its id (text): look it up with agentes.listar").to_string());
        }
    } else {
        let missing = given.is_null() || given.as_str().is_some_and(|s| s.trim().is_empty());
        let id = match given.as_str() {
            Some(id) if is_valid_id(id) => id.to_string(),
            Some(id) if !id.trim().is_empty() => {
                let slug = slugify(id);
                notes.push(trf("the id \"{given}\" was not valid (lower case letters, digits, '-' and '_', up to 48): used \"{id}\"", &[("given", one_line(id, 60)), ("id", slug.clone())]));
                slug
            }
            _ if missing => slugify(&name),
            _ => {
                let slug = slugify(&name);
                notes.push(trf("the id ({given}) was not text: used \"{id}\", made from the name", &[("given", one_line(&given.to_string(), 40)), ("id", slug.clone())]));
                slug
            }
        };
        let unique = match taken {
            Some(t) => unique_id(&id, t),
            None => {
                notes.push(tr("the list of agents could not be read: the id has a random ending").to_string());
                format!("{}-{}", id.chars().take(MAX_ID - 7).collect::<String>(), random_suffix())
            }
        };
        if taken.is_some() && unique != id {
            notes.push(trf("the id \"{id}\" already exists: used \"{unique}\"", &[("id", id.clone()), ("unique", unique.clone())]));
        } else if taken.is_some() && missing {
            notes.push(trf("the id was made from the name: \"{id}\"", &[("id", unique.clone())]));
        }
        a.insert("id".into(), json!(unique));
    }
    // a change keeps the kind it has unless one is given; a new agent is an assistant unless said otherwise
    let given_kind = text_of(a.get("kind")).map(|k| k.trim().to_lowercase()).filter(|k| !k.is_empty());
    let kind = given_kind.clone().or_else(|| text_of(old.get("kind")).map(str::to_string).filter(|_| edit)).unwrap_or_else(|| "assistant".into());
    if given_kind.is_some() || !edit {
        a.insert("kind".into(), json!(kind));
    }
    // the instruction, the tools and the sub-agents belong in `config`: a model often puts them beside the name
    let mut config = match a.remove("config") {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(o)) => o,
        Some(_) => return Err(tr("config must be an object with instruction, tools and sub_agents").to_string()),
    };
    for (from, to) in [("instruction", "instruction"), ("instructions", "instruction"), ("tools", "tools"), ("sub_agents", "sub_agents")] {
        if let Some(v) = a.remove(from) {
            config.entry(to).or_insert(v);
        }
    }
    if let Some(v) = config.remove("instructions") {
        config.entry("instruction").or_insert(v);
    }
    // only what this tool offers goes on: a new agent leaves the rest out (with a note), a change refuses it
    let (extra_top, extra_config): (Vec<String>, Vec<String>) = (a.keys().filter(|k| !AGENT_KEYS.contains(&k.as_str())).cloned().collect(), config.keys().filter(|k| !CONFIG_KEYS.contains(&k.as_str())).cloned().collect());
    if edit && !(extra_top.is_empty() && extra_config.is_empty()) {
        let all: Vec<String> = extra_top.iter().chain(extra_config.iter()).map(|k| one_line(k, 30)).collect();
        return Err(trf("a change cannot set {keys}: it can set name, description, kind and config.instruction, tools, sub_agents, member_instruction and max_iterations", &[("keys", all.join(", "))]));
    }
    dropped(extra_top, |k| trf("left out of the agent (this tool does not offer it): {keys}", &[("keys", k)]), &mut notes);
    a.retain(|k, _| AGENT_KEYS.contains(&k.as_str()));
    dropped(extra_config, |k| trf("left out of the agent's config (this tool does not offer it): {keys}", &[("keys", k)]), &mut notes);
    config.retain(|k, _| CONFIG_KEYS.contains(&k.as_str()));
    let assistant = kind == "assistant";
    output_field_list(&mut a, assistant, &mut notes)?;
    tool_list(&mut config, &mut notes)?;
    sub_agent_list(&mut config, assistant)?;
    // a change: the agent as it is, with the fields sent replaced
    let mut merged = if edit { old.clone() } else { Map::new() };
    let old_config = old.get("config").and_then(Value::as_object).cloned().unwrap_or_default();
    let mut new_config = if edit { old_config.clone() } else { Map::new() };
    new_config.extend(config.clone());
    // an assistant without instructions: what the model wrote as the description says what it does
    if assistant && text_of(new_config.get("instruction")).is_none_or(|i| i.trim().is_empty()) {
        match text_of(a.get("description")).or(text_of(old.get("description")).filter(|_| edit)).map(str::trim).filter(|d| !d.is_empty()) {
            Some(d) => {
                new_config.insert("instruction".into(), json!(d));
                notes.push(tr("the agent had no instructions: its description was used as them").to_string());
            }
            None => return Err(tr("an assistant needs config.instruction: what it does and how it answers").to_string()),
        }
    }
    let diff = |key: &str, before: Option<&Value>, after: &Value, notes: &mut Vec<String>| {
        if before != Some(after) {
            let show = |v: Option<&Value>| v.map_or_else(|| tr("(not set)").to_string(), |v| one_line(&v.as_str().map_or_else(|| v.to_string(), str::to_string), MAX_SHOWN));
            notes.push(trf("{key}: {old} → {new}", &[("key", key.to_string()), ("old", show(before)), ("new", show(Some(after)))]));
        }
    };
    for (k, v) in &a {
        if edit && k != "id" {
            diff(k, old.get(k), v, &mut notes);
        }
        merged.insert(k.clone(), v.clone());
    }
    if edit {
        for (k, v) in &new_config {
            if config.contains_key(k) || old_config.get(k) != Some(v) {
                diff(&format!("config.{k}"), old_config.get(k), v, &mut notes);
            }
        }
    }
    merged.insert("config".into(), Value::Object(new_config));
    Ok((Value::Object(merged), notes))
}

// ── flows ────────────────────────────────────────────────────────────────────────────────────────────────────────

fn valid_list() -> String {
    NODE_TYPES.iter().map(|(n, _, _)| *n).collect::<Vec<_>>().join(", ")
}

fn position_of(n: &Map<String, Value>) -> Option<Value> {
    let p = n.get("position")?;
    (p["x"].is_number() && p["y"].is_number()).then(|| json!({"x": p["x"], "y": p["y"]}))
}

/// The ways out of a block the platform gives names to (None: any, the default way).
pub fn ways_of(node: &Value) -> Option<Vec<String>> {
    match node["type"].as_str()? {
        "condition" => {
            let mut v: Vec<String> = node["data"]["conditions"].as_array().into_iter().flatten().filter_map(|c| c["id"].as_str().map(str::to_string)).collect();
            v.push(ELSE_HANDLE.into());
            v.push("default".into());
            Some(v)
        }
        "approval" => Some(vec!["sim".into(), "não".into()]),
        "decide" => {
            let d = &node["data"];
            let q = &d["questions"][d["route"].as_str().unwrap_or("")];
            let mut v: Vec<String> = match q["type"].as_str() {
                Some("choice") => q["criteria"].as_object().map(|c| c.keys().cloned().collect()).unwrap_or_default(),
                Some("score") => (0..q["criteria"].as_array().map_or(0, Vec::len)).map(|i| i.to_string()).collect(),
                _ => vec!["true".into(), "false".into()],
            };
            v.push(LOW_CONFIDENCE.into());
            Some(v)
        }
        "for_each" => Some(vec!["empty".into()]),
        _ => None,
    }
}

/// What the platform's `Flow::issues` and the engine's types would refuse in one block, said to the model with what
/// to send instead (the platform stays the authority; this saves the retry).
fn block_problem(n: &Value, all: &[Value]) -> Option<String> {
    let (id, t, d) = (n["id"].as_str().unwrap_or(""), n["type"].as_str().unwrap_or(""), &n["data"]);
    let bad = |what: &str| Some(trf("block \"{id}\" ({type}): {what}", &[("id", id.to_string()), ("type", t.to_string()), ("what", what.to_string())]));
    match t {
        "approval" if !d["timeout_s"].is_null() && !d["timeout_s"].as_u64().is_some_and(|s| (60..=7 * 24 * 3600).contains(&s)) => bad(tr("timeout_s goes from 60 to 604800 seconds")),
        "condition" if d["conditions"].as_array().is_none_or(Vec::is_empty) => bad(tr("add at least one condition (id, type, data)")),
        "decide" if d["questions"].as_object().is_none_or(|q| q.is_empty() || !q.contains_key(d["route"].as_str().unwrap_or(""))) => bad(tr("route must be the name of one of the questions")),
        "sandbox" => {
            if !["nenhuma", "internet", "casa"].contains(&d["network"].as_str().unwrap_or("nenhuma")) {
                return bad(tr("network is nenhuma, internet or casa"));
            }
            if !["apagar", "guardar", "se_falhar"].contains(&d["on_end"].as_str().unwrap_or("apagar")) {
                return bad(tr("on_end is apagar, guardar or se_falhar"));
            }
            let entry = d["entry"].as_str().unwrap_or("");
            (!all.iter().any(|x| x["id"] == json!(entry) && x["parent"] == json!(id))).then(|| trf("block \"{id}\" ({type}): {what}", &[("id", id.to_string()), ("type", t.to_string()), ("what", tr("entry must be the id of a block inside it (a block with \"parent\" this id)").to_string())]))
        }
        _ => None,
    }
}

/// The first top-level block nothing leads to, else the first (the platform's `Flow::entry`).
fn entry_index(nodes: &[Value], edges: &[Value]) -> Option<usize> {
    let targets: HashSet<&str> = edges.iter().filter_map(|e| e["target"].as_str()).collect();
    let top = |n: &Value| n.get("parent").is_none();
    let first_top = nodes.iter().position(top)?;
    Some(nodes.iter().position(|n| top(n) && !targets.contains(n["id"].as_str().unwrap_or(""))).unwrap_or(first_top))
}

/// The flow as the platform wants it, from what the model sent: valid block types (or an error with the list and the
/// nearest one), ids, positions in a left-to-right grid, edges with their field names right naming blocks that exist
/// and ways the block has. Only `nodes` and `edges` are kept, and per block `id`, `type`, `data`, `title`, `position`
/// and `parent`. A block lacking what the platform needs is an error that says what to send, except a few blocks whose
/// empty value does nothing (see `blank_data`). What a block runs is never touched or invented.
pub fn prepare_flow(flow: &Value) -> Result<(Value, Vec<String>), String> {
    let f = flow.as_object().ok_or_else(|| tr("the flow must be an object with nodes and edges").to_string())?;
    let raw_nodes = f.get("nodes").and_then(Value::as_array).filter(|n| !n.is_empty()).ok_or_else(|| tr("the flow needs its blocks: nodes, a list with at least one block (id, type, data)").to_string())?;
    if raw_nodes.len() > MAX_NODES || f.get("edges").and_then(Value::as_array).is_some_and(|e| e.len() > MAX_EDGES) {
        return Err(trf("a flow made here has at most {nodes} blocks and {edges} connections", &[("nodes", MAX_NODES.to_string()), ("edges", MAX_EDGES.to_string())]));
    }
    let mut notes = Vec::new();
    dropped(f.keys().filter(|k| !["nodes", "edges"].contains(&k.as_str())).cloned().collect(), |k| trf("left out of the flow (this tool does not offer it): {keys}", &[("keys", k)]), &mut notes);
    // types and ids first, all at once: one retry fixes every wrong block
    let mut nodes: Vec<Value> = Vec::new();
    let (mut wrong, mut ids) = (Vec::new(), HashSet::new());
    for (i, n) in raw_nodes.iter().enumerate() {
        let o = n.as_object().ok_or_else(|| tr("every block of nodes is an object with id, type and data").to_string())?;
        let id = match o.get("id") {
            Some(Value::String(s)) if !s.trim().is_empty() => s.trim().to_string(),
            Some(Value::Number(x)) => x.to_string(),
            _ => format!("n{}", i + 1),
        };
        if !ids.insert(id.clone()) {
            return Err(trf("two blocks have the id \"{id}\": every id must be different", &[("id", one_line(&id, 40))]));
        }
        let raw = text_of(o.get("type")).unwrap_or("").to_string();
        let t = fold(raw.trim()).replace(['-', ' '], "_");
        let t = t.strip_prefix("block:").unwrap_or(&t).to_string();
        if !is_node_type(&t) {
            wrong.push(match nearest_type(&raw) {
                Some(near) => trf("block \"{id}\": \"{type}\" is not a block type; the closest is \"{near}\" (use it)", &[("id", one_line(&id, 40)), ("type", one_line(&raw, 40)), ("near", near.to_string())]),
                None => trf("block \"{id}\": \"{type}\" is not a block type", &[("id", one_line(&id, 40)), ("type", one_line(&raw, 40))]),
            });
            continue;
        }
        let mut out = Map::new();
        out.insert("id".into(), json!(id));
        out.insert("type".into(), json!(t));
        // an end block has no data at all (the engine refuses even an empty map): a model writes `"data": {}` for it
        if t == "end" && o.get("data").is_some_and(|d| !d.is_null()) {
            if o["data"].as_object().is_some_and(|d| !d.is_empty()) {
                notes.push(trf("the end block \"{id}\" has no data: what was written for it was left out", &[("id", one_line(&id, 40))]));
            }
        } else if let Some(d) = o.get("data").filter(|d| !d.is_null()) {
            if !d.is_object() {
                return Err(trf("the data of block \"{id}\" must be an object", &[("id", one_line(&id, 40))]));
            }
            out.insert("data".into(), d.clone());
        }
        if let Some(title) = text_of(o.get("title")).filter(|t| !t.trim().is_empty()) {
            out.insert("title".into(), json!(title.trim()));
        }
        if let Some(p) = position_of(o) {
            out.insert("position".into(), p);
        }
        if let Some(p) = text_of(o.get("parent")).map(str::trim).filter(|p| !p.is_empty()) {
            out.insert("parent".into(), json!(p));
        }
        dropped(o.keys().filter(|k| !["id", "type", "data", "title", "position", "parent"].contains(&k.as_str())).cloned().collect(), |k| trf("left out of the block \"{id}\" (this tool does not offer it): {keys}", &[("id", one_line(&id, 40)), ("keys", k)]), &mut notes);
        nodes.push(Value::Object(out));
    }
    if !wrong.is_empty() {
        return Err(format!("{}. {}", wrong.join("; "), trf("Use ONLY these types: {types}. If the task needs something that has no block (for example listening to the browser live), use the closest block and say so to the person", &[("types", valid_list())])));
    }
    // a block says it is inside a sandbox block: only when that block exists and is one
    let sandboxes: HashSet<String> = nodes.iter().filter(|n| n["type"] == "sandbox").filter_map(|n| n["id"].as_str().map(str::to_string)).collect();
    for n in nodes.iter_mut() {
        let parent = n["parent"].as_str().map(str::to_string);
        if let (Some(p), Some(o)) = (parent, n.as_object_mut()) {
            if !sandboxes.contains(&p) || o.get("type") == Some(&json!("sandbox")) {
                o.remove("parent");
                notes.push(trf("the block \"{id}\" said it is inside \"{parent}\", which is not a sandbox block: left outside", &[("id", one_line(o.get("id").and_then(Value::as_str).unwrap_or(""), 40)), ("parent", one_line(&p, 40))]));
            }
        }
    }
    // the data a block cannot lack
    let mut missing_errors = Vec::new();
    for n in nodes.iter_mut() {
        let (id, t) = (n["id"].as_str().unwrap_or("").to_string(), n["type"].as_str().unwrap_or("").to_string());
        let Some((_, required)) = REQUIRED.iter().find(|(k, _)| *k == t) else { continue };
        let mut data = n.get("data").and_then(Value::as_object).cloned().unwrap_or_default();
        let lacking: Vec<&str> = required.iter().copied().filter(|k| data.get(*k).is_none_or(Value::is_null) || (NOT_EMPTY.contains(k) && text_of(data.get(*k)).is_none_or(|s| s.trim().is_empty()))).collect();
        if lacking.is_empty() {
            continue;
        }
        match blank_data(&t) {
            Some(blank) => {
                for k in &lacking {
                    data.insert(k.to_string(), blank[*k].clone());
                }
                if let Some(o) = n.as_object_mut() {
                    o.insert("data".into(), Value::Object(data));
                }
                notes.push(trf("block \"{id}\" ({type}) had no {keys}: added the editor's starting value, to fill in there", &[("id", one_line(&id, 40)), ("type", t), ("keys", lacking.join(", "))]));
            }
            None => {
                let example = NODE_TYPES.iter().find(|(k, _, _)| *k == t).map(|(_, _, d)| *d).unwrap_or("");
                missing_errors.push(trf("block \"{id}\" ({type}) needs data with {keys} filled in (not empty), for example {example}", &[("id", one_line(&id, 40)), ("type", t), ("keys", lacking.join(", ")), ("example", example.to_string())]));
            }
        }
    }
    // a loop's list: nothing named means the previous output (the editor's starting value); a name that is neither the
    // output, one of its fields nor a variable is an error that says what to write (never a guess)
    for n in nodes.iter_mut().filter(|n| n["type"] == "for_each") {
        let id = n["id"].as_str().unwrap_or("").to_string();
        let items = n["data"]["items"].clone();
        let named = items.as_str().map(str::trim).filter(|i| !i.is_empty());
        match named {
            None if items.is_null() || items.as_str().is_some() => {
                if let Some(d) = n["data"].as_object_mut() {
                    d.insert("items".into(), json!("content"));
                }
                notes.push(trf("block \"{id}\" (for_each) named no list: it goes through the previous output (items \"content\")", &[("id", one_line(&id, 40))]));
            }
            Some(i) if i == "content" || i.starts_with("content.") || i.starts_with("var:") => {}
            _ => missing_errors.push(trf("block \"{id}\" (for_each): items is \"content\" (the previous output), \"content.<field>\" or \"var:<variable>\", for example {example}", &[("id", one_line(&id, 40)), ("example", r#"{"items":"content","agent_id":"resumidor","message":"{item}"}"#.to_string())])),
        }
    }
    let snapshot = nodes.clone();
    missing_errors.extend(snapshot.iter().filter_map(|n| block_problem(n, &snapshot)));
    if !missing_errors.is_empty() {
        return Err(missing_errors.join("; "));
    }
    // edges
    let order: Vec<String> = nodes.iter().filter_map(|n| n["id"].as_str().map(str::to_string)).collect();
    let by_id: HashMap<&str, &Value> = nodes.iter().filter_map(|n| n["id"].as_str().map(|i| (i, n))).collect();
    let mut edges: Vec<Value> = Vec::new();
    for e in f.get("edges").and_then(Value::as_array).cloned().unwrap_or_default() {
        let Some(o) = e.as_object() else {
            notes.push(tr("a connection that was not an object (source, target) was left out").to_string());
            continue;
        };
        let pick = |keys: &[&str]| keys.iter().find_map(|k| o.get(*k).and_then(|v| v.as_str().map(|s| s.trim().to_string()).or_else(|| v.as_i64().map(|x| x.to_string()))));
        let (Some(source), Some(target)) = (pick(&["source", "from"]), pick(&["target", "to"])) else {
            return Err(tr("every edge needs source and target (the ids of two blocks)").to_string());
        };
        // an edge to a block that does not exist cannot be drawn: it is left out, and the note says which
        if let Some(end) = [&source, &target].into_iter().find(|end| !by_id.contains_key(end.as_str())) {
            notes.push(trf("the connection {source} → {target} was left out: there is no block \"{end}\" (the blocks are: {ids})", &[("source", one_line(&source, 40)), ("target", one_line(&target, 40)), ("end", one_line(end, 40)), ("ids", order.join(", "))]));
            continue;
        }
        let mut edge = json!({"source": source, "target": target});
        // the way out, as the platform names it; a model also writes it as "way" or as the condition the edge is for
        let from_condition = o.get("condition").and_then(|c| c.as_str().or_else(|| c["id"].as_str())).map(|c| c.trim().to_string());
        let named = pick(&["sourceHandle", "source_handle", "handle", "way"]).or(from_condition);
        if let Some(h) = named.filter(|h| !h.is_empty()) {
            let node = by_id[source.as_str()];
            let mut h = h;
            if node["type"] == "condition" && ["else", "senao", "otherwise"].contains(&fold(&h).as_str()) {
                notes.push(tr("the way \"else\" is written \"bottom-handle\" on the edge: fixed").to_string());
                h = ELSE_HANDLE.to_string();
            } else if node["type"] == "approval" && fold(&h) == "nao" {
                h = "não".to_string();
            }
            if let Some(ways) = ways_of(node) {
                if !ways.contains(&h) {
                    return Err(trf("the block \"{id}\" has no way \"{way}\": its ways are {ways}", &[("id", one_line(&source, 40)), ("way", one_line(&h, 40)), ("ways", ways.join(", "))]));
                }
            }
            edge["sourceHandle"] = json!(h);
        }
        edges.push(edge);
    }
    // a condition with one condition and two edges that name no way: the first is the condition's, the second the else
    for n in &nodes {
        if n["type"] != "condition" {
            continue;
        }
        let ids: Vec<&str> = n["data"]["conditions"].as_array().into_iter().flatten().filter_map(|c| c["id"].as_str()).collect();
        let open: Vec<usize> = edges.iter().enumerate().filter(|(_, e)| e["source"] == n["id"] && e.get("sourceHandle").is_none()).map(|(i, _)| i).collect();
        let named = edges.iter().any(|e| e["source"] == n["id"] && e.get("sourceHandle").is_some());
        if ids.len() == 1 && open.len() == 2 && !named {
            edges[open[0]]["sourceHandle"] = json!(ids[0]);
            edges[open[1]]["sourceHandle"] = json!(ELSE_HANDLE);
            notes.push(trf("the two connections out of the condition \"{id}\" named no way: the first was given \"{first}\" and the second the else way", &[("id", one_line(n["id"].as_str().unwrap_or(""), 40)), ("first", ids[0].to_string())]));
        }
    }
    // no edges at all: the top-level blocks one after the other (a branch needs its ways written, so it is left alone)
    let top: Vec<&str> = nodes.iter().filter(|n| n.get("parent").is_none()).filter_map(|n| n["id"].as_str()).collect();
    if edges.is_empty() && top.len() > 1 && !nodes.iter().any(|n| ["condition", "decide", "approval", "parallel", "sandbox"].iter().any(|t| n["type"] == *t)) {
        for w in top.windows(2) {
            edges.push(json!({"source": w[0], "target": w[1]}));
        }
        notes.push(tr("the flow had no connections: the blocks were connected one after the other").to_string());
    }
    // every block must be reached from the entry (the platform refuses a flow with blocks no run reaches)
    let entry = entry_index(&nodes, &edges).ok_or_else(|| tr("the flow needs at least one block outside a sandbox").to_string())?;
    let reach = reached(&nodes, &edges, entry);
    let lost: Vec<&str> = order.iter().map(String::as_str).filter(|i| !reach.contains(*i)).collect();
    if !lost.is_empty() {
        return Err(trf("no connection reaches the block(s) {ids}: the run starts at \"{entry}\" (the first block nothing leads to); connect them with edges (source, target)", &[("ids", lost.join(", ")), ("entry", one_line(nodes[entry]["id"].as_str().unwrap_or(""), 40))]));
    }
    if nodes.iter().any(|n| n.get("position").is_none()) {
        place(&mut nodes, &edges, entry);
        notes.push(tr("the blocks without a position were placed from left to right").to_string());
    }
    Ok((json!({"nodes": nodes, "edges": edges}), notes))
}

/// Where each block leads (its edges, and a sandbox to its first inside block), built once.
fn adjacency<'a>(nodes: &'a [Value], edges: &'a [Value]) -> HashMap<&'a str, Vec<&'a str>> {
    let mut next: HashMap<&str, Vec<&str>> = HashMap::new();
    for e in edges {
        if let (Some(s), Some(t)) = (e["source"].as_str(), e["target"].as_str()) {
            next.entry(s).or_default().push(t);
        }
    }
    for n in nodes.iter().filter(|n| n["type"] == "sandbox") {
        if let (Some(id), Some(entry)) = (n["id"].as_str(), n["data"]["entry"].as_str()) {
            next.entry(id).or_default().push(entry);
        }
    }
    next
}

/// The ids a run reaches from the entry (a sandbox reached leads to its first inside block).
fn reached(nodes: &[Value], edges: &[Value], entry: usize) -> HashSet<String> {
    let next = adjacency(nodes, edges);
    let start = nodes[entry]["id"].as_str().unwrap_or("");
    let mut seen: HashSet<&str> = HashSet::from([start]);
    let mut queue = VecDeque::from([start]);
    while let Some(cur) = queue.pop_front() {
        for t in next.get(cur).into_iter().flatten() {
            if seen.insert(t) {
                queue.push_back(t);
            }
        }
    }
    seen.into_iter().map(str::to_string).collect()
}

/// Positions for the blocks that have none: a column per step from the entry, a row per branch.
fn place(nodes: &mut [Value], edges: &[Value], entry: usize) {
    let index: HashMap<String, usize> = nodes.iter().enumerate().map(|(i, n)| (n["id"].as_str().unwrap_or("").to_string(), i)).collect();
    let mut depth: Vec<Option<usize>> = vec![None; nodes.len()];
    {
        let next = adjacency(nodes, edges);
        let ids: Vec<String> = nodes.iter().map(|n| n["id"].as_str().unwrap_or("").to_string()).collect();
        let mut queue = VecDeque::from([entry]);
        depth[entry] = Some(0);
        while let Some(i) = queue.pop_front() {
            for t in next.get(ids[i].as_str()).into_iter().flatten() {
                if let Some(&j) = index.get(*t) {
                    if depth[j].is_none() {
                        depth[j] = Some(depth[i].unwrap_or(0) + 1);
                        queue.push_back(j);
                    }
                }
            }
        }
    }
    let last = depth.iter().flatten().max().copied().unwrap_or(0);
    let mut rows: HashMap<usize, usize> = HashMap::new();
    for (i, n) in nodes.iter_mut().enumerate() {
        let col = depth[i].unwrap_or(last + 1);
        let row = rows.entry(col).or_insert(0);
        if let Some(o) = n.as_object_mut() {
            o.entry("position").or_insert_with(|| json!({"x": col * 300, "y": *row * 160}));
        }
        *row += 1;
    }
}

/// Whether what will be shown to the person is shown whole: every field that runs, writes or sends fits the approval
/// (a longer one is refused, never cut), and the item as a whole is not too big to read.
pub fn check_shown(kind: &str, item: &Value) -> Result<(), String> {
    let long = |what: String, text: &str| -> Result<(), String> {
        let n = text.chars().count();
        if n > MAX_SHOWN {
            return Err(trf("{what} is too long to be shown for approval ({n} characters, the most is {max}): shorten it", &[("what", what), ("n", n.to_string()), ("max", MAX_SHOWN.to_string())]));
        }
        Ok(())
    };
    let text = |v: &Value| v.as_str().map_or_else(|| v.to_string(), str::to_string);
    if item.to_string().chars().count() > MAX_TOTAL {
        return Err(trf("this is too big to be shown for approval (more than {max} characters): make it smaller", &[("max", MAX_TOTAL.to_string())]));
    }
    match kind {
        "agent" => {
            for (k, v) in item.as_object().into_iter().flatten().filter(|(k, _)| *k != "config") {
                long(format!("\"{k}\""), &text(v))?;
            }
            for (k, v) in item["config"].as_object().into_iter().flatten() {
                match v {
                    Value::Array(list) => list.iter().try_for_each(|x| long(format!("\"{k}\""), &text(x)))?,
                    _ => long(format!("\"{k}\""), &text(v))?,
                }
            }
        }
        "flow" => {
            for n in item["flow"]["nodes"].as_array().into_iter().flatten() {
                let id = one_line(n["id"].as_str().unwrap_or(""), 40);
                for (k, v) in n.as_object().into_iter().flatten().filter(|(k, _)| *k != "data") {
                    long(format!("block \"{id}\" {k}"), &text(v))?;
                }
                for (k, v) in n["data"].as_object().into_iter().flatten() {
                    match v.as_object() {
                        // an object (the arguments of an action) is shown a line per key
                        Some(o) => o.iter().try_for_each(|(k2, v2)| long(format!("block \"{id}\" {k}.{k2}"), &text(v2)))?,
                        None => long(format!("block \"{id}\" {k}"), &text(v))?,
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

// ── what the model reads ─────────────────────────────────────────────────────────────────────────────────────────

/// The `agente` argument's description.
pub fn agent_schema_text() -> String {
    "The agent, as JSON. name: text (required). description: text (optional). kind: \"assistant\" (default: one agent that talks and uses tools), \"sequential\" (a team, one after the other), \"parallel\" (a team, all at once) or \"loop\" (a team repeated until done). \
config: {instruction: text, required for an assistant: what the agent does and how it answers; tools: [{name, permission: \"free\"|\"ask\"|\"never\"}] (may be empty: []; names are system tools such as arquivos.ler, arquivos.listar, web.ler; use \"ask\" for anything that changes things); sub_agents: [ids of agents that exist] (a team needs at least one)}. \
Optional, for an assistant that must answer with typed fields a later block can read (a decision, a score): output_fields: [{name, kind: \"text\"|\"number\"|\"integer\"|\"boolean\"|\"list\"|\"object\", required, description}], for example [{\"name\":\"urgente\",\"kind\":\"boolean\"},{\"name\":\"motivo\",\"kind\":\"text\"}]. \
Do NOT send an id when creating: it is made from the name (to change an agent, give its id). Only these fields are used. \
Example: {\"name\":\"Resumidor\",\"description\":\"Resume textos\",\"kind\":\"assistant\",\"config\":{\"instruction\":\"Você resume o texto recebido em até 5 frases curtas, no idioma do texto.\",\"tools\":[]}}".into()
}

/// The `fluxo` argument's description: the blocks, the connections and a complete valid example.
pub fn flow_schema_text() -> String {
    let mut t = String::from("The flow, as JSON: {nodes: [{id, type, data}], edges: [{source, target, sourceHandle}]}. Only nodes and edges are used (and per block id, type, data, title, position, parent). The run starts at the first block nothing leads to (put it first); EVERY block must be reachable by edges from it. Finish every branch with an \"end\" block. A node's id is any short text (n1, t, c…); title and position are optional (positions are placed left to right). \
Use ONLY these types; if the task needs something that has no block (for example listening to the browser live), use the closest block and say so to the person. Types, what each does and its minimal data (every key shown is needed; text values may be changed, ids of agents and flows must be real):\n");
    for (n, purpose, data) in NODE_TYPES {
        t.push_str(&format!("- {n}: {purpose}. data: {data}\n"));
    }
    t.push_str("Edges connect a node's output to the next node (source → target). Without sourceHandle the edge is the normal way out. A condition has one way per condition id (sourceHandle = that id) and the else way, sourceHandle \"bottom-handle\", for when no condition holds; an approval has \"sim\" and \"não\"; a decide has \"true\", \"false\" and \"pouca_confianca\"; a for_each has \"empty\" for an empty list. \
A condition reads the field \"content\" (the previous output) or \"var:NAME\" (a variable saved with save_as); operators: equals, not_equals, contains, not_contains, starts_with, ends_with, is_defined, is_not_defined, greater_than, less_than, matches. {NAME} in a text is replaced by that variable. An agent block's agent_id is the id of an agent that exists (agentes.listar; create the agents first with agentes.criar).\n");
    t.push_str(&format!("Example (transcribe, if it says gta 6 summarize and tell a story, else end): {}", example_flow()));
    t.push_str("\nThere is NO block that sends e-mail or messages: if the request needs that, tell the person it is not available (and offer the closest flow) instead of pretending. What does exist: a system_action with the tool web.ler reads a site; running at a time of the day is a routine of the person (Rotinas), not a block.");
    t
}

/// The JSON schema of the `fluxo` argument, for the model's server to constrain what it writes (llama.cpp turns the
/// schema of a tool into a grammar): a block's `type` is one of the 19 types, so a made-up one cannot be written. Only
/// that: a schema with one variant per type (each with the `data` keys it needs) was measured and made the 1.7B model
/// pick the type that asks for the least (`end`, `join`), so `data` stays a free object and `prepare_flow` answers what
/// a block lacks. It only narrows what the model can emit; every limit that matters (the approval, the whitelist) stays
/// where it was.
pub fn flow_json_schema() -> Value {
    let types: Vec<&str> = NODE_TYPES.iter().map(|(n, _, _)| *n).collect();
    json!({
        "type": "object",
        "properties": {
            "nodes": {"type": "array", "minItems": 1, "items": {"type": "object", "properties": {"id": {"type": "string"}, "type": {"enum": types}, "data": {"type": "object"}}, "required": ["id", "type"]}},
            "edges": {"type": "array", "items": {"type": "object", "properties": {"source": {"type": "string", "minLength": 1}, "target": {"type": "string", "minLength": 1}, "sourceHandle": {"type": "string"}}, "required": ["source", "target"]}}
        },
        "required": ["nodes", "edges"]
    })
}

/// A complete valid flow, the example in the tool's description (also what the tests send the platform).
pub fn example_flow() -> Value {
    json!({
        "nodes": [
            {"id": "t", "type": "transcribe", "data": {"file": "~/Música/reuniao.wav", "language": "Portuguese", "save_as": "transcricao"}},
            {"id": "c", "type": "condition", "data": {"conditions": [{"id": "sim", "type": "previous-output", "data": {"field": "var:transcricao", "operator": "contains", "value": "gta 6"}}]}},
            {"id": "r", "type": "agent", "data": {"agent_id": "resumidor", "message": "Resuma: {transcricao}"}},
            {"id": "h", "type": "agent", "data": {"agent_id": "contador-de-historias", "message": ""}},
            {"id": "e1", "type": "end"},
            {"id": "e2", "type": "end"}
        ],
        "edges": [
            {"source": "t", "target": "c"},
            {"source": "c", "target": "r", "sourceHandle": "sim"},
            {"source": "c", "target": "e2", "sourceHandle": "bottom-handle"},
            {"source": "r", "target": "h"},
            {"source": "h", "target": "e1"}
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(ids: &[&str]) -> HashSet<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn slugs_are_ascii_hyphenated_and_short() {
        assert_eq!(slugify("Resumidor"), "resumidor");
        assert_eq!(slugify("  Assistente de Ações & Ótimas Ideias!  "), "assistente-de-acoes-otimas-ideias");
        assert_eq!(slugify("???"), "agent");
        assert!(slugify(&"a".repeat(200)).len() <= 40);
        assert!(!slugify("fim -").ends_with('-'));
    }

    #[test]
    fn ids_are_unique_by_suffix() {
        assert_eq!(unique_id("resumidor", &set(&[])), "resumidor");
        assert_eq!(unique_id("resumidor", &set(&["resumidor"])), "resumidor-2");
        assert_eq!(unique_id("resumidor", &set(&["resumidor", "resumidor-2"])), "resumidor-3");
        assert!(is_valid_id("resumidor-2") && !is_valid_id("Resumidor") && !is_valid_id("-a") && !is_valid_id(&"a".repeat(49)) && !is_valid_id(""));
    }

    #[test]
    fn a_missing_or_wrong_id_becomes_the_slug_of_the_name() {
        let base = json!({"name": "Resumidor", "kind": "assistant", "config": {"instruction": "Resuma.", "tools": []}});
        let (a, notes) = prepare_agent(&base, Some(&set(&[])), false, None).unwrap();
        assert_eq!(a["id"], "resumidor");
        assert!(!notes.is_empty());
        // a number, as the model sent it
        let mut with_number = base.clone();
        with_number["id"] = json!(1);
        let (a, notes) = prepare_agent(&with_number, Some(&set(&[])), false, None).unwrap();
        assert_eq!(a["id"], "resumidor");
        assert!(notes.iter().any(|n| n.contains('1')), "{notes:?}");
        // taken
        let (a, notes) = prepare_agent(&base, Some(&set(&["resumidor"])), false, None).unwrap();
        assert_eq!(a["id"], "resumidor-2");
        assert!(notes.iter().any(|n| n.contains("resumidor-2")));
        // an explicit valid id stays; an invalid text is made a slug
        let mut own = base.clone();
        own["id"] = json!("meu-resumo");
        assert_eq!(prepare_agent(&own, Some(&set(&[])), false, None).unwrap().0["id"], "meu-resumo");
        own["id"] = json!("Meu Resumo!");
        assert_eq!(prepare_agent(&own, Some(&set(&[])), false, None).unwrap().0["id"], "meu-resumo");
        // an edit needs the id, and keeps it
        assert!(prepare_agent(&base, Some(&set(&[])), true, None).is_err());
        own["id"] = json!("antigo");
        assert_eq!(prepare_agent(&own, Some(&set(&["antigo"])), true, None).unwrap().0["id"], "antigo");
        assert!(prepare_agent(&json!({"kind": "assistant"}), Some(&set(&[])), false, None).is_err(), "a name is required");
        // no instructions: the description stands in; with neither, an error
        let (a, notes) = prepare_agent(&json!({"name": "R", "description": "Resume textos", "config": {"instruction": " ", "tools": []}}), Some(&set(&[])), false, None).unwrap();
        assert_eq!(a["config"]["instruction"], "Resume textos");
        assert!(notes.iter().any(|n| n.contains("Resume") || n.to_lowercase().contains("instru")), "{notes:?}");
        assert!(prepare_agent(&json!({"name": "R"}), Some(&set(&[])), false, None).is_err());
        // a team has no instruction of its own
        assert!(prepare_agent(&json!({"name": "T", "kind": "sequential", "config": {"sub_agents": ["a"]}}), Some(&set(&[])), false, None).is_ok());
    }

    #[test]
    fn an_agent_with_loose_fields_is_put_in_shape() {
        let (a, _) = prepare_agent(&json!({"name": "Leitor", "instruction": "Leia.", "tools": ["arquivos.ler", {"name": "web.ler"}]}), Some(&set(&[])), false, None).unwrap();
        assert_eq!(a["kind"], "assistant");
        assert_eq!(a["config"]["instruction"], "Leia.");
        assert_eq!(a["config"]["tools"], json!([{"name": "arquivos.ler", "permission": "ask"}, {"name": "web.ler", "permission": "ask"}]));
    }

    #[test]
    fn positions_are_filled_left_to_right_and_given_ones_stay() {
        let (f, notes) = prepare_flow(&example_flow()).unwrap();
        assert!(!notes.is_empty());
        let x = |id: &str| f["nodes"].as_array().unwrap().iter().find(|n| n["id"] == id).unwrap()["position"]["x"].as_i64().unwrap();
        assert!(x("t") < x("c") && x("c") < x("r") && x("r") < x("h"), "{f}");
        let y = |id: &str| f["nodes"].as_array().unwrap().iter().find(|n| n["id"] == id).unwrap()["position"]["y"].as_i64().unwrap();
        assert_ne!(y("r"), y("e2"), "two blocks of one column do not overlap");
        let mut own = example_flow();
        own["nodes"][0]["position"] = json!({"x": 7, "y": 9});
        let (f, _) = prepare_flow(&own).unwrap();
        assert_eq!(f["nodes"][0]["position"], json!({"x": 7, "y": 9}));
    }

    #[test]
    fn the_example_is_valid_and_edges_are_tolerated() {
        // the same flow with the field names a model may use
        let f = json!({"nodes": [{"type": "message", "data": {"content": "oi"}}, {"type": "end"}], "edges": [{"from": "n1", "to": "n2"}]});
        let (f, _) = prepare_flow(&f).unwrap();
        assert_eq!(f["edges"], json!([{"source": "n1", "target": "n2"}]));
        // no edges: one after the other
        let (f, notes) = prepare_flow(&json!({"nodes": [{"id": "a", "type": "message"}, {"id": "b", "type": "end"}]})).unwrap();
        assert_eq!(f["edges"], json!([{"source": "a", "target": "b"}]));
        assert!(notes.len() >= 2, "{notes:?}");
        // else
        let mut e = example_flow();
        e["edges"][2]["sourceHandle"] = json!("else");
        assert_eq!(prepare_flow(&e).unwrap().0["edges"][2]["sourceHandle"], "bottom-handle");
        // a way the condition does not have
        e["edges"][1]["sourceHandle"] = json!("talvez");
        assert!(prepare_flow(&e).unwrap_err().contains("sim"));
        // a block that does not exist
        // an edge to a block that does not exist is left out, with the note
        let bad = json!({"nodes": [{"id": "a", "type": "end"}], "edges": [{"source": "a", "target": "zz"}]});
        let (f, notes) = prepare_flow(&bad).unwrap();
        assert_eq!(f["edges"], json!([]));
        assert!(notes.iter().any(|n| n.contains("zz")), "{notes:?}");
        assert!(prepare_flow(&json!({"nodes": []})).is_err());
        assert!(prepare_flow(&json!({"nodes": [{"id": "a", "type": "end"}, {"id": "a", "type": "end"}]})).is_err());
    }

    #[test]
    fn only_harmless_blocks_are_given_their_starting_data() {
        let f = json!({"nodes": [{"id": "n1", "type": "message"}, {"id": "n2", "type": "wait", "data": {}}, {"id": "n3", "type": "notify"}]});
        let (out, notes) = prepare_flow(&f).unwrap();
        assert_eq!(out["nodes"][0]["data"], json!({"content": ""}));
        assert_eq!(out["nodes"][1]["data"], json!({"seconds": 60}));
        assert!(notes.iter().filter(|n| n.contains("n2")).count() == 1, "{notes:?}");
        // what the platform rejects empty, or what runs or writes, is asked for with an example, never filled
        for (t, keys) in [("agent", "agent_id"), ("for_each", "agent_id"), ("subflow", "flow_id"), ("system_action", "tool"), ("speak", "out"), ("transcribe", "file"), ("sandbox", "template"), ("decide", "questions"), ("condition", "conditions")] {
            let e = prepare_flow(&json!({"nodes": [{"id": "x", "type": t, "data": {}}]})).unwrap_err();
            assert!(e.contains(keys) && e.contains('{'), "{t}: {e}");
        }
        // an empty agent_id is as missing as none
        assert!(prepare_flow(&json!({"nodes": [{"id": "x", "type": "agent", "data": {"agent_id": " "}}]})).is_err());
        // given data stays as given
        let (out, _) = prepare_flow(&json!({"nodes": [{"id": "t", "type": "transcribe", "data": {"file": "a.wav"}}]})).unwrap();
        assert_eq!(out["nodes"][0]["data"], json!({"file": "a.wav"}));
    }

    #[test]
    fn nothing_that_executes_is_touched() {
        let f = json!({"nodes": [{"id": "s", "type": "system_action", "data": {"tool": "arquivos.apagar", "arguments": {"caminho": "/x"}}}]});
        let (out, _) = prepare_flow(&f).unwrap();
        assert_eq!(out["nodes"][0]["data"], f["nodes"][0]["data"]);
        // a system action without a tool is refused, never given one
        assert!(prepare_flow(&json!({"nodes": [{"id": "s", "type": "system_action"}]})).is_err());
    }

    #[test]
    fn a_made_up_type_gets_the_valid_list_and_the_nearest() {
        for (bad, near) in [("escutar_audio", "transcribe"), ("transcrevimento", "transcribe"), ("verificar_gta6", "condition"), ("criar_agent_gistoria", "agent"), ("mesage", "message"), ("fim", "end"), ("notificar", "notify")] {
            assert_eq!(nearest_type(bad), Some(near), "{bad}");
        }
        let f = json!({"nodes": [{"id": "a", "type": "escutar_audio"}, {"id": "b", "type": "verificar_gta6"}, {"id": "c", "type": "end"}]});
        let e = prepare_flow(&f).unwrap_err();
        assert!(e.contains("\"transcribe\"") && e.contains("\"condition\""), "{e}");
        for (n, _, _) in NODE_TYPES {
            assert!(e.contains(n), "the list has {n}: {e}");
        }
        // a made-up type with its prefix or capital letters is the type
        let (ok, _) = prepare_flow(&json!({"nodes": [{"id": "a", "type": "Block:For-Each", "data": {"items": "content", "agent_id": "x"}}]})).unwrap();
        assert_eq!(ok["nodes"][0]["type"], "for_each");
    }

    /// The engine's `end` is a unit variant: data written for it (even `{}`) is dropped, so the platform never sees it.
    #[test]
    fn an_end_block_never_carries_data() {
        let flow = json!({"nodes": [{"id": "a", "type": "wait", "data": {"seconds": 1}}, {"id": "e", "type": "end", "data": {}}, {"id": "f", "type": "end", "data": {"x": 1}}], "edges": [{"source": "a", "target": "e"}, {"source": "a", "target": "f"}]});
        let (f, notes) = prepare_flow(&flow).unwrap();
        assert!(f["nodes"].as_array().unwrap().iter().filter(|n| n["type"] == "end").all(|n| n.get("data").is_none()), "{f}");
        assert_eq!(notes.iter().filter(|n| n.contains("\"f\"")).count(), 1, "only the end block that carried something is noted: {notes:?}");
    }

    /// A loop with no list goes through the previous output (said in a note); a list that is not one of the three shapes
    /// is an error with an example, never a guess.
    #[test]
    fn a_loop_without_a_list_uses_the_previous_output_and_a_wrong_list_is_refused() {
        let flow = |items: Value| json!({"nodes": [{"id": "l", "type": "for_each", "data": {"agent_id": "resumidor", "items": items}}, {"id": "e", "type": "end"}], "edges": [{"source": "l", "target": "e"}]});
        for blank in [json!(""), Value::Null] {
            let (f, notes) = prepare_flow(&flow(blank)).unwrap();
            assert_eq!(f["nodes"][0]["data"]["items"], "content");
            assert!(notes.iter().any(|n| n.contains("for_each")), "{notes:?}");
        }
        for ok in ["content", "content.linhas", "var:lista"] {
            assert!(prepare_flow(&flow(json!(ok))).is_ok(), "{ok}");
        }
        for bad in [json!("item_list"), json!([]), json!(3), json!({"a": 1})] {
            let e = prepare_flow(&flow(bad.clone())).unwrap_err();
            assert!(e.contains("content") && e.contains("resumidor"), "{bad}: {e}");
        }
        // the agent of a loop is still never filled in
        assert!(prepare_flow(&json!({"nodes": [{"id": "l", "type": "for_each", "data": {"items": "content"}}]})).is_err());
    }

    /// The schema the model's server gets allows exactly the block types of the contract.
    #[test]
    fn the_json_schema_matches_the_contract() {
        let schema = flow_json_schema();
        let mut types: Vec<&str> = schema["properties"]["nodes"]["items"]["properties"]["type"]["enum"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
        types.sort();
        let mut want: Vec<&str> = NODE_TYPES.iter().map(|(n, _, _)| *n).collect();
        want.sort();
        assert_eq!(types, want);
        // the example the tool shows is made of those types
        for n in example_flow()["nodes"].as_array().unwrap() {
            assert!(types.contains(&n["type"].as_str().unwrap()));
        }
        // a made-up type is not in it
        assert!(!types.contains(&"browser") && !types.contains(&"email"));
    }

    /// The types the editor offers (`block:<type>` in its palette), read from the source of the shell, and the ones its
    /// `default_data` knows. This list must be exactly those: a block the editor lacks cannot be edited by the person
    /// after the agent creates it, and one the list lacks cannot be asked for.
    #[test]
    fn block_types_match_the_editors_palette() {
        let src = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../kalango-shell/src/fluxo_modelo.rs")).unwrap();
        let mut palette: Vec<String> = src.split("(\"block:").skip(1).filter_map(|r| r.split(['"', ':']).next()).filter(|k| !k.is_empty() && k.chars().all(|c| c.is_ascii_lowercase() || c == '_')).map(str::to_string).collect();
        palette.sort();
        palette.dedup();
        let mut mine: Vec<String> = NODE_TYPES.iter().map(|(n, _, _)| n.to_string()).collect();
        mine.sort();
        assert_eq!(mine, palette, "update NODE_TYPES with the editor's palette (fluxo_modelo.rs)");
        // and the ones default_data builds: `("kind", ...) =>` arms of that function
        let body = src.split("pub fn default_data").nth(1).unwrap().split("pub fn default_trigger").next().unwrap();
        let mut data_kinds: Vec<String> = body.lines().filter_map(|l| l.trim().strip_prefix("(\"")).filter_map(|r| r.split('"').next()).map(str::to_string).collect();
        data_kinds.sort();
        data_kinds.dedup();
        // (transcribe etc. have data; `end` and the data-less ones are not in default_data)
        for k in &data_kinds {
            assert!(mine.contains(k), "default_data has {k}: add it to NODE_TYPES");
        }
        // each minimal data of the list has the keys of the editor's default (same shape)
        for (n, _, d) in NODE_TYPES.iter().filter(|(n, _, d)| data_kinds.iter().any(|k| k == n) && !d.starts_with('(')) {
            let mine: Value = serde_json::from_str(d).unwrap();
            let theirs = editor_default(&src, n);
            if let (Some(a), Some(b)) = (mine.as_object(), theirs.as_object()) {
                let mut ka: Vec<_> = a.keys().collect();
                let mut kb: Vec<_> = b.keys().collect();
                ka.sort();
                kb.sort();
                assert_eq!(ka, kb, "the minimal data of {n} must have the keys of the editor's default_data");
            }
        }
    }

    /// What the server fills in is what the editor puts in a new block: the same value, key by key.
    #[test]
    fn blank_data_is_the_editors_default() {
        let src = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../kalango-shell/src/fluxo_modelo.rs")).unwrap();
        for (t, required) in REQUIRED {
            for k in required.iter() {
                assert!(default_data_keys(t).contains(&k.to_string()), "{t} requires {k}, which the minimal data lacks");
            }
            if let Some(blank) = blank_data(t) {
                let theirs = editor_default(&src, t);
                let (mut a, mut b): (Vec<_>, Vec<_>) = (blank.as_object().unwrap().keys().cloned().collect(), theirs.as_object().unwrap().keys().cloned().collect());
                a.sort();
                b.sort();
                assert_eq!(a, b, "{t}: the blank data must have the keys of the editor's default_data");
            }
        }
    }

    fn default_data_keys(t: &str) -> Vec<String> {
        let d = NODE_TYPES.iter().find(|(n, _, _)| n == &t).unwrap().2;
        serde_json::from_str::<Value>(d).ok().and_then(|v| v.as_object().map(|o| o.keys().cloned().collect())).unwrap_or_default()
    }

    /// The keys of one `default_data` arm of the editor, read from the source (`json!({"a": …, "b": …})`).
    fn editor_default(src: &str, kind: &str) -> Value {
        let body = src.split("pub fn default_data").nth(1).unwrap().split("pub fn default_trigger").next().unwrap();
        let lines: Vec<&str> = body.lines().collect();
        let start = lines.iter().position(|l| l.trim().starts_with(&format!("(\"{kind}\", _) =>"))).unwrap_or_else(|| panic!("no arm for {kind}"));
        let arm: Vec<&str> = std::iter::once(lines[start]).chain(lines[start + 1..].iter().copied().take_while(|l| !l.trim().starts_with("(\"") && !l.trim().starts_with("_ =>"))).collect();
        let joined = arm.join("\n");
        let object = joined.split("json!(").nth(1).unwrap_or("{}");
        // top-level keys: a quoted word followed by a colon at depth 1
        let mut keys = Map::new();
        let (mut depth, mut chars) = (0, object.chars().peekable());
        let mut word = String::new();
        let mut in_str = false;
        while let Some(c) = chars.next() {
            match c {
                '"' if in_str => {
                    in_str = false;
                    if depth == 1 && chars.peek() == Some(&':') {
                        keys.insert(std::mem::take(&mut word), Value::Null);
                    }
                    word.clear();
                }
                '"' => in_str = true,
                _ if in_str => word.push(c),
                '{' | '[' => depth += 1,
                '}' | ']' => depth -= 1,
                _ => {}
            }
        }
        Value::Object(keys)
    }

    #[test]
    fn the_texts_the_model_reads_list_every_type_and_a_valid_example() {
        let text = flow_schema_text();
        for (n, _, _) in NODE_TYPES {
            assert!(text.contains(&format!("- {n}:")), "{n}");
        }
        assert!(text.contains("use the closest block and say so to the person") && text.contains("bottom-handle") && text.contains("first block nothing leads to") && text.contains("EVERY block must be reachable"));
        // the example is itself valid and keeps its positions out
        assert!(prepare_flow(&example_flow()).is_ok());
        let agent: Value = serde_json::from_str(agent_schema_text().split("Example: ").nth(1).unwrap()).unwrap();
        assert_eq!(prepare_agent(&agent, Some(&set(&[])), false, None).unwrap().0["id"], "resumidor");
        // the tool names the agent text cites exist
        for name in ["arquivos.ler", "arquivos.listar", "web.ler"] {
            assert!(super::super::find(name).is_some(), "{name}");
        }
    }

    #[test]
    fn odd_tools_are_errors_never_panics() {
        let base = |tools: Value| json!({"name": "A", "config": {"instruction": "x", "tools": tools}});
        for odd in [json!([1]), json!([null]), json!([["x"]]), json!([true]), json!([{}]), json!([{"name": 5}]), json!([{"name": "a", "permission": 3}]), json!("x"), json!({"a": 1})] {
            let e = prepare_agent(&base(odd.clone()), Some(&set(&[])), false, None);
            assert!(e.is_err(), "{odd}: {e:?}");
        }
        // permission null is "not given": ask
        let (a, _) = prepare_agent(&base(json!([{"name": "web.ler", "permission": null}])), Some(&set(&[])), false, None).unwrap();
        assert_eq!(a["config"]["tools"][0]["permission"], "ask");
        // config that is not an object is an error, not replaced silently
        assert!(prepare_agent(&json!({"name": "A", "config": "x"}), Some(&set(&[])), false, None).is_err());
        assert!(prepare_agent(&json!({"name": "A", "config": {"instruction": "x", "sub_agents": [3]}}), Some(&set(&[])), false, None).is_err());
    }

    /// Every field of an agent and of a flow gets every odd shape in turn: the preparation answers, never panics.
    #[test]
    fn odd_shapes_never_panic() {
        let odd = [json!(null), json!(1), json!(-3.5), json!(true), json!("x"), json!(""), json!([]), json!([null]), json!([[1]]), json!({}), json!({"a": [1]}), json!("a\nb\u{202e}")];
        let agent = json!({"id": "a", "name": "N", "description": "d", "kind": "assistant", "config": {"instruction": "i", "tools": [{"name": "t", "permission": "ask"}], "sub_agents": ["s"]}});
        let flow = example_flow();
        let mut paths: Vec<Vec<String>> = Vec::new();
        fn walk(v: &Value, at: &mut Vec<String>, out: &mut Vec<Vec<String>>) {
            out.push(at.clone());
            match v {
                Value::Object(o) => o.iter().for_each(|(k, x)| {
                    at.push(k.clone());
                    walk(x, at, out);
                    at.pop();
                }),
                Value::Array(a) => a.iter().enumerate().for_each(|(i, x)| {
                    at.push(i.to_string());
                    walk(x, at, out);
                    at.pop();
                }),
                _ => {}
            }
        }
        fn put(v: &mut Value, at: &[String], new: &Value) {
            match at.split_first() {
                None => *v = new.clone(),
                Some((k, rest)) => match v {
                    Value::Object(o) => o.get_mut(k).iter_mut().for_each(|x| put(x, rest, new)),
                    Value::Array(a) => k.parse::<usize>().ok().and_then(|i| a.get_mut(i)).iter_mut().for_each(|x| put(x, rest, new)),
                    _ => {}
                },
            }
        }
        walk(&agent, &mut vec![], &mut paths);
        for p in &paths {
            for o in &odd {
                let mut a = agent.clone();
                put(&mut a, p, o);
                for edit in [false, true] {
                    let _ = prepare_agent(&a, Some(&set(&["a"])), edit, Some(&agent));
                    let _ = prepare_agent(&a, None, edit, None);
                }
            }
        }
        paths.clear();
        walk(&flow, &mut vec![], &mut paths);
        for p in &paths {
            for o in &odd {
                let mut f = flow.clone();
                put(&mut f, p, o);
                let _ = prepare_flow(&f);
            }
        }
        for o in &odd {
            let _ = prepare_agent(o, Some(&set(&[])), false, None);
            let _ = prepare_flow(o);
            let _ = prepare_flow(&json!({"nodes": [o], "edges": [o]}));
            let _ = prepare_flow(&json!({"nodes": [{"id": "a", "type": "end"}], "edges": [o, {"source": o, "target": o}]}));
        }
    }

    #[test]
    fn only_what_the_tool_offers_goes_on() {
        let mut f = example_flow();
        f["limits"] = json!({"max_steps": 1000000});
        f["inputs"] = json!([{"name": "x"}]);
        f["input_map"] = json!({"a": "b"});
        f["nodes"][0]["disabled"] = json!(true);
        f["nodes"][2]["extra"] = json!(1);
        let (out, notes) = prepare_flow(&f).unwrap();
        assert!(out.get("limits").is_none() && out.get("inputs").is_none() && out.get("input_map").is_none());
        assert!(out["nodes"][0].get("disabled").is_none() && out["nodes"][2].get("extra").is_none());
        let joined = notes.join("\n");
        assert!(joined.contains("limits") && joined.contains("disabled") && joined.contains("extra"), "{joined}");
        // a parent only for a real sandbox block
        let mut g = example_flow();
        g["nodes"][2]["parent"] = json!("nope");
        let (out, notes) = prepare_flow(&g).unwrap();
        assert!(out["nodes"][2].get("parent").is_none() && notes.iter().any(|n| n.contains("nope")));
        // an agent keeps only its own fields on create
        let (a, notes) = prepare_agent(&json!({"name": "A", "icon": "x", "config": {"instruction": "i", "allow_cloud": true, "sandbox": {"template": "t"}, "model": {"provider": "p", "model": "m"}}}), Some(&set(&[])), false, None).unwrap();
        assert!(a.get("icon").is_none() && a["config"].get("allow_cloud").is_none() && a["config"].get("sandbox").is_none() && a["config"].get("model").is_none());
        assert!(notes.join("\n").contains("allow_cloud"));
    }

    #[test]
    fn a_sandbox_needs_its_children_and_every_block_must_be_reached() {
        let sandbox = json!({"nodes": [
            {"id": "s", "type": "sandbox", "data": {"template": "debian", "network": "nenhuma", "on_end": "apagar", "inputs": [], "outputs": [], "entry": "k"}},
            {"id": "k", "type": "message", "parent": "s", "data": {"content": "oi"}},
            {"id": "e", "type": "end"}], "edges": [{"source": "s", "target": "e"}]});
        assert!(prepare_flow(&sandbox).is_ok());
        let mut bad = sandbox.clone();
        bad["nodes"][0]["data"]["entry"] = json!("e");
        assert!(prepare_flow(&bad).unwrap_err().contains("parent"));
        bad = sandbox.clone();
        bad["nodes"][0]["data"]["network"] = json!("tudo");
        assert!(prepare_flow(&bad).is_err());
        // a block nothing leads to
        let lost = json!({"nodes": [{"id": "a", "type": "message", "data": {"content": "1"}}, {"id": "b", "type": "message", "data": {"content": "2"}}, {"id": "c", "type": "end"}], "edges": [{"source": "a", "target": "c"}, {"source": "b", "target": "c"}]});
        // (b leads to c, nothing leads to b: it is a second entry, and the first one, a, does not reach it)
        let e = prepare_flow(&lost).unwrap_err();
        assert!(e.contains('b'), "{e}");
        // branching blocks without edges are not chained: the error says what to connect
        assert!(prepare_flow(&json!({"nodes": [{"id": "c", "type": "condition", "data": {"conditions": [{"id": "sim"}]}}, {"id": "e", "type": "end"}]})).is_err());
    }

    #[test]
    fn a_condition_whose_edges_name_the_way_in_another_field_or_not_at_all_is_understood() {
        let cond = json!({"id": "c", "type": "condition", "data": {"conditions": [{"id": "sim", "type": "previous-output", "data": {"field": "content", "operator": "contains", "value": "x"}}]}});
        let nodes = |edges: Value| json!({"nodes": [cond.clone(), {"id": "a", "type": "end"}, {"id": "b", "type": "end"}], "edges": edges});
        // "condition" as an object or a text, "way"
        let (f, _) = prepare_flow(&nodes(json!([{"source": "c", "target": "a", "condition": {"id": "sim"}}, {"source": "c", "target": "b", "way": "else"}]))).unwrap();
        assert_eq!(f["edges"], json!([{"source": "c", "target": "a", "sourceHandle": "sim"}, {"source": "c", "target": "b", "sourceHandle": "bottom-handle"}]));
        // two edges with no way: the first is the condition's, the second the else
        let (f, notes) = prepare_flow(&nodes(json!([{"source": "c", "target": "a"}, {"source": "c", "target": "b"}]))).unwrap();
        assert_eq!(f["edges"], json!([{"source": "c", "target": "a", "sourceHandle": "sim"}, {"source": "c", "target": "b", "sourceHandle": "bottom-handle"}]));
        assert!(notes.iter().any(|n| n.contains('c') && n.contains("sim")), "{notes:?}");
        // ways that are named stay as named (one edge with a way and one without: nothing is guessed)
        let (f, _) = prepare_flow(&nodes(json!([{"source": "c", "target": "a", "sourceHandle": "sim"}, {"source": "c", "target": "b"}]))).unwrap();
        assert!(f["edges"][1].get("sourceHandle").is_none());
    }

    #[test]
    fn the_ways_of_every_branching_block_are_checked() {
        let flow = |from: Value, handle: &str| json!({"nodes": [from, {"id": "x", "type": "end"}], "edges": [{"source": "b", "target": "x", "sourceHandle": handle}]});
        let approval = json!({"id": "b", "type": "approval", "data": {"question": "ok?", "timeout_s": 3600}});
        assert!(prepare_flow(&flow(approval.clone(), "sim")).is_ok());
        assert!(prepare_flow(&flow(approval.clone(), "nao")).is_ok(), "nao is não");
        assert!(prepare_flow(&flow(approval.clone(), "talvez")).unwrap_err().contains("sim"));
        let decide = json!({"id": "b", "type": "decide", "data": {"model": "decisao", "route": "r", "questions": {"r": {"type": "noul", "instructions": "x"}}}});
        assert!(prepare_flow(&flow(decide.clone(), "true")).is_ok() && prepare_flow(&flow(decide.clone(), "pouca_confianca")).is_ok());
        assert!(prepare_flow(&flow(decide, "sim")).unwrap_err().contains("true"));
        // an approval that never expires, or at once, is refused
        let mut quick = approval.clone();
        quick["data"]["timeout_s"] = json!(5);
        assert!(prepare_flow(&flow(quick, "sim")).is_err());
        // an edge's ends are trimmed
        let (f, _) = prepare_flow(&json!({"nodes": [{"id": "a", "type": "message", "data": {"content": "1"}}, {"id": "b", "type": "end"}], "edges": [{"source": " a ", "target": "b "}]})).unwrap();
        assert_eq!(f["edges"], json!([{"source": "a", "target": "b"}]));
    }

    #[test]
    fn an_unreadable_agent_list_gives_a_random_ending_and_a_note() {
        let base = json!({"name": "Resumidor", "config": {"instruction": "i"}});
        let (a, notes) = prepare_agent(&base, None, false, None).unwrap();
        let id = a["id"].as_str().unwrap();
        assert!(is_valid_id(id) && id.starts_with("resumidor-") && id.len() > "resumidor-".len(), "{id}");
        assert!(!notes.is_empty());
        let (b, _) = prepare_agent(&base, None, false, None).unwrap();
        assert_ne!(a["id"], b["id"]);
        // the ids of a team are plain, an assistant's are objects
        let (t, _) = prepare_agent(&json!({"name": "T", "kind": "sequential", "config": {"sub_agents": [{"agent_id": "a"}, "b"]}}), Some(&set(&[])), false, None).unwrap();
        assert_eq!(t["config"]["sub_agents"], json!(["a", "b"]));
        let (x, _) = prepare_agent(&json!({"name": "X", "config": {"instruction": "i", "sub_agents": ["a"]}}), Some(&set(&[])), false, None).unwrap();
        assert_eq!(x["config"]["sub_agents"], json!([{"agent_id": "a", "mode": "chat"}]));
    }

    #[test]
    fn a_value_never_starts_a_line() {
        assert_eq!(one_line("a\n· fake\r\n\tline\u{202e}x", 100), "a · fake linex");
        assert!(one_line(&"é".repeat(500), 10).chars().count() <= 11);
        assert_eq!(one_line("\n\n", 10), "");
    }

    /// Every range of the table, at both ends and in the middle, and every control character, is dropped; the text
    /// around it stays one line.
    #[test]
    fn invisible_characters_are_dropped_in_every_range() {
        for (lo, hi) in INVISIBLE {
            for c in [*lo, *hi, char::from_u32((*lo as u32 + *hi as u32) / 2).unwrap_or(*lo)] {
                assert_eq!(one_line(&format!("a{c}b"), 10), "ab", "U+{:04X}", c as u32);
            }
        }
        for c in (0u32..0x20).chain(0x7f..0xa0).filter_map(char::from_u32) {
            assert!(!one_line(&format!("a{c}b"), 10).contains(c), "U+{:04X}", c as u32);
            assert_eq!(one_line(&format!("a{c}b"), 10).lines().count(), 1);
        }
        // the ones the review named
        for c in ['\u{061C}', '\u{2060}', '\u{2064}', '\u{00AD}', '\u{180E}', '\u{FFF9}', '\u{FFFB}', '\u{E0041}', '\u{FE0F}', '\u{E0100}', '\u{034F}', '\u{202E}', '\u{2066}'] {
            assert_eq!(one_line(&format!("x{c}y"), 10), "xy", "U+{:04X}", c as u32);
        }
        // ordinary text, accents and emoji stay
        assert_eq!(one_line("ação ✓ 日本", 20), "ação ✓ 日本");
    }

    #[test]
    fn a_flow_has_a_size_and_a_skipped_connection_is_said() {
        let nodes = |n: usize| (0..n).map(|i| json!({"id": format!("n{i}"), "type": "end"})).collect::<Vec<_>>();
        assert!(prepare_flow(&json!({"nodes": nodes(MAX_NODES + 1)})).is_err());
        let edges: Vec<Value> = (0..=MAX_EDGES).map(|_| json!({"source": "n0", "target": "n1"})).collect();
        assert!(prepare_flow(&json!({"nodes": nodes(3), "edges": edges})).is_err());
        // a connection that is not an object is left out with a note
        let (_, notes) = prepare_flow(&json!({"nodes": [{"id": "a", "type": "message", "data": {"content": "x"}}, {"id": "b", "type": "end"}], "edges": [5, {"source": "a", "target": "b"}]})).unwrap();
        assert!(notes.iter().any(|n| n.contains(tr("a connection that was not an object (source, target) was left out"))), "{notes:?}");
        // the biggest flow is placed and checked at once (a chain of 200 blocks)
        let chain: Vec<Value> = (0..MAX_NODES).map(|i| json!({"id": format!("n{i}"), "type": "message", "data": {"content": "x"}})).collect();
        let (f, _) = prepare_flow(&json!({"nodes": chain})).unwrap();
        assert_eq!(f["edges"].as_array().unwrap().len(), MAX_NODES - 1);
        assert_eq!(f["nodes"][MAX_NODES - 1]["position"]["x"], json!((MAX_NODES - 1) * 300));
    }

    #[test]
    fn what_runs_must_fit_the_approval_whole() {
        let big = "x".repeat(MAX_SHOWN + 1);
        let flow = |d: Value| json!({"flow": {"nodes": [{"id": "b", "type": "system_action", "data": d}]}});
        assert!(check_shown("flow", &flow(json!({"tool": "t", "arguments": {"k": big.clone()}}))).is_err());
        assert!(check_shown("flow", &flow(json!({"tool": "t", "arguments": {"k": "x".repeat(MAX_SHOWN)}}))).is_ok());
        assert!(check_shown("agent", &json!({"name": "a", "config": {"instruction": big.clone()}})).is_err());
        let many: Vec<Value> = (0..30).map(|i| json!({"id": format!("n{i}"), "type": "message", "data": {"content": "y".repeat(MAX_SHOWN)}})).collect();
        assert!(check_shown("flow", &json!({"flow": {"nodes": many}})).is_err(), "the whole is too big");
    }

    #[test]
    fn a_change_is_the_agent_as_it_is_with_the_fields_sent() {
        let old = json!({"id": "a", "name": "A", "description": "d", "kind": "sequential", "allow_cloud": true, "config": {"sub_agents": ["x"], "model": {"provider": "openai", "model": "m"}, "member_instruction": "velho"}});
        let (m, notes) = prepare_agent(&json!({"id": "a", "config": {"member_instruction": "novo"}}), None, true, Some(&old)).unwrap();
        assert_eq!(m["kind"], "sequential", "the kind it has");
        assert_eq!(m["allow_cloud"], true);
        assert_eq!(m["config"]["model"], old["config"]["model"]);
        assert_eq!(m["config"]["sub_agents"], json!(["x"]));
        assert_eq!(m["config"]["member_instruction"], "novo");
        assert!(notes.iter().any(|n| n.contains("config.member_instruction") && n.contains("velho") && n.contains("novo")), "{notes:?}");
        assert!(!notes.iter().any(|n| n.contains("sub_agents")), "what did not change is not listed: {notes:?}");
        for odd in [json!({"id": "a", "allow_cloud": false}), json!({"id": "a", "config": {"model": {"provider": "local", "model": "m"}}}), json!({"id": "a", "config": {"sandbox": {}}}), json!({"id": "a", "limits": {}}), json!({"id": "a", "system_action": "x"}), json!({"id": "a", "disabled": true})] {
            assert!(prepare_agent(&odd, None, true, Some(&old)).is_err(), "{odd}");
        }
    }

    #[test]
    fn an_assistant_may_answer_with_typed_fields_and_nothing_else_slips_in() {
        let ok = json!({"name": "Triagem", "config": {"instruction": "Diga se o chamado é urgente"}, "output_fields": [
            {"name": "urgente", "kind": "boolean", "description": "o chamado é urgente", "level": "total"}, {"name": "motivo", "kind": "text", "required": false}]});
        let (a, notes) = prepare_agent(&ok, Some(&set(&[])), false, None).unwrap();
        assert_eq!(a["output_fields"], json!([{"name": "urgente", "kind": "boolean", "required": true, "description": "o chamado é urgente"}, {"name": "motivo", "kind": "text", "required": false}]), "only name, kind, required and description are kept");
        assert!(notes.iter().any(|n| n.contains("urgente")));
        for bad in [
            json!([{"name": "1x", "kind": "text"}]), json!([{"name": "a b"}]), json!([{"name": "a", "kind": "enum"}]), json!([{"name": "a"}, {"name": "a"}]), json!("urgente"),
            Value::Array((0..31).map(|i| json!({"name": format!("f{i}")})).collect()),
        ] {
            let mut a = ok.clone();
            a["output_fields"] = bad.clone();
            assert!(prepare_agent(&a, Some(&set(&[])), false, None).is_err(), "{bad}");
        }
        // a team has no typed answer of its own
        let team = json!({"name": "Equipe", "kind": "sequential", "config": {"sub_agents": ["a"]}, "output_fields": [{"name": "x"}]});
        assert!(prepare_agent(&team, Some(&set(&[])), false, None).is_err());
    }
}
