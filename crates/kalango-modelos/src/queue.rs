//! The downloads running in the background, one model at a time, for the Central de Apps (which only shows
//! [`Queue::snapshot`] and sends pause, resume, cancel and retry).

use crate::catalog::ModelEntry;
use crate::download::{download_files, is_verified, remove_files, Control, DownloadError, Event, Options, Source};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// the sum is not the catalog's: refused and deleted
    Mismatch,
    WrongSize,
    NoSpace { missing: u64 },
    Unreachable(String),
    Timeout,
    Denied(u16),
    NotFound,
    /// the source's configuration is not valid
    SourceInvalid,
    /// the server answered with this HTTP error
    Server(u16),
    Other(String),
    /// the model's licence needs the person's acceptance and there is none (or it is for an older licence version)
    LicenseNotAccepted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    Queued,
    Downloading,
    Verifying,
    Paused,
    /// kept; `verified`: checked against the catalog's SHA-256
    Installed { verified: bool },
    /// the network is down: it goes on by itself when it returns
    WaitingNetwork,
    /// failed on the network; trying again in `wait_s` seconds (attempt `attempt` of `max`)
    Retrying { attempt: u32, max: u32, wait_s: u64 },
    Failed(Failure),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub id: String,
    pub phase: Phase,
    pub done: u64,
    pub total: u64,
    pub bytes_per_s: f64,
}

struct State {
    items: Vec<Item>,
    catalog: Vec<ModelEntry>,
    dest: PathBuf,
    /// read-only folders that already hold models (the system's, `/usr/share/kalango/modelos`: an ISO that carries
    /// models): a model whose files are all there, with the catalog's sizes, is installed and is never downloaded again
    system: Vec<PathBuf>,
    /// where the person's licence acceptances are kept (`licenca::aceites_path()`)
    licences: PathBuf,
    source: Source,
    current: Option<String>,
    stop: bool,
}

struct Shared {
    state: Mutex<State>,
    cv: Condvar,
    ctl: Control,
    opts: Box<dyn Fn() -> Options + Send + Sync>,
}

#[derive(Clone)]
pub struct Queue {
    shared: Arc<Shared>,
}

fn failure_of(e: &DownloadError) -> Failure {
    match e {
        DownloadError::Mismatch { .. } => Failure::Mismatch,
        DownloadError::WrongSize { .. } => Failure::WrongSize,
        DownloadError::NoSpace { missing } => Failure::NoSpace { missing: *missing },
        DownloadError::Unreachable(s) => Failure::Unreachable(s.clone()),
        DownloadError::Network(s) => Failure::Unreachable(s.clone()),
        DownloadError::Timeout => Failure::Timeout,
        DownloadError::Denied(c) => Failure::Denied(*c),
        DownloadError::NotFound(_) => Failure::NotFound,
        DownloadError::Status(c) => Failure::Server(*c),
        DownloadError::SourceInvalid => Failure::SourceInvalid,
        DownloadError::Redirect(s) | DownloadError::Io(s) => Failure::Other(s.clone()),
        DownloadError::MissingChecksum { file } => Failure::Other(format!("{file}: no checksum in the catalog")),
        DownloadError::Paused | DownloadError::Cancelled | DownloadError::Offline => Failure::Other(String::new()),
    }
}

impl Queue {
    /// Starts the worker. `opts` builds the engine's options for each model (tests make them quick).
    pub fn start(dest: PathBuf, catalog: Vec<ModelEntry>, source: Source, opts: impl Fn() -> Options + Send + Sync + 'static) -> Queue {
        let shared = Arc::new(Shared {
            state: Mutex::new(State { items: Vec::new(), catalog, dest, system: Vec::new(), licences: crate::licenca::aceites_path(), source, current: None, stop: false }),
            cv: Condvar::new(),
            ctl: Control::default(),
            opts: Box::new(opts),
        });
        let s2 = shared.clone();
        std::thread::spawn(move || worker(s2));
        Queue { shared }
    }

