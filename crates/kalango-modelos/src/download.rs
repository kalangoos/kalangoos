//! The download engine. A file is written to `<name>.part` (resumed with an HTTP `Range` after an interruption), its
//! SHA-256 is computed while it streams, and it is checked (size, then sum) BEFORE the atomic rename that installs
//! it: a file that does not match the catalog is refused and deleted, never kept.

use crate::catalog::{is_sha256, ModelFile};
use crate::net::{Client, NetError};
use crate::urlpolicy::ServerUrl;
use crate::Secret;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Where a model's files come from.
#[derive(Clone, Debug)]
pub enum Source {
    /// the catalog's own addresses (the public source)
    Default,
    /// `<base>/<file name>`, with the token (only this server gets it)
    Server { url: ServerUrl, token: Option<Secret> },
    /// files by name in a folder or on a USB drive
    Folder(PathBuf),
    /// the configuration is not valid: nothing is downloaded
    Disabled,
}

/// Pause and cancel, asked from another thread.
#[derive(Debug, Default)]
pub struct Control {
    pub cancel: AtomicBool,
    pub pause: AtomicBool,
}

impl Control {
    pub fn reset(&self) {
        self.cancel.store(false, Ordering::Relaxed);
        self.pause.store(false, Ordering::Relaxed);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DownloadError {
    Paused,
    Cancelled,
    Offline,
    Timeout,
    Unreachable(String),
    /// 401 or 403
    Denied(u16),
    NotFound(String),
    Status(u16),
    Redirect(String),
    /// the SHA-256 is not the catalog's: the file was refused and deleted
    Mismatch { file: String },
    WrongSize { file: String, found: u64, expected: u64 },
    /// not enough free space: how many bytes are missing
    NoSpace { missing: u64 },
    /// the catalog has no SHA-256 for this file, so a private server or a folder cannot provide it
    MissingChecksum { file: String },
    /// the connection dropped in the middle (the part is kept, the next try resumes it)
    Network(String),
    /// the disk
    Io(String),
    /// the source's configuration is not valid
    SourceInvalid,
}

impl DownloadError {
    /// Worth trying again (the network, not the file or the disk)?
    pub fn retryable(&self) -> bool {
        match self {
            DownloadError::Timeout | DownloadError::Unreachable(_) | DownloadError::Network(_) => true,
            DownloadError::Status(c) => *c >= 500 || *c == 429,
            _ => false,
        }
    }
}

impl From<NetError> for DownloadError {
    fn from(e: NetError) -> Self {
        match e {
            NetError::Offline => DownloadError::Offline,
            NetError::Timeout => DownloadError::Timeout,
            NetError::Unreachable(s) => DownloadError::Unreachable(s),
            NetError::Denied(c) => DownloadError::Denied(c),
            NetError::NotFound => DownloadError::NotFound(String::new()),
            NetError::Status(c) => DownloadError::Status(c),
            NetError::Redirect(s) => DownloadError::Redirect(s),
        }
    }
}

pub enum Event<'a> {
    Progress { file: &'a str, done: u64, total: u64, bytes_per_s: f64 },
    Verifying { file: &'a str },
    /// the network failed; trying again after `wait` (attempt `attempt` of `max`)
    Retry { attempt: u32, max: u32, wait: Duration, reason: &'a DownloadError },
}

pub struct Options {
    /// tries per file (the first one included)
    pub max_attempts: u32,
    /// the wait after the first failure; doubles each time, up to `max_backoff`
    pub backoff: Duration,
    pub max_backoff: Duration,
    /// free bytes where `path` is (tests inject a small number; the default asks the file system)
    pub free_space: Box<dyn Fn(&Path) -> Option<u64> + Send + Sync>,
    pub connect_timeout: Duration,
    pub read_timeout: Duration,
    /// tests: the network is up
    pub online: fn() -> bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            max_attempts: 5,
            backoff: Duration::from_secs(2),
            max_backoff: Duration::from_secs(60),
            free_space: Box::new(free_space),
            connect_timeout: Duration::from_secs(10),
            read_timeout: Duration::from_secs(30),
            online: crate::net::network_up,
        }
    }
}

/// Free bytes on the file system of `path` (or of its nearest existing parent).
pub fn free_space(path: &Path) -> Option<u64> {
    let mut p = path;
    while !p.exists() {
        p = p.parent()?;
    }
    let c = std::ffi::CString::new(p.as_os_str().as_encoded_bytes()).ok()?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a valid NUL-terminated path and `s` a zeroed statvfs that the call fills.
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
        return None;
    }
    Some(s.f_bavail as u64 * s.f_frsize as u64)
}

fn hex(d: &[u8]) -> String {
    d.iter().map(|b| format!("{b:02x}")).collect()
}

/// The SHA-256 of a file (lower-case hex); stops with `Cancelled` when asked.
pub fn hash_file(path: &Path, ctl: &Control) -> Result<String, DownloadError> {
    let mut f = std::fs::File::open(path).map_err(|e| DownloadError::Io(e.to_string()))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        if ctl.cancel.load(Ordering::Relaxed) {
            return Err(DownloadError::Cancelled);
        }
        let n = f.read(&mut buf).map_err(|e| DownloadError::Io(e.to_string()))?;
        if n == 0 {
            return Ok(hex(&h.finalize()));
        }
        h.update(&buf[..n]);
    }
}

