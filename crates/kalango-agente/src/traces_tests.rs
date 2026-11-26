//! Tests of the trace capture against a fake receiver on a Unix socket that checks the bodies against the contract
//! (docs/INGEST.md, schema version 1). The real-model test is in tests/traces.rs.

use super::*;
use crate::modelo::ToolCall;
use std::io::Read;
use std::os::unix::net::UnixListener;
use std::sync::atomic::AtomicBool;

// ---------------------------------------------------------------- a fake receiver

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// answers 200
    Ok,
    /// accepts and never reads or answers (a daemon that hangs)
    Stall,
    /// answers this status
    Status(u16),
}

struct Receiver {
    bodies: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
}

impl Drop for Receiver {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Receiver {
    fn start(path: &Path, mode: Mode) -> Receiver {
        let _ = std::fs::remove_file(path);
        let listener = UnixListener::bind(path).unwrap();
        listener.set_nonblocking(true).unwrap();
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (b, s) = (bodies.clone(), stop.clone());
        std::thread::spawn(move || {
            let mut held = Vec::new();
            while !s.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut c, _)) => {
                        if mode == Mode::Stall {
                            held.push(c);
                            continue;
                        }
                        c.set_nonblocking(false).unwrap();
                        c.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                        let mut buf = Vec::new();
                        let mut chunk = [0u8; 65536];
                        // headers, then Content-Length bytes
                        let (head_end, len) = loop {
                            let n = c.read(&mut chunk).unwrap_or(0);
                            if n == 0 {
                                break (0, 0);
                            }
                            buf.extend_from_slice(&chunk[..n]);
                            if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                                let head = String::from_utf8_lossy(&buf[..p]).to_string();
                                assert!(head.starts_with("POST /v1/traces HTTP/1.1"), "{head}");
                                assert!(head.to_lowercase().contains("content-type: application/json"), "{head}");
                                let len = head
                                    .lines()
                                    .find_map(|l| l.to_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap()))
                                    .unwrap();
                                break (p + 4, len);
                            }
                        };
                        while buf.len() < head_end + len {
                            let n = c.read(&mut chunk).unwrap_or(0);
                            if n == 0 {
                                break;
                            }
                            buf.extend_from_slice(&chunk[..n]);
                        }
                        if head_end > 0 {
                            b.lock().unwrap().push(String::from_utf8_lossy(&buf[head_end..]).to_string());
                        }
                        let (code, text) = match mode {
                            Mode::Status(c) => (c, "x"),
                            _ => (200, "OK"),
                        };
                        let body = "{\"partialSuccess\":{}}";
                        let _ = write!(
                            c,
                            "HTTP/1.1 {code} {text}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(2)),
                }
            }
        });
        Receiver { bodies, stop }
    }

    fn requests(&self) -> Vec<Value> {
        self.bodies.lock().unwrap().iter().map(|b| serde_json::from_str(b).expect("the body is JSON")).collect()
    }

    fn raw(&self) -> String {
        self.bodies.lock().unwrap().join("\n")
    }

    /// Every span received, with the privacy mode of its resource.
    fn spans(&self) -> Vec<(Value, String)> {
        let mut out = Vec::new();
        for r in self.requests() {
            validate_request(&r);
            for rs in r["resourceSpans"].as_array().unwrap() {
                let mode =
                    attr(&rs["resource"]["attributes"], "kalango.privacy.mode").map(|v| v["stringValue"].as_str().unwrap().to_string()).unwrap();
                for ss in rs["scopeSpans"].as_array().unwrap() {
                    for sp in ss["spans"].as_array().unwrap() {
                        out.push((sp.clone(), mode.clone()));
                    }
                }
            }
        }
        out
    }

    fn wait_for(&self, spans: usize) -> Vec<(Value, String)> {
        let end = Instant::now() + Duration::from_secs(5);
        loop {
            let s = self.spans();
            if s.len() >= spans || Instant::now() > end {
                return s;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

fn attr<'a>(attrs: &'a Value, key: &str) -> Option<&'a Value> {
    attrs.as_array()?.iter().find(|a| a["key"] == key).map(|a| &a["value"])
}

fn s_attr<'a>(span: &'a Value, key: &str) -> Option<&'a str> {
    attr(&span["attributes"], key).and_then(|v| v["stringValue"].as_str())
}

fn list_attr(span: &Value, key: &str) -> Option<Vec<String>> {
    attr(&span["attributes"], key)
        .map(|v| v["arrayValue"]["values"].as_array().unwrap().iter().map(|x| x["stringValue"].as_str().unwrap().to_string()).collect())
}

fn is_hex(s: &str, n: usize) -> bool {
    s.len() == n && s.bytes().all(|c| c.is_ascii_hexdigit()) && s.bytes().any(|c| c != b'0')
}

/// The contract of docs/INGEST.md, sections 2, 3 and 5.
fn validate_request(r: &Value) {
    let rss = r["resourceSpans"].as_array().expect("resourceSpans");
    let mut total = 0;
    for rs in rss {
        let res = &rs["resource"]["attributes"];
        let res_span = json!({"attributes": res});
        assert_eq!(s_attr(&res_span, "service.name"), Some("kalango-agente"));
        assert_eq!(attr(res, "kalango.schema_version").unwrap()["intValue"], "1", "schema_version is required, int, in the resource");
        let mode = s_attr(&res_span, "kalango.privacy.mode").expect("privacy mode in the resource");
        assert!(matches!(mode, "metadata" | "content"), "the hook never sends `none` and never an unknown mode: {mode}");
        for ss in rs["scopeSpans"].as_array().unwrap() {
            for sp in ss["spans"].as_array().unwrap() {
                total += 1;
                validate_span(sp, mode);
            }
        }
    }
    assert!(total <= 2000);
}

fn validate_span(sp: &Value, mode: &str) {
    assert!(is_hex(sp["traceId"].as_str().unwrap(), 32), "traceId: 16 bytes in hex, not zero");
    assert!(is_hex(sp["spanId"].as_str().unwrap(), 16), "spanId: 8 bytes in hex, not zero");
    if let Some(p) = sp["parentSpanId"].as_str() {
        assert!(is_hex(p, 16));
    }
    assert!(sp["startTimeUnixNano"].is_string() && sp["endTimeUnixNano"].is_string(), "times are strings");
    let (a, b) = (sp["startTimeUnixNano"].as_str().unwrap().parse::<u64>().unwrap(), sp["endTimeUnixNano"].as_str().unwrap().parse::<u64>().unwrap());
    assert!(b >= a);
    let attrs = sp["attributes"].as_array().unwrap();
    assert!(attrs.len() <= 128);
    for a in attrs {
        let k = a["key"].as_str().unwrap();
        assert!(
            ["gen_ai.", "kalango.", "service.", "error.", "http.", "server.", "exception."].iter().any(|p| k.starts_with(p)),
            "attribute outside the allowed families: {k}"
        );
        if let Some(s) = a["value"]["stringValue"].as_str() {
            assert!(s.len() <= 64 * 1024 + 64, "strings are cut at 64 KiB: {k}");
        }
    }
    let events = sp["events"].as_array().cloned().unwrap_or_default();
    assert!(events.len() <= 64);
    if mode != "content" {
        assert!(events.is_empty(), "no events outside content mode");
    }
    for e in &events {
        assert!(e["name"].is_string() && e["timeUnixNano"].is_string());
        for a in e["attributes"].as_array().unwrap() {
            let k = a["key"].as_str().unwrap();
            // (the event attributes of the contract: content, role, index, finish_reason, message, body, provenance…)
            assert!(["content", "role", "index", "finish_reason", "message", "body"].contains(&k) || k.starts_with("kalango."), "{k}");
        }
    }
    let name = sp["name"].as_str().unwrap();
    let op = s_attr(sp, "gen_ai.operation.name");
    if name == "kalango.turn" {
        assert!(sp["parentSpanId"].is_null(), "the root has no parent");
        assert_eq!(sp["kind"], 1);
        for k in ["kalango.session.id", "kalango.agent.id", "kalango.privacy.mode"] {
            assert!(s_attr(sp, k).is_some(), "root: {k}");
        }
        assert_eq!(s_attr(sp, "kalango.privacy.mode"), Some(mode));
        assert!(attr(&sp["attributes"], "kalango.turn.index").is_some());
        if mode == "content" {
            assert!(events.iter().any(|e| e["name"] == "gen_ai.user.message"));
        }
    } else if let Some(model) = name.strip_prefix("chat ") {
        assert_eq!(op, Some("chat"));
        assert_eq!(sp["kind"], 3);
        assert!(sp["parentSpanId"].is_string());
        assert_eq!(s_attr(sp, "gen_ai.request.model"), Some(model));
        assert!(s_attr(sp, "gen_ai.provider.name").is_some());
        for k in ["gen_ai.usage.input_tokens", "gen_ai.usage.output_tokens"] {
            assert!(attr(&sp["attributes"], k).unwrap()["intValue"].is_string(), "{k}");
        }
        assert!(list_attr(sp, "gen_ai.response.finish_reasons").is_some());
        for e in &events {
            assert!(e["name"].as_str().unwrap().starts_with("gen_ai."), "{e}");
        }
    } else if let Some(tool) = name.strip_prefix("execute_tool ") {
        assert_eq!(op, Some("execute_tool"));
        assert_eq!(s_attr(sp, "gen_ai.tool.name"), Some(tool));
        assert_eq!(s_attr(sp, "kalango.tool.name"), Some(tool));
        // (the agent's own levels, as they are: the receiver derives what it needs from them and from read_only)
        assert!(matches!(s_attr(sp, "kalango.tool.level"), Some("observar" | "usuario" | "sistema" | "total")));
        assert!(attr(&sp["attributes"], "kalango.tool.read_only").unwrap()["boolValue"].is_boolean());
        let appr = s_attr(sp, "kalango.approval").unwrap();
        assert!(matches!(appr, "none" | "asked" | "approved" | "denied"));
        assert_eq!(attr(&sp["attributes"], "kalango.approval.latency_ms").is_some(), appr != "none", "latency when the person was asked");
        // (never a bare `external`: the kind is always said)
        assert!(matches!(
            s_attr(sp, "kalango.provenance"),
            Some("tool_local" | "external:file" | "external:web" | "external:mail" | "external:tool" | "external:paste")
        ));
    }
    if let Some(t) = list_attr(sp, "kalango.taint") {
        assert!(!t.is_empty());
        assert!(
            t.iter().all(|x| matches!(x.as_str(), "external:mail" | "external:web" | "external:file" | "external:tool" | "external:paste")),
            "{t:?}"
        );
    }
    assert!(sp["status"]["code"] == 1 || sp["status"]["code"] == 2);
}

// ---------------------------------------------------------------- helpers

