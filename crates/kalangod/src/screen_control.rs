//! Screen-control sessions: one per person at a time, approved for a window (or the whole screen) and a time, with
//! a cap on actions. The session owns its screen helper (kalango-tela, started as the person in their session) and
//! the helper's line in niri's allow file (patch 0008, with the word `agente`: niri then tells kalangod when the
//! person uses their own mouse or keyboard — patch 0011 — and the session ends, the agent yields).
//!
//! The rules themselves are in `screen.rs`; this module holds the state, the helper and the shell's indicator.

use crate::peer::Peer;
use crate::policy::Config;
use crate::screen::{self, Facts, ImageMap, Rect, Scope, Screen};
use kalango_i18n::{tr, trf};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::process::CommandExt;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// The minimum time between two actions of a session (the rate limit).
pub const MIN_INTERVAL_MS: u64 = 150;
/// An approval decided this soon after an agent's click or key is refused (it could be the agent's own).
pub const DECISION_GUARD: Duration = Duration::from_secs(2);

pub struct Helper {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    pub pid: i32,
}

impl Helper {
    pub fn request(&mut self, r: Value) -> Result<Value, String> {
        writeln!(self.stdin, "{r}").and_then(|_| self.stdin.flush()).map_err(|e| trf("the screen helper stopped: {e}", &[("e", e.to_string())]))?;
        let mut line = String::new();
        if self.stdout.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
            return Err(tr("the screen helper stopped").into());
        }
        let v: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
        if v["ok"] == true { Ok(v) } else { Err(v["erro"].as_str().unwrap_or("?").to_string()) }
    }
}

