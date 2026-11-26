//! Trace capture for Kalango Train (contract: docs/INGEST.md of the kalango-train repository, schema version 1).
//!
//! The agent turns what it does into OTLP spans: one root span per conversation turn (`kalango.turn`), one child per
//! model call (`chat <model>`) and one per tool call (`execute_tool <name>`). They go as OTLP/HTTP JSON over the
//! Unix socket `$XDG_RUNTIME_DIR/kalango/traind.sock` (`KALANGO_TRAIND_SOCKET` overrides it) to the Train daemon.
//!
//! The rules this module keeps:
//! - **Never in the agent's way.** Spans go into a bounded queue (the oldest is dropped when it is full, and counted);
//!   one background thread batches and sends. A daemon that is absent, slow or answering errors loses spans, never
//!   time: no blocking, no spool on disk, no panic. With recording off, or no socket, a turn costs one settings read.
//! - **Redaction at the source** (`traces_redact`): secrets, environment, private keys, and strings over 64 KiB never
//!   leave the process as they are. The redaction runs on the sender thread, still in this process, before the bytes
//!   are written to the socket.
//! - **The person decides** (`rastros` in `~/.config/kalango/configuracoes.json`): `gravar` (default true) and
//!   `conteudo` (`"completo"` default, or `"so_metadados"`). Metadata only sends no prompt, answer, argument or
//!   result text; recording off sends nothing.
//! - **Taint**: external content (a page, a file, a connector, the screen) that entered the conversation's context
//!   is named in `kalango.taint`, so the curation can leave those turns out.

use crate::kalangod::Tool;
use crate::modelo::{tool_name, Reply};
use crate::motor::Event;
use crate::traces_redact::{Redactor, Stats};
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const SCHEMA_VERSION: i64 = 1;
/// The provenance and taint kind of what the person pasted into their request.
const PASTE: &str = "external:paste";
/// Events of content one span carries at most (the receiver keeps 64 per span, the choice and the headroom included).
const MAX_CONTENT_EVENTS: usize = 60;
/// Bytes of content one span carries at most (a request body is 4 MiB at the receiver, and a batch holds many spans).
const MAX_CONTENT_BYTES: usize = 256 * 1024;

// ---------------------------------------------------------------- the person's switches

/// What the person allowed (Settings › Privacy and security › Privacy and traces).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    pub record: bool,
    /// the text of prompts, answers, arguments and results goes too (`"completo"`), or only the structure
    pub content: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config { record: true, content: true }
    }
}

impl Config {
    /// From the settings file's object: `rastros: {"gravar": true|false, "conteudo": "completo"|"so_metadados"}`.
    /// A missing key is the default; an unknown `conteudo` is the safer one (metadata only).
    pub fn from_settings(settings: &Value) -> Config {
        let r = &settings["rastros"];
        Config {
            record: r["gravar"].as_bool().unwrap_or(true),
            content: match r["conteudo"].as_str() {
                None | Some("completo") => true,
                Some(_) => false,
            },
        }
    }

    pub fn load() -> Config {
        let file = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .map(|d| d.join("kalango/configuracoes.json"));
        let settings = file.and_then(|f| std::fs::read(f).ok()).and_then(|b| serde_json::from_slice::<Value>(&b).ok()).unwrap_or(Value::Null);
        Config::from_settings(&settings)
    }

    /// The contract's `kalango.privacy.mode`: `content` or `metadata` (`none` is never sent: recording off sends nothing).
    fn privacy_mode(&self) -> &'static str {
        if self.content {
            "content"
        } else {
            "metadata"
        }
    }
}

// ---------------------------------------------------------------- spans

#[derive(Clone, Debug)]
enum Attr {
    Str(String),
    Int(i64),
    Bool(bool),
    List(Vec<String>),
    /// JSON that goes as text, redacted
    Json(Value),
}

#[derive(Clone, Debug)]
struct EventData {
    name: &'static str,
    time_ns: u64,
    attrs: Vec<(String, Attr)>,
}

#[derive(Clone, Debug)]
struct SpanData {
    trace_id: [u8; 16],
    span_id: [u8; 8],
    parent: Option<[u8; 8]>,
    name: String,
    /// OTLP SpanKind: 1 internal, 3 client
    kind: u8,
    start_ns: u64,
    end_ns: u64,
    attrs: Vec<(String, Attr)>,
    events: Vec<EventData>,
    /// the span failed; the message is short and never has the person's content
    error: Option<String>,
    is_root: bool,
    privacy: &'static str,
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn now_ns() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0)
}

fn random_id<const N: usize>() -> [u8; N] {
    use ring::rand::SecureRandom;
    let rng = ring::rand::SystemRandom::new();
    let mut id = [0u8; N];
    loop {
        if rng.fill(&mut id).is_err() {
            // (never all zero, even if the system's randomness failed: time and a counter)
            static N_CALLS: AtomicU64 = AtomicU64::new(1);
            let v = now_ns() ^ N_CALLS.fetch_add(1, Ordering::Relaxed).wrapping_mul(0x9E37_79B9_7F4A_7C15);
            for (i, b) in id.iter_mut().enumerate() {
                *b = (v >> ((i % 8) * 8)) as u8 | 1;
            }
        }
        if id.iter().any(|b| *b != 0) {
            return id;
        }
    }
}

fn attr_json(a: &Attr, red: &Redactor, redact_text: bool, stats: &mut Stats) -> Value {
    match a {
        Attr::Str(s) if redact_text => json!({"stringValue": red.redact_str(s, stats)}),
        Attr::Str(s) => json!({"stringValue": s}),
        Attr::Int(n) => json!({"intValue": n.to_string()}),
        Attr::Bool(b) => json!({"boolValue": b}),
        Attr::List(l) => json!({"arrayValue": {"values": l.iter().map(|s| json!({"stringValue": s})).collect::<Vec<_>>()}}),
        Attr::Json(v) => json!({"stringValue": red.redact_value(v, stats).to_string()}),
    }
}

impl SpanData {
    /// The span as OTLP/HTTP JSON. Redaction happens here: event text and bodies, and the error message.
    fn to_otlp(&self, red: &Redactor) -> Value {
        let mut stats = Stats::default();
        let events: Vec<Value> = self
            .events
            .iter()
            .map(|e| {
                json!({
                    "timeUnixNano": e.time_ns.to_string(),
                    "name": e.name,
                    "attributes": e.attrs.iter().map(|(k, v)| json!({"key": k, "value": attr_json(v, red, true, &mut stats)})).collect::<Vec<_>>(),
                })
            })
            .collect();
        let mut attrs: Vec<Value> = self.attrs.iter().map(|(k, v)| json!({"key": k, "value": attr_json(v, red, false, &mut stats)})).collect();
        if stats.omitted_large > 0 {
            attrs.push(json!({"key": "kalango.truncated", "value": {"boolValue": true}}));
        }
        if stats.redacted > 0 {
            attrs.push(json!({"key": "kalango.redactions", "value": {"intValue": stats.redacted.to_string()}}));
        }
        let mut span = json!({
            "traceId": hex(&self.trace_id),
            "spanId": hex(&self.span_id),
            "name": self.name,
            "kind": self.kind,
            "startTimeUnixNano": self.start_ns.to_string(),
            "endTimeUnixNano": self.end_ns.max(self.start_ns).to_string(),
            "attributes": attrs,
        });
        if let Some(p) = &self.parent {
            span["parentSpanId"] = json!(hex(p));
        }
        if !events.is_empty() {
            span["events"] = json!(events);
        }
        span["status"] = match &self.error {
            Some(m) => json!({"code": 2, "message": m}),
            None => json!({"code": 1}),
        };
        span
    }
}

/// What the final pass did over a body.
#[derive(Debug, Default, Clone, Copy)]
struct Tally {
    /// spans in which it still found a secret (their content was taken out)
    hits: u64,
    /// spans it could not clean and left out
    dropped: u64,
}