impl Attr {
    fn into_str(self) -> String {
        match self {
            Attr::Str(s) => s,
            other => panic!("not a string: {other:?}"),
        }
    }
}

fn tool(name: &str, level: &str, read_only: bool) -> Tool {
    Tool { name: name.into(), description: String::new(), schema: json!({}), level: level.into(), read_only, destructive: false }
}

fn tools() -> Vec<Tool> {
    vec![
        tool("arquivos.listar", "observar", true),
        tool("web.ler", "observar", true),
        tool("arquivos.escrever", "usuario", false),
        tool("sistema.executar", "sistema", false),
    ]
}

fn fast() -> Tuning {
    Tuning { retry_base: Duration::from_millis(40), retry_max: Duration::from_millis(80), linger: Duration::from_millis(20), ..Tuning::default() }
}

struct Setup {
    _dir: tempfile::TempDir,
    rx: Receiver,
    sink: Sink,
}

fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("traind.sock");
    let rx = Receiver::start(&path, Mode::Ok);
    let sink = Sink::start(&path, fast());
    Setup { _dir: dir, rx, sink }
}

fn begin(s: &Setup, cfg: Config, request: &str) -> Turn {
    Turn::begin(
        &s.sink,
        &cfg,
        Begin {
            session: "conv-1",
            agent: "agente",
            index: 0,
            request,
            tools: &tools(),
            local_tools: Vec::new,
            connectors: vec!["gmail".into()],
            taint: vec![],
            secret: false,
            pasted: vec![],
        },
    )
}

fn reply(content: &str, calls: &[(&str, &str, Value)]) -> Result<Reply, String> {
    Ok(Reply {
        content: content.into(),
        tool_calls: calls
            .iter()
            .map(|(id, n, a)| ToolCall { id: id.to_string(), name: n.replace('.', "__"), arguments: a.clone(), raw_arguments: a.to_string() })
            .collect(),
        cut: false,
        tokens: 140,
        prompt_tokens: 100,
        evaluated: 100,
    })
}

fn call_tool(t: &Turn, name: &str, args: Value, ok: bool, text: &str) {
    t.observe(&Event::ToolStart { tool: name.into(), args, level: "observar".into(), description: String::new() });
    t.observe(&Event::ToolDone { tool: name.into(), ok, text: text.into(), summary: String::new() });
}

fn msgs(user: &str) -> Vec<Value> {
    vec![json!({"role": "system", "content": "You are the agent."}), json!({"role": "user", "content": user})]
}

fn by_name<'a>(spans: &'a [(Value, String)], prefix: &str) -> Vec<&'a Value> {
    spans.iter().map(|(s, _)| s).filter(|s| s["name"].as_str().unwrap().starts_with(prefix)).collect()
}

/// A whole turn: one model call asking for a tool, the tool, a second call that answers.
fn full_turn(t: &Turn, user: &str, tool_name: &str, tool_text: &str) {
    let c = t.clock();
    t.chat("lfm3-test", &msgs(user), c, &reply("", &[("call-1", tool_name, json!({"caminho": "~/Documentos"}))]));
    call_tool(t, tool_name, json!({"caminho": "~/Documentos"}), true, tool_text);
    let mut m = msgs(user);
    m.push(json!({"role": "assistant", "content": "", "tool_calls": [{"id": "call-1", "type": "function", "function": {"name": tool_name.replace('.', "__"), "arguments": "{}"}}]}));
    m.push(json!({"role": "tool", "tool_call_id": "call-1", "content": tool_text}));
    let c = t.clock();
    t.chat("lfm3-test", &m, c, &reply("Done.", &[]));
    t.observe(&Event::Answer("Done.".into()));
    t.end(&Ok(()));
}

// ---------------------------------------------------------------- the switches

#[test]
fn settings_default_to_recording_everything() {
    assert_eq!(Config::from_settings(&Value::Null), Config { record: true, content: true });
    assert_eq!(Config::from_settings(&json!({"idioma_respostas": "pt"})), Config::default());
    assert_eq!(Config::from_settings(&json!({"rastros": {"gravar": false}})), Config { record: false, content: true });
    assert_eq!(Config::from_settings(&json!({"rastros": {"gravar": true, "conteudo": "so_metadados"}})), Config { record: true, content: false });
    assert_eq!(Config::from_settings(&json!({"rastros": {"conteudo": "completo"}})), Config::default());
    // what is not understood is the safer choice
    assert!(!Config::from_settings(&json!({"rastros": {"conteudo": "tudo!"}})).content);
}

#[test]
fn recording_off_sends_nothing() {
    let s = setup();
    let t = begin(&s, Config { record: false, content: true }, "oi");
    assert!(!t.is_on());
    full_turn(&t, "oi", "arquivos.listar", "a.txt");
    s.sink.flush(Duration::from_secs(1));
    std::thread::sleep(Duration::from_millis(100));
    assert!(s.rx.requests().is_empty());
    assert_eq!(s.sink.counters(), Counters::default());
}

#[test]
fn metadata_only_sends_structure_and_no_text() {
    let s = setup();
    let t = begin(&s, Config { record: true, content: false }, "listar meus arquivos secretos");
    full_turn(&t, "listar meus arquivos secretos", "arquivos.listar", "conteudo-do-resultado-xyz");
    let spans = s.rx.wait_for(4);
    assert_eq!(spans.len(), 4, "root, 2 chats, 1 tool");
    let raw = s.rx.raw();
    for text in ["listar meus arquivos secretos", "conteudo-do-resultado-xyz", "You are the agent", "Done.", "~/Documentos"] {
        assert!(!raw.contains(text), "{text} must not be sent in metadata mode");
    }
    for (sp, mode) in &spans {
        assert_eq!(mode, "metadata");
        assert!(sp["events"].is_null(), "no events at all");
    }
    let root = by_name(&spans, "kalango.turn")[0];
    assert_eq!(s_attr(root, "kalango.privacy.mode"), Some("metadata"));
    assert!(attr(&root["attributes"], "kalango.content.length").is_some(), "lengths stay");
    // the structure stays: names, levels, tokens
    let tool = by_name(&spans, "execute_tool")[0];
    assert_eq!(s_attr(tool, "kalango.tool.name"), Some("arquivos.listar"));
    assert_eq!(s_attr(tool, "kalango.tool.level"), Some("observar"));
    let chat = by_name(&spans, "chat")[0];
    assert_eq!(attr(&chat["attributes"], "gen_ai.usage.output_tokens").unwrap()["intValue"], "40");
}

#[test]
fn full_content_sends_the_prompts_answers_and_tool_bodies_as_events() {
    let s = setup();
    let t = begin(&s, Config::default(), "listar meus arquivos");
    full_turn(&t, "listar meus arquivos", "arquivos.listar", "a.txt b.txt");
    let spans = s.rx.wait_for(4);
    assert_eq!(spans.len(), 4);
    let root = by_name(&spans, "kalango.turn")[0];
    assert_eq!(s_attr(root, "kalango.session.id"), Some("conv-1"));
    let names = |sp: &Value| sp["events"].as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    assert_eq!(names(root), ["gen_ai.user.message", "gen_ai.choice"]);
    let chats = by_name(&spans, "chat");
    assert_eq!(chats.len(), 2);
    let second = chats.iter().find(|c| names(c).contains(&"gen_ai.tool.message".to_string())).unwrap();
    assert_eq!(names(second), ["gen_ai.system.message", "gen_ai.user.message", "gen_ai.assistant.message", "gen_ai.tool.message", "gen_ai.choice"]);
    // the span tree: chats and tools hang from the root; the tool from the call that asked for it
    let root_id = root["spanId"].as_str().unwrap();
    assert!(chats.iter().all(|c| c["parentSpanId"] == root_id));
    let tool = by_name(&spans, "execute_tool arquivos.listar")[0];
    assert_eq!(
        tool["parentSpanId"],
        chats.iter().find(|c| list_attr(c, "gen_ai.response.finish_reasons") == Some(vec!["tool_calls".into()])).unwrap()["spanId"]
    );
    let bodies: Vec<&str> = tool["events"].as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap()).collect();
    assert_eq!(bodies, ["kalango.tool.args", "kalango.tool.result"]);
    assert_eq!(list_attr(chats[0], "gen_ai.response.finish_reasons"), Some(vec!["tool_calls".into()]));
    assert_eq!(list_attr(chats[1], "gen_ai.response.finish_reasons"), Some(vec!["stop".into()]));
    // every span of a turn shares one trace id
    let ids: BTreeSet<&str> = spans.iter().map(|(s, _)| s["traceId"].as_str().unwrap()).collect();
    assert_eq!(ids.len(), 1);
}

// ---------------------------------------------------------------- redaction in the payload

#[test]
fn a_prompt_with_a_token_leaves_no_secret_in_the_payload() {
    let s = setup();
    let token = concat!("ghp", "_abcdefghijklmnopqrstuvwxyz0123456789");
    let pem = &crate::traces_redact::test_pem("OPENSSH", "b3BlbnNzaC1rZXktdjEAAAAABG5vbmU", true);
    let user = format!("use este token {token} e a senha password=hunter2-correct-horse para entrar");
    let t = begin(&s, Config::default(), &user);
    let c = t.clock();
    let input = vec![
        json!({"role": "system", "content": "sys"}),
        json!({"role": "user", "content": user}),
        json!({"role": "tool", "tool_call_id": "x", "content": format!("env:\nHOME=/home/ana\nPATH=/bin\nSHELL=/bin/zsh\n{pem}")}),
    ];
    t.chat("lfm3-test", &input, c, &reply(concat!("ok, token sk", "-abcdefghijklmnopqrstuvwxyz123456"), &[("c1", "sistema.executar", json!({"programa": "curl", "env": {"AWS_KEY": concat!("AKIA", "ABCDEFGHIJKLMNOP")}, "api_key": concat!("k-9f8e7d", "6c5b4a"), "cabecalhos": "Authorization: Bearer abc123def456ghi789"}))]));
    call_tool(
        &t,
        "sistema.executar",
        json!({"programa": "curl", "senha": "plain-pass-777", "url": "https://u:s3cretpw@host/x"}),
        true,
        &format!("cookie: sid=zzz-cookie-999\n{pem}"),
    );
    t.observe(&Event::Answer(format!("pronto {token}")));
    t.end(&Ok(()));
    let spans = s.rx.wait_for(3);
    assert_eq!(spans.len(), 3);
    let raw = s.rx.raw();
    for secret in [
        token,
        "hunter2-correct-horse",
        "b3BlbnNzaC1rZXktdjEAAAAABG5vbmU",
        "/home/ana",
        concat!("AKIA", "ABCDEFGHIJKLMNOP"),
        concat!("sk", "-abcdefghijklmnopqrstuvwxyz123456"),
        "k-9f8e7d6c5b4a",
        "abc123def456ghi789",
        "plain-pass-777",
        "s3cretpw",
        "zzz-cookie-999",
        "PRIVATE KEY-----\nb3Blbn",
    ] {
        assert!(!raw.contains(secret), "{secret} leaked into the payload");
    }
    assert!(raw.contains("[REDACTED"), "something was redacted");
    // the trace says it redacted
    assert!(spans.iter().any(|(sp, _)| attr(&sp["attributes"], "kalango.redactions").is_some()));
}

