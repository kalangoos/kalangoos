//! The agents and automations platform (kalango-fluxos) as tools: an agent — the KalangoOS one or any the person
//! allows — can list, create and change agents, flows and tools, import an API, register a connector and list the
//! models, through kalangod like every other tool.
//!
//! The work is the service's: kalangod runs `kalango-fluxosd chamar METHOD JSON` as the caller (its uid, its runtime
//! folder, where the service's socket is). Here are the rules:
//! - creating or changing an agent, a flow, a tool or a connector always asks the person on the approval screen (that
//!   no agent imitates), showing exactly what will exist: name, instructions, tools with their permissions, the
//!   flow's actions, addresses and commands;
//! - no privilege escalation: the service makes the item fit the creator's ceiling (the person's choices for the
//!   creator and its maximum level here) before the preview, and the approval lists what was removed;
//! - everything created is marked "created by the agent" and can be undone from the timeline.

use super::plataforma_contrato as contract;
use super::{n_, obj, s, tr, trf, Ctx, Out, Tool};
use crate::peer::Peer;
use crate::policy::{Effect, Level};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::Command;

pub fn tools() -> Vec<Tool> {
    use Effect::*;
    use Level::*;
    let t = |name, description, level, effect, reversible, schema: fn() -> Value, run| Tool { name, description, level, effect, reversible, schema, run };
    vec![
        t("agentes.listar", n_("Agents of the agents platform, with their kind, tools and who created them"), Observar, Leitura, false, super::none, listar_agentes),
        t("agentes.criar", n_("Creates an agent (assistant or team) on the agents platform; the person sees exactly what it can do before it exists"), Usuario, Escreve, true,
            || obj(json!({"agente": {"type": "object", "description": contract::agent_schema_text()}}), &["agente"]), criar_agente),
        t("agentes.editar", n_("Changes an agent of the agents platform (a new version); the person approves the result"), Usuario, Escreve, true,
            || obj(json!({"agente": {"type": "object", "description": format!("The whole agent with its id (the id is required to change an agent; see agentes.listar). {}", contract::agent_schema_text())}}), &["agente"]), editar_agente),
        t("agentes.apagar_criado", n_("Deletes an agent, flow or tool created by an agent (used by undo)"), Usuario, Escreve, false,
            || obj(json!({"tipo": {"type": "string", "enum": ["agent", "flow", "http_tool"]}, "id": {"type": "string"}}), &["tipo", "id"]), apagar_criado),
        t("fluxos.criar", n_("Creates a flow (an automation of blocks) on the agents platform; the person approves it first"), Usuario, Escreve, true,
            || obj(json!({"nome": {"type": "string", "description": "the flow's name"}, "fluxo": {"type": "object", "description": contract::flow_schema_text()}}), &["nome", "fluxo"]), criar_fluxo),
        t("fluxos.editar", n_("Changes a flow of the agents platform (a new version); the person approves it"), Usuario, Escreve, true,
            || obj(json!({"id": {"type": "string", "description": "the flow's id (the one fluxos.criar returned)"}, "nome": {"type": "string"}, "fluxo": {"type": "object", "description": contract::flow_schema_text()}}), &["id", "nome", "fluxo"]), editar_fluxo),
        t("fluxos.executar", n_("Runs a flow of the agents platform now"), Usuario, Escreve, false,
            || obj(json!({"id": {"type": "string"}, "entrada": {"type": "string"}}), &["id"]), executar_fluxo),
        t("ferramentas.criar_http", n_("Creates an HTTP tool (an API call) on the agents platform; it asks before each call"), Usuario, Escreve, true,
            || obj(json!({"ferramenta": {"type": "object", "description": "{name, description, method, endpoint, parameters, headers, outputs, error_handling}"}}), &["ferramenta"]), criar_http),
        t("ferramentas.importar_openapi", n_("Reads an OpenAPI 3 specification (address or text) and lists the tools it could become, for the person to choose; creates nothing"), Observar, Leitura, false,
            || obj(json!({"url": {"type": "string"}, "spec": {"type": "string"}}), &[]), importar_openapi),
        t("mcp.registrar", n_("Adds an MCP connector to Settings › Connectors (a local command or a remote address); the person approves the exact command or address"), Usuario, Escreve, true,
            || obj(json!({"nome": {"type": "string"}, "url": {"type": "string"}, "comando": {"type": "string"}, "argumentos": {"type": "array", "items": {"type": "string"}}}), &["nome"]), registrar_mcp),
        t("modelos.listar", n_("The local models (text, vision, voice, omni, decision) and the providers, with what each can do"), Observar, Leitura, false, super::none, listar_modelos),
    ]
}

/// The service's program: `$KALANGO_FLUXOSD`, the system's, the person's ~/.local/bin, or the PATH.
fn program(peer: &Peer) -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("KALANGO_FLUXOSD").map(PathBuf::from).filter(|p| p.is_file()) {
        return Some(p);
    }
    let fixed = [PathBuf::from("/usr/lib/kalango/kalango-fluxosd"), peer.passwd_home.join(".local/bin/kalango-fluxosd")];
    if let Some(p) = fixed.into_iter().find(|p| p.is_file()) {
        return Some(p);
    }
    std::env::var_os("PATH").and_then(|path| std::env::split_paths(&path).map(|d| d.join("kalango-fluxosd")).find(|p| p.is_file()))
}

fn socket(peer: &Peer) -> String {
    std::env::var("KALANGO_FLUXOS_SOCKET").unwrap_or_else(|_| format!("/run/user/{}/kalango/fluxos.sock", peer.uid))
}

