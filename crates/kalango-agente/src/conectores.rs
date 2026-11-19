//! Third-party MCP connectors (the agents center's Ajustes › Conectores): each one is a kalangod manifest in
//! `~/.config/kalango/conectores/<name>.toml`, on unless `~/.config/kalango/conectores.json` turns it off. The agent
//! never runs a connector itself: it starts `kalangod conector rodar <manifest>`, the bridge that runs it isolated
//! (its own cgroup, Landlock, seccomp, clean environment), hides the blocked tools and logs every call where the
//! connector cannot reach. Through the bridge the agent speaks plain MCP.
//!
//! A connector's tools are named `<connector>.<tool>` and go through the same permissions as kalangod's (read only:
//! free; otherwise ask) and the same defence (their results are external data).

use crate::kalangod::Tool;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

pub fn pasta() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).filter(|p| p.is_absolute())
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config")).join("kalango/conectores")
}

fn estado_path() -> PathBuf {
    pasta().with_extension("json")
}

/// Whether a connector is on (the default) — `{"<name>": {"ligado": false}}` turns it off.
pub fn ligado(nome: &str) -> bool {
    std::fs::read_to_string(estado_path()).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v[nome]["ligado"].as_bool()).unwrap_or(true)
}

/// kalangod's binary: KALANGOD_BIN, next to this program, or the system one.
fn kalangod_bin() -> PathBuf {
    if let Some(p) = std::env::var_os("KALANGOD_BIN") {
        return p.into();
    }
    if let Some(p) = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("kalangod"))).filter(|p| p.is_file()) {
        return p;
    }
    PathBuf::from("/usr/bin/kalangod")
}

pub struct Conector {
    pub nome: String,
    child: Child,
    w: ChildStdin,
    r: Receiver<String>,
    n: u64,
    pub tools: Vec<Tool>,
}

impl Drop for Conector {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Conector {
    fn rpc(&mut self, method: &str, params: Value, prazo: Duration) -> Result<Value, String> {
        self.n += 1;
        let id = self.n;
        writeln!(self.w, "{}", json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})).map_err(|e| format!("{}: {e}", self.nome))?;
        let fim = std::time::Instant::now() + prazo;
        loop {
            let resto = fim.saturating_duration_since(std::time::Instant::now());
            let l = self.r.recv_timeout(resto).map_err(|_| kalango_i18n::trf("the connector {name} did not answer", &[("name", self.nome.clone())]))?;
            let Ok(v) = serde_json::from_str::<Value>(&l) else { continue };
            if v["id"] == json!(id) {
                return match v.get("error") {
                    Some(e) => Err(e["message"].as_str().unwrap_or("erro").to_string()),
                    None => Ok(v["result"].clone()),
                };
            }
        }
    }

    /// Starts the connector through kalangod's bridge, does the MCP handshake and lists its tools.
    pub fn abrir(manifesto: &std::path::Path) -> Result<Conector, String> {
        let nome = manifesto.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let mut child = Command::new(kalangod_bin()).args(["conector", "rodar"]).arg(manifesto)
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().map_err(|e| format!("kalangod: {e}"))?;
        let w = child.stdin.take().ok_or("stdin")?;
        let out = child.stdout.take().ok_or("stdout")?;
        let (tx, r) = channel();
        std::thread::spawn(move || {
            for l in BufReader::new(out).lines().map_while(Result::ok) {
                if tx.send(l).is_err() {
                    break;
                }
            }
        });
        let mut c = Conector { nome: nome.clone(), child, w, r, n: 0, tools: vec![] };
        c.rpc("initialize", json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "kalango-agente", "version": env!("CARGO_PKG_VERSION")}}), Duration::from_secs(20))?;
        let _ = writeln!(c.w, "{}", json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        let lista = c.rpc("tools/list", json!({}), Duration::from_secs(20))?;
        c.tools = lista["tools"].as_array().cloned().unwrap_or_default().iter().map(|t| Tool {
            name: format!("{nome}.{}", t["name"].as_str().unwrap_or("")),
            description: t["description"].as_str().unwrap_or("").to_string(),
            schema: if t["inputSchema"].is_object() { t["inputSchema"].clone() } else { json!({"type": "object"}) },
            level: "usuario".into(),
            read_only: t["annotations"]["readOnlyHint"].as_bool().unwrap_or(false),
            // without the annotation MCP says to assume the worst
            destructive: t["annotations"]["destructiveHint"].as_bool().unwrap_or(!t["annotations"]["readOnlyHint"].as_bool().unwrap_or(false)),
        }).collect();
        Ok(c)
    }

    /// Calls one of its tools (`tool` without the connector's prefix). Ok(text) or Err(text).
    pub fn chamar(&mut self, tool: &str, args: &Value) -> Result<String, String> {
        let r = self.rpc("tools/call", json!({"name": tool, "arguments": args}), Duration::from_secs(120))?;
        let texto = match r.get("structuredContent") {
            Some(v) if !v.is_null() => v.to_string(),
            _ => r["content"].as_array().map(|a| a.iter().filter_map(|c| c["text"].as_str()).collect::<Vec<_>>().join("\n")).unwrap_or_default(),
        };
        if r["isError"].as_bool().unwrap_or(false) { Err(texto) } else { Ok(texto) }
    }
}

/// What the connectors' configuration looks like now (the manifests and the on/off file, with their times): a change
/// means opening them again.
pub fn assinatura() -> Vec<(String, u128)> {
    let tempo = |p: &std::path::Path| std::fs::metadata(p).ok().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_nanos()).unwrap_or(0);
    let mut v: Vec<(String, u128)> = std::fs::read_dir(pasta()).into_iter().flatten().filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "toml")).map(|p| (p.display().to_string(), tempo(&p))).collect();
    v.sort();
    v.push((estado_path().display().to_string(), tempo(&estado_path())));
    v
}

/// The connectors that are on: (name, manifest, start only when a request needs it).
pub fn conhecidos() -> Vec<(String, PathBuf, bool)> {
    let Ok(dir) = std::fs::read_dir(pasta()) else { return vec![] };
    let mut caminhos: Vec<PathBuf> = dir.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "toml")).collect();
    caminhos.sort();
    caminhos.into_iter().filter_map(|p| {
        let nome = p.file_stem()?.to_string_lossy().into_owned();
        if !ligado(&nome) {
            return None;
        }
        let sob_demanda = std::fs::read_to_string(&p).ok().and_then(|t| t.parse::<toml::Table>().ok())
            .and_then(|t| t.get("sob_demanda").and_then(|v| v.as_bool())).unwrap_or(true);
        Some((nome, p, sob_demanda))
    }).collect()
}

/// Starts one (a failure is reported on stderr and leaves it out).
pub fn abrir(nome: &str, p: &std::path::Path) -> Option<Conector> {
    match Conector::abrir(p) {
        Ok(c) => Some(c),
        Err(e) => {
            eprintln!("kalango-agente: {}", kalango_i18n::trf("connector {name}: {error}", &[("name", nome.to_string()), ("error", e)]));
            None
        }
    }
}

/// The ones that start together with the agent (not on demand).
pub fn abrir_todos() -> Vec<Conector> {
    conhecidos().into_iter().filter(|(_, _, sob)| !sob).filter_map(|(n, p, _)| abrir(&n, &p)).collect()
}