#[test]
fn a_secret_tool_sends_no_result() {
    let s = setup();
    let t = begin(&s, Config::default(), "mostra a senha do cofre");
    call_tool(&t, "cofre.ler", json!({"nome": "banco"}), true, "SENHA-DO-BANCO-123");
    t.end(&Ok(()));
    let spans = s.rx.wait_for(2);
    let tool = by_name(&spans, "execute_tool cofre.ler")[0];
    assert_eq!(attr(&tool["attributes"], "kalango.tool.result_secret").unwrap()["boolValue"], true);
    assert!(!s.rx.raw().contains("SENHA-DO-BANCO-123"));
    let result = tool["events"].as_array().unwrap().iter().find(|e| e["name"] == "kalango.tool.result").unwrap();
    assert_eq!(attr(&result["attributes"], "kalango.tool.result_omitted").unwrap()["stringValue"], "secret");
}

#[test]
fn a_file_over_64_kib_goes_as_size_and_hash() {
    let s = setup();
    let t = begin(&s, Config::default(), "ler o arquivo grande");
    let big = format!("conteudo-{}", "x".repeat(70 * 1024));
    call_tool(&t, "arquivos.listar", json!({"caminho": "~/grande.txt"}), true, &big);
    t.end(&Ok(()));
    let spans = s.rx.wait_for(2);
    let tool = by_name(&spans, "execute_tool")[0];
    assert_eq!(attr(&tool["attributes"], "kalango.truncated").unwrap()["boolValue"], true);
    let raw = s.rx.raw();
    assert!(!raw.contains("xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"));
    assert!(raw.contains(&format!("[omitted: {} bytes, sha256 ", big.len())));
}

// ---------------------------------------------------------------- secrets in the model's context

/// The assistant's message that asks for one tool call (as the engine keeps it in the conversation).
fn assistant_call(id: &str, tool: &str, args: &str) -> Value {
    json!({"role": "assistant", "content": "", "tool_calls": [{"id": id, "type": "function", "function": {"name": tool.replace('.', "__"), "arguments": args}}]})
}

fn event_named<'a>(span: &'a Value, name: &str) -> Vec<&'a Value> {
    span["events"].as_array().map(|e| e.iter().filter(|e| e["name"] == name).collect()).unwrap_or_default()
}

fn ev_attr<'a>(event: &'a Value, key: &str) -> Option<&'a Value> {
    event["attributes"].as_array()?.iter().find(|a| a["key"] == key).map(|a| &a["value"])
}

#[test]
fn a_secret_tool_result_in_the_input_is_omitted_and_the_completion_that_follows_is_metadata_only() {
    let s = setup();
    let t = begin(&s, Config::default(), "qual a senha do banco?");
    let input = vec![
        json!({"role": "system", "content": "sys"}),
        json!({"role": "user", "content": "qual a senha do banco?"}),
        assistant_call("c1", "cofre.ler", "{\"nome\":\"banco-pessoal\"}"),
        json!({"role": "tool", "tool_call_id": "c1", "content": "SENHA-DO-BANCO-123"}),
    ];
    let c = t.clock();
    // the model repeats the secret in its reply
    t.chat("m", &input, c, &reply("A senha do banco é SENHA-DO-BANCO-123.", &[]));
    t.observe(&Event::Answer("A senha do banco é SENHA-DO-BANCO-123.".into()));
    t.end(&Ok(()));
    let spans = s.rx.wait_for(2);
    let raw = s.rx.raw();
    assert!(!raw.contains("SENHA-DO-BANCO-123"), "the secret never leaves, not even when the model says it");
    assert!(!raw.contains("banco-pessoal"), "nor the arguments of the secret tool");
    let chat = by_name(&spans, "chat")[0];
    let tool_msg = event_named(chat, "gen_ai.tool.message");
    assert_eq!(tool_msg.len(), 1);
    assert_eq!(ev_attr(tool_msg[0], "content").unwrap()["stringValue"], "[omitted: secret tool result]");
    // only a coarse size: no exact length and no hash of the value (a short secret can be guessed from either)
    assert_eq!(ev_attr(tool_msg[0], "kalango.content.length_bucket").unwrap()["stringValue"], "<64");
    assert!(ev_attr(tool_msg[0], "kalango.content.length").is_none() && ev_attr(tool_msg[0], "kalango.content.sha256").is_none());
    for hash in [crate::traces_redact::sha256_hex(b"SENHA-DO-BANCO-123"), crate::traces_redact::sha256_hex(b"{\"nome\":\"banco-pessoal\"}")] {
        assert!(!raw.contains(&hash), "no hash of the secret or of its arguments in the payload");
    }
    assert!(!raw.contains("sha256"), "the payload of a turn with a secret has no hash at all");
    assert_eq!(ev_attr(tool_msg[0], "kalango.tool.name").unwrap()["stringValue"], "cofre.ler");
    // the call that asked for it is there, without its arguments
    assert_eq!(event_named(chat, "gen_ai.assistant.message").len(), 1);
    // the completion: metadata only (tokens and finish reason stay, the text does not)
    assert!(event_named(chat, "gen_ai.choice").is_empty(), "no completion content after a secret result");
    assert_eq!(attr(&chat["attributes"], "kalango.secret_in_context").unwrap()["boolValue"], true);
    assert_eq!(attr(&chat["attributes"], "gen_ai.usage.output_tokens").unwrap()["intValue"], "40");
    // the turn says so, and its answer is not sent
    let root = by_name(&spans, "kalango.turn")[0];
    assert_eq!(attr(&root["attributes"], "kalango.secret_in_context").unwrap()["boolValue"], true);
    assert!(event_named(root, "gen_ai.choice").is_empty());
    assert_eq!(event_named(root, "gen_ai.user.message").len(), 1, "what the person asked still goes");
}

#[test]
fn a_tool_result_whose_call_id_cannot_be_resolved_is_treated_as_secret() {
    let s = setup();
    let t = begin(&s, Config::default(), "x");
    let input = vec![
        json!({"role": "system", "content": "sys"}),
        json!({"role": "user", "content": "x"}),
        json!({"role": "tool", "tool_call_id": "fantasma", "content": "CONTEUDO-DESCONHECIDO-42"}),
        json!({"role": "tool", "content": "SEM-ID-NENHUM-77"}),
    ];
    let c = t.clock();
    t.chat("m", &input, c, &reply("repito CONTEUDO-DESCONHECIDO-42", &[]));
    t.end(&Ok(()));
    let spans = s.rx.wait_for(2);
    let raw = s.rx.raw();
    assert!(!raw.contains("CONTEUDO-DESCONHECIDO-42") && !raw.contains("SEM-ID-NENHUM-77"), "fail closed");
    let chat = by_name(&spans, "chat")[0];
    let msgs = event_named(chat, "gen_ai.tool.message");
    assert_eq!(msgs.len(), 2);
    for m in msgs {
        assert_eq!(ev_attr(m, "content").unwrap()["stringValue"], "[omitted: secret tool result]");
        assert!(ev_attr(m, "kalango.tool.name").is_none(), "an unresolved call has no name to say");
        assert!(ev_attr(m, "kalango.content.length").is_none() && ev_attr(m, "kalango.content.sha256").is_none());
    }
    assert!(event_named(chat, "gen_ai.choice").is_empty());
    assert_eq!(attr(&by_name(&spans, "kalango.turn")[0]["attributes"], "kalango.secret_in_context").unwrap()["boolValue"], true);
}

#[test]
fn what_the_model_said_after_a_secret_result_is_omitted_from_later_inputs_too() {
    let s = setup();
    let t = begin(&s, Config::default(), "x");
    let input = vec![
        json!({"role": "system", "content": "sys"}),
        json!({"role": "user", "content": "qual a senha?"}),
        assistant_call("c1", "cofre.ler", "{}"),
        json!({"role": "tool", "tool_call_id": "c1", "content": "valor-secreto-xyz"}),
        json!({"role": "assistant", "content": "A senha é ECO-DA-SENHA-77"}),
        json!({"role": "user", "content": "e a conta?"}),
    ];
    let c = t.clock();
    t.chat("m", &input, c, &reply("ok", &[]));
    t.end(&Ok(()));
    let _ = s.rx.wait_for(2);
    let raw = s.rx.raw();
    assert!(!raw.contains("ECO-DA-SENHA-77") && !raw.contains("valor-secreto-xyz"));
    assert!(raw.contains("e a conta?"), "what the person said goes");
}

#[test]
fn the_arguments_of_a_secret_tool_are_omitted_in_every_place_and_the_rest_of_the_turn_sends_no_arguments() {
    let s = setup();
    let t = begin(&s, Config::default(), "pegue a senha");
    let c = t.clock();
    t.chat("m", &msgs("pegue a senha"), c, &reply("", &[("c1", "cofre.ler", json!({"nome": "conta-do-joao"}))]));
    call_tool(&t, "cofre.ler", json!({"nome": "conta-do-joao"}), true, "RESULTADO-SECRETO-5");
    // the window no longer holds the result (it was cut), but this turn already saw a secret: the next completion
    // and the next tool's arguments carry nothing
    let c = t.clock();
    t.chat("m", &msgs("de novo"), c, &reply("", &[("c2", "web.ler", json!({"url": "https://example.org/RESULTADO-SECRETO-5"}))]));
    call_tool(&t, "web.ler", json!({"url": "https://example.org/RESULTADO-SECRETO-5"}), true, "pagina");
    t.end(&Ok(()));
    let spans = s.rx.wait_for(5);
    let raw = s.rx.raw();
    assert!(!raw.contains("conta-do-joao") && !raw.contains("RESULTADO-SECRETO-5"), "{raw}");
    let secret_tool = by_name(&spans, "execute_tool cofre.ler")[0];
    let args = event_named(secret_tool, "kalango.tool.args");
    assert_eq!(ev_attr(args[0], "kalango.tool.args_omitted").unwrap()["stringValue"], "secret");
    let later = by_name(&spans, "execute_tool web.ler")[0];
    assert_eq!(ev_attr(event_named(later, "kalango.tool.args")[0], "kalango.tool.args_omitted").unwrap()["stringValue"], "secret_in_context");
    assert_eq!(ev_attr(event_named(later, "kalango.tool.result")[0], "body").unwrap()["stringValue"], "pagina", "results of ordinary tools still go");
    let chats = by_name(&spans, "chat");
    // (the call that asks for the secret tool: the choice is there, its arguments are not)
    let ask = event_named(chats[0], "gen_ai.choice");
    assert_eq!(ask.len(), 1);
    assert!(ev_attr(ask[0], "message").unwrap()["stringValue"].as_str().unwrap().contains("[omitted: secret tool arguments]"));
    assert!(event_named(chats[1], "gen_ai.choice").is_empty(), "after the secret result: metadata only");
    assert_eq!(attr(&chats[1]["attributes"], "kalango.secret_in_context").unwrap()["boolValue"], true);
}

