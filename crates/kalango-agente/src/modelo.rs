//! The local model (decision D6): llama.cpp `llama-server` with LFM3 in GGUF. It listens on a Unix socket in the
//! person's runtime folder (0700): no web page in a browser and no other user can reach it, unlike a TCP port
//! with open CORS. The engine talks to it through the OpenAI-compatible chat API with tool calling (`--jinja`).
//! If no server is running, the engine starts one and keeps it for the next requests. Nothing leaves the computer.
//!
//! Ceiling: the model's whole context (LFM3: 256K tokens) and no cap on the answer length; the model stops when
//! it is done or when the context is full.

use kalango_i18n::{tr, trf};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const DEFAULT_MODEL: &str = "LFM3-1.7B-Q4_K_M.gguf";
/// The model's whole context (LFM3 was trained with 256K tokens).
const CONTEXT: u32 = 262_144;
/// Voice: LFM3-Audio (speech recognition for the composer's microphone), with its audio projector.
pub const AUDIO_MODEL: &str = "LFM3-Audio-2B-Q4_K_M.gguf";
pub const AUDIO_PROJECTOR: &str = "mmproj-LFM3-Audio-2B-Q8_0.gguf";
const AUDIO_CONTEXT: u32 = 32_768;
/// Vision: LFM3-VL ("Resumir esta tela"), with its image projector.
pub const VISION_MODEL: &str = "LFM3-VL-2B-Q4_K_M.gguf";
pub const VISION_PROJECTOR: &str = "mmproj-LFM3-VL-2B-Q8_0.gguf";
const VISION_CONTEXT: u32 = 32_768;

/// Which local model: text (the agent), audio (speech to text) or vision (what is in a picture). Each has its own
/// server and socket.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Text,
    Audio,
    Vision,
}

/// One assistant turn: text and/or tool calls.
#[derive(Debug, Default)]
pub struct Reply {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    /// the generation hit the token limit
    pub cut: bool,
    /// tokens of this exchange (the conversation so far plus the reply), as llama-server counts them
    pub tokens: u64,
}

#[derive(Debug, Clone)]
pub struct ToolCall {
    pub id: String,
    /// the function name as the model wrote it (API form, see `api_name`)
    pub name: String,
    pub arguments: Value,
    pub raw_arguments: String,
}

pub struct Model {
    endpoint: Endpoint,
    pub name: String,
}

/// Where the server is: the Unix socket (default) or an http URL (KALANGO_MODELO_URL, e.g. another machine of
/// the person).
#[derive(Clone)]
enum Endpoint {
    Unix(PathBuf),
    Http(String),
}

impl std::fmt::Display for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Endpoint::Unix(p) => write!(f, "{}", p.display()),
            Endpoint::Http(u) => write!(f, "{u}"),
        }
    }
}

fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|p| p.is_absolute())
        .unwrap_or_else(|| PathBuf::from(format!("/run/user/{}", unsafe { getuid() })))
        .join("kalango")
}

extern "C" {
    fn getuid() -> u32;
    fn setsid() -> i32;
}

pub fn socket_path() -> PathBuf {
    std::env::var_os("KALANGO_MODELO_SOCKET").map(PathBuf::from).unwrap_or_else(|| runtime_dir().join("modelo.sock"))
}

fn audio_socket_path() -> PathBuf {
    std::env::var_os("KALANGO_MODELO_AUDIO_SOCKET").map(PathBuf::from).unwrap_or_else(|| runtime_dir().join("modelo-audio.sock"))
}

fn vision_socket_path() -> PathBuf {
    std::env::var_os("KALANGO_MODELO_VISAO_SOCKET").map(PathBuf::from).unwrap_or_else(|| runtime_dir().join("modelo-visao.sock"))
}

