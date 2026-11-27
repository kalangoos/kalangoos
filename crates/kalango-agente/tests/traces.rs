//! The trace capture with the real local model (LFM3 via llama.cpp: never a fake model) and the real kalangod (no
//! root, dry run), against a Train daemon on a Unix socket. `#[ignore]`d: it needs the model.
//!
//! Needs (the same as tests/motor.rs):
//! - `cargo build -p kalangod` in the same profile as the test run (the binary is looked up next to the test's);
//! - llama-server built and the LFM3 model in `~/.local/share/kalango/modelos` (`KALANGO_MODELOS` points elsewhere;
//!   the engine starts llama-server by itself);
//! - run: `cargo test -p kalango-agente --test traces -- --ignored --nocapture`.
//!
//! What the test points at, by environment (the engine child processes get them; the person's real folders are never
//! touched): `KALANGO_TRAIND_SOCKET` (the daemon's socket, see below), `XDG_CONFIG_HOME` (a throwaway folder whose
//! `kalango/configuracoes.json` holds the `rastros` switches), `KALANGOD_SOCKET`, and `HOME` (the test's temporary
//! folder, kalangod's `home` too). The model is not deterministic: the assertions hold for any path it takes (what the
//! model did is printed, and a check about a tool is made only when the model called it).
//!
//! Two modes for the daemon:
//! - default: a fake daemon of the test's own, on a socket in the temporary folder. It checks nothing about the receiver.
//! - against the real `kalango-traind`: `KALANGO_TRAIND_REAL=1 KALANGO_TRAIND_SOCKET=/path/to/traind.sock`. The test
//!   puts a recording proxy between the engine and that socket: the engine talks to the proxy, which forwards every
//!   request to the real daemon and gives the engine its real answer, and the test asserts the payloads as before
//!   **and** that the real daemon answered every request with 200. Start the daemon first (`kalango-train daemon`) and
//!   use a throwaway one (own `XDG_RUNTIME_DIR` and data folder): the test's turns are stored in its database. The
//!   test then reads them back with the daemon's own tools (`kalango-train traces ls`), which it does not do itself.

use serde_json::{json, Value};
use std::io::{Read, Write};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, Once};
use std::time::Duration;

struct Env {
    dir: tempfile::TempDir,
    area: PathBuf,
    sock: PathBuf,
    kalangod: Child,
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = self.kalangod.kill();
    }
}

fn kalangod_bin() -> PathBuf {
    let p = std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().join("kalangod");
    assert!(p.is_file(), "build kalangod first: cargo build -p kalangod (same profile) — {}", p.display());
    p
}

fn model_up() {
    static MODEL: Once = Once::new();
    MODEL.call_once(|| {
        let real = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap()).join(".local/share"));
        if std::env::var_os("KALANGO_MODELOS").is_none() {
            std::env::set_var("KALANGO_MODELOS", real.join("kalango/modelos"));
        }
        let d = tempfile::tempdir().unwrap();
        std::env::set_var("XDG_STATE_HOME", d.path().join("state"));
        std::env::set_var("XDG_DATA_HOME", d.path().join("data"));
        std::env::set_var("KALANGO_MEMORIA_CHAVEIRO", "nao");
        std::mem::forget(d);
        let out = Command::new(env!("CARGO_BIN_EXE_kalango-agente")).arg("modelo").output().unwrap();
        assert!(out.status.success(), "local model: {}", String::from_utf8_lossy(&out.stderr));
    });
}

const TOKEN: &str = concat!("ghp", "_abcdefghijklmnopqrstuvwxyz0123456789");

