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
            || obj(json!({"agente": {"type": "object", "description": "{name, description, kind: assistant|sequential|parallel|loop, config: {instruction, tools: [{name, permission: free|ask|never}], sub_agents…}}"}}), &["agente"]), criar_agente),
        t("agentes.editar", n_("Changes an agent of the agents platform (a new version); the person approves the result"), Usuario, Escreve, true,
            || obj(json!({"agente": {"type": "object", "description": "the whole agent, with its id"}}), &["agente"]), editar_agente),
        t("agentes.apagar_criado", n_("Deletes an agent, flow or tool created by an agent (used by undo)"), Usuario, Escreve, false,
            || obj(json!({"tipo": {"type": "string", "enum": ["agent", "flow", "http_tool"]}, "id": {"type": "string"}}), &["tipo", "id"]), apagar_criado),
        t("fluxos.criar", n_("Creates a flow (an automation of blocks) on the agents platform; the person approves it first"), Usuario, Escreve, true,
            || obj(json!({"nome": {"type": "string"}, "fluxo": {"type": "object", "description": "{nodes: [{id, type, data}], edges: [{source, target, sourceHandle}]}"}}), &["nome", "fluxo"]), criar_fluxo),
        t("fluxos.editar", n_("Changes a flow of the agents platform (a new version); the person approves it"), Usuario, Escreve, true,
            || obj(json!({"id": {"type": "string"}, "nome": {"type": "string"}, "fluxo": {"type": "object"}}), &["id", "nome", "fluxo"]), editar_fluxo),
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
    let fixed = [PathBuf::from("/usr/lib/kalango/kalango-fluxosd"), peer.home.join(".local/bin/kalango-fluxosd")];
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
        cmd.uid(peer.uid).gid(peer.gid).env_clear().env("HOME", &peer.home).env("USER", &peer.user)
            .env("XDG_RUNTIME_DIR", format!("/run/user/{}", peer.uid)).env("PATH", "/usr/local/bin:/usr/bin:/bin")
            .current_dir(&peer.home);
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

/// What the item is, for the service.
fn item_of(name: &str, a: &Value) -> Result<(&'static str, Value), String> {
    Ok(match name {
        "agentes.criar" | "agentes.editar" => ("agent", a.get("agente").cloned().ok_or(tr("argument 'agente' is required"))?),
        "fluxos.criar" | "fluxos.editar" => ("flow", json!({"id": a.get("id"), "name": a.get("nome"), "flow": a.get("fluxo")})),
        "ferramentas.criar_http" => ("http_tool", a.get("ferramenta").cloned().ok_or(tr("argument 'ferramenta' is required"))?),
        _ => return Err(trf("unknown platform tool {name}", &[("name", name.to_string())])),
    })
}

/// Creating and changing always ask the person (whatever the caller's level), with the exact result. Called by the
/// server before running the tool; Ok(None) = no approval needed.
pub fn needs_approval(peer: &Peer, rule: &crate::policy::CallerRule, name: &str, args: &Value) -> Result<Option<String>, String> {
    let creator = json!({"id": rule.name, "max_level": rule.max_level.name()});
    match name {
        "agentes.criar" | "agentes.editar" | "fluxos.criar" | "fluxos.editar" | "ferramentas.criar_http" => {
            let (kind, item) = item_of(name, args)?;
            let p = service(peer, "platform.preview", &json!({"kind": kind, "item": item, "creator": creator}))?;
            if let Some(problems) = p["problems"].as_array().filter(|v| !v.is_empty()) {
                return Err(problems.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("; "));
            }
            Ok(Some(preview(kind, &p["item"], p["notes"].as_array().map(Vec::as_slice).unwrap_or(&[]))))
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

/// The text of the approval: exactly what will exist.
fn preview(kind: &str, item: &Value, notes: &[Value]) -> String {
    let mut lines = Vec::new();
    match kind {
        "agent" => {
            lines.push(trf("create the agent \"{name}\" ({kind})", &[("name", item["name"].as_str().unwrap_or("").to_string()), ("kind", item["kind"].as_str().unwrap_or("").to_string())]));
            if let Some(i) = item["config"]["instruction"].as_str() {
                lines.push(trf("instructions: {text}", &[("text", i.chars().take(600).collect())]));
            }
            for t in item["config"]["tools"].as_array().into_iter().flatten() {
                lines.push(format!("· {} — {}", t["name"].as_str().unwrap_or(""), perm_text(t["permission"].as_str().unwrap_or("ask"))));
            }
            for s in item["config"]["sub_agents"].as_array().into_iter().flatten() {
                lines.push(format!("· {}", s.as_str().or(s["agent_id"].as_str()).unwrap_or("")));
            }
        }
        "flow" => {
            lines.push(trf("create the flow \"{name}\"", &[("name", item["name"].as_str().unwrap_or("").to_string())]));
            for n in item["flow"]["nodes"].as_array().into_iter().flatten() {
                let detail = n["data"]["tool"].as_str().or(n["data"]["agent_id"].as_str()).unwrap_or("");
                lines.push(format!("· {} {}", n["type"].as_str().unwrap_or(""), detail).trim_end().to_string());
            }
        }
        _ => {
            lines.push(trf("create the HTTP tool \"{name}\"", &[("name", item["name"].as_str().unwrap_or("").to_string())]));
            lines.push(format!("{} {}", item["method"].as_str().unwrap_or(""), item["endpoint"].as_str().unwrap_or("")));
        }
    }
    for n in notes.iter().filter_map(Value::as_str) {
        lines.push(note_text(n));
    }
    lines.join("\n")
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

fn perm_text(p: &str) -> &'static str {
    match p {
        "free" => tr("run freely"),
        "ask" => tr("ask first"),
        _ => tr("never run"),
    }
}

fn apply(c: &Ctx, name: &str, a: &Value, edit: bool) -> Result<Out, String> {
    let (kind, item) = item_of(name, a)?;
    let r = service(c.peer, "platform.apply", &json!({"kind": kind, "item": item, "creator": creator(c), "edit": edit}))?;
    let id = r["id"].as_str().unwrap_or("").to_string();
    // undo deletes what was created (an edit is undone by the item's own version history)
    let inverse = (!edit).then(|| json!({"ferramenta": "agentes.apagar_criado", "argumentos": {"tipo": kind, "id": id}}));
    Ok(Out { value: json!({"id": id, "ajustes": r["notes"]}), summary: trf("{what} created by the agent: {id}", &[("what", kind.to_string()), ("id", id.clone())]), inverse })
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
    let dir = std::env::var_os("KALANGO_CONECTORES_DIR").map(std::path::PathBuf::from).unwrap_or_else(|| c.peer.home.join(".config/kalango/conectores"));
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
        assert!(f.contains("system_action arquivos.mover"));
        assert_eq!(toml_str("a\"b"), "\"a\\\"b\"");
    }

    #[test]
    fn tools_are_registered() {
        let names: Vec<&str> = tools().iter().map(|t| t.name).collect();
        for n in ["agentes.criar", "agentes.editar", "agentes.listar", "fluxos.criar", "fluxos.editar", "fluxos.executar", "ferramentas.criar_http", "ferramentas.importar_openapi", "mcp.registrar", "modelos.listar"] {
            assert!(names.contains(&n), "{n}");
        }
    }
}