/// A model file in KALANGO_MODELOS, the person's models folder or the system one.
fn find_model(file: &str) -> Option<PathBuf> {
    std::env::var_os("KALANGO_MODELOS").map(|d| PathBuf::from(d).join(file)).into_iter()
        .chain([data_dir().join("modelos").join(file), PathBuf::from("/usr/share/kalango/modelos").join(file)])
        .find(|p| p.is_file())
}

/// Minimal HTTP/1.1 over a Unix socket (one request per connection).
fn unix_request(path: &Path, method: &str, target: &str, body: &str, timeout: Duration) -> Result<(u16, String), String> {
    use std::io::{Read, Write};
    let mut s = std::os::unix::net::UnixStream::connect(path).map_err(|e| e.to_string())?;
    s.set_read_timeout(Some(timeout)).ok();
    write!(s, "{method} {target} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
        .map_err(|e| e.to_string())?;
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).map_err(|e| e.to_string())?;
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").ok_or("invalid HTTP response")?;
    let head = String::from_utf8_lossy(&raw[..split]).to_string();
    let mut data = raw[split + 4..].to_vec();
    let status: u16 = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).ok_or("invalid HTTP response")?;
    if head.to_ascii_lowercase().contains("transfer-encoding: chunked") {
        let mut out = Vec::new();
        let mut i = 0;
        while let Some(k) = data[i..].windows(2).position(|w| w == b"\r\n") {
            let size = usize::from_str_radix(String::from_utf8_lossy(&data[i..i + k]).trim(), 16).unwrap_or(0);
            if size == 0 {
                break;
            }
            let start = i + k + 2;
            out.extend_from_slice(&data[start..(start + size).min(data.len())]);
            i = start + size + 2;
            if i >= data.len() {
                break;
            }
        }
        data = out;
    }
    Ok((status, String::from_utf8_lossy(&data).to_string()))
}

fn data_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|p| p.is_absolute())
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share"))
        .join("kalango")
}

/// Where the model file is: KALANGO_MODELO, the person's models folder, or the system one.
pub fn model_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("KALANGO_MODELO") {
        return Some(p.into());
    }
    // the model the person chose as the default agent (Settings › System › Default apps), when it is still there
    let chosen = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .and_then(|d| std::fs::read(d.join("kalango/configuracoes.json")).ok())
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| v["modelo_padrao"].as_str().map(PathBuf::from))
        .filter(|p| p.is_file());
    chosen.or_else(|| find_model(DEFAULT_MODEL))
}

/// The llama-server binary: KALANGO_LLAMA_SERVER, the system one, PATH, or the development build.
fn server_binary() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("KALANGO_LLAMA_SERVER") {
        return Some(p.into());
    }
    let dev = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../poc/.deps/src/llama.cpp/build/bin/llama-server"));
    let mut candidates = vec![PathBuf::from("/usr/lib/kalango/llama-server")];
    candidates.extend(std::env::var_os("PATH").iter().flat_map(std::env::split_paths).map(|d| d.join("llama-server")));
    candidates.push(dev);
    candidates.into_iter().find(|p| p.is_file())
}