/// The span as OTLP JSON, after the first pass (`first`: redaction as the span is written) and the final one
/// (`guard`: a separate, detection-only look at what is about to leave). The final pass decodes every string to any
/// depth, reads the keys, and reads the text with its escapes as separators; it trusts nothing the first pass parsed.
/// A span in which it finds a secret goes without its events (so without any content), with
/// `kalango.redactions.final` saying how many it found; if it is still not clean the span is left out.
fn guarded_otlp(sp: &SpanData, first: &Redactor, guard: &Redactor, tally: &mut Tally) -> Option<Value> {
    let mut v = sp.to_otlp(first);
    // (the values, each with the same rules as the first pass: the serialized JSON around a value, `"}}],"name":`,
    // must not make a kept `${{ … }}` or "Password:" look like a secret; strings in strings are decoded by scan_value)
    let hits = guard.scan_value(&v);
    if hits == 0 {
        return Some(v);
    }
    tally.hits += 1;
    if let Some(o) = v.as_object_mut() {
        o.remove("events");
        if let Some(a) = o.get_mut("attributes").and_then(Value::as_array_mut) {
            a.push(json!({"key": "kalango.redactions.final", "value": {"intValue": hits.to_string()}}));
            if !a.iter().any(|x| x["key"] == "kalango.content.omitted") {
                a.push(json!({"key": "kalango.content.omitted", "value": {"stringValue": "final_pass"}}));
            }
        }
    }
    if guard.scan_value(&v) > 0 {
        tally.dropped += 1;
        return None;
    }
    Some(v)
}

/// The request body for spans: one `resourceSpans` per privacy mode (the mode is a resource attribute).
fn request_body(spans: &[&SpanData], red: &Redactor, guard: &Redactor, tally: &mut Tally) -> Value {
    let mut by_mode: Vec<(&'static str, Vec<Value>)> = Vec::new();
    for s in spans {
        // (a bug in a detector costs this span only, never the sender thread: the text comes from outside)
        let v = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| guarded_otlp(s, red, guard, tally))) {
            Ok(Some(v)) => v,
            Ok(None) => continue,
            Err(_) => {
                tally.dropped += 1;
                continue;
            }
        };
        match by_mode.iter_mut().find(|(m, _)| *m == s.privacy) {
            Some((_, l)) => l.push(v),
            None => by_mode.push((s.privacy, vec![v])),
        }
    }
    json!({"resourceSpans": by_mode.into_iter().map(|(mode, spans)| json!({
        "resource": {"attributes": [
            {"key": "service.name", "value": {"stringValue": "kalango-agente"}},
            {"key": "service.version", "value": {"stringValue": env!("CARGO_PKG_VERSION")}},
            {"key": "kalango.schema_version", "value": {"intValue": SCHEMA_VERSION.to_string()}},
            {"key": "kalango.privacy.mode", "value": {"stringValue": mode}},
        ]},
        "scopeSpans": [{"scope": {"name": "kalango.agent", "version": env!("CARGO_PKG_VERSION")}, "spans": spans}],
    })).collect::<Vec<_>>()})
}

// ---------------------------------------------------------------- the exporter: bounded queue + one thread

/// How the sender thread behaves (the defaults follow the contract; tests shorten the waits).
#[derive(Clone, Debug)]
pub struct Tuning {
    /// spans waiting at most; the oldest is dropped beyond it
    pub capacity: usize,
    /// wait before trying a daemon that failed again, and the most it grows to (doubling)
    pub retry_base: Duration,
    pub retry_max: Duration,
    /// connection, write and answer wait
    pub io_timeout: Duration,
    /// a batch: at most this many spans / bytes of body; sent at the end of the root span or after `linger`
    pub max_batch_spans: usize,
    pub max_batch_bytes: usize,
    pub linger: Duration,
}

impl Default for Tuning {
    fn default() -> Self {
        Tuning {
            capacity: 2048,
            retry_base: Duration::from_secs(5),
            retry_max: Duration::from_secs(20),
            io_timeout: Duration::from_millis(200),
            max_batch_spans: 100,
            max_batch_bytes: 512 * 1024,
            linger: Duration::from_secs(1),
        }
    }
}

#[derive(Default)]
struct Queue {
    items: VecDeque<SpanData>,
    /// a root span is waiting (the turn ended: send now), or a flush was asked
    send_now: bool,
    inflight: bool,
    stop: bool,
}

struct Shared {
    q: Mutex<Queue>,
    cv: Condvar,
    path: PathBuf,
    tuning: Tuning,
    queued: AtomicU64,
    dropped: AtomicU64,
    sent: AtomicU64,
    /// spans in which the final pass found a secret
    redacted_final: AtomicU64,
}

/// What the capture has done (for the Train screen, which will show it; the tests assert on it).
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counters {
    pub queued: u64,
    pub sent: u64,
    pub dropped: u64,
    /// spans the final pass had to take the content out of
    pub redacted_final: u64,
}

#[derive(Clone)]
pub struct Sink {
    shared: Arc<Shared>,
    _guard: Arc<StopOnDrop>,
}

struct StopOnDrop(Arc<Shared>);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        if let Ok(mut q) = self.0.q.lock() {
            q.stop = true;
        }
        self.0.cv.notify_all();
    }
}

impl Sink {
    /// Starts the sender thread for this socket path.
    pub fn start(path: &Path, tuning: Tuning) -> Sink {
        let shared = Arc::new(Shared {
            q: Mutex::new(Queue::default()),
            cv: Condvar::new(),
            path: path.to_path_buf(),
            tuning,
            queued: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            sent: AtomicU64::new(0),
            redacted_final: AtomicU64::new(0),
        });
        let worker = shared.clone();
        // (a thread that cannot start: the capture is off, the agent unaffected)
        let _ = std::thread::Builder::new().name("kalango-traces".into()).spawn(move || worker_loop(&worker));
        Sink { _guard: Arc::new(StopOnDrop(shared.clone())), shared }
    }

    /// Whether the daemon's socket is there (a turn is not recorded when it is not: nothing to send to).
    pub fn available(&self) -> bool {
        std::fs::metadata(&self.shared.path).is_ok()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn counters(&self) -> Counters {
        Counters {
            queued: self.shared.queued.load(Ordering::Relaxed),
            sent: self.shared.sent.load(Ordering::Relaxed),
            dropped: self.shared.dropped.load(Ordering::Relaxed),
            redacted_final: self.shared.redacted_final.load(Ordering::Relaxed),
        }
    }

    fn push(&self, span: SpanData) {
        let s = &self.shared;
        let Ok(mut q) = s.q.lock() else { return };
        if q.items.len() >= s.tuning.capacity {
            q.items.pop_front();
            s.dropped.fetch_add(1, Ordering::Relaxed);
        }
        if span.is_root {
            q.send_now = true;
        }
        q.items.push_back(span);
        s.queued.fetch_add(1, Ordering::Relaxed);
        drop(q);
        s.cv.notify_one();
    }

    /// Waits until what was queued is out (or lost), at most `timeout`: for the end of a one-shot process.
    pub fn flush(&self, timeout: Duration) {
        let s = &self.shared;
        let deadline = Instant::now() + timeout;
        let Ok(mut q) = s.q.lock() else { return };
        q.send_now = true;
        s.cv.notify_all();
        while !(q.items.is_empty() && !q.inflight) {
            let now = Instant::now();
            if now >= deadline {
                return;
            }
            match s.cv.wait_timeout(q, deadline - now) {
                Ok((g, _)) => q = g,
                Err(_) => return,
            }
        }
    }
}

fn worker_loop(s: &Shared) {
    let red = Redactor::default();
    let guard = Redactor::default();
    let mut next_try = Instant::now();
    let mut failures = 0u32;
    loop {
        let batch: Vec<SpanData> = {
            let Ok(mut q) = s.q.lock() else { return };
            while q.items.is_empty() {
                if q.stop {
                    return;
                }
                q = match s.cv.wait(q) {
                    Ok(g) => g,
                    Err(_) => return,
                };
            }
            // gather for a moment, unless a turn ended or the batch is full
            let deadline = Instant::now() + s.tuning.linger;
            while !q.send_now && q.items.len() < s.tuning.max_batch_spans && !q.stop {
                let now = Instant::now();
                if now >= deadline {
                    break;
                }
                q = match s.cv.wait_timeout(q, deadline - now) {
                    Ok((g, _)) => g,
                    Err(_) => return,
                };
            }
            let n = q.items.len().min(s.tuning.max_batch_spans);
            let batch: Vec<SpanData> = q.items.drain(..n).collect();
            q.send_now = q.items.iter().any(|x| x.is_root);
            q.inflight = true;
            batch
        };
        if Instant::now() < next_try {
            // the daemon failed a moment ago: not tried again yet; these spans are lost
            s.dropped.fetch_add(batch.len() as u64, Ordering::Relaxed);
        } else {
            let (outcome, tally) = send_batch(s, &batch, &red, &guard);
            s.redacted_final.fetch_add(tally.hits, Ordering::Relaxed);
            s.dropped.fetch_add(tally.dropped, Ordering::Relaxed);
            match outcome {
                Outcome::Sent => {
                    failures = 0;
                    s.sent.fetch_add((batch.len() as u64).saturating_sub(tally.dropped), Ordering::Relaxed);
                }
                Outcome::Rejected => {
                    s.dropped.fetch_add(batch.len() as u64, Ordering::Relaxed);
                }
                Outcome::Down => {
                    s.dropped.fetch_add(batch.len() as u64, Ordering::Relaxed);
                    let wait = s.tuning.retry_base.saturating_mul(1 << failures.min(6)).min(s.tuning.retry_max);
                    failures = failures.saturating_add(1);
                    next_try = Instant::now() + wait;
                }
            }
        }
        if let Ok(mut q) = s.q.lock() {
            q.inflight = false;
        }
        s.cv.notify_all();
    }
}

enum Outcome {
    Sent,
    /// the daemon answered no (bad body, too large, unsupported): the batch is lost, the daemon is fine
    Rejected,
    /// no socket, refused, stalled, or busy: the batch is lost and the daemon is left alone for a while
    Down,
}

/// The body to write for `chunk`, or None when nothing may be sent. The last look (`scan`, the final detector over the
/// values of the whole body, the resource's included, as it will be written to the socket) runs here: after the spans were each cleaned it should never find
/// anything, and if it does, or if it fails (a panic), nothing is sent and the spans are counted as dropped.
fn final_body(chunk: &[&SpanData], red: &Redactor, guard: &Redactor, tally: &mut Tally, scan: &dyn Fn(&Value) -> usize) -> Option<String> {
    let body = request_body(chunk, red, guard, tally);
    let found = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| scan(&body))).unwrap_or(1);
    if found > 0 {
        tally.hits += 1;
        tally.dropped += chunk.len() as u64;
        return None;
    }
    Some(body.to_string())
}