fn io_err(e: std::io::Error) -> DownloadError {
    if e.raw_os_error() == Some(libc::ENOSPC) { DownloadError::NoSpace { missing: 0 } } else { DownloadError::Io(e.to_string()) }
}

/// What an attempt knows while it runs.
struct Job<'a> {
    file: &'a ModelFile,
    part: PathBuf,
    dest: PathBuf,
    /// bytes of the files before this one
    base: u64,
    total: u64,
    ctl: &'a Control,
    opts: &'a Options,
}

impl Job<'_> {
    fn interrupted(&self) -> Result<(), DownloadError> {
        if self.ctl.cancel.load(Ordering::Relaxed) {
            let _ = std::fs::remove_file(&self.part);
            return Err(DownloadError::Cancelled);
        }
        if self.ctl.pause.load(Ordering::Relaxed) {
            return Err(DownloadError::Paused);
        }
        Ok(())
    }

    fn room_for(&self, needed: u64) -> Result<(), DownloadError> {
        match (self.opts.free_space)(&self.part) {
            Some(free) if free < needed => Err(DownloadError::NoSpace { missing: needed - free }),
            _ => Ok(()),
        }
    }

    /// Size, then sum, then the atomic rename. A wrong file is deleted.
    fn finish(&self, total_len: u64, digest: String, on: &mut dyn FnMut(Event)) -> Result<(), DownloadError> {
        let f = self.file;
        if f.bytes > 0 && total_len != f.bytes {
            let _ = std::fs::remove_file(&self.part);
            return Err(DownloadError::WrongSize { file: f.nome.clone(), found: total_len, expected: f.bytes });
        }
        on(Event::Verifying { file: &f.nome });
        if is_sha256(&f.sha256) && digest != f.sha256 {
            let _ = std::fs::remove_file(&self.part);
            return Err(DownloadError::Mismatch { file: f.nome.clone() });
        }
        std::fs::rename(&self.part, &self.dest).map_err(io_err)?;
        if is_sha256(&f.sha256) {
            let _ = record_verified(self.dest.parent().unwrap_or(Path::new(".")), f);
        }
        Ok(())
    }

    /// Streams `reader` into the part file (appending when `append`), hashing as it goes.
    fn stream(&self, reader: &mut dyn Read, hasher: &mut Sha256, mut got: u64, append: bool, on: &mut dyn FnMut(Event)) -> Result<u64, DownloadError> {
        let mut out = std::fs::OpenOptions::new().create(true).write(true).append(append).truncate(!append).open(&self.part).map_err(io_err)?;
        let mut buf = vec![0u8; 1 << 16];
        let started = Instant::now();
        let start_got = got;
        loop {
            if let Err(e) = self.interrupted() {
                let _ = out.flush();
                return Err(e);
            }
            let n = match reader.read(&mut buf) {
                Ok(n) => n,
                Err(e) => {
                    let _ = out.flush();
                    return Err(DownloadError::Network(e.to_string()));
                }
            };
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n]).map_err(io_err)?;
            hasher.update(&buf[..n]);
            got += n as u64;
            let rate = (got - start_got) as f64 / started.elapsed().as_secs_f64().max(0.001);
            on(Event::Progress { file: &self.file.nome, done: self.base + got, total: self.total, bytes_per_s: rate });
        }
        out.flush().map_err(io_err)?;
        Ok(got)
    }

    /// The part file's SHA-256 state, after its own bytes.
    fn rehash(&self) -> Result<(Sha256, u64), DownloadError> {
        let mut h = Sha256::new();
        let mut f = std::fs::File::open(&self.part).map_err(io_err)?;
        let mut buf = vec![0u8; 1 << 20];
        let mut n_total = 0u64;
        loop {
            self.interrupted()?;
            let n = f.read(&mut buf).map_err(io_err)?;
            if n == 0 {
                return Ok((h, n_total));
            }
            h.update(&buf[..n]);
            n_total += n as u64;
        }
    }

    /// One try over the network: resumes the part file when there is one.
    fn attempt_http(&self, client: &Client, url: &str, on: &mut dyn FnMut(Event)) -> Result<(), DownloadError> {
        self.interrupted()?;
        let f = self.file;
        let mut have = std::fs::metadata(&self.part).map(|m| m.len()).unwrap_or(0);
        if f.bytes > 0 && have > f.bytes {
            let _ = std::fs::remove_file(&self.part);
            have = 0;
        }
        self.room_for(f.bytes.saturating_sub(have))?;
        let resp = match client.request("GET", url, Some(have)) {
            Ok(r) => r,
            // the part is the whole file (or the server's is shorter than the catalog says)
            Err(NetError::Status(416)) if have > 0 => {
                let (h, n) = self.rehash()?;
                return self.finish(n, hex(&h.finalize()), on);
            }
            Err(NetError::NotFound) => return Err(DownloadError::NotFound(f.nome.clone())),
            Err(e) => return Err(e.into()),
        };
        let resumed = resp.status() == 206 && have > 0 && resp.header("Content-Range").is_some_and(|r| r.starts_with(&format!("bytes {have}-")));
        let (mut hasher, start) = if resumed {
            self.rehash()?
        } else {
            // the server sent everything again (or ignored the Range): start the file over
            (Sha256::new(), 0)
        };
        // (never more than one byte past what the catalog says: a server that keeps sending cannot fill the disk)
        let mut reader: Box<dyn Read> = if f.bytes > 0 { Box::new(resp.into_reader().take(f.bytes.saturating_sub(start) + 1)) } else { Box::new(resp.into_reader()) };
        let got = self.stream(&mut reader, &mut hasher, start, resumed, on)?;
        if f.bytes > 0 && got < f.bytes {
            return Err(DownloadError::Network(format!("{}: {got} of {} bytes", f.nome, f.bytes)));
        }
        self.finish(got, hex(&hasher.finalize()), on)
    }

    fn copy_from_folder(&self, dir: &Path, on: &mut dyn FnMut(Event)) -> Result<(), DownloadError> {
        self.interrupted()?;
        let f = self.file;
        let src = dir.join(&f.nome);
        let md = std::fs::metadata(&src).map_err(|_| DownloadError::NotFound(f.nome.clone()))?;
        if md.len() != f.bytes {
            return Err(DownloadError::WrongSize { file: f.nome.clone(), found: md.len(), expected: f.bytes });
        }
        let _ = std::fs::remove_file(&self.part);
        self.room_for(f.bytes)?;
        let mut r = std::fs::File::open(&src).map_err(io_err)?;
        let mut h = Sha256::new();
        let got = self.stream(&mut r, &mut h, 0, false, on)?;
        self.finish(got, hex(&h.finalize()), on)
    }
}