#[test]
fn ordinary_tools_are_unaffected_by_the_secret_rules() {
    let s = setup();
    let t = begin(&s, Config::default(), "liste");
    let input = vec![
        json!({"role": "system", "content": "sys"}),
        json!({"role": "user", "content": "liste"}),
        assistant_call("c1", "arquivos.listar", "{\"caminho\":\"~/Documentos\"}"),
        json!({"role": "tool", "tool_call_id": "c1", "content": "a.txt b.txt"}),
    ];
    let c = t.clock();
    t.chat("m", &input, c, &reply("Há dois arquivos.", &[]));
    call_tool(&t, "arquivos.listar", json!({"caminho": "~/Documentos"}), true, "a.txt b.txt");
    t.observe(&Event::Answer("Há dois arquivos.".into()));
    t.end(&Ok(()));
    let spans = s.rx.wait_for(3);
    let chat = by_name(&spans, "chat")[0];
    assert_eq!(ev_attr(event_named(chat, "gen_ai.tool.message")[0], "content").unwrap()["stringValue"], "a.txt b.txt");
    assert_eq!(event_named(chat, "gen_ai.choice").len(), 1);
    assert!(attr(&chat["attributes"], "kalango.secret_in_context").is_none());
    let tool = by_name(&spans, "execute_tool")[0];
    assert!(ev_attr(event_named(tool, "kalango.tool.args")[0], "body").unwrap()["stringValue"].as_str().unwrap().contains("Documentos"));
    let root = by_name(&spans, "kalango.turn")[0];
    assert!(attr(&root["attributes"], "kalango.secret_in_context").is_none());
    assert_eq!(event_named(root, "gen_ai.choice").len(), 1);
}

#[test]
fn metadata_mode_gives_no_exact_size_for_a_secret_and_the_completion_after_it() {
    let s = setup();
    let t = begin(&s, Config { record: true, content: false }, "qual a senha?");
    let secret = "SENHA-DE-TESTE-ABCDEFGHIJ";
    let input = vec![
        json!({"role": "system", "content": "sys"}),
        json!({"role": "user", "content": "qual a senha?"}),
        assistant_call("c1", "cofre.ler", "{}"),
        json!({"role": "tool", "tool_call_id": "c1", "content": secret}),
    ];
    let c = t.clock();
    t.chat("m", &input, c, &reply(&format!("é {secret}"), &[]));
    call_tool(&t, "cofre.ler", json!({}), true, secret);
    t.observe(&Event::Answer(format!("é {secret}")));
    t.end(&Ok(()));
    let spans = s.rx.wait_for(3);
    assert_eq!(spans.len(), 3);
    let raw = s.rx.raw();
    assert!(!raw.contains(secret));
    let chat = by_name(&spans, "chat")[0];
    // once a secret is in the context nothing is an exact size: not the secret's, not the reply after it, not the sum
    // of what was sent (which leaves the secret and what the assistant said after it out)
    assert!(attr(&chat["attributes"], "kalango.content.length").is_none());
    assert_eq!(attr(&chat["attributes"], "kalango.content.length_bucket").unwrap()["stringValue"], "<64");
    assert!(attr(&chat["attributes"], "kalango.output.length").is_none());
    assert_eq!(attr(&chat["attributes"], "kalango.output.length_bucket").unwrap()["stringValue"], "<64");
    let tool = by_name(&spans, "execute_tool cofre.ler")[0];
    assert!(attr(&tool["attributes"], "kalango.content.length").is_none());
    assert_eq!(attr(&tool["attributes"], "kalango.content.length_bucket").unwrap()["stringValue"], "<64");
    let root = by_name(&spans, "kalango.turn")[0];
    assert!(attr(&root["attributes"], "kalango.output.length").is_none());
    assert_eq!(attr(&root["attributes"], "kalango.output.length_bucket").unwrap()["stringValue"], "<64");
}

#[test]
fn length_buckets_are_coarse() {
    let b = |n| length_attr("k", n, true);
    assert_eq!(b(0).1.clone().into_str(), "<64");
    assert_eq!(b(63).1.into_str(), "<64");
    assert_eq!(b(64).1.into_str(), "<1KiB");
    assert_eq!(b(1023).1.into_str(), "<1KiB");
    assert_eq!(b(1024).1.into_str(), "<64KiB");
    assert_eq!(b(65535).1.into_str(), "<64KiB");
    assert_eq!(b(65536).1.into_str(), ">=64KiB");
    assert_eq!(b(5).0, "k.length_bucket");
    assert_eq!(length_attr("k", 5, false).0, "k.length");
}

#[test]
fn the_arguments_of_a_tool_call_are_redacted_by_key_wherever_the_call_appears() {
    let s = setup();
    let t = begin(&s, Config::default(), "rode");
    let args = "{\"programa\":\"curl\",\"password\":\"pw-valor-um-abc\",\"api_key\":\"chave-valor-dois\"}";
    let input = vec![
        json!({"role": "system", "content": "sys"}),
        json!({"role": "user", "content": "rode"}),
        assistant_call("c1", "sistema.executar", args),
        json!({"role": "tool", "tool_call_id": "c1", "content": "{\"token\": \"tk-valor-tres-xyz\", \"ok\": true}"}),
        assistant_call("c2", "sistema.executar", "{\"password\": \"pw-valor-quatro\", oops"),
    ];
    let c = t.clock();
    t.chat("m", &input, c, &reply("", &[("c3", "sistema.executar", serde_json::from_str(args).unwrap())]));
    call_tool(&t, "sistema.executar", serde_json::from_str(args).unwrap(), true, "feito");
    t.end(&Ok(()));
    let spans = s.rx.wait_for(3);
    let raw = s.rx.raw();
    for secret in ["pw-valor-um-abc", "chave-valor-dois", "tk-valor-tres-xyz", "pw-valor-quatro"] {
        assert!(!raw.contains(secret), "{secret} leaked: {raw}");
    }
    assert!(raw.contains("curl"), "the rest of the arguments stays");
    assert!(raw.contains("[omitted: unparsed arguments]"), "arguments that do not parse are omitted when they look secret");
    // the structure is kept: the assistant's message is JSON the receiver can read, with the keys still named
    let chat = by_name(&spans, "chat")[0];
    let first = &event_named(chat, "gen_ai.assistant.message")[0];
    let content: Value = serde_json::from_str(ev_attr(first, "content").unwrap()["stringValue"].as_str().unwrap()).unwrap();
    let call_args: Value = serde_json::from_str(content["tool_calls"][0]["function"]["arguments"].as_str().unwrap()).unwrap();
    assert_eq!(call_args["password"], "[REDACTED]");
    assert_eq!(call_args["api_key"], "[REDACTED]");
    assert_eq!(call_args["programa"], "curl");
}

#[test]
fn a_secret_in_turn_one_keeps_turn_two_metadata_only() {
    let s = setup();
    let mut conv = Conversation::new();
    // turn 1: the model reads a secret
    let t1 = Turn::begin(
        &s.sink,
        &Config::default(),
        Begin {
            session: &conv.session,
            agent: "a",
            index: conv.turns,
            request: "qual a senha?",
            tools: &tools(),
            local_tools: Vec::new,
            connectors: vec![],
            taint: conv.taint.clone(),
            secret: conv.secret,
            pasted: vec![],
        },
    );
    let input = vec![
        json!({"role": "system", "content": "sys"}),
        json!({"role": "user", "content": "qual a senha?"}),
        assistant_call("c1", "cofre.ler", "{}"),
        json!({"role": "tool", "tool_call_id": "c1", "content": "valor-secreto-turno-um"}),
    ];
    let c = t1.clock();
    t1.chat("m", &input, c, &reply("ECO-TURNO-UM-77", &[]));
    t1.end(&Ok(()));
    assert!(t1.secret_in_context());
    conv.finish_turn(&t1, &input);
    assert!(conv.secret && conv.turns == 1);
    // turn 2: the window no longer holds the tool result, but the assistant's echo is in the history
    let t2 = Turn::begin(
        &s.sink,
        &Config::default(),
        Begin {
            session: &conv.session,
            agent: "a",
            index: conv.turns,
            request: "e agora?",
            tools: &tools(),
            local_tools: Vec::new,
            connectors: vec![],
            taint: conv.taint.clone(),
            secret: conv.secret,
            pasted: vec![],
        },
    );
    let history = vec![
        json!({"role": "system", "content": "sys"}),
        json!({"role": "user", "content": "qual a senha?"}),
        json!({"role": "assistant", "content": "A senha é ECO-TURNO-UM-77"}),
        json!({"role": "user", "content": "e agora?"}),
    ];
    let c = t2.clock();
    t2.chat("m", &history, c, &reply("de novo ECO-TURNO-UM-77", &[("c9", "web.ler", json!({"url": "https://example.org/ECO-TURNO-UM-77"}))]));
    call_tool(&t2, "web.ler", json!({"url": "https://example.org/ECO-TURNO-UM-77"}), true, "pagina");
    t2.observe(&Event::Answer("ECO-TURNO-UM-77".into()));
    t2.end(&Ok(()));
    let spans = s.rx.wait_for(6);
    let raw = s.rx.raw();
    assert!(!raw.contains("ECO-TURNO-UM-77") && !raw.contains("valor-secreto-turno-um"), "{raw}");
    let roots = by_name(&spans, "kalango.turn");
    assert_eq!(roots.len(), 2);
    for r in &roots {
        assert_eq!(attr(&r["attributes"], "kalango.secret_in_context").unwrap()["boolValue"], true);
    }
    let second_chat = by_name(&spans, "chat")
        .into_iter()
        .find(|c| event_named(c, "gen_ai.assistant.message").len() == 1 && event_named(c, "gen_ai.tool.message").is_empty())
        .unwrap();
    assert!(event_named(second_chat, "gen_ai.choice").is_empty(), "turn 2: no completion content");
    assert_eq!(ev_attr(event_named(second_chat, "gen_ai.assistant.message")[0], "content").unwrap()["stringValue"], "[omitted: secret in context]");
    assert_eq!(event_named(second_chat, "gen_ai.user.message").len(), 2, "what the person said still goes");
    let web = by_name(&spans, "execute_tool web.ler")[0];
    assert_eq!(ev_attr(event_named(web, "kalango.tool.args")[0], "kalango.tool.args_omitted").unwrap()["stringValue"], "secret_in_context");
}