fn send_batch(s: &Shared, batch: &[SpanData], red: &Redactor, guard: &Redactor) -> (Outcome, Tally) {
    // chunks of at most the body size the contract recommends
    let mut chunk: Vec<&SpanData> = Vec::new();
    let mut size = 0usize;
    let mut worst = Outcome::Sent;
    let tally = std::cell::RefCell::new(Tally::default());
    let flush = |chunk: &mut Vec<&SpanData>, worst: &mut Outcome| {
        if chunk.is_empty() {
            return;
        }
        let Some(body) = final_body(chunk, red, guard, &mut tally.borrow_mut(), &|b| guard.scan_value(b)) else {
            chunk.clear();
            return;
        };
        match post(&s.path, &body, s.tuning.io_timeout) {
            Ok(200..=299) => {}
            Ok(503) | Err(_) => *worst = Outcome::Down,
            Ok(_) => {
                if matches!(worst, Outcome::Sent) {
                    *worst = Outcome::Rejected;
                }
            }
        }
        chunk.clear();
    };
    for sp in batch {
        // (a rough size: the text of the events and attributes)
        let approx = 600
            + sp.events
                .iter()
                .flat_map(|e| e.attrs.iter())
                .map(|(_, a)| match a {
                    Attr::Str(t) => t.len(),
                    Attr::Json(v) => v.to_string().len(),
                    _ => 16,
                })
                .sum::<usize>();
        if !chunk.is_empty() && size + approx > s.tuning.max_batch_bytes {
            flush(&mut chunk, &mut worst);
            size = 0;
            if matches!(worst, Outcome::Down) {
                return (worst, tally.into_inner());
            }
        }
        chunk.push(sp);
        size += approx;
    }
    flush(&mut chunk, &mut worst);
    (worst, tally.into_inner())
}

/// Connects to the daemon's socket, giving up after `timeout`: a daemon that stopped accepting (its listen backlog
/// full) makes a plain connect wait for ever, and this runs on the sender thread.
fn connect(path: &Path, timeout: Duration) -> std::io::Result<UnixStream> {
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    let bytes = path.as_os_str().as_bytes();
    // SAFETY: an all-zero sockaddr_un is a valid value; the path is copied in below.
    let mut addr: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if bytes.len() >= addr.sun_path.len() {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "socket path too long"));
    }
    addr.sun_family = libc::AF_UNIX as libc::sa_family_t;
    for (d, s) in addr.sun_path.iter_mut().zip(bytes) {
        *d = *s as libc::c_char;
    }
    // SAFETY: socket(2) with constant arguments; the fd is owned by the UnixStream made from it right below.
    let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC, 0) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: fd is a fresh socket that nothing else owns; the stream closes it.
    let stream = unsafe { UnixStream::from_raw_fd(fd) };
    let deadline = Instant::now() + timeout;
    loop {
        // SAFETY: addr is a valid sockaddr_un and the length is its size; fd is open (owned by `stream`).
        let r = unsafe {
            libc::connect(
                stream.as_raw_fd(),
                &addr as *const libc::sockaddr_un as *const libc::sockaddr,
                std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t,
            )
        };
        if r == 0 {
            break;
        }
        let e = std::io::Error::last_os_error();
        match e.raw_os_error() {
            Some(libc::EISCONN) => break,
            // the backlog is full (a Unix socket says so at once) or the connection is on its way: wait, up to the deadline
            Some(libc::EAGAIN) | Some(libc::EINPROGRESS) | Some(libc::EALREADY) | Some(libc::EINTR) => {
                if Instant::now() >= deadline {
                    return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "the daemon did not accept the connection"));
                }
                std::thread::sleep(Duration::from_millis(5).min(deadline.saturating_duration_since(Instant::now())));
            }
            _ => return Err(e),
        }
    }
    stream.set_nonblocking(false)?;
    Ok(stream)
}

/// One HTTP/1.1 POST over the socket; the status code.
fn post(path: &Path, body: &str, timeout: Duration) -> std::io::Result<u16> {
    let mut c = connect(path, timeout)?;
    c.set_write_timeout(Some(timeout))?;
    c.set_read_timeout(Some(timeout))?;
    let head = format!(
        "POST /v1/traces HTTP/1.1\r\nHost: kalango-traind\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    c.write_all(head.as_bytes())?;
    c.write_all(body.as_bytes())?;
    let mut status = String::new();
    BufReader::new(c).read_line(&mut status)?;
    status.split_whitespace().nth(1).and_then(|c| c.parse().ok()).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "no status"))
}

/// The socket of the Train daemon: `KALANGO_TRAIND_SOCKET`, else `$XDG_RUNTIME_DIR/kalango/traind.sock` (never `/tmp`).
pub fn socket_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("KALANGO_TRAIND_SOCKET").map(PathBuf::from).filter(|p| p.is_absolute()) {
        return Some(p);
    }
    std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|p| p.is_absolute()).map(|d| d.join("kalango/traind.sock"))
}

static GLOBAL: OnceLock<Option<Sink>> = OnceLock::new();

/// The process's sink, started on first use (None without a runtime directory).
pub fn global() -> Option<&'static Sink> {
    GLOBAL.get_or_init(|| socket_path().map(|p| Sink::start(&p, Tuning::default()))).as_ref()
}

/// Called before the process ends: what is queued gets a moment to go (never more than `timeout`).
pub fn flush(timeout: Duration) {
    if let Some(s) = GLOBAL.get().and_then(Option::as_ref) {
        s.flush(timeout);
    }
}

// ---------------------------------------------------------------- the turn

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ApprovalState {
    None,
    Asked,
    Approved,
    Denied,
}

impl ApprovalState {
    fn name(self) -> &'static str {
        match self {
            ApprovalState::None => "none",
            ApprovalState::Asked => "asked",
            ApprovalState::Approved => "approved",
            ApprovalState::Denied => "denied",
        }
    }
}