fn sleep_checking(d: Duration, ctl: &Control) -> Result<(), DownloadError> {
    let end = Instant::now() + d;
    while Instant::now() < end {
        if ctl.cancel.load(Ordering::Relaxed) {
            return Err(DownloadError::Cancelled);
        }
        if ctl.pause.load(Ordering::Relaxed) {
            return Err(DownloadError::Paused);
        }
        std::thread::sleep(Duration::from_millis(20).min(end.saturating_duration_since(Instant::now())));
    }
    Ok(())
}

/// Downloads (or copies, for a folder) a model's files into `dest`. Files already there with the right size are
/// kept. Off the interface's thread. `on` hears the progress, the check and each retry.
pub fn download_files(files: &[ModelFile], dest: &Path, source: &Source, ctl: &Control, opts: &Options, on: &mut dyn FnMut(Event)) -> Result<(), DownloadError> {
    if matches!(source, Source::Disabled) {
        return Err(DownloadError::SourceInvalid);
    }
    std::fs::create_dir_all(dest).map_err(io_err)?;
    let total: u64 = files.iter().map(|f| f.bytes).sum();
    let mut base = 0u64;
    for f in files {
        let dest_file = dest.join(&f.nome);
        if std::fs::metadata(&dest_file).is_ok_and(|m| m.len() == f.bytes && f.bytes > 0) {
            // a file of the right size is kept when its sum is the catalog's (checked now if it never was);
            // one that is not is refused and deleted, and comes again
            if !is_sha256(&f.sha256) || is_verified(dest, f) {
                base += f.bytes;
                continue;
            }
            on(Event::Verifying { file: &f.nome });
            // (the stamp before the hash: a file changed while it is hashed is not recorded as verified)
            let before = stamp(&dest_file);
            if hash_file(&dest_file, ctl)? == f.sha256 {
                if let Some(b) = before {
                    // (a ledger that cannot be written leaves it "installed", not "verified": it is not hashed again this session)
                    let _ = record_verified_at(dest, f, &b);
                }
                base += f.bytes;
                continue;
            }
            let _ = std::fs::remove_file(&dest_file);
        }
        if !crate::catalog::safe_file_name(&f.nome) {
            return Err(DownloadError::Io(format!("unsafe file name {:?}", f.nome)));
        }
        if !matches!(source, Source::Default) && !is_sha256(&f.sha256) {
            return Err(DownloadError::MissingChecksum { file: f.nome.clone() });
        }
        let job = Job { file: f, part: dest.join(format!("{}.part", f.nome)), dest: dest_file, base, total, ctl, opts };
        match source {
            Source::Folder(dir) => job.copy_from_folder(dir, on)?,
            Source::Disabled => return Err(DownloadError::SourceInvalid),
            Source::Default | Source::Server { .. } => {
                let (pinned, token, url) = match source {
                    Source::Server { url, token } => (Some(url.clone()), token.clone(), url.file_url(&f.nome)),
                    _ => (None, None, f.url.clone()),
                };
                let client = Client::with_timeouts(pinned, token, opts.connect_timeout, opts.read_timeout).with_online(opts.online);
                let mut attempt = 1;
                loop {
                    match job.attempt_http(&client, &url, on) {
                        Ok(()) => break,
                        Err(e) if e.retryable() && attempt < opts.max_attempts => {
                            let wait = (opts.backoff * 2u32.saturating_pow(attempt - 1)).min(opts.max_backoff);
                            on(Event::Retry { attempt, max: opts.max_attempts, wait, reason: &e });
                            sleep_checking(wait, ctl)?;
                            attempt += 1;
                        }
                        Err(e) => return Err(e),
                    }
                }
            }
        }
        base += f.bytes;
    }
    Ok(())
}