fn post(endpoint: &Endpoint, target: &str, body: &Value, timeout: Duration) -> Result<Value, String> {
    let (status, text) = match endpoint {
        Endpoint::Unix(p) => unix_request(p, "POST", target, &body.to_string(), timeout)?,
        Endpoint::Http(u) => match ureq::post(&format!("{u}{target}")).timeout(timeout).send_json(body.clone()) {
            Ok(r) => (r.status(), r.into_string().map_err(|e| e.to_string())?),
            Err(ureq::Error::Status(c, r)) => (c, r.into_string().unwrap_or_default()),
            Err(e) => return Err(e.to_string()),
        },
    };
    if status != 200 {
        return Err(format!("{status}: {text}"));
    }
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

fn healthy(endpoint: &Endpoint) -> bool {
    match endpoint {
        Endpoint::Unix(p) => unix_request(p, "GET", "/health", "", Duration::from_secs(2)).map(|(c, _)| c == 200).unwrap_or(false),
        Endpoint::Http(u) => ureq::get(&format!("{u}/health")).timeout(Duration::from_secs(2)).call().map(|r| r.status() == 200).unwrap_or(false),
    }
}

impl Model {
    pub fn location(&self) -> String {
        self.endpoint.to_string()
    }

    /// Uses the running server (the socket, or KALANGO_MODELO_URL) or starts one.
    pub fn ensure(on_status: &dyn Fn(&str)) -> Result<Model, String> {
        Self::ensure_kind(Kind::Text, on_status)
    }

    /// The audio model (speech recognition), started on demand like the text one.
    pub fn ensure_audio(on_status: &dyn Fn(&str)) -> Result<Model, String> {
        Self::ensure_kind(Kind::Audio, on_status)
    }

    /// The vision model, started on demand like the others.
    pub fn ensure_vision(on_status: &dyn Fn(&str)) -> Result<Model, String> {
        Self::ensure_kind(Kind::Vision, on_status)
    }

    fn ensure_kind(kind: Kind, on_status: &dyn Fn(&str)) -> Result<Model, String> {
        let file = match kind {
            Kind::Text => model_path(),
            Kind::Audio => find_model(AUDIO_MODEL),
            Kind::Vision => find_model(VISION_MODEL),
        };
        let name = file.as_ref().and_then(|p| p.file_stem().map(|s| s.to_string_lossy().to_string())).unwrap_or_else(|| "LFM3".into());
        if kind == Kind::Text {
            if let Ok(url) = std::env::var("KALANGO_MODELO_URL") {
                let endpoint = Endpoint::Http(url.clone());
                return if healthy(&endpoint) { Ok(Model { endpoint, name }) } else { Err(trf("the model at {url} is not answering", &[("url", url)])) };
            }
        }
        let sock = match kind {
            Kind::Text => socket_path(),
            Kind::Audio => audio_socket_path(),
            Kind::Vision => vision_socket_path(),
        };
        let endpoint = Endpoint::Unix(sock.clone());
        if healthy(&endpoint) {
            usado(&sock);
            return Ok(Model { endpoint, name });
        }
        let (model, projector) = match kind {
            Kind::Text => (file.ok_or_else(|| tr("no local model installed (LFM3-1.7B-Q4_K_M.gguf in ~/.local/share/kalango/modelos)").to_string())?, None),
            Kind::Audio => (
                file.ok_or_else(|| trf("the voice model is not installed ({file} in ~/.local/share/kalango/modelos)", &[("file", AUDIO_MODEL.into())]))?,
                Some(find_model(AUDIO_PROJECTOR).ok_or_else(|| trf("the voice model is not installed ({file} in ~/.local/share/kalango/modelos)", &[("file", AUDIO_PROJECTOR.into())]))?),
            ),
            Kind::Vision => (
                file.ok_or_else(|| trf("the vision model is not installed ({file} in ~/.local/share/kalango/modelos)", &[("file", VISION_MODEL.into())]))?,
                Some(find_model(VISION_PROJECTOR).ok_or_else(|| trf("the vision model is not installed ({file} in ~/.local/share/kalango/modelos)", &[("file", VISION_PROJECTOR.into())]))?),
            ),
        };
        let bin = server_binary().ok_or_else(|| tr("llama-server not found").to_string())?;
        on_status(match kind {
            Kind::Text => tr("starting the local model…"),
            Kind::Audio => tr("starting the voice model…"),
            Kind::Vision => tr("starting the vision model…"),
        });
        let (log_name, ctx) = match kind {
            Kind::Text => ("modelo.log", CONTEXT),
            Kind::Audio => ("modelo-audio.log", AUDIO_CONTEXT),
            Kind::Vision => ("modelo-visao.log", VISION_CONTEXT),
        };
        let mut child = start_server(&bin, &model, projector.as_deref(), &sock, ctx, log_name)?;
        if kind != Kind::Text {
            let _ = std::fs::write(sock.with_extension("pid"), child.id().to_string());
            // (a mark left by a server of an earlier run would make this new one look idle: the next engine run
            // ended it 0.8 s into its first task — 19/11/2026)
            usado(&sock);
        }
        let t0 = Instant::now();
        while t0.elapsed() < Duration::from_secs(120) {
            if healthy(&endpoint) {
                return Ok(Model { endpoint, name });
            }
            if let Ok(Some(_)) = child.try_wait() {
                let log = std::fs::read_to_string(data_dir().join(log_name)).unwrap_or_default();
                let why = log.lines().rfind(|l| l.contains(" E ")).unwrap_or("").trim().to_string();
                return Err(trf("the local model stopped while starting: {reason}", &[("reason", why)]));
            }
            std::thread::sleep(Duration::from_millis(300));
        }
        Err(tr("the local model did not start within 120 s").to_string())
    }

    /// Speech to text with the audio model: a WAV recording, the language the person speaks.
    pub fn transcribe(&self, wav: &[u8], language: &str) -> Result<String, String> {
        use base64::Engine;
        let data = base64::engine::general_purpose::STANDARD.encode(wav);
        let body = json!({
            "messages": [
                // the model card's recognition prompt
                {"role": "system", "content": format!("Perform ASR in {language}.")},
                {"role": "user", "content": [{"type": "input_audio", "input_audio": {"data": data, "format": "wav"}}]},
            ],
            "temperature": 0,
        });
        self.mark_used();
        let v = post(&self.endpoint, "/v1/chat/completions", &body, Duration::from_secs(300))?;
        registrar_uso(&self.name, &v);
        Ok(v["choices"][0]["message"]["content"].as_str().unwrap_or("").trim().to_string())
    }

    /// (a voice or vision server in use is not idle: marked before each request, which may take minutes)
    fn mark_used(&self) {
        if let Endpoint::Unix(sock) = &self.endpoint {
            usado(sock);
        }
    }

    /// What is in a picture (PNG or JPEG), answering `question` in the person's language. The answer is for the
    /// person to read: it never drives a tool.
    pub fn describe_image(&self, image: &[u8], mime: &str, question: &str) -> Result<String, String> {
        use base64::Engine;
        let data = base64::engine::general_purpose::STANDARD.encode(image);
        let body = json!({
            "messages": [{"role": "user", "content": [
                {"type": "image_url", "image_url": {"url": format!("data:{mime};base64,{data}")}},
                {"type": "text", "text": question},
            ]}],
            "temperature": 0.2,
        });
        self.mark_used();
        let v = post(&self.endpoint, "/v1/chat/completions", &body, Duration::from_secs(300))?;
        registrar_uso(&self.name, &v);
        Ok(v["choices"][0]["message"]["content"].as_str().unwrap_or("").trim().to_string())
    }

    /// One completion constrained to a JSON schema (llama.cpp grammar), parsed.
    pub fn complete_json(&self, prompt: &str, schema: &Value) -> Result<Value, String> {
        let body = json!({
            "messages": [{"role": "user", "content": prompt}],
            "temperature": 0.2,
            "response_format": {"type": "json_schema", "json_schema": {"name": "resposta", "schema": schema}},
            // a short constrained answer: no reasoning (10× faster)
            "chat_template_kwargs": {"enable_thinking": false},
        });
        let v = post(&self.endpoint, "/v1/chat/completions", &body, Duration::from_secs(120))?;
        registrar_uso(&self.name, &v);
        let text = v["choices"][0]["message"]["content"].as_str().unwrap_or("");
        serde_json::from_str(text).map_err(|e| format!("{e}: {text}"))
    }

    /// One chat completion with the tools offered.
    pub fn chat(&self, messages: &[Value], tools: &[Value]) -> Result<Reply, String> {
        let mut body = json!({
            "messages": messages,
            // LFM3 recommended sampling for agentic use (model card)
            "temperature": 0.3, "top_p": 0.95, "top_k": 50,
            // no max_tokens: the model stops when it is done or when the context is full
        });
        if !tools.is_empty() {
            body["tools"] = json!(tools);
        }
        // long answers over a big context take time: up to 30 min per turn
        let v = post(&self.endpoint, "/v1/chat/completions", &body, Duration::from_secs(1800))?;
        registrar_uso(&self.name, &v);
        let msg = &v["choices"][0]["message"];
        let tool_calls = msg["tool_calls"].as_array().cloned().unwrap_or_default().into_iter().enumerate().map(|(i, c)| {
            let raw = c["function"]["arguments"].as_str().map(str::to_string).unwrap_or_else(|| c["function"]["arguments"].to_string());
            ToolCall {
                id: c["id"].as_str().map(str::to_string).unwrap_or_else(|| format!("call_{i}")),
                name: c["function"]["name"].as_str().unwrap_or("").to_string(),
                arguments: serde_json::from_str(&raw).unwrap_or(json!({})),
                raw_arguments: raw,
            }
        }).collect();
        let cut = v["choices"][0]["finish_reason"] == "length";
        let tokens = v["usage"]["total_tokens"].as_u64().unwrap_or(0);
        Ok(Reply { content: msg["content"].as_str().unwrap_or("").trim().to_string(), tool_calls, cut, tokens })
    }
}

/// The voice and vision models are used now and then; their servers end after this long without use (each engine
/// run checks), so their memory goes back to the GPU. The text model, used all the time, stays.
const OCIOSO: Duration = Duration::from_secs(10 * 60);

/// Marks a server as just used (next to its socket).
fn usado(sock: &Path) {
    let marca = sock.with_extension("usado");
    let _ = std::fs::write(marca, b"");
}

/// A server last used (or started) longer than `limit` ago.
fn idle(sock: &Path, limit: Duration) -> bool {
    let ultimo = std::fs::metadata(sock.with_extension("usado")).or_else(|_| std::fs::metadata(sock.with_extension("pid"))).and_then(|m| m.modified()).ok();
    ultimo.and_then(|t| t.elapsed().ok()).is_some_and(|e| e > limit)
}

/// Ends the voice and vision servers that have been idle for `OCIOSO` (only those this engine started: their pid
/// file, and the process must be llama-server with that socket).
pub fn encerrar_ociosos() {
    extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    for sock in [audio_socket_path(), vision_socket_path()] {
        let pidf = sock.with_extension("pid");
        let Some(pid) = std::fs::read_to_string(&pidf).ok().and_then(|s| s.trim().parse::<i32>().ok()) else { continue };
        let cmd = std::fs::read(format!("/proc/{pid}/cmdline")).map(|b| String::from_utf8_lossy(&b).replace('\0', " ")).unwrap_or_default();
        if !cmd.contains("llama-server") || !cmd.contains(&sock.display().to_string()) {
            let _ = std::fs::remove_file(&pidf);
            continue;
        }
        if idle(&sock, OCIOSO) {
            unsafe { kill(pid, 15) };
            let _ = std::fs::remove_file(&pidf);
            let _ = std::fs::remove_file(sock.with_extension("usado"));
        }
    }
}

/// The token use, one JSON line per reply of the model ({"t", "tipo", "tokens", "tok_s", "modelo"}): the desktop's
/// "Tokens hoje" and "Modelo local" read it. A short line appended at once is never mixed with another engine's.
pub fn uso_path() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).filter(|p| p.is_absolute())
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/state"))
        .join("kalango/uso-tokens.jsonl")
}