#[derive(Clone, Copy)]
struct Approval {
    outcome: ApprovalState,
    auto: bool,
    asked_at: Option<Instant>,
    latency_ms: Option<u64>,
}

const NO_APPROVAL: Approval = Approval { outcome: ApprovalState::None, auto: false, asked_at: None, latency_ms: None };

struct OpenTool {
    name: String,
    args: Value,
    level: String,
    read_only: bool,
    start_ns: u64,
    start: Instant,
    approval: Approval,
    parent: Option<[u8; 8]>,
    /// the id the model gave this call (`gen_ai.tool.call.id`), when the call came from a model reply
    call_id: Option<String>,
}

struct State {
    taint: BTreeSet<String>,
    gates: Vec<String>,
    pending_gate: Option<&'static str>,
    last_chat: Option<[u8; 8]>,
    open: Vec<OpenTool>,
    pending_approval: Approval,
    answer: Option<String>,
    signal: Option<&'static str>,
    stopped: bool,
    warned: bool,
    chats: u32,
    tools: u32,
    /// a secret tool's result is (or was) in this turn's context: the model may repeat it, so nothing it says or
    /// asks for after that goes out as text
    secret_in_context: bool,
    /// the calls of the model's last reply that have not started yet: (id, tool name), in order
    pending_calls: Vec<(String, String)>,
}

struct Inner {
    sink: Sink,
    content: bool,
    privacy: &'static str,
    trace_id: [u8; 16],
    root_id: [u8; 8],
    start_ns: u64,
    start: Instant,
    session: String,
    agent: String,
    index: u32,
    request: String,
    pasted: Vec<(usize, usize)>,
    meta: HashMap<String, (String, bool)>,
    connectors: Vec<String>,
    st: Mutex<State>,
}

/// One conversation turn being recorded. A cheap handle: disabled (no daemon, recording off) it holds nothing and
/// every call on it returns at once.
#[derive(Clone, Default)]
pub struct Turn(Option<Arc<Inner>>);

/// A model call's start (None when the turn is not recorded).
pub struct Clock(Option<(u64, Instant)>);

pub struct Begin<'a> {
    pub session: &'a str,
    pub agent: &'a str,
    pub index: u32,
    pub request: &'a str,
    pub tools: &'a [Tool],
    /// the tools the engine answers inside itself (the calculator, the plan, the question, memory…): their levels and
    /// whether they only read, asked for only when the turn is recorded
    pub local_tools: fn() -> Vec<Tool>,
    /// names of the connectors that are on (their tools' results are external content)
    pub connectors: Vec<String>,
    /// what the conversation already holds from outside (earlier turns)
    pub taint: Vec<String>,
    /// a secret tool's result was in the conversation before this turn (the model sees it again in the history)
    pub secret: bool,
    /// the character ranges of `request` the person pasted (it came from somewhere else): see `request.rs`
    pub pasted: Vec<(usize, usize)>,
}

/// What the traces keep from one turn to the next of a conversation: its id, how many turns there were, what it holds
/// from outside, and whether a secret ever came into it (it stays in the history, so every later turn is marked).
/// A new conversation starts clean.
#[derive(Debug, Clone)]
pub struct Conversation {
    pub session: String,
    pub turns: u32,
    pub taint: Vec<String>,
    pub secret: bool,
}

impl Conversation {
    pub fn new() -> Conversation {
        Conversation { session: new_session_id(), turns: 0, taint: Vec::new(), secret: false }
    }

    /// A kept conversation picked up again: it has the id of its folder, and a secret in its history marks it.
    pub fn restore_secret(&mut self, messages: &[Value]) {
        self.secret |= context_has_secret(messages);
    }

    /// A kept conversation picked up again in a new process: what its history holds from outside marks it again, as
    /// the turns that brought it in did (a new process forgot it): a request with pasted text, the tools' results that
    /// came in an envelope of outside data (when in doubt, `external:tool`), and a secret in it.
    pub fn restore(&mut self, messages: &[Value]) {
        let by_call = call_names(messages);
        for m in messages {
            let content = m["content"].as_str().unwrap_or("");
            // a request with pasted text (its mark is saved with the conversation: context::PASTED_KEY)
            if m["role"] == "user" && m.get(crate::context::PASTED_KEY).and_then(Value::as_bool) == Some(true) {
                self.add_taint(PASTE);
            }
            if m["role"] == "tool" && content.contains("<<<DADOS EXTERNOS") {
                let tool = m["tool_call_id"].as_str().and_then(|id| by_call.get(id)).cloned().unwrap_or_default();
                self.add_taint(result_kind(&tool, content, &[]).unwrap_or("external:tool"));
            }
        }
        self.restore_secret(messages);
    }

    pub fn add_taint(&mut self, kind: &str) {
        if !self.taint.iter().any(|t| t == kind) {
            self.taint.push(kind.to_string());
        }
    }

    /// The turn is over: what it learned about the conversation is kept for the next one. The secret flag is kept
    /// whether or not the turn was recorded (recording may be off now and on later): the history is looked at every time.
    pub fn finish_turn(&mut self, turn: &Turn, messages: &[Value]) {
        self.turns += 1;
        if turn.is_on() {
            self.taint = turn.taint();
            self.secret |= turn.secret_in_context();
        }
        if !self.secret {
            self.secret = context_has_secret(messages);
        }
    }
}

impl Default for Conversation {
    fn default() -> Self {
        Conversation::new()
    }
}

/// An id for a conversation that has none kept on disk (random; the kept ones use their folder's name).
pub fn new_session_id() -> String {
    format!("s-{}", hex(&random_id::<8>()))
}

/// A call id as the trace says it. The ids a model server makes are 32 random letters and digits, which every secret
/// detector (the receiver's too) takes for a credential; so a long id goes as a stable alias, `call_` and 16 hex digits of
/// its SHA-256, the same in every span and event of the turn (a short id, like `call-1`, is left as it is).
pub fn trace_call_id(id: &str) -> String {
    if id.len() >= 20 {
        format!("call_{}", &crate::traces_redact::sha256_hex(id.as_bytes())[..16])
    } else {
        id.to_string()
    }
}

/// Tools whose result is a secret (the vault, keys, tokens): the trace says only that it was. By name, so it is wide on
/// purpose: `sessao.entrar_sem_senha` ("log in without a password") has "senha" in it and is treated as secret too. A
/// known false positive, harmless: nothing leaks, and the turn loses its content for training (the tool is rare).
pub fn is_secret_tool(name: &str) -> bool {
    let n = name.to_lowercase();
    ["cofre", "segredo", "senha", "credencial", "secret", "password", "token", "keyring", "chaveiro", "vault"].iter().any(|w| n.contains(w))
}

/// The tools whose results are the agent's own or the computer's: its plan and drafts, its memory and calculator, the
/// clock, the system's own state, the management of agents, flows, tools and models. Their results are not third-party
/// content. (The list is of prefixes of kalangod's tool names; what is not in it and not named in `external_kind` is
/// marked external: when in doubt, mark.)
const OWN_PREFIXES: &[&str] = &[
    "fluxos",
    "flows",
    "agentes",
    "agents",
    "agente",
    "relogio",
    "sistema",
    "memoria",
    "calculadora",
    "pessoa",
    "conversa",
    "pontos",
    "servicos",
    "processos",
    "rede",
    "firewall",
    "discos",
    "usuarios",
    "pacotes",
    "atualizacoes",
    "sysctl",
    "kernel",
    "linha_do_tempo",
    "energia",
    "gpu",
    "sessao",
    "ferramentas",
    "modelos",
    "models",
    "mcp",
    "idioma",
    "platform",
    "craft",
    "sonda",
    "gatilho_internet",
    "internet",
    "tunel",
    "grafico",
];