// ------------------------------------------------------------------------------------------ what was verified

const LEDGER: &str = ".verificados.json";

fn mtime_secs(p: &Path) -> Option<u64> {
    std::fs::metadata(p).ok()?.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_secs())
}

/// A file's size and time stamp, taken before it is hashed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub bytes: u64,
    pub mtime: Option<u64>,
}

pub fn stamp(path: &Path) -> Option<Stamp> {
    let m = std::fs::metadata(path).ok()?;
    Some(Stamp { bytes: m.len(), mtime: mtime_secs(path) })
}

/// Remembers that a file was checked against the catalog. `before` is the stamp taken BEFORE the file was hashed: if
/// the file changed meanwhile, nothing is recorded (the sum is of other bytes). False when nothing was written
/// (changed file, or a ledger that cannot be written: a read-only folder).
pub fn record_verified_at(dir: &Path, f: &ModelFile, before: &Stamp) -> bool {
    let p = dir.join(&f.nome);
    if stamp(&p) != Some(*before) {
        return false;
    }
    let path = dir.join(LEDGER);
    let mut m: serde_json::Map<String, serde_json::Value> = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    m.insert(f.nome.clone(), serde_json::json!({"sha256": f.sha256, "bytes": before.bytes, "mtime": before.mtime}));
    std::fs::write(&path, serde_json::Value::Object(m).to_string()).is_ok()
}

/// [`record_verified_at`] for a file that was just written by the engine (its stamp is now).
pub fn record_verified(dir: &Path, f: &ModelFile) -> bool {
    stamp(&dir.join(&f.nome)).is_some_and(|st| record_verified_at(dir, f, &st))
}

/// Was this file checked by the engine against the very sum the catalog has now, and is it unchanged since?
pub fn is_verified(dir: &Path, f: &ModelFile) -> bool {
    let Some(m) = std::fs::read_to_string(dir.join(LEDGER)).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) else { return false };
    let e = &m[&f.nome];
    let p = dir.join(&f.nome);
    is_sha256(&f.sha256) && e["sha256"].as_str() == Some(&f.sha256) && e["bytes"].as_u64() == std::fs::metadata(&p).ok().map(|m| m.len()) && e["mtime"].as_u64() == mtime_secs(&p)
}

/// Deletes a model's files (and a leftover part) and forgets their checks.
pub fn remove_files(dir: &Path, files: &[ModelFile]) {
    for f in files {
        let _ = std::fs::remove_file(dir.join(&f.nome));
        let _ = std::fs::remove_file(dir.join(format!("{}.part", f.nome)));
    }
}
