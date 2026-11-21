//! Conexões com o niri (IPC JSON por linha em $NIRI_SOCKET) e com o kalangod (pedidos de aprovação).

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::time::Duration;

pub enum Msg {
    Niri(Value),
    NiriCaiu,
    Pendentes(Option<usize>),
}

fn niri_socket() -> Option<PathBuf> {
    std::env::var_os("NIRI_SOCKET").map(PathBuf::from)
}

/// Lê o fluxo de eventos do niri; reconecta se cair.
pub fn ouvir_niri(tx: Sender<Msg>) {
    std::thread::spawn(move || loop {
        if let Some(sock) = niri_socket() {
            if let Ok(mut s) = UnixStream::connect(&sock) {
                if writeln!(s, "\"EventStream\"").is_ok() {
                    let r = BufReader::new(s);
                    for line in r.lines().map_while(Result::ok) {
                        if let Ok(v) = serde_json::from_str::<Value>(&line) {
                            if v.get("Ok").is_some() {
                                continue;
                            }
                            if tx.send(Msg::Niri(v)).is_err() {
                                return;
                            }
                        }
                    }
                }
            }
        }
        if tx.send(Msg::NiriCaiu).is_err() {
            return;
        }
        std::thread::sleep(Duration::from_secs(1));
    });
}

/// Pede uma ação ao niri, ex.: `{"FocusWindow": {"id": 3}}`.
pub fn niri_acao(acao: Value) -> Result<(), String> {
    let sock = niri_socket().ok_or("NIRI_SOCKET não definido (o shell precisa rodar dentro do niri)")?;
    let mut s = UnixStream::connect(sock).map_err(|e| e.to_string())?;
    s.set_read_timeout(Some(Duration::from_secs(2))).ok();
    writeln!(s, "{}", json!({ "Action": acao })).map_err(|e| e.to_string())?;
    let mut l = String::new();
    BufReader::new(s).read_line(&mut l).map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(&l).map_err(|e| e.to_string())?;
    match v.get("Err") {
        Some(e) => Err(e.to_string()),
        None => Ok(()),
    }
}

/// niri's version ("26.04"), asked over its socket.
pub fn niri_versao() -> Option<String> {
    let mut s = UnixStream::connect(niri_socket()?).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(2))).ok();
    writeln!(s, "\"Version\"").ok()?;
    let mut l = String::new();
    BufReader::new(s).read_line(&mut l).ok()?;
    let v: Value = serde_json::from_str(&l).ok()?;
    v["Ok"]["Version"].as_str()?.split_whitespace().next().map(str::to_string)
}

fn kalangod_socket() -> PathBuf {
    std::env::var_os("KALANGOD_SOCKET").map(PathBuf::from).unwrap_or_else(|| "/run/kalango/mcp.sock".into())
}

/// Acompanha os pedidos de aprovação do kalangod (espera longa); `None` quando ele não responde.
pub fn ouvir_kalangod(tx: Sender<Msg>) {
    std::thread::spawn(move || loop {
        if let Ok(s) = UnixStream::connect(kalangod_socket()) {
            let mut w = match s.try_clone() {
                Ok(w) => w,
                Err(_) => continue,
            };
            let mut r = BufReader::new(s);
            let mut geracao = u64::MAX;
            loop {
                let req = json!({"jsonrpc": "2.0", "id": 1, "method": "kalango/aguardar", "params": {"geracao": geracao, "max_s": 60}});
                if writeln!(w, "{req}").is_err() {
                    break;
                }
                let mut l = String::new();
                if r.read_line(&mut l).unwrap_or(0) == 0 {
                    break;
                }
                let Ok(v) = serde_json::from_str::<Value>(&l) else { break };
                geracao = v["result"]["geracao"].as_u64().unwrap_or(u64::MAX);
                let n = v["result"]["pendentes"].as_array().map(|a| a.len());
                if tx.send(Msg::Pendentes(n)).is_err() {
                    return;
                }
            }
        }
        if tx.send(Msg::Pendentes(None)).is_err() {
            return;
        }
        std::thread::sleep(Duration::from_secs(3));
    });
}

/// Pedido simples ao niri (ex.: `"Outputs"`); devolve o conteúdo de `Ok`.
pub fn niri_pedido(req: Value) -> Result<Value, String> {
    let sock = niri_socket().ok_or("NIRI_SOCKET não definido")?;
    let mut s = UnixStream::connect(sock).map_err(|e| e.to_string())?;
    s.set_read_timeout(Some(Duration::from_secs(2))).ok();
    writeln!(s, "{req}").map_err(|e| e.to_string())?;
    let mut l = String::new();
    BufReader::new(s).read_line(&mut l).map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(&l).map_err(|e| e.to_string())?;
    v.get("Ok").cloned().ok_or_else(|| v.get("Err").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| v.to_string()))
}

pub(crate) fn kalangod_rpc(metodo: &str, params: Value) -> Result<Value, String> {
    kalangod_rpc_ate(metodo, params, Duration::from_secs(5))
}