/// What a tool's result brings into the context from outside, as the contract names it (`external:mail|web|file|tool`),
/// or None for what is the agent's own or the computer's (see `OWN_PREFIXES`). Third-party content: the web, mail, the
/// content of files, a connector's results, what a command prints, the screen, the logs, other machines (virtual and
/// remote); anything not known is marked too.
/// What one tool's result brings in from outside (see `external_kind`). By the tool's name, except for the tools that
/// bring back what an earlier result said: `conversa.abrir` (a kept result again: the tool named in its envelope of
/// outside data; without one, unknown, so marked) and `conversa.buscar` (passages of the archive: the most external of
/// where they came from).
pub fn result_kind(name: &str, text: &str, connectors: &[String]) -> Option<&'static str> {
    match name {
        "conversa.abrir" => match crate::context::unwrap_envelope(text) {
            // (a reopened search or reopened result: what is inside it, which is shorter, so this ends)
            Some((origin, body)) if origin.starts_with("conversa.") && body.len() < text.len() => result_kind(&origin, &body, connectors),
            Some((origin, _)) => external_kind(&origin, connectors),
            None => Some("external:tool"),
        },
        "conversa.buscar" => {
            let body = match crate::context::unwrap_envelope(text) {
                Some((origin, body)) if origin == "conversa.buscar" => body,
                _ => text.to_string(),
            };
            let kinds: Vec<&'static str> = body
                .lines()
                .filter_map(|l| l.strip_prefix('[').and_then(|r| r.split_once("] ")).map(|(h, _)| h))
                .filter_map(|h| match h.rsplit(" · ").next().unwrap_or("").trim() {
                    "pessoa" | "agente" => None,
                    "ferramenta" | "" => Some("external:tool"),
                    who => external_kind(who, connectors),
                })
                .collect();
            // (the most external, in the receiver's order for a trace: web, mail, a tool, a file)
            ["external:web", "external:mail", "external:tool", "external:file"].into_iter().find(|k| kinds.contains(k))
        }
        _ => external_kind(name, connectors),
    }
}

pub fn external_kind(name: &str, connectors: &[String]) -> Option<&'static str> {
    if let Some(c) = connectors.iter().find(|c| name.starts_with(&format!("{c}."))) {
        let l = c.to_lowercase();
        return Some(if ["mail", "gmail", "imap", "outlook"].iter().any(|w| l.contains(w)) { "external:mail" } else { "external:tool" });
    }
    // the calls that came into an internet trigger (their paths, headers and bodies are the internet's)
    if matches!(name, "gatilho_internet.chamadas" | "internet.calls" | "internet.call_body") {
        return Some("external:web");
    }
    // tools under the agent's own prefixes whose results are someone else's: an MCP server's tool (only registering one
    // is the agent's), a creative app's tool (its documents), the Wi-Fi networks in range (their names are the
    // neighbours')
    if (name.starts_with("mcp.") && name != "mcp.registrar") || matches!(name, "craft.chamar" | "rede.wifi") {
        return Some("external:tool");
    }
    let prefix = name.split('.').next().unwrap_or("");
    match prefix {
        "web" => Some("external:web"),
        "correio" => Some("external:mail"),
        "arquivos" => kalango_d1::taints_conversation(name).then_some("external:file"),
        // (what a command prints can be anything: a file, a page, another machine's answer)
        "sistema" if name == "sistema.executar" => Some("external:tool"),
        p if OWN_PREFIXES.contains(&p) => None,
        _ => Some("external:tool"),
    }
}

impl Turn {
    pub fn disabled() -> Turn {
        Turn(None)
    }

    pub fn is_on(&self) -> bool {
        self.0.is_some()
    }

    /// Starts recording a turn with the process's sink and the person's settings.
    pub fn begin_default(b: Begin) -> Turn {
        let cfg = Config::load();
        if !cfg.record {
            return Turn(None);
        }
        match global() {
            Some(s) => Turn::begin(s, &cfg, b),
            None => Turn(None),
        }
    }

    pub fn begin(sink: &Sink, cfg: &Config, b: Begin) -> Turn {
        if !cfg.record || !sink.available() {
            return Turn(None);
        }
        let meta = b.tools.iter().cloned().chain((b.local_tools)()).map(|t| (t.name.clone(), (t.level.clone(), t.read_only))).collect();
        Turn(Some(Arc::new(Inner {
            sink: sink.clone(),
            content: cfg.content,
            privacy: cfg.privacy_mode(),
            trace_id: random_id(),
            root_id: random_id(),
            start_ns: now_ns(),
            start: Instant::now(),
            session: b.session.to_string(),
            agent: b.agent.to_string(),
            index: b.index,
            request: b.request.to_string(),
            pasted: b.pasted.clone(),
            meta,
            connectors: b.connectors,
            st: Mutex::new(State {
                // (what the person pasted is not their own words: the turn is tainted from its first call)
                taint: b.taint.into_iter().chain((!b.pasted.is_empty()).then(|| PASTE.to_string())).collect(),
                gates: Vec::new(),
                pending_gate: None,
                last_chat: None,
                open: Vec::new(),
                pending_approval: NO_APPROVAL,
                answer: None,
                signal: None,
                stopped: false,
                warned: false,
                chats: 0,
                tools: 0,
                secret_in_context: b.secret,
                pending_calls: Vec::new(),
            }),
        })))
    }

    /// The conversation holds a secret from before this turn (found in its history after the turn began).
    pub fn mark_secret(&self) {
        if let Some(i) = &self.0 {
            if let Ok(mut s) = i.st.lock() {
                s.secret_in_context = true;
            }
        }
    }

    /// Whether a secret tool's result is (or was) in the conversation's context.
    pub fn secret_in_context(&self) -> bool {
        self.0.as_ref().and_then(|i| i.st.lock().ok().map(|s| s.secret_in_context)).unwrap_or(false)
    }

    /// What the conversation holds from outside now (the agent keeps it for the next turns).
    pub fn taint(&self) -> Vec<String> {
        self.0.as_ref().and_then(|i| i.st.lock().ok().map(|s| s.taint.iter().cloned().collect())).unwrap_or_default()
    }

    /// The engine forces the next model call (the intent gate: `forced_planejar`, `forced_concluir`) or takes the tools away.
    pub fn gate(&self, name: &'static str) {
        if let Some(i) = &self.0 {
            if let Ok(mut s) = i.st.lock() {
                s.pending_gate = Some(name);
                if !s.gates.iter().any(|g| g == name) {
                    s.gates.push(name.to_string());
                }
            }
        }
    }

    pub fn clock(&self) -> Clock {
        Clock(self.0.as_ref().map(|_| (now_ns(), Instant::now())))
    }

    /// The person was asked (the card is up): the time to the answer is measured from here.
    fn approval_asked(&self) {
        if let Some(i) = &self.0 {
            if let Ok(mut s) = i.st.lock() {
                s.pending_approval = Approval { outcome: ApprovalState::Asked, auto: false, asked_at: Some(Instant::now()), latency_ms: None };
            }
        }
    }

    /// The person answered the card that was up.
    pub fn approval_answer(&self, approved: bool) {
        if let Some(i) = &self.0 {
            if let Ok(mut s) = i.st.lock() {
                let a = s.pending_approval;
                s.pending_approval = Approval {
                    outcome: if approved { ApprovalState::Approved } else { ApprovalState::Denied },
                    auto: false,
                    asked_at: a.asked_at,
                    latency_ms: a.asked_at.map(|t| t.elapsed().as_millis() as u64),
                };
                s.signal = Some(if approved { s.signal.unwrap_or("approve") } else { "deny" });
            }
        }
    }

    /// "Aprovar sozinho" let the action through: approved with no card.
    pub fn approval_auto(&self) {
        if let Some(i) = &self.0 {
            if let Ok(mut s) = i.st.lock() {
                s.pending_approval = Approval { outcome: ApprovalState::Approved, auto: true, asked_at: None, latency_ms: Some(0) };
            }
        }
    }