#[test]
fn a_restored_conversation_with_a_secret_tool_result_starts_secret() {
    let with_secret = vec![
        json!({"role": "system", "content": "sys"}),
        json!({"role": "user", "content": "qual a senha?"}),
        assistant_call("c1", "cofre.ler", "{}"),
        json!({"role": "tool", "tool_call_id": "c1", "content": "x"}),
        json!({"role": "assistant", "content": "pronto"}),
    ];
    let mut conv = Conversation::new();
    conv.restore_secret(&with_secret);
    assert!(conv.secret);
    // a result whose call is nowhere in the history: unknown origin, treated as secret
    let mut conv = Conversation::new();
    conv.restore_secret(&[json!({"role": "user", "content": "oi"}), json!({"role": "tool", "tool_call_id": "perdido", "content": "?"})]);
    assert!(conv.secret);
    // ordinary tools do not mark it
    let mut conv = Conversation::new();
    conv.restore_secret(&[
        json!({"role": "user", "content": "oi"}),
        assistant_call("c1", "arquivos.listar", "{}"),
        json!({"role": "tool", "tool_call_id": "c1", "content": "a.txt"}),
    ]);
    assert!(!conv.secret);
}

#[test]
fn a_new_conversation_starts_clean() {
    let a = Conversation::new();
    assert!(!a.secret && a.taint.is_empty() && a.turns == 0);
    let s = setup();
    let t = begin(&s, Config::default(), "x");
    call_tool(&t, "cofre.ler", json!({}), true, "segredo-qualquer");
    t.end(&Ok(()));
    let mut conv = Conversation::new();
    conv.finish_turn(&t, &[]);
    assert!(conv.secret);
    let fresh = Conversation::new();
    assert!(!fresh.secret, "a new conversation does not inherit it");
    assert_ne!(fresh.session, conv.session);
}

// ---------------------------------------------------------------- the final pass and the property tests

/// A span with one content event, for the tests that go straight to the body that would be sent.
fn span_with(content: Attr) -> SpanData {
    SpanData {
        trace_id: [1; 16],
        span_id: [2; 8],
        parent: Some([3; 8]),
        name: "chat m".into(),
        kind: 3,
        start_ns: 1,
        end_ns: 2,
        attrs: vec![("gen_ai.operation.name".into(), Attr::Str("chat".into()))],
        events: vec![EventData {
            name: "gen_ai.user.message",
            time_ns: 1,
            attrs: vec![("role".into(), Attr::Str("user".into())), ("content".into(), content)],
        }],
        error: None,
        is_root: false,
        privacy: "content",
    }
}

fn body_text(span: &SpanData, first: &Redactor, guard: &Redactor) -> (String, Tally) {
    let mut tally = Tally::default();
    let body = request_body(&[span], first, guard, &mut tally);
    (body.to_string(), tally)
}

/// A small deterministic generator (the tests need no randomness crate and must be repeatable).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// What the property tests plant: the text that holds it, and the needle that must never reach the socket.
fn planted(rng: &mut Rng, n: usize) -> (String, String) {
    let tokens = [
        concat!("ghp", "_abcdefghijklmnopqrstuvwxyz0123456789"),
        concat!("xox", "b-1234567890-abcdefghijklmnop"),
        concat!("sk", "-abcdefghijklmnopqrstuvwxyz123456"),
        concat!("AKIA", "ABCDEFGHIJKLMNOP"),
        "Zx9Qm3Lp7Vb2Nc8Rt5Yw1Ke4Hd6Gf0Js",
        concat!("hf", "_abcdefghijklmnopqrstuvwxyzABCD"),
    ];
    let plain = format!("valor-simples-{n}-zq");
    match rng.below(9) {
        k @ 0..=5 => (format!("antes {} depois", tokens[k]), tokens[k].to_string()),
        6 => (format!("{{\"password\":\"{plain}\",\"ok\":1}}"), plain),
        7 => (format!("api_key={plain} e mais texto"), plain),
        _ => (
            crate::traces_redact::test_pem("OPENSSH", &format!("QUJDREVGR0hJSktMTU5PUFFSU1RVVldYWVo{n}"), true),
            format!("QUJDREVGR0hJSktMTU5PUFFSU1RVVldYWVo{n}"),
        ),
    }
}

/// Wraps the text in 0 to 3 levels of string-encoded JSON of varied shapes (what a stringify-before-redact bug does).
fn wrapped(rng: &mut Rng, text: String) -> String {
    let mut cur = text;
    for _ in 0..rng.below(4) {
        cur = match rng.below(6) {
            0 => serde_json::to_string(&cur).unwrap(),
            1 => json!({"arguments": cur}).to_string(),
            2 => json!({"content": cur, "role": "tool"}).to_string(),
            3 => json!({"tool_calls": [{"function": {"name": "x", "arguments": cur}}]}).to_string(),
            4 => json!([{"body": cur}, 1, null]).to_string(),
            _ => format!("resultado:\n{cur}\n"),
        };
    }
    cur
}

fn plant_case(rng: &mut Rng, n: usize) -> (Attr, String) {
    let (text, needle) = planted(rng, n);
    let w = wrapped(rng, text);
    // as text, or as structured JSON holding the text (the shape the emitter uses for assistant messages)
    let attr = if rng.below(2) == 0 { Attr::Str(w) } else { Attr::Json(json!({"content": w, "tool_calls": [{"function": {"arguments": w}}]})) };
    (attr, needle)
}

#[test]
fn property_a_planted_secret_in_any_nesting_never_reaches_the_socket() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let (first, guard) = (Redactor::default(), Redactor::default());
    for n in 0..3000 {
        let (attr, needle) = plant_case(&mut rng, n);
        let (body, _) = body_text(&span_with(attr.clone()), &first, &guard);
        assert!(!body.contains(&needle), "case {n}: {needle} reached the body from {attr:?}\n{body}");
    }
}

#[test]
fn property_b_the_final_pass_alone_stops_what_a_weak_first_pass_lets_through() {
    // a first pass with no deny list and the tokens it can see: the bugs a review found are exactly this kind of miss
    let mut rng = Rng(0xD1B5_4A32_D192_ED03);
    let weak = Redactor::new(&[]);
    let guard = Redactor::default();
    let mut degraded = 0;
    for n in 0..3000 {
        let (attr, needle) = plant_case(&mut rng, n);
        let (body, tally) = body_text(&span_with(attr.clone()), &weak, &guard);
        assert!(!body.contains(&needle), "case {n}: {needle} reached the body past the final pass from {attr:?}\n{body}");
        if tally.hits > 0 {
            degraded += 1;
        }
    }
    assert!(degraded > 500, "the final pass had real work to do ({degraded} spans)");
}

#[test]
fn a_span_the_final_pass_stops_is_sent_without_its_content_and_says_so() {
    let plain = "valor-simples-final-pass";
    let weak = Redactor::new(&[]);
    let guard = Redactor::default();
    let span = span_with(Attr::Str(format!("{{\"password\":\"{plain}\"}}")));
    let mut tally = Tally::default();
    let body = request_body(&[&span], &weak, &guard, &mut tally);
    let text = body.to_string();
    assert!(!text.contains(plain));
    assert_eq!(tally.hits, 1);
    let sent = &body["resourceSpans"][0]["scopeSpans"][0]["spans"][0];
    assert!(sent["events"].is_null(), "the content is gone, the span stays (the tree keeps its shape)");
    assert!(
        attr(&sent["attributes"], "kalango.redactions.final").unwrap()["intValue"].as_str().unwrap().parse::<u64>().unwrap() >= 1,
        "it counts what it found"
    );
    assert_eq!(s_attr(sent, "gen_ai.operation.name"), Some("chat"));
    assert_eq!(sent["parentSpanId"], hex(&[3; 8]));
}

#[test]
fn ordinary_turns_do_not_trip_the_final_pass() {
    let s = setup();
    let t = begin(&s, Config::default(), "listar meus arquivos");
    full_turn(&t, "listar meus arquivos", "arquivos.listar", "a.txt b.txt relatorio-trimestral-final-2026.pdf");
    let t2 = begin(&s, Config::default(), "rode");
    call_tool(
        &t2,
        "sistema.executar",
        json!({"programa": "ls", "args": ["-la", "/home/ana/Documentos"]}),
        true,
        "total 8\ndrwxr-xr-x 2 ana ana 4096 nov 25 10:00 .\n",
    );
    t2.end(&Ok(()));
    let spans = s.rx.wait_for(6);
    assert_eq!(spans.len(), 6);
    assert_eq!(s.sink.counters().redacted_final, 0, "no false positive on a normal payload");
    assert!(spans.iter().all(|(sp, _)| attr(&sp["attributes"], "kalango.redactions.final").is_none()));
}

// ---------------------------------------------------------------- side channels

#[test]
fn the_size_of_what_the_model_said_after_a_secret_is_not_in_the_metadata() {
    let s = setup();
    let t = begin(&s, Config { record: true, content: false }, "x");
    let mk = |echo: &str| {
        vec![
            json!({"role": "system", "content": "sys"}),
            json!({"role": "user", "content": "qual a senha?"}),
            assistant_call("c1", "cofre.ler", "{}"),
            json!({"role": "tool", "tool_call_id": "c1", "content": "valor-secreto-xyz"}),
            json!({"role": "assistant", "content": echo}),
        ]
    };
    let c = t.clock();
    t.chat("m", &mk("curto"), c, &reply("ok", &[]));
    let c = t.clock();
    t.chat("m", &mk(&"a senha é valor-secreto-xyz ".repeat(40)), c, &reply("ok", &[]));
    t.end(&Ok(()));
    let spans = s.rx.wait_for(3);
    let chats = by_name(&spans, "chat");
    assert_eq!(chats.len(), 2);
    for c in &chats {
        assert!(attr(&c["attributes"], "kalango.content.length").is_none(), "no exact length once a secret is in the context");
    }
    assert_eq!(
        attr(&chats[0]["attributes"], "kalango.content.length_bucket"),
        attr(&chats[1]["attributes"], "kalango.content.length_bucket"),
        "the assistant's echo is not counted"
    );
}