/// A request that may wait long for its answer (kalangod's long waits).
pub(crate) fn kalangod_rpc_wait(metodo: &str, params: Value, prazo: Duration) -> Result<Value, String> {
    kalangod_rpc_ate(metodo, params, prazo)
}

fn kalangod_rpc_ate(metodo: &str, params: Value, prazo: Duration) -> Result<Value, String> {
    let s = UnixStream::connect(kalangod_socket()).map_err(|e| format!("kalangod: {e}"))?;
    s.set_read_timeout(Some(prazo)).ok();
    let mut w = s.try_clone().map_err(|e| e.to_string())?;
    writeln!(w, "{}", json!({"jsonrpc": "2.0", "id": 1, "method": metodo, "params": params})).map_err(|e| e.to_string())?;
    let mut l = String::new();
    BufReader::new(s).read_line(&mut l).map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(&l).map_err(|e| e.to_string())?;
    match v.get("error") {
        Some(e) => Err(e["message"].as_str().unwrap_or("erro").to_string()),
        None => Ok(v["result"].clone()),
    }
}

/// Pedidos de aprovação esperando (para a barra lateral, modo Agente).
pub fn kalangod_pendentes() -> Result<Vec<Value>, String> {
    kalangod_rpc("kalango/pendentes", json!({})).map(|v| v.as_array().cloned().unwrap_or_default())
}

/// Answers a pending request (the overview only refuses: approving goes through the approval screen).
pub fn kalangod_decidir(id: u64, aprovar: bool) -> Result<Value, String> {
    kalangod_rpc("kalango/decidir", json!({"id": id, "aprovar": aprovar}))
}

/// Últimos registros da linha do tempo de quem chama (o shell roda como a pessoa), os mais novos primeiro, sem que a
/// leitura vire registro.
pub fn kalangod_linha_do_tempo(limite: u32) -> Result<Vec<Value>, String> {
    kalangod_registros(0, limite).map(|mut v| {
        v.reverse();
        v
    })
}

/// Takes back every "always" answer the person gave a program on the system approval screen (by the executable's
/// name: "Revogar tudo" in the agents center). Ok(how many).
pub fn kalangod_revogar_sempre_de(programa: &str) -> Result<usize, String> {
    let r = kalangod_rpc("kalango/sempre", json!({}))?;
    let lista = r["respostas"].as_array().cloned().unwrap_or_default();
    let mut n = 0;
    // from the end, so the indices of the ones before stay the same
    for (i, s) in lista.iter().enumerate().rev() {
        let exe = s["exe"].as_str().unwrap_or("");
        if std::path::Path::new(exe).file_name().is_some_and(|f| f == programa) {
            kalangod_rpc("kalango/revogar_sempre", json!({"indice": i}))?;
            n += 1;
        }
    }
    Ok(n)
}

/// The person's records (the agents center's timeline), without being recorded itself: since `desde` (unix time).
pub fn kalangod_registros(desde: i64, limite: u32) -> Result<Vec<Value>, String> {
    kalangod_rpc("kalango/registros", json!({"desde": desde.max(0), "limite": limite})).map(|v| v["registros"].as_array().cloned().unwrap_or_default())
}

/// A tool call that may wait for the person's approval on the system screen (up to kalangod's limit and a bit).
/// Ok(structured result) or Err(why).
pub fn kalangod_chamar(ferramenta: &str, args: Value) -> Result<Value, String> {
    let r = kalangod_rpc_ate("tools/call", json!({"name": ferramenta, "arguments": args}), Duration::from_secs(150))?;
    if r["isError"].as_bool().unwrap_or(false) {
        return Err(r["content"][0]["text"].as_str().unwrap_or("").to_string());
    }
    Ok(r["structuredContent"].clone())
}

/// The agents' visible browser (kalangod's Moli, held): an action and the page it leaves, with its picture.
pub fn kalangod_navegador(params: Value) -> Result<Value, String> {
    kalangod_rpc_ate("kalango/navegador", params, Duration::from_secs(60))
}

/// Where agents' commands write (kalangod's allowlist): the work folders and the project folders.
pub fn kalangod_pastas() -> Result<Value, String> {
    kalangod_rpc("kalango/pastas", json!({}))
}

/// Takes a project folder out (commands only read it again).
pub fn kalangod_pastas_remover(caminho: &str) -> Result<Value, String> {
    kalangod_rpc("kalango/pastas_remover", json!({"caminho": caminho}))
}

/// Adds a project folder: kalangod asks on the approval screen first (waits for the answer).
pub fn kalangod_pastas_adicionar(caminho: &str) -> Result<Value, String> {
    kalangod_rpc_ate("kalango/pastas_adicionar", json!({"caminho": caminho}), Duration::from_secs(150))
}

/// kalangod's tools as this process sees them (the agents center's built-in connectors).
pub fn kalangod_ferramentas() -> Result<Vec<Value>, String> {
    kalangod_rpc("tools/list", json!({})).map(|v| v["tools"].as_array().cloned().unwrap_or_default())
}