    /// What the engine shows the person, as the trace needs it: the tool spans start and end here.
    pub fn observe(&self, ev: &Event) {
        let Some(i) = &self.0 else { return };
        match ev {
            Event::Ask { .. } => self.approval_asked(),
            Event::ToolStart { tool, args, level, .. } => {
                let (lvl, ro) = i.meta.get(tool).cloned().unwrap_or_else(|| (level.clone(), level == "observar"));
                if let Ok(mut s) = i.st.lock() {
                    let approval = std::mem::replace(&mut s.pending_approval, NO_APPROVAL);
                    let parent = s.last_chat;
                    // the first call of the last reply with this tool's name (a call the engine made itself has none)
                    let call_id = s.pending_calls.iter().position(|(_, n)| n == tool).map(|p| s.pending_calls.remove(p).0);
                    s.open.push(OpenTool {
                        name: tool.clone(),
                        args: args.clone(),
                        level: lvl,
                        read_only: ro,
                        start_ns: now_ns(),
                        start: Instant::now(),
                        approval,
                        parent,
                        call_id,
                    });
                }
            }
            Event::ToolDone { tool, ok, text, .. } => self.tool_done(i, tool, *ok, text),
            Event::Blocked { tool, rule, .. } => {
                if let Ok(mut s) = i.st.lock() {
                    s.warned = true;
                }
                let t = now_ns();
                i.sink.push(SpanData {
                    trace_id: i.trace_id,
                    span_id: random_id(),
                    parent: Some(i.root_id),
                    name: "kalango.event".into(),
                    kind: 1,
                    start_ns: t,
                    end_ns: t,
                    attrs: vec![
                        ("kalango.event.name".into(), Attr::Str("blocked".into())),
                        ("kalango.tool.name".into(), Attr::Str(tool.clone())),
                        ("kalango.block.rule".into(), Attr::Str(rule.clone())),
                    ],
                    events: vec![],
                    error: None,
                    is_root: false,
                    privacy: i.privacy,
                });
            }
            Event::Stopped { .. } => {
                if let Ok(mut s) = i.st.lock() {
                    s.stopped = true;
                }
            }
            Event::Answer(a) => {
                if let Ok(mut s) = i.st.lock() {
                    s.answer = Some(a.clone());
                }
            }
            Event::Question { question, .. } => {
                if let Ok(mut s) = i.st.lock() {
                    s.answer = Some(question.clone());
                }
            }
            _ => {}
        }
    }

    fn tool_done(&self, i: &Inner, name: &str, ok: bool, text: &str) {
        let secret = is_secret_tool(name);
        let (open, taint_kind, secret_before) = {
            let Ok(mut s) = i.st.lock() else { return };
            let pos = s.open.iter().rposition(|o| o.name == name);
            let open = match pos {
                Some(p) => s.open.remove(p),
                None => return,
            };
            s.tools += 1;
            let kind = if ok { result_kind(name, text, &i.connectors) } else { None };
            if let Some(k) = kind {
                s.taint.insert(k.to_string());
            }
            // (a secret tool's result is in the context from now on, whether the call worked or not)
            let before = s.secret_in_context;
            s.secret_in_context |= secret;
            (open, kind, before)
        };
        let end_ns = open.start_ns + open.start.elapsed().as_nanos() as u64;
        let mut attrs: Vec<(String, Attr)> = vec![
            ("gen_ai.operation.name".into(), Attr::Str("execute_tool".into())),
            ("gen_ai.tool.name".into(), Attr::Str(name.to_string())),
            ("kalango.tool.name".into(), Attr::Str(name.to_string())),
            // (the agent's own level, as it is: observar, usuario, sistema, total; the receiver derives what it needs)
            ("kalango.tool.level".into(), Attr::Str(open.level.clone())),
            ("kalango.tool.read_only".into(), Attr::Bool(open.read_only)),
            ("kalango.approval".into(), Attr::Str(open.approval.outcome.name().into())),
            // (never the bare word: the kind of external content is always said)
            ("kalango.provenance".into(), Attr::Str(taint_kind.unwrap_or("tool_local").into())),
        ];
        if let Some(id) = &open.call_id {
            attrs.push(("gen_ai.tool.call.id".into(), Attr::Str(trace_call_id(id))));
        }
        if open.approval.outcome != ApprovalState::None {
            attrs.push(("kalango.approval.latency_ms".into(), Attr::Int(open.approval.latency_ms.unwrap_or(0) as i64)));
        }
        if open.approval.auto {
            attrs.push(("kalango.approval.auto".into(), Attr::Bool(true)));
        }
        if secret {
            attrs.push(("kalango.tool.result_secret".into(), Attr::Bool(true)));
        }
        if let Some(k) = taint_kind {
            attrs.push(("kalango.taint".into(), Attr::List(vec![k.to_string()])));
        }
        if name.starts_with("fluxos.") {
            attrs.push(("kalango.flow.draft".into(), Attr::Bool(true)));
        }
        let mut events = Vec::new();
        if i.content {
            // the arguments of a secret tool, and of any call the model makes once a secret is in its context (it may
            // have put the secret in them), are not sent
            let args_attrs = if secret {
                vec![("kalango.tool.args_omitted".into(), Attr::Str("secret".into()))]
            } else if secret_before {
                vec![("kalango.tool.args_omitted".into(), Attr::Str("secret_in_context".into()))]
            } else {
                vec![("body".into(), Attr::Json(open.args.clone()))]
            };
            events.push(EventData { name: "kalango.tool.args", time_ns: open.start_ns, attrs: args_attrs });
            if secret {
                events.push(EventData {
                    name: "kalango.tool.result",
                    time_ns: end_ns,
                    attrs: vec![("kalango.tool.result_omitted".into(), Attr::Str("secret".into()))],
                });
            } else {
                events.push(EventData { name: "kalango.tool.result", time_ns: end_ns, attrs: vec![("body".into(), Attr::Str(text.to_string()))] });
            }
        } else {
            attrs.push(length_attr("kalango.content", text.chars().count(), secret));
        }
        // a denial is the person's answer, not the tool's failure
        let failed = !ok && open.approval.outcome != ApprovalState::Denied;
        i.sink.push(SpanData {
            trace_id: i.trace_id,
            span_id: random_id(),
            parent: Some(open.parent.unwrap_or(i.root_id)),
            name: format!("execute_tool {name}"),
            kind: 1,
            start_ns: open.start_ns,
            end_ns,
            attrs,
            events,
            error: failed.then(|| "tool_failed".to_string()),
            is_root: false,
            privacy: i.privacy,
        });
    }

    /// One call of the model, finished (or failed). `input` is what was sent.
    #[cfg(test)]
    pub fn chat(&self, model: &str, input: &[Value], clock: Clock, result: &Result<Reply, String>) {
        self.chat_marked(model, input, &[], clock, result)
    }