/// Writes one reply's use (what was read and written, and how fast) to the log.
fn registrar_uso(modelo: &str, v: &Value) {
    if cfg!(test) {
        return;
    }
    let u = &v["usage"];
    let tokens = u["prompt_tokens"].as_u64().unwrap_or(0) + u["completion_tokens"].as_u64().unwrap_or(0);
    if tokens == 0 {
        return;
    }
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let mut linha = json!({"t": t, "tipo": "local", "tokens": tokens, "modelo": modelo});
    if let Some(s) = v["timings"]["predicted_per_second"].as_f64() {
        linha["tok_s"] = json!((s * 10.0).round() / 10.0);
    }
    let p = uso_path();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
        let _ = f.write_all(format!("{linha}\n").as_bytes());
    }
}

/// Starts llama-server in the background, detached from this process (it serves later requests too), on the
/// Unix socket only. The whole context; --fit puts on the GPU as many layers and as much of the cache as the free
/// memory allows and leaves the rest on the CPU.
fn start_server(bin: &Path, model: &Path, projector: Option<&Path>, sock: &Path, ctx: u32, log_name: &str) -> Result<std::process::Child, String> {
    use std::os::unix::fs::PermissionsExt;
    let dir = sock.parent().ok_or("socket without folder")?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(sock);
    let log = data_dir().join(log_name);
    let _ = std::fs::create_dir_all(data_dir());
    let out = std::fs::File::create(&log).map_err(|e| e.to_string())?;
    let mut cmd = Command::new(bin);
    cmd.args(["--model"]).arg(model)
        .arg("--host").arg(sock)
        .args(["--jinja", "--ctx-size", &ctx.to_string(), "--fit", "on", "--parallel", "1"])
        .stdin(Stdio::null()).stdout(out.try_clone().map_err(|e| e.to_string())?).stderr(out);
    if let Some(p) = projector {
        cmd.arg("--mmproj").arg(p);
    }
    // its own session, so closing the terminal or the shell does not take the model down
    use std::os::unix::process::CommandExt;
    unsafe {
        cmd.pre_exec(|| {
            setsid();
            Ok(())
        });
    }
    cmd.spawn().map_err(|e| e.to_string())
}

