//! MCP client for kalangod over its Unix socket. Every system action of an agent goes through here: kalangod
//! identifies the caller by the kernel (SO_PEERCRED + cgroup), applies the levels, asks the person on the approval
//! screen and records everything in the timeline. The engine never acts on the system by itself.

use kalango_i18n::trf;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

/// A tool as kalangod describes it, with the contract the defence needs.
#[derive(Clone, Debug)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub schema: Value,
    /// "observar", "usuario", "sistema" or "total"
    pub level: String,
    pub read_only: bool,
    pub destructive: bool,
}

pub struct Client {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    next_id: u64,
    pub instructions: String,
}

pub fn socket_path() -> PathBuf {
    std::env::var_os("KALANGOD_SOCKET").map(PathBuf::from).unwrap_or_else(|| "/run/kalango/mcp.sock".into())
}

impl Client {
    pub fn connect() -> Result<Client, String> {
        let path = socket_path();
        let s = UnixStream::connect(&path)
            .map_err(|e| trf("kalangod is not running at {socket}: {error}", &[("socket", path.display().to_string()), ("error", e.to_string())]))?;
        let writer = s.try_clone().map_err(|e| e.to_string())?;
        let mut c = Client { reader: BufReader::new(s), writer, next_id: 1, instructions: String::new() };
        let r = c.request("initialize", json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "kalango-agente", "version": env!("CARGO_PKG_VERSION")}
        }))?;
        c.instructions = r["instructions"].as_str().unwrap_or("").to_string();
        c.notify("notifications/initialized")?;
        Ok(c)
    }

    fn notify(&mut self, method: &str) -> Result<(), String> {
        writeln!(self.writer, "{}", json!({"jsonrpc": "2.0", "method": method})).map_err(|e| e.to_string())
    }

    /// One JSON-RPC request; waits as long as needed (a call can be waiting for the person's approval).
    fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        writeln!(self.writer, "{}", json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})).map_err(|e| e.to_string())?;
        loop {
            let mut line = String::new();
            if self.reader.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
                return Err("kalangod closed the connection".into());
            }
            let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
            if v["id"] != json!(id) {
                continue;
            }
            if let Some(e) = v.get("error") {
                return Err(e["message"].as_str().unwrap_or("error").to_string());
            }
            return Ok(v["result"].clone());
        }
    }

    pub fn tools(&mut self) -> Result<Vec<Tool>, String> {
        let r = self.request("tools/list", json!({}))?;
        Ok(r["tools"].as_array().cloned().unwrap_or_default().into_iter().map(|t| Tool {
            name: t["name"].as_str().unwrap_or("").to_string(),
            description: t["description"].as_str().unwrap_or("").to_string(),
            schema: t["inputSchema"].clone(),
            level: t["_meta"]["os.kalango/nivel"].as_str().unwrap_or("total").to_string(),
            read_only: t["annotations"]["readOnlyHint"].as_bool().unwrap_or(false),
            destructive: t["annotations"]["destructiveHint"].as_bool().unwrap_or(false),
        }).collect())
    }

/// The folders no agent reaches, at any level (kalangod's protected paths).
    pub fn protected(&mut self) -> Vec<String> {
        self.request("kalango/protegidas", json!({})).ok().and_then(|v| v["pastas"].as_array().cloned()).unwrap_or_default()
            .into_iter().filter_map(|p| p.as_str().map(str::to_string)).collect()
    }

    /// Whether kalangod will ask the person itself, on the system approval screen, before this call: it changes a
    /// file that runs at login and the person wants those changes asked, or it changes something outside the folders
    /// agents work in (always asked; with the diff, never "always"). The
    /// conversation then does not ask the same question first. An old kalangod without the method: false.
    pub fn asks_for_startup_file(&mut self, name: &str, args: &Value) -> bool {
        self.request("kalango/inicio", json!({"ferramenta": name, "argumentos": args})).ok().and_then(|v| v["pede"].as_bool()).unwrap_or(false)
    }

        /// How many approval requests wait for the person.
    pub fn pending(&mut self) -> Result<usize, String> {
        Ok(self.request("kalango/pendentes", json!({}))?.as_array().map(|a| a.len()).unwrap_or(0))
    }

    /// Calls a tool. Ok((text, summary)) on success, Err(text) when kalangod or the tool refused or failed (the
    /// text goes back to the model either way). The summary is kalangod's one-line description of what happened,
    /// in the system language.
    pub fn call(&mut self, name: &str, args: &Value) -> Result<(String, String), String> {
        let r = self.request("tools/call", json!({"name": name, "arguments": args}))?;
        let text = match r.get("structuredContent") {
            Some(v) if !v.is_null() => v.to_string(),
            _ => r["content"].as_array().map(|a| a.iter().filter_map(|c| c["text"].as_str()).collect::<Vec<_>>().join("\n")).unwrap_or_default(),
        };
        if r["isError"].as_bool().unwrap_or(false) {
            Err(text)
        } else {
            Ok((text, r["_meta"]["os.kalango/resumo"].as_str().unwrap_or("").to_string()))
        }
    }
}