fn setup() -> Env {
    model_up();
    let dir = tempfile::tempdir().unwrap();
    let area = dir.path().join("Documentos/area");
    std::fs::create_dir_all(&area).unwrap();
    std::fs::write(area.join("orcamento.txt"), "Orçamento de novembro: 1200 reais.").unwrap();
    std::fs::write(area.join("lista.txt"), "pão, leite, café").unwrap();
    // (invented: a token the model may read and the capture must not send)
    std::fs::write(area.join("chaves.txt"), format!("conta de teste\ntoken: {TOKEN}\npassword=hunter2-correct-horse\n")).unwrap();
    let sock = dir.path().join("mcp.sock");
    let cfg = dir.path().join("kalangod.toml");
    std::fs::write(
        &cfg,
        format!(
            r#"
home = "{home}"
socket = "{sock}"
state_dir = "{state}"
protected_paths = []
approval_requires_tty = false
approval_timeout_s = 5
total_grant_s = 900
dry_run = true
restore_root = "{raiz}"
trash_dir = "{raiz}/lixeira"

[[callers]]
name = "teste"
max_level = "total"
allowed_paths = ["{area}"]
"#,
            home = dir.path().display(),
            sock = sock.display(),
            state = dir.path().join("estado").display(),
            raiz = dir.path().join("raiz").display(),
            area = area.display()
        ),
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("raiz/etc")).unwrap();
    let kalangod = Command::new(kalangod_bin()).args(["servir", "--config"]).arg(&cfg).stderr(Stdio::null()).spawn().unwrap();
    for _ in 0..100 {
        if sock.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    Env { dir, area, sock, kalangod }
}

/// The daemon the engine talks to: the body of every POST /v1/traces, and the status it was answered with. A fake
/// (answers 200) or, with KALANGO_TRAIND_REAL=1, a proxy that forwards each request to the real daemon's socket
/// (`KALANGO_TRAIND_SOCKET`) and gives the engine the real answer.
fn daemon(path: &Path) -> (Arc<Mutex<Vec<Value>>>, Arc<Mutex<Vec<u16>>>) {
    let upstream = real_daemon();
    let listener = UnixListener::bind(path).unwrap();
    let bodies = Arc::new(Mutex::new(Vec::new()));
    let statuses = Arc::new(Mutex::new(Vec::new()));
    let (b0, s0) = (bodies.clone(), statuses.clone());
    std::thread::spawn(move || {
        for c in listener.incoming().flatten() {
            let (b, st, upstream) = (b0.clone(), s0.clone(), upstream.clone());
            std::thread::spawn(move || {
                let mut c = c;
                let mut buf = Vec::new();
                let mut chunk = [0u8; 65536];
                loop {
                    let n = c.read(&mut chunk).unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&buf[..p]).to_lowercase();
                        let len: usize = head.lines().find_map(|l| l.strip_prefix("content-length:").map(|v| v.trim().parse().unwrap())).unwrap_or(0);
                        while buf.len() < p + 4 + len {
                            let n = c.read(&mut chunk).unwrap_or(0);
                            if n == 0 {
                                break;
                            }
                            buf.extend_from_slice(&chunk[..n]);
                        }
                        if let Ok(v) = serde_json::from_slice::<Value>(&buf[p + 4..]) {
                            b.lock().unwrap().push(v);
                        }
                        let answer = match &upstream {
                            Some(sock) => forward(sock, &buf),
                            None => b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 20\r\nConnection: close\r\n\r\n{\"partialSuccess\":{}}".to_vec(),
                        };
                        let status = String::from_utf8_lossy(&answer).split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
                        st.lock().unwrap().push(status);
                        let _ = c.write_all(&answer);
                        return;
                    }
                }
            });
        }
    });
    (bodies, statuses)
}

/// The real daemon's socket when the test is asked to use it (`KALANGO_TRAIND_REAL=1`), else None.
fn real_daemon() -> Option<PathBuf> {
    if std::env::var("KALANGO_TRAIND_REAL").as_deref() != Ok("1") {
        return None;
    }
    let sock = std::env::var_os("KALANGO_TRAIND_SOCKET").map(PathBuf::from).filter(|p| p.is_absolute());
    let sock = sock.expect("KALANGO_TRAIND_REAL=1 needs KALANGO_TRAIND_SOCKET=/absolute/path/to/traind.sock");
    // the handshake says which daemon it is before any turn is sent to it
    let hello = forward(&sock, b"GET /v1/hello HTTP/1.1\r\nHost: kalango-traind\r\nConnection: close\r\n\r\n");
    let text = String::from_utf8_lossy(&hello).to_string();
    assert!(
        text.starts_with("HTTP/1.1 200") && text.contains("kalango-traind"),
        "the daemon at {} did not answer the handshake: {text}",
        sock.display()
    );
    Some(sock)
}