#[test]
fn the_secret_of_a_conversation_is_known_even_when_the_turn_was_not_recorded() {
    let secret_history = vec![
        json!({"role": "user", "content": "qual a senha?"}),
        assistant_call("c1", "cofre.ler", "{}"),
        json!({"role": "tool", "tool_call_id": "c1", "content": "x"}),
    ];
    let mut conv = Conversation::new();
    conv.finish_turn(&Turn::disabled(), &secret_history);
    assert!(conv.secret, "recording was off for the turn: the history is still looked at");
    assert_eq!(conv.turns, 1);
    // and an ordinary history leaves it clean
    let mut conv = Conversation::new();
    conv.finish_turn(&Turn::disabled(), &[json!({"role": "user", "content": "oi"})]);
    assert!(!conv.secret);
}

// ---------------------------------------------------------------- what a tool span says (E2E-1: D3, D5, D8, D10)

#[test]
fn the_provenance_of_a_tool_span_always_names_the_kind() {
    let s = setup();
    let t = begin(&s, Config::default(), "leia");
    call_tool(&t, "arquivos.ler", json!({"caminho": "~/a.txt"}), true, "conteudo");
    call_tool(&t, "web.ler", json!({"url": "https://example.org"}), true, "pagina");
    call_tool(&t, "gmail.buscar", json!({}), true, "um e-mail");
    call_tool(&t, "arquivos.listar", json!({}), true, "a.txt");
    t.end(&Ok(()));
    let spans = s.rx.wait_for(5);
    let prov = |name: &str| s_attr(by_name(&spans, name)[0], "kalango.provenance").map(str::to_string);
    assert_eq!(prov("execute_tool arquivos.ler").as_deref(), Some("external:file"));
    assert_eq!(prov("execute_tool web.ler").as_deref(), Some("external:web"));
    assert_eq!(prov("execute_tool gmail.buscar").as_deref(), Some("external:mail"));
    assert_eq!(prov("execute_tool arquivos.listar").as_deref(), Some("tool_local"));
    assert!(spans.iter().all(|(sp, _)| s_attr(sp, "kalango.provenance") != Some("external")), "never the bare word");
}

#[test]
fn the_level_is_the_agents_own_and_read_only_comes_from_the_tool() {
    let s = setup();
    // a read-only tool with the `usuario` level, as arquivos.ler has in kalangod
    let tools = vec![
        tool("arquivos.ler", "usuario", true),
        tool("arquivos.escrever", "usuario", false),
        tool("sistema.executar", "sistema", false),
        tool("pacotes.instalar", "total", false),
    ];
    let t = Turn::begin(
        &s.sink,
        &Config::default(),
        Begin {
            session: "s",
            agent: "a",
            index: 0,
            request: "x",
            tools: &tools,
            local_tools: Vec::new,
            connectors: vec![],
            taint: vec![],
            secret: false,
            pasted: vec![],
        },
    );
    for n in ["arquivos.ler", "arquivos.escrever", "sistema.executar", "pacotes.instalar"] {
        call_tool(&t, n, json!({}), true, "ok");
    }
    t.end(&Ok(()));
    let spans = s.rx.wait_for(5);
    let level = |n: &str| {
        let sp = by_name(&spans, &format!("execute_tool {n}"))[0];
        (
            s_attr(sp, "kalango.tool.level").unwrap().to_string(),
            attr(&sp["attributes"], "kalango.tool.read_only").unwrap()["boolValue"].as_bool().unwrap(),
        )
    };
    assert_eq!(level("arquivos.ler"), ("usuario".to_string(), true), "a read is not a write");
    assert_eq!(level("arquivos.escrever"), ("usuario".to_string(), false));
    assert_eq!(level("sistema.executar"), ("sistema".to_string(), false));
    assert_eq!(level("pacotes.instalar"), ("total".to_string(), false));
}

#[test]
fn the_agents_own_tools_are_not_external_content() {
    let c = vec!["gmail".to_string()];
    for own in [
        "fluxos.planejar",
        "fluxos.adicionar_bloco",
        "fluxos.concluir",
        "agentes.listar",
        "relogio.agora",
        "sistema.info",
        "memoria.guardar",
        "calculadora.calcular",
        "agente.plano",
        "pessoa.perguntar",
        "conversa.buscar",
        "pontos.listar",
        "servicos.listar",
        "processos.listar",
        "rede.estado",
        "sessao.entrar_sem_senha",
        "ferramentas.listar",
        "modelos.listar",
    ] {
        assert_eq!(external_kind(own, &c), None, "{own} is the agent's own or the computer's");
    }
    // what brings third-party content in is marked, and so is anything not known (when in doubt, mark)
    assert_eq!(external_kind("correio.ler", &c), Some("external:mail"));
    assert_eq!(external_kind("web.buscar", &c), Some("external:web"));
    assert_eq!(external_kind("arquivos.ler", &c), Some("external:file"));
    assert_eq!(external_kind("tela.ver", &c), Some("external:tool"));
    assert_eq!(external_kind("tarefas.executar", &c), Some("external:tool"));
    assert_eq!(external_kind("sistema.executar", &c), Some("external:tool"));
    assert_eq!(external_kind("registros.ler", &c), Some("external:tool"));
    assert_eq!(external_kind("vm.executar", &c), Some("external:tool"));
    assert_eq!(external_kind("remoto.conectar", &c), Some("external:tool"));
    assert_eq!(external_kind("gmail.buscar", &c), Some("external:mail"));
    assert_eq!(external_kind("ferramenta_nova_do_futuro.fazer", &c), Some("external:tool"));
}

#[test]
fn a_turn_that_only_plans_a_flow_carries_no_taint() {
    let s = setup();
    let t = begin(&s, Config::default(), "crie um fluxo");
    call_tool(&t, "fluxos.planejar", json!({"descricao": "x"}), true, "{\"passos\":[]}");
    call_tool(&t, "relogio.agora", json!({}), true, "10:00");
    call_tool(&t, "agentes.listar", json!({}), true, "[]");
    t.end(&Ok(()));
    let spans = s.rx.wait_for(4);
    assert!(spans.iter().all(|(sp, _)| list_attr(sp, "kalango.taint").is_none()), "the agent's own plan is not third-party content");
    assert!(t.taint().is_empty());
    assert!(by_name(&spans, "execute_tool").iter().all(|sp| s_attr(sp, "kalango.provenance") == Some("tool_local")));
}

#[test]
fn a_tool_that_looks_secret_by_name_is_documented_as_a_harmless_false_positive() {
    // "entrar_sem_senha" has "senha" in it: its result is treated as a secret (nothing leaks, a little is lost)
    assert!(is_secret_tool("sessao.entrar_sem_senha"));
    assert!(is_secret_tool("gatilho_internet.girar_segredo"));
    assert!(!is_secret_tool("arquivos.ler"));
}

#[test]
fn the_tool_call_id_the_model_gave_is_on_the_tool_span() {
    let s = setup();
    let t = begin(&s, Config::default(), "liste duas vezes");
    let c = t.clock();
    t.chat(
        "m",
        &msgs("liste duas vezes"),
        c,
        &reply(
            "",
            &[
                ("call-a", "arquivos.listar", json!({"caminho": "~/um"})),
                ("call-b", "arquivos.listar", json!({"caminho": "~/dois"})),
                ("call-c", "web.ler", json!({"url": "https://example.org"})),
            ],
        ),
    );
    call_tool(&t, "arquivos.listar", json!({"caminho": "~/um"}), true, "a");
    call_tool(&t, "arquivos.listar", json!({"caminho": "~/dois"}), true, "b");
    call_tool(&t, "web.ler", json!({"url": "https://example.org"}), true, "p");
    // a call the engine made itself (no model call asked for it): no id
    call_tool(&t, "relogio.agora", json!({}), true, "10:00");
    t.end(&Ok(()));
    let spans = s.rx.wait_for(6);
    let ids: Vec<Option<String>> = spans
        .iter()
        .map(|(sp, _)| sp)
        .filter(|sp| sp["name"].as_str().unwrap().starts_with("execute_tool"))
        .map(|sp| s_attr(sp, "gen_ai.tool.call.id").map(str::to_string))
        .collect();
    assert_eq!(
        ids,
        [Some("call-a".to_string()), Some("call-b".to_string()), Some("call-c".to_string()), None],
        "the calls of one reply are matched in order, by name"
    );
}

#[test]
fn random_looking_call_ids_go_as_a_stable_alias_everywhere_and_trip_nothing() {
    // llama.cpp ids are 32 random letters and digits: they look like secrets to every detector, and are not
    let long = "6Y7r0xwCZJZDkUOxZ6bJAHJALbV1vlPi";
    let alias = trace_call_id(long);
    assert!(alias.starts_with("call_") && alias.len() == 21 && alias.bytes().skip(5).all(|b| b.is_ascii_hexdigit()), "{alias}");
    assert_eq!(alias, trace_call_id(long), "stable: the same id is the same alias in every span and event");
    assert_ne!(alias, trace_call_id("6Y7r0xwCZJZDkUOxZ6bJAHJALbV1vlPj"));
    assert_eq!(trace_call_id("call-1"), "call-1", "a short id is left as it is");

    let s = setup();
    let t = begin(&s, Config::default(), "liste");
    let input = vec![
        json!({"role": "system", "content": "sys"}),
        json!({"role": "user", "content": "liste"}),
        json!({"role": "assistant", "content": "", "tool_calls": [{"id": long, "type": "function", "function": {"name": "arquivos__listar", "arguments": "{}"}}]}),
        json!({"role": "tool", "tool_call_id": long, "content": "a.txt"}),
    ];
    let c = t.clock();
    t.chat("m", &input, c, &reply("", &[(long, "arquivos.listar", json!({}))]));
    call_tool(&t, "arquivos.listar", json!({}), true, "a.txt");
    t.end(&Ok(()));
    let spans = s.rx.wait_for(3);
    assert_eq!(s.sink.counters().redacted_final, 0, "an id is not a secret to the final pass");
    let raw = s.rx.raw();
    assert!(!raw.contains(long), "the model's own id does not go: its alias does");
    let tool = by_name(&spans, "execute_tool")[0];
    assert_eq!(s_attr(tool, "gen_ai.tool.call.id"), Some(alias.as_str()));
    let chat = by_name(&spans, "chat")[0];
    // the call in the history, the result that answers it and the call the model just made all carry the same alias
    let hist: Value =
        serde_json::from_str(ev_attr(event_named(chat, "gen_ai.assistant.message")[0], "content").unwrap()["stringValue"].as_str().unwrap()).unwrap();
    assert_eq!(hist["tool_calls"][0]["id"], alias.as_str());
    assert_eq!(ev_attr(event_named(chat, "gen_ai.tool.message")[0], "kalango.tool.call.id").unwrap()["stringValue"], alias.as_str());
    let choice: Value =
        serde_json::from_str(ev_attr(event_named(chat, "gen_ai.choice")[0], "message").unwrap()["stringValue"].as_str().unwrap()).unwrap();
    assert_eq!(choice["tool_calls"][0]["id"], alias.as_str());
}