/// Function names in the API cannot contain dots: `arquivos.ler` ↔ `arquivos__ler`.
pub fn api_name(tool: &str) -> String {
    tool.replace('.', "__")
}

pub fn tool_name(api: &str) -> String {
    api.replace("__", ".")
}

#[cfg(test)]
mod idle_tests {
    use std::time::{Duration, SystemTime};

    /// A server just started is not idle, even with an old "used" mark of an earlier server beside its socket — the
    /// mark is renewed when it starts (19/11/2026: the vision server was ended 0.8 s into its first task).
    #[test]
    fn a_new_server_is_not_idle() {
        let dir = std::env::temp_dir().join(format!("kalango-ocioso-novo-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("modelo-visao.sock");
        let old = std::fs::File::create(sock.with_extension("usado")).unwrap();
        old.set_modified(SystemTime::now() - Duration::from_secs(3600)).unwrap();
        drop(old);
        std::fs::write(sock.with_extension("pid"), "1").unwrap();
        assert!(super::idle(&sock, Duration::from_secs(600)), "the old mark alone makes it look idle");
        super::usado(&sock);
        assert!(!super::idle(&sock, Duration::from_secs(600)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A pid file that points to anything but that llama-server is dropped and the process is left alone.
    #[test]
    fn idle_cleanup_only_ends_its_own_servers() {
        let dir = std::env::temp_dir().join(format!("kalango-ocioso-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("modelo-audio.sock");
        std::env::set_var("KALANGO_MODELO_AUDIO_SOCKET", &sock);
        std::env::set_var("KALANGO_MODELO_VISAO_SOCKET", dir.join("modelo-visao.sock"));
        let mut outro = std::process::Command::new("sleep").arg("30").spawn().unwrap();
        std::fs::write(sock.with_extension("pid"), outro.id().to_string()).unwrap();
        super::encerrar_ociosos();
        assert!(outro.try_wait().unwrap().is_none(), "another process was ended");
        assert!(!sock.with_extension("pid").exists());
        outro.kill().unwrap();
        let _ = outro.wait();
        std::env::remove_var("KALANGO_MODELO_AUDIO_SOCKET");
        std::env::remove_var("KALANGO_MODELO_VISAO_SOCKET");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