    /// `chat` with where each message of `input` came from when the conversation's own marks say so (the context
    /// window's `marks`: an engine's note is `agent`, a request with pasted text is `external:paste`, in any turn).
    pub fn chat_marked(&self, model: &str, input: &[Value], marks: &[Option<&str>], clock: Clock, result: &Result<Reply, String>) {
        let (Some(i), Some((start_ns, started))) = (&self.0, clock.0) else { return };
        let end_ns = start_ns + started.elapsed().as_nanos() as u64;
        let span_id = random_id();
        let secret_in_input = context_has_secret(input);
        let (taint, gate, secret_now, secret_before) = {
            let Ok(mut s) = i.st.lock() else { return };
            s.chats += 1;
            s.last_chat = Some(span_id);
            s.pending_calls = match result {
                Ok(r) => r.tool_calls.iter().map(|c| (c.id.clone(), tool_name(&c.name))).collect(),
                Err(_) => Vec::new(),
            };
            let before = s.secret_in_context;
            s.secret_in_context |= secret_in_input;
            (s.taint.iter().cloned().collect::<Vec<_>>(), s.pending_gate.take(), s.secret_in_context, before)
        };
        let mut attrs: Vec<(String, Attr)> = vec![
            ("gen_ai.operation.name".into(), Attr::Str("chat".into())),
            ("gen_ai.provider.name".into(), Attr::Str("llama.cpp".into())),
            ("gen_ai.request.model".into(), Attr::Str(model.to_string())),
            ("gen_ai.response.model".into(), Attr::Str(model.to_string())),
        ];
        if let Some(g) = gate {
            attrs.push(("kalango.gate".into(), Attr::Str(g.to_string())));
        }
        if !taint.is_empty() {
            attrs.push(("kalango.taint".into(), Attr::List(taint)));
        }
        if secret_now {
            attrs.push(("kalango.secret_in_context".into(), Attr::Bool(true)));
        }
        // the ids of the tool results in the input, in order: labels a reader pairs with the calls that asked for them. They
        // go in metadata mode too (they are ids, never content), as an attribute because that mode sends no events.
        let result_ids: Vec<String> =
            input.iter().filter(|m| m["role"] == "tool").filter_map(|m| m["tool_call_id"].as_str()).take(200).map(trace_call_id).collect();
        if !result_ids.is_empty() {
            attrs.push(("kalango.tool.call.ids".into(), Attr::List(result_ids)));
        }
        let mut events = Vec::new();
        let mut error = None;
        match result {
            Ok(r) => {
                let finish = if r.cut {
                    "length"
                } else if !r.tool_calls.is_empty() {
                    "tool_calls"
                } else {
                    "stop"
                };
                attrs.push(("gen_ai.usage.input_tokens".into(), Attr::Int(r.prompt_tokens as i64)));
                attrs.push(("gen_ai.usage.output_tokens".into(), Attr::Int(r.tokens.saturating_sub(r.prompt_tokens) as i64)));
                attrs.push(("gen_ai.response.finish_reasons".into(), Attr::List(vec![finish.to_string()])));
                if i.content {
                    let calls: Vec<Value> = r
                        .tool_calls
                        .iter()
                        .map(|c| {
                            let args = if is_secret_tool(&tool_name(&c.name)) { SECRET_ARGS_OMITTED } else { c.raw_arguments.as_str() };
                            json!({"id": trace_call_id(&c.id), "type": "function", "function": {"name": c.name, "arguments": args}})
                        })
                        .collect();
                    let mut msg = json!({"role": "assistant", "content": r.content});
                    if !calls.is_empty() {
                        msg["tool_calls"] = json!(calls);
                    }
                    events = content_events(i, input, marks, start_ns, secret_before);
                    let omitted = input.len().saturating_sub(events.len());
                    if omitted > 0 {
                        attrs.push(("kalango.input.omitted_messages".into(), Attr::Int(omitted as i64)));
                    }
                    if secret_now {
                        // the model has seen a secret: what it said or asked for may repeat it, so only metadata goes
                        attrs.push(("kalango.content.omitted".into(), Attr::Str("secret_in_context".into())));
                        attrs.push(length_attr("kalango.output", r.content.chars().count(), true));
                    } else {
                        events.push(EventData {
                            name: "gen_ai.choice",
                            time_ns: end_ns,
                            attrs: vec![
                                ("index".into(), Attr::Int(0)),
                                ("finish_reason".into(), Attr::Str(finish.to_string())),
                                ("message".into(), Attr::Json(msg)),
                                ("kalango.provenance".into(), Attr::Str("agent".into())),
                            ],
                        });
                    }
                } else {
                    // (counted as the content events are: not what a secret tool returned, and not what the assistant said
                    // after it, which may repeat it; and only as a bucket once a secret is in the context)
                    let by_call = call_names(input);
                    let first_secret = if secret_before { Some(0) } else { input.iter().position(|m| tool_message_is_secret(m, &by_call)) };
                    let sent: usize = input
                        .iter()
                        .enumerate()
                        .filter(|(idx, m)| {
                            !tool_message_is_secret(m, &by_call) && !(m["role"] == "assistant" && first_secret.is_some_and(|f| *idx > f))
                        })
                        .map(|(_, m)| m["content"].as_str().map_or(0, |c| c.chars().count()))
                        .sum();
                    attrs.push(length_attr("kalango.content", sent, secret_now));
                    attrs.push(length_attr("kalango.output", r.content.chars().count(), secret_now));
                }
            }
            Err(_) => {
                attrs.push(("error.type".into(), Attr::Str("model_error".into())));
                attrs.push(("gen_ai.usage.input_tokens".into(), Attr::Int(0)));
                attrs.push(("gen_ai.usage.output_tokens".into(), Attr::Int(0)));
                attrs.push(("gen_ai.response.finish_reasons".into(), Attr::List(vec!["error".into()])));
                error = Some("model_error".to_string());
            }
        }
        i.sink.push(SpanData {
            trace_id: i.trace_id,
            span_id,
            parent: Some(i.root_id),
            name: format!("chat {model}"),
            kind: 3,
            start_ns,
            end_ns,
            attrs,
            events,
            error,
            is_root: false,
            privacy: i.privacy,
        });
    }

    /// The turn is over: the root span goes (last, so a receiver that sends at the root's end has the children).
    pub fn end(&self, result: &Result<(), String>) {
        let Some(i) = &self.0 else { return };
        let (taint, gates, answer, signal, stopped, warned, chats, tools, secret) = {
            let Ok(s) = i.st.lock() else { return };
            (
                s.taint.iter().cloned().collect::<Vec<_>>(),
                s.gates.clone(),
                s.answer.clone(),
                s.signal,
                s.stopped,
                s.warned,
                s.chats,
                s.tools,
                s.secret_in_context,
            )
        };
        let end_ns = i.start_ns + i.start.elapsed().as_nanos() as u64;
        let mut attrs: Vec<(String, Attr)> = vec![
            ("kalango.session.id".into(), Attr::Str(i.session.clone())),
            ("kalango.agent.id".into(), Attr::Str(i.agent.clone())),
            ("kalango.turn.index".into(), Attr::Int(i.index as i64)),
            ("kalango.privacy.mode".into(), Attr::Str(i.privacy.into())),
            ("kalango.release".into(), Attr::Str(env!("CARGO_PKG_VERSION").into())),
            (
                "kalango.level".into(),
                Attr::Str(
                    if result.is_err() || stopped {
                        "error"
                    } else if warned {
                        "warning"
                    } else {
                        "default"
                    }
                    .into(),
                ),
            ),
            ("kalango.model_calls".into(), Attr::Int(chats as i64)),
            ("kalango.tool_calls".into(), Attr::Int(tools as i64)),
        ];
        if !taint.is_empty() {
            attrs.push(("kalango.taint".into(), Attr::List(taint)));
        }
        if !gates.is_empty() {
            attrs.push(("kalango.gates".into(), Attr::List(gates)));
        }
        if let Some(s) = signal {
            attrs.push(("kalango.user.signal".into(), Attr::Str(s.into())));
        }
        if stopped {
            attrs.push(("kalango.stopped".into(), Attr::Bool(true)));
        }
        // what the person pasted (the ranges are offsets in the request: they say where, never what)
        let pasted = (!i.pasted.is_empty()).then(|| json!(i.pasted).to_string());
        // (the root is the person's request: theirs, or with text pasted from somewhere else; in every mode)
        attrs.push(("kalango.provenance".into(), Attr::Str(if pasted.is_some() { PASTE } else { "user" }.into())));
        if let Some(r) = &pasted {
            attrs.push(("kalango.paste.ranges".into(), Attr::Str(r.clone())));
        }
        if secret {
            attrs.push(("kalango.secret_in_context".into(), Attr::Bool(true)));
        }
        let mut events = Vec::new();
        if i.content {
            let mut user_attrs = vec![
                ("role".into(), Attr::Str("user".into())),
                ("content".into(), Attr::Str(i.request.clone())),
                ("kalango.provenance".into(), Attr::Str(if pasted.is_some() { PASTE } else { "user" }.into())),
            ];
            if let Some(r) = &pasted {
                user_attrs.push(("kalango.paste.ranges".into(), Attr::Str(r.clone())));
            }
            events.push(EventData { name: "gen_ai.user.message", time_ns: i.start_ns, attrs: user_attrs });
            // (the answer of a turn that had a secret in its context may repeat it: only its length goes)
            if let (Some(a), true) = (&answer, secret) {
                attrs.push(length_attr("kalango.output", a.chars().count(), true));
            }
            if let (Some(a), false) = (&answer, secret) {
                events.push(EventData {
                    name: "gen_ai.choice",
                    time_ns: end_ns,
                    attrs: vec![
                        ("index".into(), Attr::Int(0)),
                        ("finish_reason".into(), Attr::Str("stop".into())),
                        ("message".into(), Attr::Json(json!({"role": "assistant", "content": a}))),
                        ("kalango.provenance".into(), Attr::Str("agent".into())),
                    ],
                });
            }
        } else {
            attrs.push(("kalango.content.length".into(), Attr::Int(i.request.chars().count() as i64)));
            attrs.push(length_attr("kalango.output", answer.as_deref().map_or(0, |a| a.chars().count()), secret));
        }
        let error = match result {
            Err(_) => Some("turn_failed".to_string()),
            Ok(()) if stopped => Some("stopped_by_defence".to_string()),
            Ok(()) => None,
        };
        i.sink.push(SpanData {
            trace_id: i.trace_id,
            span_id: i.root_id,
            parent: None,
            name: "kalango.turn".into(),
            kind: 1,
            start_ns: i.start_ns,
            end_ns,
            attrs,
            events,
            error,
            is_root: true,
            privacy: i.privacy,
        });
    }
}