// ---------------------------------------------------------------- what the person pasted (N1) and the labels in metadata mode (N2)

fn begin_pasted(s: &Setup, cfg: Config, request: &str, pasted: Vec<(usize, usize)>) -> Turn {
    Turn::begin(
        &s.sink,
        &cfg,
        Begin {
            session: "conv-1",
            agent: "agente",
            index: 0,
            request,
            tools: &tools(),
            local_tools: Vec::new,
            connectors: vec![],
            taint: vec![],
            secret: false,
            pasted,
        },
    )
}

#[test]
fn what_the_person_pasted_is_external_paste_on_the_root_and_the_message_and_taints_the_turn() {
    let s = setup();
    let request = "resuma isto: texto copiado de uma pagina qualquer. obrigado";
    // chars 12..49 are the pasted part
    let t = begin_pasted(&s, Config::default(), request, vec![(12, 49)]);
    let c = t.clock();
    t.chat("m", &msgs(request), c, &reply("ok", &[]));
    t.observe(&Event::Answer("ok".into()));
    t.end(&Ok(()));
    let spans = s.rx.wait_for(2);
    let root = by_name(&spans, "kalango.turn")[0];
    assert_eq!(s_attr(root, "kalango.provenance"), Some("external:paste"));
    assert_eq!(s_attr(root, "kalango.paste.ranges"), Some("[[12,49]]"));
    assert_eq!(list_attr(root, "kalango.taint"), Some(vec!["external:paste".to_string()]));
    let user = &event_named(root, "gen_ai.user.message")[0];
    assert_eq!(ev_attr(user, "kalango.provenance").unwrap()["stringValue"], "external:paste");
    assert_eq!(ev_attr(user, "kalango.paste.ranges").unwrap()["stringValue"], "[[12,49]]");
    // the calls that follow carry the taint; the person's words are all there
    assert_eq!(list_attr(by_name(&spans, "chat")[0], "kalango.taint"), Some(vec!["external:paste".to_string()]));
    assert_eq!(ev_attr(user, "content").unwrap()["stringValue"], request);
    assert_eq!(t.taint(), ["external:paste"], "the conversation keeps it for the next turns");
}

#[test]
fn what_the_person_typed_is_user_and_not_tainted() {
    let s = setup();
    let t = begin_pasted(&s, Config::default(), "oi, tudo bem?", vec![]);
    t.end(&Ok(()));
    let spans = s.rx.wait_for(1);
    let root = by_name(&spans, "kalango.turn")[0];
    assert_eq!(s_attr(root, "kalango.provenance"), None);
    assert!(list_attr(root, "kalango.taint").is_none() && attr(&root["attributes"], "kalango.paste.ranges").is_none());
    assert_eq!(ev_attr(event_named(root, "gen_ai.user.message")[0], "kalango.provenance").unwrap()["stringValue"], "user");
}

#[test]
fn a_paste_is_marked_in_metadata_mode_too_without_the_text() {
    let s = setup();
    let t = begin_pasted(&s, Config { record: true, content: false }, "cole: PAGINA-COPIADA-123", vec![(6, 24)]);
    t.end(&Ok(()));
    let spans = s.rx.wait_for(1);
    let root = by_name(&spans, "kalango.turn")[0];
    assert_eq!(s_attr(root, "kalango.provenance"), Some("external:paste"));
    assert_eq!(s_attr(root, "kalango.paste.ranges"), Some("[[6,24]]"));
    assert_eq!(list_attr(root, "kalango.taint"), Some(vec!["external:paste".to_string()]));
    assert!(root["events"].is_null() && !s.rx.raw().contains("PAGINA-COPIADA-123"));
}

#[test]
fn the_ids_of_the_tool_results_in_the_input_are_labels_in_metadata_mode_without_any_content() {
    let s = setup();
    let long = "6Y7r0xwCZJZDkUOxZ6bJAHJALbV1vlPi";
    let input = vec![
        json!({"role": "system", "content": "sys"}),
        json!({"role": "user", "content": "liste e leia"}),
        json!({"role": "assistant", "content": "", "tool_calls": [{"id": "call-a", "type": "function", "function": {"name": "arquivos__listar", "arguments": "{}"}}, {"id": long, "type": "function", "function": {"name": "arquivos__ler", "arguments": "{}"}}]}),
        json!({"role": "tool", "tool_call_id": "call-a", "content": "RESULTADO-UM-SECRETO"}),
        json!({"role": "tool", "tool_call_id": long, "content": "RESULTADO-DOIS-SECRETO"}),
    ];
    for content in [false, true] {
        let t = begin(&s, Config { record: true, content }, "liste e leia");
        let c = t.clock();
        t.chat("m", &input, c, &reply("ok", &[]));
        t.end(&Ok(()));
    }
    let spans = s.rx.wait_for(4);
    let chats = by_name(&spans, "chat");
    assert_eq!(chats.len(), 2);
    for chat in &chats {
        assert_eq!(list_attr(chat, "kalango.tool.call.ids"), Some(vec!["call-a".to_string(), trace_call_id(long)]), "in the order of the results");
    }
    let (meta, full) = (chats.iter().find(|c| c["events"].is_null()).unwrap(), chats.iter().find(|c| !c["events"].is_null()).unwrap());
    assert!(!event_named(full, "gen_ai.tool.message").is_empty());
    // no content from the metadata chat: the text of the results is only in the full one's events
    let meta_text = serde_json::to_string(meta).unwrap();
    assert!(!meta_text.contains("RESULTADO-") && !meta_text.contains(long));
}

// ---------------------------------------------------------------- taint, gates, approvals

#[test]
fn external_content_taints_what_follows_and_the_root() {
    let s = setup();
    let t = begin(&s, Config::default(), "o que diz o site?");
    // before anything external: no taint
    let c = t.clock();
    t.chat("m", &msgs("o que diz o site?"), c, &reply("", &[("c1", "web.ler", json!({"url": "https://example.org"}))]));
    call_tool(&t, "web.ler", json!({"url": "https://example.org"}), true, "texto do site");
    let c = t.clock();
    t.chat("m", &msgs("segunda"), c, &reply("resposta", &[]));
    call_tool(&t, "gmail.buscar", json!({}), true, "um e-mail");
    t.end(&Ok(()));
    let spans = s.rx.wait_for(5);
    let chats = by_name(&spans, "chat");
    assert_eq!(list_attr(chats[0], "kalango.taint"), None, "the first call saw nothing external");
    assert_eq!(list_attr(chats[1], "kalango.taint"), Some(vec!["external:web".to_string()]), "every call after it carries the taint");
    let web = by_name(&spans, "execute_tool web.ler")[0];
    assert_eq!(list_attr(web, "kalango.taint"), Some(vec!["external:web".to_string()]));
    assert_eq!(s_attr(web, "kalango.provenance"), Some("external:web"));
    let mail = by_name(&spans, "execute_tool gmail.buscar")[0];
    assert_eq!(list_attr(mail, "kalango.taint"), Some(vec!["external:mail".to_string()]));
    let root = by_name(&spans, "kalango.turn")[0];
    assert_eq!(list_attr(root, "kalango.taint"), Some(vec!["external:mail".to_string(), "external:web".to_string()]));
    // and the agent keeps it for the next turns of the conversation
    assert_eq!(t.taint(), ["external:mail", "external:web"]);
}

#[test]
fn a_turn_with_only_local_tools_has_no_taint_and_a_new_turn_inherits_the_conversations() {
    let s = setup();
    let t = begin(&s, Config::default(), "listar");
    full_turn(&t, "listar", "arquivos.escrever", "ok");
    let spans = s.rx.wait_for(4);
    assert!(spans.iter().all(|(sp, _)| list_attr(sp, "kalango.taint").is_none()));
    let t2 = Turn::begin(
        &s.sink,
        &Config::default(),
        Begin {
            session: "conv-1",
            agent: "agente",
            index: 1,
            request: "de novo",
            tools: &tools(),
            local_tools: Vec::new,
            connectors: vec![],
            taint: vec!["external:web".into()],
            secret: false,
            pasted: vec![],
        },
    );
    let c = t2.clock();
    t2.chat("m", &msgs("de novo"), c, &reply("ok", &[]));
    t2.end(&Ok(()));
    let spans = s.rx.wait_for(6);
    let second_root = spans
        .iter()
        .map(|(s, _)| s)
        .find(|s| s["name"] == "kalango.turn" && attr(&s["attributes"], "kalango.turn.index").unwrap()["intValue"] == "1")
        .unwrap();
    assert_eq!(list_attr(second_root, "kalango.taint"), Some(vec!["external:web".to_string()]));
}

#[test]
fn external_kind_names_what_each_tool_brings_in() {
    let c = vec!["gmail".to_string(), "notion".to_string()];
    assert_eq!(external_kind("web.buscar", &c), Some("external:web"));
    assert_eq!(external_kind("arquivos.ler", &c), Some("external:file"));
    assert_eq!(external_kind("arquivos.listar", &c), None, "a folder listing is not content");
    assert_eq!(external_kind("gmail.ler", &c), Some("external:mail"));
    assert_eq!(external_kind("notion.pagina", &c), Some("external:tool"));
    assert_eq!(external_kind("tela.ver", &c), Some("external:tool"));
    assert_eq!(external_kind("calculadora.calcular", &c), None);
    assert_eq!(external_kind("memoria.guardar", &c), None);
}

#[test]
fn the_gate_that_forced_a_call_is_on_that_calls_span() {
    let s = setup();
    let t = begin(&s, Config::default(), "crie um fluxo");
    t.gate("forced_planejar");
    let c = t.clock();
    t.chat("m", &msgs("crie um fluxo"), c, &reply("", &[("c1", "fluxos.planejar", json!({"descricao": "x"}))]));
    call_tool(&t, "fluxos.planejar", json!({"descricao": "x"}), true, "{\"passos\":[]}");
    let c = t.clock();
    t.chat("m", &msgs("crie um fluxo"), c, &reply("pronto", &[]));
    t.end(&Ok(()));
    let spans = s.rx.wait_for(4);
    let chats = by_name(&spans, "chat");
    assert_eq!(s_attr(chats[0], "kalango.gate"), Some("forced_planejar"));
    assert_eq!(s_attr(chats[1], "kalango.gate"), None, "only the forced call");
    let draft = by_name(&spans, "execute_tool fluxos.planejar")[0];
    assert_eq!(attr(&draft["attributes"], "kalango.flow.draft").unwrap()["boolValue"], true);
    assert_eq!(list_attr(by_name(&spans, "kalango.turn")[0], "kalango.gates"), Some(vec!["forced_planejar".to_string()]));
}