impl Drop for Helper {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The session environment of the process that asked (its compositor and buses), checked: the sockets belong to
/// the person. kalangod (root) never trusts a path it cannot check.
pub fn session_env(pid: i32, uid: u32) -> Result<Vec<(String, String)>, String> {
    use std::os::unix::fs::MetadataExt;
    let raw = std::fs::read(format!("/proc/{pid}/environ")).map_err(|e| trf("cannot read the session of process {pid}: {e}", &[("pid", pid.to_string()), ("e", e.to_string())]))?;
    let env: HashMap<String, String> = raw.split(|b| *b == 0).filter_map(|kv| {
        let s = String::from_utf8_lossy(kv);
        s.split_once('=').map(|(k, v)| (k.to_string(), v.to_string()))
    }).collect();
    let runtime = env.get("XDG_RUNTIME_DIR").cloned().unwrap_or_else(|| format!("/run/user/{uid}"));
    let wayland = env.get("WAYLAND_DISPLAY").cloned().ok_or_else(|| tr("the agent does not run in a graphical session (no WAYLAND_DISPLAY)").to_string())?;
    let niri = env.get("NIRI_SOCKET").cloned().ok_or_else(|| tr("the agent does not run in a niri session (no NIRI_SOCKET)").to_string())?;
    let wl_path = if wayland.starts_with('/') { wayland.clone() } else { format!("{runtime}/{wayland}") };
    for p in [&wl_path, &niri] {
        let m = std::fs::metadata(p).map_err(|e| format!("{p}: {e}"))?;
        if m.uid() != uid {
            return Err(trf("{path} is not this person's", &[("path", p.to_string())]));
        }
    }
    let mut out = vec![("XDG_RUNTIME_DIR".into(), runtime), ("WAYLAND_DISPLAY".into(), wayland), ("NIRI_SOCKET".into(), niri)];
    for k in ["DBUS_SESSION_BUS_ADDRESS", "LANG", "LC_ALL", "HOME"] {
        if let Some(v) = env.get(k) {
            out.push((k.into(), v.clone()));
        }
    }
    Ok(out)
}

/// Starts the helper as the person (setuid when kalangod is root). It ends when kalangod closes its stdin or kills it
/// (no PR_SET_PDEATHSIG: that follows the thread that started it, and each client's connection is a thread).
pub fn start_helper(cfg: &Config, peer: &Peer) -> Result<Helper, String> {
    let env = session_env(peer.pid, peer.uid)?;
    let mut cmd = Command::new(&cfg.tela_auxiliar);
    cmd.env_clear().envs(env).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    let (uid, gid) = (peer.uid, peer.gid);
    let root = nix::unistd::geteuid().is_root();
    unsafe {
        cmd.pre_exec(move || {
            if root && (libc::setgroups(0, std::ptr::null()) != 0 || libc::setgid(gid) != 0 || libc::setuid(uid) != 0) {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = cmd.spawn().map_err(|e| trf("the screen helper {path} did not start: {e}", &[("path", cfg.tela_auxiliar.display().to_string()), ("e", e.to_string())]))?;
    let stdin = child.stdin.take().ok_or("no stdin")?;
    let stdout = BufReader::new(child.stdout.take().ok_or("no stdout")?);
    let pid = child.id() as i32;
    Ok(Helper { child, stdin, stdout, pid })
}

/// The compositor of both sockets must be the same niri (a fake IPC server could hide an approval on screen).
pub fn check_compositor(state: &Value) -> Result<(), String> {
    let ipc = state["compositor"]["ipc"].as_i64();
    let wl = state["compositor"]["wayland"].as_i64();
    match (ipc, wl) {
        (Some(a), Some(b)) if a == b => {
            let exe = std::fs::read_link(format!("/proc/{a}/exe")).map(|p| p.display().to_string()).unwrap_or_default();
            let name = std::path::Path::new(&exe).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            if name.starts_with("niri") { Ok(()) } else { Err(trf("the compositor is not niri ({exe})", &[("exe", exe)])) }
        }
        _ => Err(tr("the window list and the screen do not come from the same compositor").into()),
    }
}

pub struct Session {
    pub uid: u32,
    pub agent: String,
    pub scope: Scope,
    pub scope_title: String,
    pub until: Instant,
    pub minutes: u64,
    pub max_actions: u32,
    pub actions: u32,
    pub last_action: Option<Instant>,
    pub helper: Helper,
    /// the last picture the agent saw in this session
    pub look: Look,
    /// where the agent's pointer is (output, x, y): the shell draws its name there
    pub pointer: Option<(String, f64, f64)>,
    /// the scope window's rectangle (the shell outlines it)
    pub rect: Option<(String, Rect)>,
}

#[derive(Default)]
struct Inner {
    sessions: HashMap<u32, Arc<Mutex<Session>>>,
    /// the indicator's view of each session
    snapshots: HashMap<u32, Value>,
    /// people whose screen is locked now (the shell tells)
    locked: std::collections::HashSet<u32>,
    /// when an agent last clicked or typed, per person (approvals decided right after are refused)
    injected: HashMap<u32, Instant>,
    /// how the last session of each person ended: (reason, when, actions, the agent's name)
    ended: HashMap<u32, (String, u64, u32, String)>,
    /// the banner's Stop button on screen, per person (the shell tells): (output, rectangle)
    stop_buttons: HashMap<u32, (String, Rect)>,
    /// the last picture an agent looked at, per person (an action before a session starts aims with it)
    pub looks: HashMap<u32, Look>,
    generation: u64,
}

/// A picture the agent saw: its mapping and the elements shown with it.
#[derive(Clone, Default)]
pub struct Look {
    pub output: String,
    pub map: Option<ImageMap>,
    pub centres: Vec<(String, f64, f64)>,
    pub names: Vec<(String, String)>,
    pub elements: Vec<Value>,
}

#[derive(Default)]
pub struct ScreenControl {
    inner: Mutex<Inner>,
    cv: Condvar,
    /// each person's helper pid, outside the main lock: stopping kills it at once, even in the middle of typing
    helpers: Mutex<HashMap<u32, i32>>,
}

/// Why a session ended (the shell shows it, the timeline records it).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EndReason {
    Person,
    Stopped,
    Locked,
    Expired,
    Limit,
    Agent,
}

impl EndReason {
    pub fn word(self) -> &'static str {
        match self {
            EndReason::Person => "pessoa",
            EndReason::Stopped => "parada",
            EndReason::Locked => "bloqueio",
            EndReason::Expired => "tempo",
            EndReason::Limit => "limite",
            EndReason::Agent => "agente",
        }
    }

    pub fn text(self) -> String {
        match self {
            EndReason::Person => tr("the person used the mouse or the keyboard: the agent stopped and waits for them").into(),
            EndReason::Stopped => tr("the person stopped the agent's control of the screen").into(),
            EndReason::Locked => tr("the screen was locked").into(),
            EndReason::Expired => tr("the time the person allowed is over").into(),
            EndReason::Limit => tr("the session reached its maximum number of actions").into(),
            EndReason::Agent => tr("the agent finished").into(),
        }
    }
}

impl ScreenControl {
    fn bump(&self, g: &mut Inner) {
        g.generation += 1;
        self.cv.notify_all();
    }

    pub fn has_session(&self, uid: u32) -> bool {
        self.inner.lock().unwrap().sessions.contains_key(&uid)
    }

    pub fn is_locked(&self, uid: u32) -> bool {
        self.inner.lock().unwrap().locked.contains(&uid)
    }

    pub fn set_locked(&self, uid: u32, on: bool) {
        let mut g = self.inner.lock().unwrap();
        if on {
            g.locked.insert(uid);
        } else {
            g.locked.remove(&uid);
        }
        self.bump(&mut g);
    }

    /// An agent clicked or typed for this person less than `DECISION_GUARD` ago.
    pub fn injected_recently(&self, uid: u32) -> bool {
        self.inner.lock().unwrap().injected.get(&uid).is_some_and(|t| t.elapsed() < DECISION_GUARD)
    }

    pub fn mark_injected(&self, uid: u32) {
        self.inner.lock().unwrap().injected.insert(uid, Instant::now());
    }

    /// Ends the person's session: the helper is killed at once (even in the middle of typing) and with it every
    /// protocol object it held; its grant goes.
    pub fn end(&self, uid: u32, why: EndReason, rc: &crate::remote_control::RemoteControl) -> Option<(String, u32)> {
        if let Some(pid) = self.helpers.lock().unwrap().remove(&uid) {
            rc.revoke_own(pid);
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
        let mut g = self.inner.lock().unwrap();
        let s = g.sessions.remove(&uid)?;
        let snap = g.snapshots.remove(&uid).unwrap_or_default();
        let n = snap["acoes"].as_u64().unwrap_or(0) as u32;
        let agent = snap["agente"].as_str().unwrap_or("").to_string();
        g.ended.insert(uid, (why.word().to_string(), crate::timeline::now(), n, agent));
        self.bump(&mut g);
        drop(g);
        drop(s);
        Some((snap["janela"].as_str().unwrap_or("").to_string(), n))
    }

    pub fn insert(&self, s: Session) {
        self.helpers.lock().unwrap().insert(s.uid, s.helper.pid);
        let mut g = self.inner.lock().unwrap();
        let uid = s.uid;
        g.ended.remove(&uid);
        g.snapshots.insert(uid, snapshot(&s));
        g.sessions.insert(uid, Arc::new(Mutex::new(s)));
        self.bump(&mut g);
    }

    /// Runs `f` with the person's session (None: there is none). The session's own lock, not the whole state's:
    /// stopping, the indicator and the approvals never wait for an action.
    pub fn with<R>(&self, uid: u32, f: impl FnOnce(&mut Session) -> R) -> Option<R> {
        let arc = self.inner.lock().unwrap().sessions.get(&uid).cloned()?;
        let mut s = arc.lock().unwrap_or_else(|e| e.into_inner());
        let r = f(&mut s);
        let snap = snapshot(&s);
        drop(s);
        let mut g = self.inner.lock().unwrap();
        if g.sessions.get(&uid).is_some_and(|a| Arc::ptr_eq(a, &arc)) {
            g.snapshots.insert(uid, snap);
        }
        self.bump(&mut g);
        Some(r)
    }

    /// How the person's last session ended, if it was the person (or the lock) that ended it lately: the agent yields.
    pub fn yielded(&self, uid: u32) -> Option<String> {
        let g = self.inner.lock().unwrap();
        let (why, when, _, _) = g.ended.get(&uid)?;
        (crate::timeline::now().saturating_sub(*when) < 600 && matches!(why.as_str(), "pessoa" | "parada" | "bloqueio")).then(|| why.clone())
    }

    /// Where the banner's Stop button is now (None: no banner).
    pub fn set_stop_button(&self, uid: u32, at: Option<(String, Rect)>) {
        let mut g = self.inner.lock().unwrap();
        match at {
            Some(a) => g.stop_buttons.insert(uid, a),
            None => g.stop_buttons.remove(&uid),
        };
    }

    pub fn stop_button(&self, uid: u32) -> Option<(String, Rect)> {
        self.inner.lock().unwrap().stop_buttons.get(&uid).cloned()
    }

    pub fn set_look(&self, uid: u32, look: Look) {
        self.inner.lock().unwrap().looks.insert(uid, look);
    }

    pub fn look(&self, uid: u32) -> Option<Look> {
        self.inner.lock().unwrap().looks.get(&uid).cloned()
    }

    /// The indicator's state (for the shell), after `since` changes (long wait, at most `max`).
    pub fn wait_state(&self, uid: u32, since: u64, max: Duration) -> Value {
        let deadline = Instant::now() + max;
        let mut g = self.inner.lock().unwrap();
        while g.generation == since {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            g = self.cv.wait_timeout(g, left).unwrap().0;
        }
        let mut v = json!({"geracao": g.generation, "ativa": false, "bloqueada": g.locked.contains(&uid)});
        if let Some(snap) = g.snapshots.get(&uid) {
            v = snap.clone();
            v["geracao"] = json!(g.generation);
            v["ativa"] = json!(true);
            v["bloqueada"] = json!(g.locked.contains(&uid));
            let until = snap["ate_ms"].as_u64().unwrap_or(0);
            v["restante_s"] = json!(until.saturating_sub(now_ms()) / 1000);
        }
        if let Some((why, when, n, agent)) = g.ended.get(&uid) {
            v["fim"] = json!({"motivo": why, "quando": when, "acoes": n, "agente": agent});
        }
        v
    }

    /// Sessions whose time is over (checked on every action and every indicator request).
    pub fn expired(&self) -> Vec<u32> {
        let g = self.inner.lock().unwrap();
        g.snapshots.iter().filter(|(_, s)| s["ate_ms"].as_u64().unwrap_or(0) <= now_ms()).map(|(u, _)| *u).collect()
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// What the indicator shows of a session (kept outside its lock).
fn snapshot(s: &Session) -> Value {
    let mut v = json!({
        "agente": s.agent, "janela": s.scope_title, "tela_inteira": matches!(s.scope, Scope::Screen { .. }),
        "acoes": s.actions, "max_acoes": s.max_actions, "minutos": s.minutes,
        "ate_ms": now_ms() + s.until.saturating_duration_since(Instant::now()).as_millis() as u64,
    });
    if let Scope::Window { id, .. } = s.scope {
        v["janela_id"] = json!(id);
    }
    if let Some((o, x, y)) = &s.pointer {
        v["ponteiro"] = json!({"saida": o, "x": x, "y": y});
    }
    if let Some((o, r)) = &s.rect {
        v["retangulo"] = json!({"saida": o, "x": r.x, "y": r.y, "w": r.w, "h": r.h});
    }
    v
}

/// The screen now, from the helper.
pub fn read_screen(h: &mut Helper) -> Result<(Value, Screen), String> {
    let state = h.request(json!({"op": "estado"}))?;
    check_compositor(&state)?;
    let screen = screen::parse_screen(&state);
    Ok((state, screen))
}

/// The facts besides the screen for a person.
pub fn facts(sc: &ScreenControl, approvals: &crate::approval::Approvals, uid: u32) -> Facts {
    Facts { locked: sc.is_locked(uid), pending_approvals: approvals.list().iter().filter(|p| p.uid == uid).count(), stop_button: sc.stop_button(uid) }
}