/// The size of some text as an attribute: exact (`<base>.length`), or only a coarse bucket (`<base>.length_bucket`) for
/// what may be, or may repeat, a secret.
fn length_attr(base: &str, n: usize, coarse: bool) -> (String, Attr) {
    if coarse {
        let bucket = match n {
            0..=63 => "<64",
            64..=1023 => "<1KiB",
            1024..=65535 => "<64KiB",
            _ => ">=64KiB",
        };
        (format!("{base}.length_bucket"), Attr::Str(bucket.into()))
    } else {
        (format!("{base}.length"), Attr::Int(n as i64))
    }
}

const SECRET_RESULT_OMITTED: &str = "[omitted: secret tool result]";
const SECRET_ARGS_OMITTED: &str = "[omitted: secret tool arguments]";
const SECRET_CONTEXT_OMITTED: &str = "[omitted: secret in context]";

/// Which tool each call of the conversation is (the assistant's messages name them), by call id.
fn call_names(input: &[Value]) -> HashMap<String, String> {
    let mut by_call = HashMap::new();
    for m in input {
        for c in m["tool_calls"].as_array().into_iter().flatten() {
            if let (Some(id), Some(n)) = (c["id"].as_str(), c["function"]["name"].as_str()) {
                by_call.insert(id.to_string(), tool_name(n));
            }
        }
    }
    by_call
}

/// A tool's result that must not go out: the tool is a secret one, or the call it answers cannot be found (a result
/// of unknown origin is treated as a secret: fail closed).
fn tool_message_is_secret(m: &Value, by_call: &HashMap<String, String>) -> bool {
    m["role"] == "tool" && m["tool_call_id"].as_str().and_then(|id| by_call.get(id)).is_none_or(|n| is_secret_tool(n))
}

/// Whether the model's input holds a result of a secret (or unknown) tool.
pub fn context_has_secret(input: &[Value]) -> bool {
    let by_call = call_names(input);
    input.iter().any(|m| tool_message_is_secret(m, &by_call))
}

/// The model's input as `gen_ai.*.message` events: the first message (the system prompt) and as many of the latest
/// as fit; what is left out is counted. Each says where it came from (`kalango.provenance`).
fn content_events(i: &Inner, input: &[Value], marks: &[Option<&str>], t: u64, secret_before: bool) -> Vec<EventData> {
    let by_call = call_names(input);
    // from the first secret result on, what the assistant said may repeat it
    // (when a secret was already in the conversation before this call, everything the assistant said is after it)
    let first_secret = if secret_before { Some(0) } else { input.iter().position(|m| tool_message_is_secret(m, &by_call)) };
    // the request of this turn, when part of it was pasted: the latest person's message that starts with it (the
    // context window adds its notes, the clock, to the end of the request; an older turn may have said the same words)
    let pasted_request = (!i.pasted.is_empty() && !i.request.is_empty())
        .then(|| input.iter().rposition(|m| m["role"] == "user" && m["content"].as_str().is_some_and(|c| c.starts_with(i.request.as_str()))))
        .flatten();
    let as_event = |idx: usize, m: &Value| -> EventData {
        let role = m["role"].as_str().unwrap_or("user");
        let name = match role {
            "system" => "gen_ai.system.message",
            "assistant" => "gen_ai.assistant.message",
            "tool" => "gen_ai.tool.message",
            _ => "gen_ai.user.message",
        };
        let mark = marks.get(idx).copied().flatten();
        let provenance = match role {
            "system" | "assistant" => "agent".to_string(),
            "tool" => m["tool_call_id"]
                .as_str()
                .and_then(|id| by_call.get(id))
                .map(|n| result_kind(n, m["content"].as_str().unwrap_or(""), &i.connectors).unwrap_or("tool_local").to_string())
                .unwrap_or_else(|| "unknown".into()),
            // the conversation's own marks first (an engine's note; a request of this or an earlier turn with pasted
            // text); without them (an input built by hand), the note's key and this turn's request (as on the root)
            _ => match mark {
                Some(p) => p,
                None if m.get(crate::context::NOTE_KEY).is_some() => "agent",
                None if pasted_request == Some(idx) => PASTE,
                None => "user",
            }
            .to_string(),
        };
        let text = m["content"].as_str().map(str::to_string).unwrap_or_else(|| m["content"].to_string());
        // a secret tool's result: its name, size and hash say what happened; the text does not go
        if tool_message_is_secret(m, &by_call) {
            let mut attrs = vec![
                ("role".into(), Attr::Str(role.into())),
                ("content".into(), Attr::Str(SECRET_RESULT_OMITTED.into())),
                // (no hash and no exact size: a short secret can be guessed from either)
                length_attr("kalango.content", text.len(), true),
                ("kalango.provenance".into(), Attr::Str(provenance)),
            ];
            if let Some(n) = m["tool_call_id"].as_str().and_then(|id| by_call.get(id)) {
                attrs.push(("kalango.tool.name".into(), Attr::Str(n.clone())));
            }
            if let Some(id) = m["tool_call_id"].as_str() {
                attrs.push(("kalango.tool.call.id".into(), Attr::Str(trace_call_id(id))));
            }
            return EventData { name, time_ns: t, attrs };
        }
        let after_secret = role == "assistant" && first_secret.is_some_and(|f| idx > f);
        let shown = if after_secret && !text.is_empty() { SECRET_CONTEXT_OMITTED.to_string() } else { text };
        let content = if m["tool_calls"].is_array() {
            // (the calls of a secret tool go without their arguments)
            let calls: Vec<Value> = m["tool_calls"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|c| {
                    let secret_call = c["function"]["name"].as_str().is_some_and(|n| is_secret_tool(&tool_name(n)));
                    let mut c = c.clone();
                    if secret_call || after_secret {
                        c["function"]["arguments"] = json!(SECRET_ARGS_OMITTED);
                    }
                    if let Some(id) = c["id"].as_str() {
                        c["id"] = json!(trace_call_id(id));
                    }
                    c
                })
                .collect();
            // (kept structured: the redactor reads the keys of the arguments before the event is written)
            Attr::Json(json!({"content": shown, "tool_calls": calls}))
        } else {
            Attr::Str(shown)
        };
        let mut attrs =
            vec![("role".into(), Attr::Str(role.into())), ("content".into(), content), ("kalango.provenance".into(), Attr::Str(provenance))];
        // (a tool's result says which call it answers, so a reader can pair them)
        if let Some(id) = m["tool_call_id"].as_str().filter(|_| role == "tool") {
            attrs.push(("kalango.tool.call.id".into(), Attr::Str(trace_call_id(id))));
        }
        EventData { name, time_ns: t, attrs }
    };
    let size = |m: &Value| m["content"].as_str().map_or(64, str::len) + m["tool_calls"].to_string().len();
    let mut keep: Vec<usize> = Vec::new();
    let mut used = 0usize;
    // the system prompt first, then from the end backwards while it fits
    if !input.is_empty() {
        keep.push(0);
        used += size(&input[0]).min(MAX_CONTENT_BYTES / 4);
    }
    for idx in (1..input.len()).rev() {
        let s = size(&input[idx]).min(MAX_CONTENT_BYTES);
        if keep.len() >= MAX_CONTENT_EVENTS || used + s > MAX_CONTENT_BYTES {
            break;
        }
        used += s;
        keep.push(idx);
    }
    let from = 1.min(keep.len());
    keep[from..].reverse();
    keep.iter().map(|&k| as_event(k, &input[k])).collect()
}

#[cfg(test)]
#[path = "traces_tests.rs"]
mod tests;