/// One request/answer over a Unix socket (the answer read to the end: the engine and the test close the connection).
fn forward(sock: &Path, request: &[u8]) -> Vec<u8> {
    let mut up = std::os::unix::net::UnixStream::connect(sock).unwrap_or_else(|e| panic!("the real daemon at {}: {e}", sock.display()));
    up.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    up.write_all(request).unwrap();
    let mut out = Vec::new();
    let _ = up.read_to_end(&mut out);
    out
}

/// In the real mode: every request the real daemon was given was answered 200 (in the fake mode this is trivially so).
fn assert_accepted(statuses: &Arc<Mutex<Vec<u16>>>) {
    let st = statuses.lock().unwrap().clone();
    assert!(!st.is_empty(), "no request reached the daemon");
    assert!(st.iter().all(|s| *s == 200), "the daemon refused a request: {st:?}");
}

fn spans_of(bodies: &Arc<Mutex<Vec<Value>>>) -> Vec<Value> {
    bodies
        .lock()
        .unwrap()
        .iter()
        .flat_map(|r| r["resourceSpans"].as_array().unwrap().clone())
        .flat_map(|rs| rs["scopeSpans"].as_array().unwrap().clone())
        .flat_map(|ss| ss["spans"].as_array().unwrap().clone())
        .collect()
}

fn attr<'a>(span: &'a Value, key: &str) -> Option<&'a Value> {
    span["attributes"].as_array()?.iter().find(|a| a["key"] == key).map(|a| &a["value"])
}

