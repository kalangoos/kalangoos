//! A small real HTTP server on 127.0.0.1 for the tests: serves files with Range, checks a bearer token, can drop a
//! transfer half way, redirect, and records every request it saw.
#![allow(dead_code)]

use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct Seen {
    pub method: String,
    pub path: String,
    pub authorization: Option<String>,
    pub range: Option<String>,
}

#[derive(Default)]
pub struct Behavior {
    /// path (without the leading slash) → body
    pub files: Mutex<HashMap<String, Vec<u8>>>,
    /// answer this status to everything (401, 403, 500…)
    pub always: Mutex<Option<u16>>,
    /// when set, every request must carry `Authorization: Bearer <this>` (401 without, 403 with another)
    pub token: Mutex<Option<String>>,
    /// the first GET of a file stops after this many bytes (a dropped connection)
    pub drop_first_after: Mutex<Option<usize>>,
    /// `/redirect/<name>` answers 302 to this absolute URL
    pub redirect_to: Mutex<Option<String>>,
    /// pause this long between 16 KB chunks
    pub chunk_delay: Mutex<Duration>,
    /// serve this many bytes less than the file (a shorter file than the catalog says is the test's business)
    pub log: Mutex<Vec<Seen>>,
    pub gets: AtomicUsize,
    pub dropped_once: AtomicBool,
    /// every GET answers with a body that never ends (no Content-Length)
    pub endless: AtomicBool,
    /// bytes an endless answer managed to send
    pub endless_sent: AtomicUsize,
}

pub struct Server {
    pub port: u16,
    pub b: Arc<Behavior>,
    stop: Arc<AtomicBool>,
}

impl Server {
    pub fn start() -> Server {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        l.set_nonblocking(true).unwrap();
        let b = Arc::new(Behavior::default());
        let stop = Arc::new(AtomicBool::new(false));
        let (b2, s2) = (b.clone(), stop.clone());
        std::thread::spawn(move || {
            while !s2.load(Ordering::Relaxed) {
                match l.accept() {
                    Ok((s, _)) => {
                        let b3 = b2.clone();
                        std::thread::spawn(move || {
                            let _ = handle(s, &b3);
                        });
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(5)),
                }
            }
        });
        Server { port, b, stop }
    }

    pub fn base(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    pub fn put(&self, name: &str, body: Vec<u8>) {
        self.b.files.lock().unwrap().insert(name.to_string(), body);
    }

    pub fn seen(&self) -> Vec<Seen> {
        self.b.log.lock().unwrap().clone()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn handle(mut s: TcpStream, b: &Behavior) -> std::io::Result<()> {
    s.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut r = BufReader::new(s.try_clone()?);
    let mut line = String::new();
    r.read_line(&mut line)?;
    let mut it = line.split_whitespace();
    let (method, path) = (it.next().unwrap_or("").to_string(), it.next().unwrap_or("/").to_string());
    let (mut auth, mut range) = (None, None);
    loop {
        let mut h = String::new();
        r.read_line(&mut h)?;
        if h.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            match k.to_ascii_lowercase().as_str() {
                "authorization" => auth = Some(v.trim().to_string()),
                "range" => range = Some(v.trim().to_string()),
                _ => {}
            }
        }
    }
    b.log.lock().unwrap().push(Seen { method: method.clone(), path: path.clone(), authorization: auth.clone(), range: range.clone() });
    let reply = |s: &mut TcpStream, status: &str, extra: &str, body: &[u8], len: usize, head: bool| -> std::io::Result<()> {
        write!(s, "HTTP/1.1 {status}\r\n{extra}Content-Length: {len}\r\nConnection: close\r\n\r\n")?;
        if !head {
            s.write_all(body)?;
        }
        s.flush()
    };
    if let Some(c) = *b.always.lock().unwrap() {
        return reply(&mut s, &format!("{c} X"), "", b"", 0, false);
    }
    if let Some(t) = b.token.lock().unwrap().clone() {
        match auth.as_deref() {
            None => return reply(&mut s, "401 Unauthorized", "", b"", 0, false),
            Some(a) if a != format!("Bearer {t}") => return reply(&mut s, "403 Forbidden", "", b"", 0, false),
            _ => {}
        }
    }
    let name = path.trim_start_matches('/').to_string();
    if let Some(n) = name.strip_prefix("redirect/") {
        let _ = n;
        if let Some(to) = b.redirect_to.lock().unwrap().clone() {
            return reply(&mut s, "302 Found", &format!("Location: {to}\r\n"), b"", 0, false);
        }
    }
    let body = b.files.lock().unwrap().get(&name).cloned();
    let Some(body) = body else { return reply(&mut s, "404 Not Found", "", b"", 0, false) };
    let head = method == "HEAD";
    if !head && b.endless.load(Ordering::Relaxed) {
        write!(s, "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n")?;
        let chunk = vec![7u8; 64 * 1024];
        loop {
            s.write_all(&chunk)?;
            b.endless_sent.fetch_add(chunk.len(), Ordering::Relaxed);
        }
    }
    if !head {
        b.gets.fetch_add(1, Ordering::Relaxed);
    }
    let (start, partial) = match range.as_deref().and_then(|r| r.strip_prefix("bytes=")).and_then(|r| r.strip_suffix('-')).and_then(|n| n.parse::<usize>().ok()) {
        Some(n) => (n, true),
        None => (0, false),
    };
    if start >= body.len() && partial {
        return reply(&mut s, "416 Range Not Satisfiable", &format!("Content-Range: bytes */{}\r\n", body.len()), b"", 0, false);
    }
    let slice = &body[start..];
    let (status, extra) = if partial { ("206 Partial Content", format!("Content-Range: bytes {start}-{}/{}\r\n", body.len() - 1, body.len())) } else { ("200 OK", String::new()) };
    write!(s, "HTTP/1.1 {status}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n", slice.len())?;
    if head {
        return s.flush();
    }
    let drop_after = if !b.dropped_once.load(Ordering::Relaxed) { *b.drop_first_after.lock().unwrap() } else { None };
    let delay = *b.chunk_delay.lock().unwrap();
    let mut sent = 0;
    for chunk in slice.chunks(16 * 1024) {
        if let Some(n) = drop_after {
            if sent >= n {
                b.dropped_once.store(true, Ordering::Relaxed);
                return Ok(()); // closes the connection short of Content-Length
            }
        }
        s.write_all(chunk)?;
        sent += chunk.len();
        if !delay.is_zero() {
            std::thread::sleep(delay);
        }
    }
    s.flush()
}

/// Deterministic pseudo-random bytes (so a wrong resume would corrupt the sum).
pub fn bytes(n: usize, seed: u64) -> Vec<u8> {
    let mut x = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    (0..n).map(|_| {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        (x >> 24) as u8
    }).collect()
}

pub fn sha256(b: &[u8]) -> String {
    Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect()
}

pub fn drain(mut r: impl Read) {
    let mut v = Vec::new();
    let _ = r.read_to_end(&mut v);
}