/// One call to the service, as the caller.
fn service(peer: &Peer, method: &str, params: &Value) -> Result<Value, String> {
    let exe = program(peer).ok_or_else(|| tr("the agents platform (kalango-fluxos) is not installed").to_string())?;
    let mut cmd = Command::new(exe);
    cmd.args(["chamar", method, &params.to_string(), "--socket", &socket(peer)]);
    if nix::unistd::geteuid().is_root() && peer.uid != 0 {
        use std::os::unix::process::CommandExt;
        cmd.uid(peer.uid).gid(peer.gid).env_clear().env("HOME", &peer.passwd_home).env("USER", &peer.user)
            .env("XDG_RUNTIME_DIR", format!("/run/user/{}", peer.uid)).env("PATH", "/usr/local/bin:/usr/bin:/bin")
            .current_dir(&peer.passwd_home);
    }
    let o = cmd.output().map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(String::from_utf8_lossy(&o.stdout).trim())
        .map_err(|_| trf("the agents platform did not answer: {e}", &[("e", String::from_utf8_lossy(&o.stderr).trim().to_string())]))?;
    match v.get("error") {
        Some(e) => Err(e["message"].as_str().unwrap_or("error").to_string()),
        None => Ok(v["result"].clone()),
    }
}

fn creator(c: &Ctx) -> Value {
    json!({"id": c.rule.name, "max_level": c.rule.max_level.name()})
}

/// The ids of the agents that exist (to make a new id unique); None when the service does not answer.
fn agent_ids(peer: &Peer) -> Option<std::collections::HashSet<String>> {
    let list = service(peer, "agents.list", &json!({})).ok()?;
    Some(list.as_array()?.iter().filter_map(|a| a["agent"]["id"].as_str().or(a["id"].as_str()).map(str::to_string)).collect())
}