#[test]
fn approval_outcomes_none_asked_approved_denied_auto() {
    let s = setup();
    let t = begin(&s, Config::default(), "mexer em arquivos");
    // none
    call_tool(&t, "arquivos.listar", json!({}), true, "a");
    // approved (the card was up, the person said yes)
    t.observe(&Event::Ask { id: 1, tool: "arquivos.escrever".into(), description: String::new(), args: json!({}), level: "usuario".into() });
    std::thread::sleep(Duration::from_millis(15));
    t.approval_answer(true);
    call_tool(&t, "arquivos.escrever", json!({}), true, "feito");
    // denied
    t.observe(&Event::Ask { id: 2, tool: "arquivos.escrever".into(), description: String::new(), args: json!({}), level: "usuario".into() });
    t.approval_answer(false);
    call_tool(&t, "arquivos.escrever", json!({}), false, "você não permitiu");
    // asked and never answered (the turn went on to the end)
    t.observe(&Event::Ask { id: 3, tool: "sistema.executar".into(), description: String::new(), args: json!({}), level: "sistema".into() });
    call_tool(&t, "sistema.executar", json!({}), false, "expirou");
    // let through by "Aprovar sozinho"
    t.approval_auto();
    call_tool(&t, "arquivos.escrever", json!({}), true, "feito");
    t.end(&Ok(()));
    let spans = s.rx.wait_for(6);
    let tools: Vec<&Value> = spans.iter().map(|(s, _)| s).filter(|s| s["name"].as_str().unwrap().starts_with("execute_tool")).collect();
    let outcomes: Vec<&str> = tools.iter().map(|t| s_attr(t, "kalango.approval").unwrap()).collect();
    assert_eq!(outcomes, ["none", "approved", "denied", "asked", "approved"]);
    assert!(attr(&tools[1]["attributes"], "kalango.approval.latency_ms").unwrap()["intValue"].as_str().unwrap().parse::<u64>().unwrap() >= 10);
    assert_eq!(tools[2]["status"]["code"], 1, "a denial is the person's answer, not a tool failure");
    assert_eq!(tools[3]["status"]["code"], 2);
    assert_eq!(attr(&tools[4]["attributes"], "kalango.approval.auto").unwrap()["boolValue"], true);
    // levels follow the contract's names; read_only comes from the tool's contract
    assert_eq!(s_attr(tools[0], "kalango.tool.level"), Some("observar"));
    assert_eq!(s_attr(tools[3], "kalango.tool.level"), Some("sistema"));
    assert_eq!(attr(&tools[1]["attributes"], "kalango.tool.read_only").unwrap()["boolValue"], false);
    assert_eq!(s_attr(by_name(&spans, "kalango.turn")[0], "kalango.user.signal"), Some("deny"));
}

#[test]
fn a_failed_model_call_and_a_failed_turn_are_errors_without_content() {
    let s = setup();
    let t = begin(&s, Config::default(), "oi");
    let c = t.clock();
    t.chat("m", &msgs("oi"), c, &Err("llama-server: connection to /home/ana/secret-path failed".to_string()));
    t.end(&Err("llama-server: connection to /home/ana/secret-path failed".to_string()));
    let spans = s.rx.wait_for(2);
    let chat = by_name(&spans, "chat")[0];
    assert_eq!(chat["status"]["code"], 2);
    assert_eq!(s_attr(chat, "error.type"), Some("model_error"));
    assert_eq!(by_name(&spans, "kalango.turn")[0]["status"]["code"], 2);
    assert!(!s.rx.raw().contains("secret-path"), "an error message never carries the person's paths");
}

#[test]
fn a_long_conversation_keeps_the_system_prompt_and_the_latest_messages_within_the_event_limit() {
    let s = setup();
    let t = begin(&s, Config::default(), "x");
    let mut m = vec![json!({"role": "system", "content": "SYS"})];
    for n in 0..150 {
        m.push(json!({"role": if n % 2 == 0 { "user" } else { "assistant" }, "content": format!("mensagem {n}")}));
    }
    let c = t.clock();
    t.chat("m", &m, c, &reply("fim", &[]));
    t.end(&Ok(()));
    let spans = s.rx.wait_for(2);
    let chat = by_name(&spans, "chat")[0];
    let events = chat["events"].as_array().unwrap();
    assert!(events.len() <= 62);
    assert_eq!(events[0]["name"], "gen_ai.system.message");
    assert!(s.rx.raw().contains("mensagem 149") && !s.rx.raw().contains("\"mensagem 0\""));
    assert!(attr(&chat["attributes"], "kalango.input.omitted_messages").is_some());
}

// ---------------------------------------------------------------- the producer never gets in the way

#[test]
fn without_a_daemon_nothing_is_recorded_and_nothing_fails() {
    let dir = tempfile::tempdir().unwrap();
    let sink = Sink::start(&dir.path().join("nobody.sock"), fast());
    let t = Turn::begin(
        &sink,
        &Config::default(),
        Begin {
            session: "s",
            agent: "a",
            index: 0,
            request: "oi",
            tools: &tools(),
            local_tools: Vec::new,
            connectors: vec![],
            taint: vec![],
            secret: false,
            pasted: vec![],
        },
    );
    assert!(!t.is_on(), "no socket: the turn is not even built");
    full_turn(&t, "oi", "arquivos.listar", "x");
    t.gate("forced_planejar");
    t.approval_answer(true);
    assert_eq!(sink.counters(), Counters::default());
}

#[test]
fn a_stalled_daemon_costs_the_agent_less_than_a_millisecond_per_call_and_the_queue_drops_the_oldest() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("traind.sock");
    let _rx = Receiver::start(&path, Mode::Stall);
    let sink = Sink::start(&path, Tuning { capacity: 16, io_timeout: Duration::from_millis(150), ..fast() });
    // bodies bigger than the socket's buffer, so the write itself stalls
    let big = "y".repeat(60 * 1024);
    let mut times = Vec::new();
    for n in 0..300 {
        let t = Turn::begin(
            &sink,
            &Config::default(),
            Begin {
                session: "s",
                agent: "a",
                index: n,
                request: "oi",
                tools: &tools(),
                local_tools: Vec::new,
                connectors: vec![],
                taint: vec![],
                secret: false,
                pasted: vec![],
            },
        );
        let started = Instant::now();
        let c = t.clock();
        t.chat("m", &msgs(&big), c, &reply("ok", &[("c", "arquivos.listar", json!({}))]));
        t.observe(&Event::ToolStart {
            tool: "arquivos.listar".into(),
            args: json!({"caminho": "~"}),
            level: "observar".into(),
            description: String::new(),
        });
        t.observe(&Event::ToolDone { tool: "arquivos.listar".into(), ok: true, text: big.clone(), summary: String::new() });
        t.end(&Ok(()));
        times.push(started.elapsed());
    }
    times.sort();
    let per_call = |d: Duration| d.as_secs_f64() * 1000.0 / 4.0; // chat + tool start/done + end: four calls
    let mean: f64 = times.iter().map(|d| per_call(*d)).sum::<f64>() / times.len() as f64;
    let p99 = per_call(times[times.len() * 99 / 100]);
    eprintln!("traces overhead with a stalled daemon: mean {mean:.4} ms per call, p99 {p99:.4} ms (turn of 4 calls, 60 KiB bodies)");
    assert!(mean < 1.0, "mean {mean} ms per call");
    assert!(p99 < 1.0, "p99 {p99} ms per call");
    let c = sink.counters();
    assert!(c.dropped > 0, "the bounded queue dropped spans: {c:?}");
    assert_eq!(c.sent, 0);
    // and the queue never grows past its capacity
    assert!(sink.shared.q.lock().unwrap().items.len() <= 16);
}

#[test]
fn a_daemon_that_comes_back_is_reached_again_after_the_backoff() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("traind.sock");
    // a stale socket: the file is there, nobody listens
    drop(UnixListener::bind(&path).unwrap());
    let sink = Sink::start(&path, fast());
    let mk = |n: u32| {
        Turn::begin(
            &sink,
            &Config::default(),
            Begin {
                session: "s",
                agent: "a",
                index: n,
                request: "oi",
                tools: &tools(),
                local_tools: Vec::new,
                connectors: vec![],
                taint: vec![],
                secret: false,
                pasted: vec![],
            },
        )
    };
    let t = mk(0);
    assert!(t.is_on());
    t.end(&Ok(()));
    sink.flush(Duration::from_secs(1));
    assert_eq!(sink.counters().dropped, 1, "lost, counted, and no panic");
    // during the backoff the next spans are lost without trying
    let t = mk(1);
    t.end(&Ok(()));
    sink.flush(Duration::from_secs(1));
    assert_eq!(sink.counters().dropped, 2);
    // the daemon starts, the backoff passes: spans flow again
    let rx = Receiver::start(&path, Mode::Ok);
    std::thread::sleep(Duration::from_millis(150));
    let t = mk(2);
    t.end(&Ok(()));
    let spans = rx.wait_for(1);
    assert_eq!(spans.len(), 1);
    assert_eq!(sink.counters().sent, 1);
}

#[test]
fn a_daemon_that_refuses_a_batch_loses_it_without_retrying() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("traind.sock");
    let rx = Receiver::start(&path, Mode::Status(400));
    let sink = Sink::start(&path, fast());
    let t = Turn::begin(
        &sink,
        &Config::default(),
        Begin {
            session: "s",
            agent: "a",
            index: 0,
            request: "oi",
            tools: &tools(),
            local_tools: Vec::new,
            connectors: vec![],
            taint: vec![],
            secret: false,
            pasted: vec![],
        },
    );
    t.end(&Ok(()));
    sink.flush(Duration::from_secs(1));
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(rx.requests().len(), 1, "never repeated");
    assert_eq!(sink.counters().dropped, 1);
}

#[test]
fn big_batches_are_split_under_the_body_limit() {
    let s = setup();
    let t = begin(&s, Config::default(), "x");
    // 30 tool spans of ~60 KiB each: well over the 512 KiB a request should carry
    let body = "z".repeat(60 * 1024);
    for _ in 0..30 {
        call_tool(&t, "arquivos.listar", json!({}), true, &body);
    }
    t.end(&Ok(()));
    let spans = s.rx.wait_for(31);
    assert_eq!(spans.len(), 31);
    assert!(s.rx.requests().len() >= 3);
    assert!(s.rx.bodies.lock().unwrap().iter().all(|b| b.len() <= 4 * 1024 * 1024));
}

#[test]
fn ids_are_random_and_never_zero() {
    let a: [u8; 16] = random_id();
    let b: [u8; 16] = random_id();
    assert_ne!(a, b);
    assert!(a.iter().any(|x| *x != 0));
    let s1 = new_session_id();
    assert!(s1.starts_with("s-") && s1.len() == 18);
}