/// One request with the engine as a child process (it exits at the end, which flushes the capture).
fn ask(env: &Env, config_home: &Path, daemon: &Path, request: &str) -> Vec<Value> {
    let out = Command::new(env!("CARGO_BIN_EXE_kalango-agente"))
        .arg("--json")
        .arg(request)
        .env("KALANGOD_SOCKET", &env.sock)
        .env("LANGUAGE", "pt_BR")
        .env("XDG_CONFIG_HOME", config_home)
        .env("KALANGO_TRAIND_SOCKET", daemon)
        // (the test's folder, never the person's: kalangod's `home` is the same)
        .env("HOME", env.dir.path())
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    eprintln!("── {request}\n{text}");
    text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

fn config(env: &Env, name: &str, rastros: Option<Value>) -> PathBuf {
    let home = env.dir.path().join(name);
    std::fs::create_dir_all(home.join("kalango")).unwrap();
    if let Some(r) = rastros {
        std::fs::write(home.join("kalango/configuracoes.json"), json!({"rastros": r}).to_string()).unwrap();
    }
    home
}

fn wait_spans(bodies: &Arc<Mutex<Vec<Value>>>, at_least: usize) -> Vec<Value> {
    for _ in 0..100 {
        let s = spans_of(bodies);
        if s.len() >= at_least {
            return s;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    spans_of(bodies)
}

#[test]
#[ignore = "needs the real LFM3 model and kalangod (see the header)"]
fn a_tool_calling_conversation_is_captured_with_the_real_model_and_leaks_no_secret() {
    let env = setup();
    let socket = env.dir.path().join("traind.sock");
    let (bodies, statuses) = daemon(&socket);
    let cfg = config(&env, "cfg-full", None);
    let ev = ask(&env, &cfg, &socket, &format!("Quais arquivos existem na pasta {}? Depois leia o arquivo chaves.txt dela.", env.area.display()));
    assert!(!ev.iter().any(|e| e["tipo"] == "erro"), "{ev:?}");
    let spans = wait_spans(&bodies, 3);
    assert!(!spans.is_empty(), "the engine sent spans to the daemon");
    let roots: Vec<&Value> = spans.iter().filter(|s| s["name"] == "kalango.turn").collect();
    assert_eq!(roots.len(), 1, "one root per turn");
    let root = roots[0];
    assert!(root["parentSpanId"].is_null());
    assert_eq!(attr(root, "kalango.privacy.mode").unwrap()["stringValue"], "content");
    let trace = root["traceId"].as_str().unwrap();
    assert!(spans.iter().all(|s| s["traceId"] == trace), "one trace");
    let chats: Vec<&Value> = spans.iter().filter(|s| s["name"].as_str().unwrap().starts_with("chat ")).collect();
    assert!(!chats.is_empty(), "at least one model call");
    for c in &chats {
        assert!(
            attr(c, "gen_ai.usage.input_tokens").unwrap()["intValue"].as_str().unwrap().parse::<u64>().unwrap() > 0,
            "the model's own token count"
        );
        assert!(c["events"].as_array().unwrap().iter().any(|e| e["name"] == "gen_ai.choice"));
    }
    let tools: Vec<&Value> = spans.iter().filter(|s| s["name"].as_str().unwrap().starts_with("execute_tool ")).collect();
    let called: Vec<&str> = ev.iter().filter(|e| e["tipo"] == "ferramenta").filter_map(|e| e["nome"].as_str()).collect();
    eprintln!("the model called {called:?}; the trace has {} tool spans in {} model calls", tools.len(), chats.len());
    // (a tool the engine answers inside itself, like the plan, is a span too: every ToolStart/ToolDone pair is one)
    assert_eq!(tools.len(), called.len(), "one tool span per tool call");
    // what a tool span says: the agent's own level (never the receiver's names), whether it reads, the kind of external
    // content (never the bare word), and the id the model gave the call
    for t in &tools {
        let name = attr(t, "kalango.tool.name").unwrap()["stringValue"].as_str().unwrap();
        assert!(
            matches!(attr(t, "kalango.tool.level").unwrap()["stringValue"].as_str().unwrap(), "observar" | "usuario" | "sistema" | "total"),
            "{name}"
        );
        let prov = attr(t, "kalango.provenance").unwrap()["stringValue"].as_str().unwrap();
        assert!(matches!(prov, "tool_local" | "external:file" | "external:web" | "external:mail" | "external:tool"), "{name}: {prov}");
        if matches!(name, "arquivos.listar" | "arquivos.ler" | "relogio.agora" | "calculadora.calcular") {
            assert_eq!(attr(t, "kalango.tool.read_only").unwrap()["boolValue"], true, "{name} only reads");
        }
        if matches!(name, "relogio.agora" | "calculadora.calcular") {
            assert_eq!(prov, "tool_local", "{name} is the agent's own: no external taint");
        }
        if !name.starts_with("web.") {
            let id = attr(t, "gen_ai.tool.call.id").unwrap_or_else(|| panic!("{name}: the call id is sent"))["stringValue"].as_str().unwrap();
            assert!(!id.is_empty() && id.len() < 20 || id.starts_with("call_"), "{name}: {id}");
        }
    }
    if called.contains(&"arquivos.listar") {
        assert!(tools.iter().any(|t| attr(t, "kalango.tool.name").unwrap()["stringValue"] == "arquivos.listar"));
    }
    // the secret the model read never left the process, whatever it did with it
    let raw = bodies.lock().unwrap().iter().map(|b| b.to_string()).collect::<Vec<_>>().join("\n");
    assert!(!raw.contains(TOKEN) && !raw.contains("hunter2-correct-horse"), "no secret in the payload");
    if called.contains(&"arquivos.ler") {
        assert!(raw.contains("[REDACTED"), "the file with the token was read: the trace shows it was redacted");
    }
    assert_accepted(&statuses);
    // the answer is on the root, as the person got it
    if ev.iter().any(|e| e["tipo"] == "resposta") {
        assert!(root["events"].as_array().unwrap().iter().any(|e| e["name"] == "gen_ai.choice"), "the root carries the answer");
    }
}

#[test]
#[ignore = "needs the real LFM3 model and kalangod (see the header)"]
fn metadata_only_and_recording_off_with_the_real_model() {
    let env = setup();
    let socket = env.dir.path().join("traind.sock");
    let (bodies, statuses) = daemon(&socket);
    let meta = config(&env, "cfg-meta", Some(json!({"gravar": true, "conteudo": "so_metadados"})));
    let request = format!("Liste a pasta {}.", env.area.display());
    ask(&env, &meta, &socket, &request);
    let spans = wait_spans(&bodies, 2);
    assert!(!spans.is_empty());
    assert!(spans.iter().all(|s| s["events"].is_null()), "metadata only: no content events");
    let raw = bodies.lock().unwrap().iter().map(|b| b.to_string()).collect::<Vec<_>>().join("\n");
    assert!(!raw.contains("orcamento") && !raw.contains("Liste a pasta"), "no text of the conversation");
    assert_accepted(&statuses);
    let (n, requests) = (spans.len(), statuses.lock().unwrap().len());
    // paused by the person: nothing more is sent
    let off = config(&env, "cfg-off", Some(json!({"gravar": false})));
    ask(&env, &off, &socket, &request);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(spans_of(&bodies).len(), n, "recording off sends nothing");
    assert_eq!(statuses.lock().unwrap().len(), requests, "not even a request");
}

#[test]
#[ignore = "needs the real LFM3 model and kalangod (see the header)"]
fn a_pasted_part_of_the_request_is_marked_external_paste_with_the_real_model() {
    use std::io::{BufRead, BufReader};
    let env = setup();
    let socket = env.dir.path().join("traind.sock");
    let (bodies, statuses) = daemon(&socket);
    let cfg = config(&env, "cfg-paste", None);
    // a conversation over the shell's channel: a request in parts, the way the shell sends what was pasted into the composer
    let typed = "Responda só com a palavra ok. Texto: ";
    let pasted = "PAGINA-COLADA-XYZ";
    let mut child = Command::new(env!("CARGO_BIN_EXE_kalango-agente"))
        .arg("--json")
        .env("KALANGOD_SOCKET", &env.sock)
        .env("LANGUAGE", "pt_BR")
        .env("XDG_CONFIG_HOME", &cfg)
        .env("KALANGO_TRAIND_SOCKET", &socket)
        .env("HOME", env.dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    writeln!(stdin, "{}", json!({"parts": [{"text": typed, "origin": "typed"}, {"text": pasted, "origin": "pasted"}]})).unwrap();
    for line in BufReader::new(child.stdout.take().unwrap()).lines().map_while(Result::ok) {
        if serde_json::from_str::<Value>(&line).is_ok_and(|v| v["tipo"] == "fim") {
            break;
        }
    }
    drop(stdin);
    let _ = child.wait();
    let spans = wait_spans(&bodies, 2);
    let root = spans.iter().find(|s| s["name"] == "kalango.turn").expect("the turn's root");
    let from = typed.chars().count();
    assert_eq!(attr(root, "kalango.provenance").unwrap()["stringValue"], "external:paste");
    assert_eq!(attr(root, "kalango.paste.ranges").unwrap()["stringValue"], format!("[[{from},{}]]", from + pasted.chars().count()));
    let taint: Vec<&str> =
        attr(root, "kalango.taint").unwrap()["arrayValue"]["values"].as_array().unwrap().iter().filter_map(|v| v["stringValue"].as_str()).collect();
    assert!(taint.contains(&"external:paste"), "{taint:?}");
    let user = root["events"].as_array().unwrap().iter().find(|e| e["name"] == "gen_ai.user.message").unwrap();
    assert!(user["attributes"].as_array().unwrap().iter().any(|a| a["key"] == "kalango.provenance" && a["value"]["stringValue"] == "external:paste"));
    assert!(user.to_string().contains(pasted), "the person's words, pasted part included, are the content");
    assert_accepted(&statuses);
}