/// What the item is, for the service, and what was adjusted to make it fit (see `plataforma_contrato`).
type Prepared = (&'static str, Value, Vec<String>);

fn prepare(peer: &Peer, name: &str, a: &Value) -> Result<Prepared, String> {
    let prepared = match name {
        "agentes.criar" | "agentes.editar" => {
            let edit = name == "agentes.editar";
            let agent = a.get("agente").ok_or(tr("argument 'agente' is required"))?;
            // a change starts from the agent as it is (what the person already has stays); a new one needs the ids taken
            let existing = match (edit, agent["id"].as_str().filter(|i| !i.is_empty())) {
                (true, Some(id)) => Some(service(peer, "agents.get", &json!({"id": id})).map_err(|_| tr("the agent to change could not be read: look it up with agentes.listar").to_string())?),
                _ => None,
            };
            let taken = if edit { None } else { agent_ids(peer) };
            let (item, notes) = contract::prepare_agent(agent, if edit { None } else { taken.as_ref() }, edit, existing.as_ref())?;
            ("agent", item, notes)
        }
        "fluxos.criar" | "fluxos.editar" => {
            let (flow, mut notes) = contract::prepare_flow(a.get("fluxo").unwrap_or(&Value::Null))?;
            if name == "fluxos.editar" {
                // a change replaces the whole flow: what the old one carried beyond the blocks and connections is not kept
                let old = a["id"].as_str().filter(|i| !i.is_empty()).and_then(|id| service(peer, "flows.get", &json!({"id": id})).ok());
                let old = old.as_ref().map(|o| &o["flow"]);
                let mut gone: Vec<String> = ["limits", "inputs", "input_map"].iter().filter(|k| old.is_some_and(|o| !o[**k].is_null() && o[**k] != json!([]) && o[**k] != json!({}))).map(|k| k.to_string()).collect();
                let off = old.and_then(|o| o["nodes"].as_array()).map_or(0, |n| n.iter().filter(|n| n["disabled"] == json!(true)).count());
                if off > 0 {
                    gone.push(trf("the switched-off state of {n} block(s)", &[("n", off.to_string())]));
                }
                if !gone.is_empty() {
                    notes.push(trf("this change replaces the whole flow: the old one's {keys} are not kept (the blocks and connections are exactly those written here)", &[("keys", gone.join(", "))]));
                }
            }
            ("flow", json!({"id": a.get("id"), "name": a.get("nome"), "flow": flow}), notes)
        }
        "ferramentas.criar_http" => ("http_tool", a.get("ferramenta").cloned().ok_or(tr("argument 'ferramenta' is required"))?, vec![]),
        _ => return Err(trf("unknown platform tool {name}", &[("name", name.to_string())])),
    };
    contract::check_shown(prepared.0, &prepared.1)?;
    Ok(prepared)
}

/// What the approval screen showed, for the call that follows: the item applied is exactly the item the person saw
/// (the id included), not one prepared again from a list that may have changed in between. Kept per caller (user and
/// rule), for ten minutes, and used once.
static SHOWN: std::sync::Mutex<Vec<(String, std::time::Instant, Prepared)>> = std::sync::Mutex::new(Vec::new());
const SHOWN_FOR: std::time::Duration = std::time::Duration::from_secs(600);

fn call_key(peer: &Peer, rule: &str, name: &str, args: &Value) -> String {
    format!("{}\n{rule}\n{name}\n{args}", peer.uid)
}

fn item_of(peer: &Peer, rule: &str, name: &str, a: &Value) -> Result<Prepared, String> {
    let prepared = prepare(peer, name, a)?;
    let mut shown = SHOWN.lock().unwrap_or_else(|e| e.into_inner());
    shown.retain(|(k, at, _)| at.elapsed() < SHOWN_FOR && *k != call_key(peer, rule, name, a));
    shown.push((call_key(peer, rule, name, a), std::time::Instant::now(), prepared.clone()));
    if shown.len() > 16 {
        shown.remove(0);
    }
    Ok(prepared)
}

/// The item the person approved for this call (used once, within ten minutes), or one prepared now when none was shown
/// (a standing "always").
fn approved_item(peer: &Peer, rule: &str, name: &str, a: &Value) -> Result<Prepared, String> {
    let key = call_key(peer, rule, name, a);
    let mut shown = SHOWN.lock().unwrap_or_else(|e| e.into_inner());
    shown.retain(|(_, at, _)| at.elapsed() < SHOWN_FOR);
    if let Some(i) = shown.iter().position(|(k, _, _)| *k == key) {
        return Ok(shown.remove(i).2);
    }
    drop(shown);
    prepare(peer, name, a)
}

/// Creating and changing always ask the person (whatever the caller's level), with the exact result. Called by the
/// server before running the tool; Ok(None) = no approval needed.
pub fn needs_approval(peer: &Peer, rule: &crate::policy::CallerRule, name: &str, args: &Value) -> Result<Option<String>, String> {
    let creator = json!({"id": rule.name, "max_level": rule.max_level.name()});
    match name {
        "agentes.criar" | "agentes.editar" | "fluxos.criar" | "fluxos.editar" | "ferramentas.criar_http" => {
            let (kind, item, mine) = item_of(peer, &rule.name, name, args)?;
            let p = service(peer, "platform.preview", &json!({"kind": kind, "item": item, "creator": creator}))?;
            if let Some(problems) = p["problems"].as_array().filter(|v| !v.is_empty()) {
                return Err(problems.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("; "));
            }
            let notes: Vec<Value> = mine.into_iter().map(Value::String).chain(p["notes"].as_array().cloned().unwrap_or_default()).collect();
            Ok(Some(preview(kind, &p["item"], &notes)))
        }
        "mcp.registrar" => {
            let what = match (args.get("url").and_then(Value::as_str), args.get("comando").and_then(Value::as_str)) {
                (Some(u), _) => trf("the remote address {url}", &[("url", u.to_string())]),
                (None, Some(cmd)) => {
                    let argv: Vec<&str> = args.get("argumentos").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
                    trf("the local command: {cmd}", &[("cmd", format!("{cmd} {}", argv.join(" ")).trim().to_string())])
                }
                _ => return Err(tr("give the connector's address (url) or command (comando)").into()),
            };
            Ok(Some(trf("add the connector \"{name}\" — {what}", &[("name", args.get("nome").and_then(Value::as_str).unwrap_or("").to_string()), ("what", what)])))
        }
        _ => Ok(None),
    }
}

/// The text of the approval: exactly what will exist, one line per fact. A value written by the model is cut to one
/// line (a block's text can never start a line of its own here) and the fields that run or write are always shown.
fn preview(kind: &str, item: &Value, notes: &[Value]) -> String {
    let one = |v: &Value, max: usize| match v {
        Value::String(t) => contract::one_line(t, max),
        Value::Null => String::new(),
        other => contract::one_line(&other.to_string(), max),
    };
    let mut lines = Vec::new();
    match kind {
        "agent" => {
            lines.push(trf("create the agent \"{name}\" ({kind})", &[("name", one(&item["name"], 80)), ("kind", one(&item["kind"], 20))]));
            // (every field is shown whole: a field longer than the approval holds is refused before it gets here)
            if let Some(d) = item["description"].as_str().filter(|d| !d.is_empty()) {
                lines.push(trf("description: {text}", &[("text", contract::one_line(d, contract::MAX_SHOWN))]));
            }
            if let Some(i) = item["config"]["instruction"].as_str() {
                lines.push(trf("instructions: {text}", &[("text", contract::one_line(i, contract::MAX_SHOWN))]));
            }
            if let Some(m) = item["config"]["member_instruction"].as_str().filter(|m| !m.is_empty()) {
                lines.push(trf("told to every member: {text}", &[("text", contract::one_line(m, contract::MAX_SHOWN))]));
            }
            if !item["config"]["max_iterations"].is_null() {
                lines.push(trf("repeats at most {n} times", &[("n", one(&item["config"]["max_iterations"], 10))]));
            }
            for t in item["config"]["tools"].as_array().into_iter().flatten() {
                lines.push(format!("· {} — {}", one(&t["name"], contract::MAX_SHOWN), perm_text(t["permission"].as_str().unwrap_or("ask"))));
            }
            for s in item["config"]["sub_agents"].as_array().into_iter().flatten() {
                lines.push(format!("· {}", one(if s.is_string() { s } else { &s["agent_id"] }, 80)));
            }
        }
        "flow" => {
            lines.push(trf("create the flow \"{name}\"", &[("name", one(&item["name"], 80))]));
            for n in item["flow"]["nodes"].as_array().into_iter().flatten() {
                let id = n["id"].as_str().filter(|i| !i.is_empty()).map(|i| format!("{} — ", contract::one_line(i, 40))).unwrap_or_default();
                lines.push(format!("· {id}{}", one(&n["type"], 30)));
                lines.extend(node_fields(n, &one).into_iter().map(|l| format!("    {l}")));
            }
            let edges: Vec<&Value> = item["flow"]["edges"].as_array().into_iter().flatten().collect();
            if !edges.is_empty() {
                lines.push(tr("connections:").to_string());
                for e in edges {
                    let way = e["sourceHandle"].as_str().map(|w| format!(" ({})", if w == "bottom-handle" { tr("else").to_string() } else { contract::one_line(w, 40) })).unwrap_or_default();
                    lines.push(format!("· {} → {}{way}", one(&e["source"], 40), one(&e["target"], 40)));
                }
            }
        }
        _ => {
            lines.push(trf("create the HTTP tool \"{name}\"", &[("name", one(&item["name"], 80))]));
            lines.push(format!("{} {}", one(&item["method"], 10), one(&item["endpoint"], 300)));
        }
    }
    for n in notes.iter().filter_map(Value::as_str) {
        lines.push(contract::one_line(&note_text(n), 400));
    }
    lines.join("\n")
}

/// The fields of a block that run, write or send something (or say what it does), one line each.
fn node_fields(n: &Value, one: &dyn Fn(&Value, usize) -> String) -> Vec<String> {
    let t = n["type"].as_str().unwrap_or("");
    let keys: &[&str] = match t {
        "agent" => &["agent_id", "message"],
        "for_each" => &["items", "agent_id", "message"],
        "message" => &["content"],
        "wait" => &["seconds"],
        "ask" => &["message", "save_as"],
        "approval" => &["question", "details", "timeout_s"],
        "notify" => &["title", "body"],
        "subflow" => &["flow_id"],
        "decide" => &["state", "route", "threshold"],
        "sandbox" => &["template", "network", "on_end", "entry"],
        "transcribe" => &["file", "language", "save_as"],
        "speak" => &["text", "voice", "out"],
        "describe_image" => &["file", "question", "save_as"],
        "system_action" => &["tool", "arguments", "save_as"],
        _ => &[],
    };
    let d = &n["data"];
    let mut out = Vec::new();
    if let Some(title) = n["title"].as_str().filter(|t| !t.is_empty()) {
        out.push(trf("name on the canvas: {title}", &[("title", contract::one_line(title, 80))]));
    }
    if let Some(p) = n["parent"].as_str() {
        out.push(trf("inside the sandbox: {id}", &[("id", contract::one_line(p, 40))]));
    }
    // (a field that runs, writes or sends is refused when too long to show whole, so none of these is ever cut)
    let max = contract::MAX_SHOWN;
    for k in keys {
        if *k == "arguments" {
            match d[*k].as_object() {
                Some(o) if !o.is_empty() => out.extend(o.iter().map(|(k2, v2)| format!("arguments.{}: {}", contract::one_line(k2, 60), one(v2, max)))),
                _ => out.push(format!("arguments: {}", one(&d[*k], max))),
            }
            continue;
        }
        let always = ["tool", "agent_id", "flow_id", "template", "entry", "out", "file"].contains(k);
        let v = if *k == "file" && t == "describe_image" && d["file"].as_str().is_none_or(str::is_empty) { tr("(empty: takes a screenshot of the screen)").to_string() } else { one(&d[*k], max) };
        if !v.is_empty() || always {
            out.push(format!("{k}: {v}"));
        }
    }
    if t == "condition" {
        for c in d["conditions"].as_array().into_iter().flatten() {
            let r = &c["data"];
            out.push(format!("{}: {} {} {}", one(&c["id"], 30), one(&r["field"], 80), one(&r["operator"], 30), one(&r["value"], max)));
        }
    }
    if t == "set_variables" {
        for v in d["variables"].as_array().into_iter().flatten() {
            out.push(format!("{} {} {}", one(&v["op"], 20), one(&v["name"], 70), one(&v["value"], max)));
        }
    }
    // whatever else the block carries is shown too (name and value), so nothing hides behind what is listed above
    let listed: Vec<&str> = keys.iter().copied().chain(["conditions", "variables"]).collect();
    let rest: Vec<(&String, &Value)> = d.as_object().into_iter().flatten().filter(|(k, _)| !listed.contains(&k.as_str())).collect();
    if !rest.is_empty() {
        out.push(tr("other settings:").to_string());
        out.extend(rest.into_iter().map(|(k, v)| format!("  {}: {}", contract::one_line(k, 60), one(v, max))));
    }
    out
}

/// The service's codes of what was fitted to the creator's ceiling, in the person's language.
fn note_text(code: &str) -> String {
    let p: Vec<&str> = code.split(':').collect();
    match p.as_slice() {
        ["removed", tool] => trf("removed {tool}: above what you allow the agent that asked", &[("tool", tool.to_string())]),
        ["tightened", tool, _from, to] => trf("{tool} will {perm}: no looser than what you allow the agent that asked", &[("tool", tool.to_string()), ("perm", perm_text(to).to_string())]),
        ["removed_sub_agent", name] => trf("removed the sub-agent {name}: it can do more than the agent that asked", &[("name", name.to_string())]),
        ["asks_each_call"] => tr("the new tool asks you before each call").into(),
        _ => code.to_string(),
    }
}

fn perm_text(p: &str) -> String {
    match p {
        "free" => tr("run freely").to_string(),
        "ask" => tr("ask first").to_string(),
        "never" => tr("never run").to_string(),
        // (anything else is shown as written: the platform refuses it, the person must not read it as "never")
        other => contract::one_line(other, 40),
    }
}

fn apply(c: &Ctx, name: &str, a: &Value, edit: bool) -> Result<Out, String> {
    let (kind, item, mine) = approved_item(c.peer, &c.rule.name, name, a)?;
    let r = service(c.peer, "platform.apply", &json!({"kind": kind, "item": item, "creator": creator(c), "edit": edit}))?;
    let id = r["id"].as_str().unwrap_or("").to_string();
    let notes: Vec<Value> = mine.into_iter().map(Value::String).chain(r["notes"].as_array().cloned().unwrap_or_default()).collect();
    // undo deletes what was created (an edit is undone by the item's own version history)
    let inverse = (!edit).then(|| json!({"ferramenta": "agentes.apagar_criado", "argumentos": {"tipo": kind, "id": id}}));
    // (a small model creates it again to "complete" it: the result says it exists and how to change it)
    let warning = if edit || !contract::is_valid_id(&id) && kind == "agent" { String::new() } else { trf("created with the id {id}: it is done, do not create it again; to change it use the editing tool with this id", &[("id", contract::one_line(&id, 60))]) };
    Ok(Out { value: json!({"id": id, "ajustes": notes, "aviso": warning}), summary: trf("{what} created by the agent: {id}", &[("what", kind.to_string()), ("id", id.clone())]), inverse })
}

fn criar_agente(c: &Ctx, a: &Value) -> Result<Out, String> {
    apply(c, "agentes.criar", a, false)
}
fn editar_agente(c: &Ctx, a: &Value) -> Result<Out, String> {
    apply(c, "agentes.editar", a, true)
}
fn criar_fluxo(c: &Ctx, a: &Value) -> Result<Out, String> {
    apply(c, "fluxos.criar", a, false)
}
fn editar_fluxo(c: &Ctx, a: &Value) -> Result<Out, String> {
    apply(c, "fluxos.editar", a, true)
}
fn criar_http(c: &Ctx, a: &Value) -> Result<Out, String> {
    apply(c, "ferramentas.criar_http", a, false)
}

fn apagar_criado(c: &Ctx, a: &Value) -> Result<Out, String> {
    service(c.peer, "platform.remove_created", &json!({"kind": s(a, "tipo")?, "id": s(a, "id")?}))?;
    Ok(Out { value: json!({"apagado": true}), summary: trf("deleted {id}", &[("id", s(a, "id")?.to_string())]), inverse: None })
}

fn listar_agentes(c: &Ctx, _: &Value) -> Result<Out, String> {
    let v = service(c.peer, "agents.list", &json!({}))?;
    Ok(Out { value: json!({"agentes": v}), summary: tr("listed the agents").into(), inverse: None })
}

fn executar_fluxo(c: &Ctx, a: &Value) -> Result<Out, String> {
    let v = service(c.peer, "flows.run", &json!({"id": s(a, "id")?, "input": a.get("entrada").and_then(Value::as_str).unwrap_or("")}))?;
    Ok(Out { value: v, summary: trf("ran the flow {id}", &[("id", s(a, "id")?.to_string())]), inverse: None })
}

fn importar_openapi(c: &Ctx, a: &Value) -> Result<Out, String> {
    let v = service(c.peer, "tools.openapi_inspect", &json!({"url": a.get("url"), "spec": a.get("spec")}))?;
    Ok(Out { value: v, summary: tr("read the API's operations (nothing created)").into(), inverse: None })
}

fn listar_modelos(c: &Ctx, _: &Value) -> Result<Out, String> {
    let local = service(c.peer, "models.local", &json!({}))?;
    let providers = service(c.peer, "models.providers", &json!({}))?;
    Ok(Out { value: json!({"locais": local, "provedores": providers}), summary: tr("listed the models").into(), inverse: None })
}

/// The connector manifest in the person's connectors folder (~/.config/kalango/conectores/NAME.toml), the one Settings
/// › Connectors and the agent engine read. Remote ones have network; local commands none until the person opens it.
fn registrar_mcp(c: &Ctx, a: &Value) -> Result<Out, String> {
    let nome = s(a, "nome")?;
    if nome.is_empty() || !nome.chars().all(|ch| ch.is_ascii_alphanumeric() || "-_".contains(ch)) {
        return Err(tr("the connector's name has letters, digits, '-' and '_'").into());
    }
    let mut toml = format!("nome = {}\n", toml_str(nome));
    match (a.get("url").and_then(Value::as_str), a.get("comando").and_then(Value::as_str)) {
        (Some(u), _) => {
            if !(u.starts_with("https://") || u.starts_with("http://127.0.0.1") || u.starts_with("http://localhost")) {
                return Err(tr("a remote connector's address must be https").into());
            }
            toml.push_str(&format!("url = {}\nrede = true\n", toml_str(u)));
        }
        (None, Some(cmd)) => {
            let argv: Vec<String> = a.get("argumentos").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(toml_str).collect();
            toml.push_str(&format!("comando = {}\nargumentos = [{}]\nrede = false\n", toml_str(cmd), argv.join(", ")));
        }
        _ => return Err(tr("give the connector's address (url) or command (comando)").into()),
    }
    toml.push_str("sob_demanda = true\n");
    // (KALANGO_CONECTORES_DIR: the tests' folder — kalangod resolves the home by passwd, never the test's $HOME)
    let dir = std::env::var_os("KALANGO_CONECTORES_DIR").map(std::path::PathBuf::from).unwrap_or_else(|| c.peer.passwd_home.join(".config/kalango/conectores"));
    let p = dir.join(format!("{nome}.toml"));
    if p.exists() {
        return Err(trf("a connector named {name} already exists", &[("name", nome.to_string())]));
    }
    // written as the person, inside the connectors' folder's parent (the folder itself may not exist yet)
    let root = dir.parent().map(std::path::Path::to_path_buf).unwrap_or_else(|| dir.clone());
    crate::sandbox::run_as_user(c.peer, &[root], json!({"tipo": "escrever", "caminho": p, "conteudo": toml}))?;
    Ok(Out {
        value: json!({"manifesto": p}),
        summary: trf("connector {name} added by the agent", &[("name", nome.to_string())]),
        inverse: Some(json!({"ferramenta": "arquivos.apagar_criado", "argumentos": {"caminho": p}})),
    })
}

fn toml_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previews_say_what_will_exist() {
        let p = preview("agent", &json!({"name": "Monitor de preços", "kind": "assistant", "config": {"instruction": "Confira o preço.", "tools": [{"name": "web.ler", "permission": "free"}]}}), &[json!("removed:arquivos.apagar")]);
        assert!(p.contains("Monitor de preços") && p.contains("web.ler — ") && p.contains("arquivos.apagar"), "{p}");
        let f = preview("flow", &json!({"name": "F", "flow": {"nodes": [{"type": "system_action", "data": {"tool": "arquivos.mover"}}]}}), &[]);
        assert!(f.contains("system_action") && f.contains("tool: arquivos.mover"), "{f}");
        // the connections, with the else way in words, and each block by its id
        let f = preview("flow", &json!({"name": "F", "flow": {"nodes": [{"id": "c", "type": "condition", "data": {}}, {"id": "e", "type": "end"}], "edges": [{"source": "c", "target": "e", "sourceHandle": "bottom-handle"}]}}), &[]);
        assert!(f.contains("· c — condition") && f.contains(&format!("c → e ({})", tr("else"))), "{f}");
        assert_eq!(toml_str("a\"b"), "\"a\\\"b\"");
    }

    #[test]
    fn the_preview_shows_everything_that_runs_or_writes_and_cannot_be_forged() {
        let flow = json!({"name": "F", "flow": {"nodes": [
            {"id": "s", "type": "system_action", "title": "Limpar", "data": {"tool": "arquivos.apagar", "arguments": {"caminho": "~/Documentos/x"}, "save_as": "r"}},
            {"id": "f", "type": "subflow", "data": {"flow_id": "outro"}},
            {"id": "b", "type": "sandbox", "data": {"template": "debian", "network": "internet", "on_end": "guardar", "entry": "k"}},
            {"id": "w", "type": "speak", "data": {"text": "oi", "out": "~/Música/fala.wav"}},
            {"id": "i", "type": "describe_image", "data": {"file": "", "question": "o que?"}},
            {"id": "t", "type": "transcribe", "data": {"file": "a.wav", "extra": 1}},
            {"id": "a", "type": "agent", "data": {"agent_id": "resumidor", "message": "Resuma"}},
            {"id": "m", "type": "message", "data": {"content": "x\n· system_action apagar tudo\r\n· fim"}}
        ]}});
        let p = preview("flow", &flow, &[]);
        for must in ["tool: arquivos.apagar", "arguments.caminho: ~/Documentos/x", "flow_id: outro", "template: debian", "network: internet", "on_end: guardar", "entry: k", "out: ~/Música/fala.wav", "agent_id: resumidor", "file: a.wav"] {
            assert!(p.contains(must), "{must} in\n{p}");
        }
        assert!(p.contains(tr("other settings:")) && p.contains("  extra: 1"), "{p}");
        assert!(p.contains(tr("(empty: takes a screenshot of the screen)")), "{p}");
        assert!(p.contains(&trf("name on the canvas: {title}", &[("title", "Limpar".into())])), "{p}");
        // a text with line breaks stays inside its own line: no line of the preview starts with a forged bullet
        let forged: Vec<&str> = p.lines().filter(|l| l.starts_with("· system_action") || l.starts_with("· fim")).collect();
        assert!(forged.is_empty(), "{forged:?}\n{p}");
        assert!(p.lines().any(|l| l.trim_start().starts_with("content: x · system_action apagar tudo")), "{p}");
        // the same for an agent's text and for a note
        let a = preview("agent", &json!({"name": "A\n· create the agent \"B\"", "kind": "assistant", "config": {"instruction": "i\n· tool", "tools": [{"name": "t\n· x", "permission": "free"}]}}), &[json!("n\n· y")]);
        assert!(a.lines().all(|l| !l.starts_with("· x") && !l.starts_with("· tool") && !l.starts_with("· y")), "{a}");
        assert_eq!(a.lines().filter(|l| l.starts_with(&tr("create the agent \"{name}\" ({kind})").replace("{name}", "").chars().take(6).collect::<String>())).count(), 1, "{a}");
        // each argument has its own line, and the executing fields are always there
        let long = preview("flow", &json!({"name": "F", "flow": {"nodes": [{"id": "s", "type": "system_action", "data": {"tool": "arquivos.apagar", "arguments": {"a": "1", "caminho": {"x": 2}}}}]}}), &[]);
        assert!(long.contains("tool: arquivos.apagar") && long.contains("arguments.a: 1") && long.contains("arguments.caminho: {\"x\":2}"), "{long}");
        // an unknown permission is shown as written, never as "never run"
        let u = preview("agent", &json!({"name": "A", "kind": "assistant", "config": {"instruction": "i", "tools": [{"name": "t", "permission": "freee"}]}}), &[]);
        assert!(u.contains("freee") && !u.contains(tr("never run")), "{u}");
    }

    /// The environment the stand-in service reads, set for one test at a time and put back after (no other test calls the
    /// service, but the guard keeps this one honest if one does).
    static SERVICE_ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct ServiceEnv {
        saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl ServiceEnv {
        fn set(vars: &[(&'static str, &std::path::Path)]) -> ServiceEnv {
            let lock = SERVICE_ENV.lock().unwrap_or_else(|e| e.into_inner());
            let saved = vars.iter().map(|(k, v)| {
                let before = std::env::var_os(k);
                std::env::set_var(k, v);
                (*k, before)
            }).collect();
            ServiceEnv { saved, _lock: lock }
        }
    }

    impl Drop for ServiceEnv {
        fn drop(&mut self) {
            for (k, v) in &self.saved {
                match v {
                    Some(v) => std::env::set_var(k, v),
                    None => std::env::remove_var(k),
                }
            }
        }
    }

    fn peer(uid: u32) -> Peer {
        Peer { pid: 1, uid, gid: uid, exe: "/usr/bin/teste".into(), cgroup: String::new(), home: "/home/x".into(), passwd_home: "/home/x".into(), user: "x".into(), has_tty: false }
    }

    fn rule(name: &str) -> crate::policy::CallerRule {
        serde_json::from_value(json!({"name": name, "max_level": "total"})).unwrap()
    }

    /// A stand-in for the SERVICE only, a program that answers `chamar METHOD JSON` from files in its folder.
    fn stand_in(dir: &std::path::Path) -> ServiceEnv {
        let program = dir.join("fluxosd");
        std::fs::write(&program, r#"#!/usr/bin/env python3
import json, os, sys
d = os.environ["PLAT_DIR"]
method, p = sys.argv[2], json.loads(sys.argv[3])
if method == "agents.list":
    if os.path.exists(d + "/fail"):
        sys.exit(1)
    print(open(d + "/list.json").read())
elif method in ("agents.get", "flows.get"):
    f = d + "/" + method + ".json"
    if not os.path.exists(f):
        sys.exit(1)
    print(open(f).read())
elif method == "platform.preview":
    print(json.dumps({"result": {"item": p["item"], "notes": [], "problems": []}}))
else:
    sys.exit(1)
"#).unwrap();
        std::fs::set_permissions(&program, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        ServiceEnv::set(&[("KALANGO_FLUXOSD", &program), ("PLAT_DIR", dir)])
    }

    fn put(dir: &std::path::Path, file: &str, v: Value) {
        std::fs::write(dir.join(file), json!({"result": v}).to_string()).unwrap();
    }

    /// Through the entry points the server calls (`needs_approval`, then the apply that follows), with a stand-in for the
    /// SERVICE only: what the person was shown is what is applied (the id too), a list of agents that cannot be read
    /// gives a random ending, and odd model input is an error, never a panic.
    #[test]
    fn the_entry_points_apply_what_was_shown_and_survive_odd_input() {
        let dir = tempfile::tempdir().unwrap();
        let _env = stand_in(dir.path());
        let list = |ids: &[&str]| put(dir.path(), "list.json", json!(ids.iter().map(|i| json!({"agent": {"id": i}})).collect::<Vec<_>>()));
        let (peer, rule) = (peer(1000), rule("t"));
        let args = json!({"agente": {"name": "Resumidor", "config": {"instruction": "Resuma."}}});
        list(&["resumidor"]);
        let shown = needs_approval(&peer, &rule, "agentes.criar", &args).unwrap().unwrap();
        assert!(shown.contains("Resumidor"), "{shown}");
        // someone else creates resumidor-2 meanwhile: the person approved resumidor-2, and that is what is applied
        list(&["resumidor", "resumidor-2"]);
        let (_, item, _) = approved_item(&peer, "t", "agentes.criar", &args).unwrap();
        assert_eq!(item["id"], "resumidor-2");
        // (used once: a later call is prepared again, from the list as it is)
        assert_eq!(approved_item(&peer, "t", "agentes.criar", &args).unwrap().1["id"], "resumidor-3");
        // a list that cannot be read: a random ending and the note
        std::fs::write(dir.path().join("fail"), "").unwrap();
        let (_, item, notes) = prepare(&peer, "agentes.criar", &args).unwrap();
        assert!(item["id"].as_str().unwrap().starts_with("resumidor-") && notes.iter().any(|n| n.contains(tr("the list of agents could not be read: the id has a random ending"))), "{item} {notes:?}");
        std::fs::remove_file(dir.path().join("fail")).unwrap();
        // odd input: clear errors from both entry points, no panic
        for tools in [json!([1]), json!([null]), json!([["x"]]), json!([{"name": "a", "permission": 3}])] {
            let odd = json!({"agente": {"name": "A", "config": {"instruction": "i", "tools": tools}}});
            assert!(needs_approval(&peer, &rule, "agentes.criar", &odd).is_err());
            assert!(approved_item(&peer, "t", "agentes.criar", &odd).is_err());
        }
        for odd in [json!({"fluxo": 1}), json!({"fluxo": {"nodes": [1]}}), json!({"fluxo": {"nodes": [{"type": 5}]}}), json!({"agente": 3}), json!({})] {
            for tool in ["fluxos.criar", "agentes.criar", "agentes.editar"] {
                assert!(needs_approval(&peer, &rule, tool, &odd).is_err(), "{tool} {odd}");
            }
        }
    }

    /// The shown item belongs to the caller that was shown it: another user, or another rule, with the same arguments
    /// gets its own; one shown more than ten minutes ago is not used.
    #[test]
    fn the_shown_item_is_kept_per_caller_for_a_while_and_used_once() {
        let dir = tempfile::tempdir().unwrap();
        let _env = stand_in(dir.path());
        let args = json!({"agente": {"name": "Resumidor", "config": {"instruction": "Resuma."}}});
        put(dir.path(), "list.json", json!([]));
        let (a, b) = (peer(1000), peer(1001));
        needs_approval(&a, &rule("t"), "agentes.criar", &args).unwrap();
        put(dir.path(), "list.json", json!([{"agent": {"id": "resumidor"}}]));
        // the other user was shown nothing: its item is made now (and does not use up the first one's)
        assert_eq!(approved_item(&b, "t", "agentes.criar", &args).unwrap().1["id"], "resumidor-2");
        assert_eq!(approved_item(&a, "other-rule", "agentes.criar", &args).unwrap().1["id"], "resumidor-2");
        assert_eq!(approved_item(&a, "t", "agentes.criar", &args).unwrap().1["id"], "resumidor", "what the first was shown is still its own");
        // an entry older than ten minutes is not used
        needs_approval(&a, &rule("t"), "agentes.criar", &args).unwrap();
        {
            let mut shown = SHOWN.lock().unwrap();
            let old = std::time::Instant::now().checked_sub(SHOWN_FOR + std::time::Duration::from_secs(1)).unwrap();
            shown.iter_mut().for_each(|e| e.1 = old);
        }
        put(dir.path(), "list.json", json!([{"agent": {"id": "resumidor"}}, {"agent": {"id": "resumidor-2"}}]));
        assert_eq!(approved_item(&a, "t", "agentes.criar", &args).unwrap().1["id"], "resumidor-3");
    }

    /// A change keeps what the agent already has, refuses what the tool does not offer, and shows `key: old → new`.
    #[test]
    fn a_change_keeps_the_agent_and_shows_what_changes() {
        let dir = tempfile::tempdir().unwrap();
        let _env = stand_in(dir.path());
        let existing = json!({"id": "a1", "name": "A", "description": "d", "kind": "assistant", "allow_cloud": false,
            "config": {"instruction": "antiga", "tools": [{"name": "web.ler", "permission": "ask"}], "model": {"provider": "local", "model": "lfm3"}}});
        put(dir.path(), "agents.get.json", existing);
        let (peer, rule) = (peer(1000), rule("t"));
        let edit = |agent: Value| json!({"agente": agent});
        let shown = needs_approval(&peer, &rule, "agentes.editar", &edit(json!({"id": "a1", "config": {"instruction": "nova"}}))).unwrap().unwrap();
        assert!(shown.contains(&trf("{key}: {old} → {new}", &[("key", "config.instruction".into()), ("old", "antiga".into()), ("new", "nova".into())])), "{shown}");
        let (_, item, _) = approved_item(&peer, "t", "agentes.editar", &edit(json!({"id": "a1", "config": {"instruction": "nova"}}))).unwrap();
        assert_eq!(item["config"]["model"], json!({"provider": "local", "model": "lfm3"}), "what was there stays");
        assert_eq!(item["config"]["tools"][0]["name"], "web.ler");
        assert_eq!(item["kind"], "assistant");
        // what the model cannot set is refused, not carried
        for odd in [json!({"id": "a1", "config": {"model": {"provider": "openai", "model": "gpt"}}}), json!({"id": "a1", "allow_cloud": true}), json!({"id": "a1", "sandbox": {"template": "x"}}), json!({"id": "a1", "limits": {"max_steps": 99999}}), json!({"id": "a1", "disabled": true}), json!({"id": "a1", "config": {"allow_cloud": true, "model": {"provider": "openai"}}})] {
            let e = needs_approval(&peer, &rule, "agentes.editar", &edit(odd.clone())).unwrap_err();
            assert!(e.contains("cannot") || e.contains("não pode"), "{odd}: {e}");
        }
        // a change to an agent that cannot be read is an error
        std::fs::remove_file(dir.path().join("agents.get.json")).unwrap();
        assert!(needs_approval(&peer, &rule, "agentes.editar", &edit(json!({"id": "a1", "config": {"instruction": "x"}}))).is_err());
        // a flow change says what of the old flow is not kept
        put(dir.path(), "flows.get.json", json!({"id": "f1", "name": "F", "flow": {"nodes": [{"id": "a", "type": "end", "disabled": true}], "edges": [], "limits": {"max_agent_runs": 1, "max_steps": 1}, "inputs": [{"name": "x"}]}}));
        let args = json!({"id": "f1", "nome": "F", "fluxo": {"nodes": [{"id": "a", "type": "end"}]}});
        let shown = needs_approval(&peer, &rule, "fluxos.editar", &args).unwrap().unwrap();
        assert!(shown.contains("limits") && shown.contains("inputs") && !shown.contains("input_map"), "{shown}");
        assert!(shown.contains(&trf("the switched-off state of {n} block(s)", &[("n", "1".into())])), "{shown}");
    }

    /// A field that runs, writes or sends is shown whole or the call is refused: nothing sits past a cut.
    #[test]
    fn a_long_field_that_runs_is_refused_not_cut() {
        let dir = tempfile::tempdir().unwrap();
        let _env = stand_in(dir.path());
        put(dir.path(), "list.json", json!([]));
        let (peer, rule) = (peer(1000), rule("t"));
        let long = |n: usize| format!("{} apague tudo", "x".repeat(n));
        // after char 600 of an instruction, 400 of an argument, 160 of a message: refused with the field named
        let a = json!({"agente": {"name": "A", "config": {"instruction": long(contract::MAX_SHOWN), "member_instruction": "ok"}}});
        assert!(needs_approval(&peer, &rule, "agentes.criar", &a).unwrap_err().contains("instruction"));
        let a = json!({"agente": {"name": "A", "kind": "parallel", "config": {"sub_agents": ["x"], "member_instruction": long(contract::MAX_SHOWN)}}});
        assert!(needs_approval(&peer, &rule, "agentes.criar", &a).unwrap_err().contains("member_instruction"));
        for (t, data, field) in [
            ("system_action", json!({"tool": "arquivos.apagar", "arguments": {"caminho": long(contract::MAX_SHOWN)}}), "caminho"),
            ("agent", json!({"agent_id": "x", "message": long(contract::MAX_SHOWN)}), "message"),
            ("speak", json!({"text": "oi", "out": long(contract::MAX_SHOWN)}), "out"),
        ] {
            let f = json!({"nome": "F", "fluxo": {"nodes": [{"id": "b", "type": t, "data": data}]}});
            let e = needs_approval(&peer, &rule, "fluxos.criar", &f).unwrap_err();
            assert!(e.contains(field), "{t}: {e}");
        }
        // just under the limit is shown whole, the dangerous tail included
        let ok = json!({"nome": "F", "fluxo": {"nodes": [{"id": "b", "type": "system_action", "data": {"tool": "arquivos.apagar", "arguments": {"caminho": long(contract::MAX_SHOWN - 20)}}}]}});
        let shown = needs_approval(&peer, &rule, "fluxos.criar", &ok).unwrap().unwrap();
        assert!(shown.contains("apague tudo") && shown.contains("tool: arquivos.apagar"), "tail hidden");
        let a = json!({"agente": {"name": "A", "kind": "loop", "config": {"sub_agents": ["x"], "member_instruction": "seja breve", "max_iterations": 7}}});
        let shown = needs_approval(&peer, &rule, "agentes.criar", &a).unwrap().unwrap();
        assert!(shown.contains("seja breve") && shown.contains('7'), "{shown}");
        // too many blocks
        let nodes: Vec<Value> = (0..201).map(|i| json!({"id": format!("n{i}"), "type": "end"})).collect();
        assert!(needs_approval(&peer, &rule, "fluxos.criar", &json!({"nome": "F", "fluxo": {"nodes": nodes}})).is_err());
    }

    #[test]
    fn tools_are_registered() {
        let names: Vec<&str> = tools().iter().map(|t| t.name).collect();
        for n in ["agentes.criar", "agentes.editar", "agentes.listar", "fluxos.criar", "fluxos.editar", "fluxos.executar", "ferramentas.criar_http", "ferramentas.importar_openapi", "mcp.registrar", "modelos.listar"] {
            assert!(names.contains(&n), "{n}");
        }
    }
}
