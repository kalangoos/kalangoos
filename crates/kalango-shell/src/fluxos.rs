//! Client of kalango-fluxos (the agents and automations service): JSON-RPC in lines on its Unix socket
//! ($XDG_RUNTIME_DIR/kalango/fluxos.sock, or KALANGO_FLUXOS_SOCKET). The screens of the agents center (Meus agentes,
//! Ferramentas, the flow builder, Execuções, Modelos e provedores, Chaves, Sandbox) read and change everything through
//! here; nothing of the service runs in the shell.
//!
//! Calls never block the interface: `call` runs the request on a worker thread and hands the answer to a callback on
//! the main loop. `subscribe` delivers the service's events (run steps, approvals, decisions) the same way.

use gtk::glib;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

pub fn socket_path() -> PathBuf {
    if let Some(p) = std::env::var_os("KALANGO_FLUXOS_SOCKET") {
        return p.into();
    }
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(glib::user_runtime_dir);
    runtime.join("kalango/fluxos.sock")
}

/// One call, blocking (worker threads and tests only). Chat turns and model starts may take minutes.
pub fn call_blocking(method: &str, params: &Value, timeout: Duration) -> Result<Value, String> {
    let sock = socket_path();
    let mut s = UnixStream::connect(&sock).map_err(|e| format!("kalango-fluxos: {e}"))?;
    s.set_read_timeout(Some(timeout)).ok();
    writeln!(s, "{}", json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params})).map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(s).read_line(&mut line).map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
    match v.get("error") {
        Some(e) => Err(e["message"].as_str().unwrap_or("error").to_string()),
        None => Ok(v["result"].clone()),
    }
}

/// One call without blocking the interface; `done` runs on the main loop with the answer.
pub fn call(method: &str, params: Value, done: impl FnOnce(Result<Value, String>) + 'static) {
    let method = method.to_string();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(call_blocking(&method, &params, Duration::from_secs(600)));
    });
    let mut done = Some(done);
    glib::timeout_add_local(Duration::from_millis(30), move || match rx.try_recv() {
        Ok(r) => {
            if let Some(d) = done.take() {
                d(r);
            }
            glib::ControlFlow::Break
        }
        Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
        Err(_) => glib::ControlFlow::Break,
    });
}

/// The service's events of these topics ("run.", "approval", "agent.changed"…), on the main loop, while `on_event`
/// returns true. Reconnects when the service restarts (socket activation brings it back).
pub fn subscribe(topics: &[&str], mut on_event: impl FnMut(Value) -> bool + 'static) {
    let topics: Vec<String> = topics.iter().map(|t| t.to_string()).collect();
    let (tx, rx) = std::sync::mpsc::channel::<Value>();
    std::thread::spawn(move || loop {
        if let Ok(mut s) = UnixStream::connect(socket_path()) {
            if writeln!(s, "{}", json!({"jsonrpc": "2.0", "id": 1, "method": "events.subscribe", "params": {"topics": topics}})).is_ok() {
                for line in BufReader::new(s).lines().map_while(Result::ok) {
                    let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
                    if v["method"] == "event" && tx.send(v["params"].clone()).is_err() {
                        return;
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_secs(2));
    });
    glib::timeout_add_local(Duration::from_millis(50), move || loop {
        match rx.try_recv() {
            Ok(ev) => {
                if !on_event(ev) {
                    return glib::ControlFlow::Break;
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
            Err(_) => return glib::ControlFlow::Break,
        }
    });
}

/// Whether the service answers now.
pub fn running() -> bool {
    call_blocking("service.status", &json!({}), Duration::from_secs(2)).is_ok()
}