    fn with<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        let mut g = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        let r = f(&mut g);
        self.shared.cv.notify_all();
        r
    }

    pub fn snapshot(&self) -> Vec<Item> {
        self.shared.state.lock().unwrap_or_else(|e| e.into_inner()).items.clone()
    }

    pub fn item(&self, id: &str) -> Option<Item> {
        self.snapshot().into_iter().find(|i| i.id == id)
    }

    /// Changes where models come from (the next download uses it).
    pub fn set_source(&self, source: Source) {
        self.with(|s| s.source = source);
    }

    /// Where the licence acceptances are read from (tests use their own folder).
    pub fn set_licences_file(&self, path: PathBuf) {
        self.with(|s| s.licences = path);
    }

    /// Folders of the system that may already hold models (the next `enqueue` looks there before queueing a download).
    pub fn set_system_dirs(&self, dirs: Vec<PathBuf>) {
        self.with(|s| s.system = dirs);
    }

    /// Asks for a model: queued unless it is already installed or already in the queue.
    pub fn enqueue(&self, id: &str) {
        self.with(|s| {
            let Some(m) = s.catalog.iter().find(|m| m.id == id).cloned() else { return };
            if let Some(i) = s.items.iter_mut().find(|i| i.id == id) {
                if matches!(i.phase, Phase::Failed(_) | Phase::Paused | Phase::WaitingNetwork) {
                    i.phase = Phase::Queued;
                }
                return;
            }
            let installed = !m.arquivos.is_empty() && m.arquivos.iter().all(|f| std::fs::metadata(s.dest.join(&f.nome)).is_ok_and(|md| md.len() == f.bytes));
            // (carried by the system: nothing to download, and nothing of ours to verify or to delete)
            let in_system = !installed && !m.arquivos.is_empty() && m.arquivos.iter().all(|f| s.system.iter().any(|d| std::fs::metadata(d.join(&f.nome)).is_ok_and(|md| md.len() == f.bytes)));
            if in_system {
                s.items.push(Item { id: id.to_string(), phase: Phase::Installed { verified: false }, done: m.bytes(), total: m.bytes(), bytes_per_s: 0.0 });
                return;
            }
            let verified = installed && m.arquivos.iter().all(|f| is_verified(&s.dest, f));
            // (an installed file never checked, whose catalog has the sums, is queued: the engine hashes it and records it)
            let phase = if verified { Phase::Installed { verified } } else if installed && !m.verifiable() { Phase::Installed { verified: false } } else { Phase::Queued };
            let done = if installed { m.bytes() } else { 0 };
            s.items.push(Item { id: id.to_string(), phase, done, total: m.bytes(), bytes_per_s: 0.0 });
        });
    }

    pub fn pause(&self, id: &str) {
        self.with(|s| {
            if s.current.as_deref() == Some(id) {
                self.shared.ctl.pause.store(true, std::sync::atomic::Ordering::Relaxed);
            } else if let Some(i) = s.items.iter_mut().find(|i| i.id == id && matches!(i.phase, Phase::Queued | Phase::WaitingNetwork)) {
                i.phase = Phase::Paused;
            }
        });
    }

    pub fn resume(&self, id: &str) {
        self.with(|s| {
            if let Some(i) = s.items.iter_mut().find(|i| i.id == id && i.phase == Phase::Paused) {
                i.phase = Phase::Queued;
            }
        });
    }

    /// Gives up: the part file is deleted and the model leaves the queue.
    pub fn cancel(&self, id: &str) {
        self.with(|s| {
            if s.current.as_deref() == Some(id) {
                self.shared.ctl.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            } else if let Some(pos) = s.items.iter().position(|i| i.id == id && !matches!(i.phase, Phase::Installed { .. })) {
                s.items.remove(pos);
                if let Some(m) = s.catalog.iter().find(|m| m.id == id) {
                    for f in &m.arquivos {
                        let _ = std::fs::remove_file(s.dest.join(format!("{}.part", f.nome)));
                    }
                }
            }
        });
    }

    /// Try again now (after a failure).
    pub fn retry(&self, id: &str) {
        self.with(|s| {
            if let Some(i) = s.items.iter_mut().find(|i| i.id == id && matches!(i.phase, Phase::Failed(_) | Phase::WaitingNetwork)) {
                i.phase = Phase::Queued;
            }
        });
    }

    /// Deletes an installed model's files.
    pub fn remove(&self, id: &str) {
        self.with(|s| {
            if let Some(m) = s.catalog.iter().find(|m| m.id == id) {
                remove_files(&s.dest, &m.arquivos);
            }
            s.items.retain(|i| i.id != id);
        });
    }

    /// Tests: waits until nothing is queued, running or waiting for the network.
    pub fn wait_idle(&self, timeout: Duration) -> bool {
        let end = Instant::now() + timeout;
        while Instant::now() < end {
            if self.snapshot().iter().all(|i| !matches!(i.phase, Phase::Queued | Phase::Downloading | Phase::Verifying | Phase::Retrying { .. })) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    pub fn stop(&self) {
        self.with(|s| s.stop = true);
        self.shared.ctl.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

fn worker(sh: Arc<Shared>) {
    loop {
        // the next queued model (the network's return re-queues the ones that waited for it)
        let (id, m, dest, source) = {
            let mut g = sh.state.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if g.stop {
                    return;
                }
                if crate::net::network_up() {
                    for i in g.items.iter_mut().filter(|i| i.phase == Phase::WaitingNetwork) {
                        i.phase = Phase::Queued;
                    }
                }
                if let Some(i) = g.items.iter().find(|i| i.phase == Phase::Queued).map(|i| i.id.clone()) {
                    // (the licence is checked here, where every download passes: whatever asked for it, a model whose
                    // licence the person has not accepted is refused, and asking again after accepting queues it again)
                    let on_disk = g.catalog.iter().find(|m| m.id == i).is_some_and(|m| !m.arquivos.is_empty() && m.arquivos.iter().all(|f| g.dest.join(&f.nome).is_file()));
                    if !on_disk && crate::licenca::falta_aceite(&g.licences, &i) {
                        if let Some(it) = g.items.iter_mut().find(|it| it.id == i) {
                            it.phase = Phase::Failed(Failure::LicenseNotAccepted);
                        }
                        continue;
                    }
                    if let Some(m) = g.catalog.iter().find(|m| m.id == i).cloned() {
                        break (i, m, g.dest.clone(), g.source.clone());
                    }
                }
                g = sh.cv.wait_timeout(g, Duration::from_millis(500)).unwrap_or_else(|e| e.into_inner()).0;
            }
        };
        sh.ctl.reset();
        {
            let mut g = sh.state.lock().unwrap_or_else(|e| e.into_inner());
            g.current = Some(id.clone());
            if let Some(i) = g.items.iter_mut().find(|i| i.id == id) {
                i.phase = Phase::Downloading;
            }
        }
        let opts = (sh.opts)();
        let mut last = Instant::now() - Duration::from_secs(1);
        let r = download_files(&m.arquivos, &dest, &source, &sh.ctl, &opts, &mut |e| {
            let mut g = sh.state.lock().unwrap_or_else(|e| e.into_inner());
            let Some(i) = g.items.iter_mut().find(|i| i.id == id) else { return };
            match e {
                Event::Progress { done, total, bytes_per_s, .. } => {
                    if last.elapsed() < Duration::from_millis(100) {
                        return;
                    }
                    last = Instant::now();
                    (i.phase, i.done, i.total, i.bytes_per_s) = (Phase::Downloading, done, total, bytes_per_s);
                }
                Event::Verifying { .. } => i.phase = Phase::Verifying,
                Event::Retry { attempt, max, wait, .. } => i.phase = Phase::Retrying { attempt, max, wait_s: wait.as_secs().max(1) },
            }
        });
        let mut g = sh.state.lock().unwrap_or_else(|e| e.into_inner());
        g.current = None;
        match r {
            Ok(()) => {
                if let Some(i) = g.items.iter_mut().find(|i| i.id == id) {
                    i.done = i.total;
                    i.bytes_per_s = 0.0;
                    i.phase = Phase::Installed { verified: m.arquivos.iter().all(|f| is_verified(&dest, f)) };
                }
            }
            Err(DownloadError::Cancelled) => g.items.retain(|i| i.id != id),
            Err(e) => {
                if let Some(i) = g.items.iter_mut().find(|i| i.id == id) {
                    i.bytes_per_s = 0.0;
                    i.phase = match &e {
                        DownloadError::Paused => Phase::Paused,
                        DownloadError::Offline => Phase::WaitingNetwork,
                        e => Phase::Failed(failure_of(e)),
                    };
                }
            }
        }
    }
}
