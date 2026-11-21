//! Files that run at the person's next login or in every terminal they open (`policy::STARTUP_PATHS`).
//!
//! An agent's change to one of them would run later as the person, outside every rule the agent runs under. The
//! person chooses (Agents center › Permissions, "Files that run at login"):
//! - "perguntar" (the default): every change asks on the system approval screen, with the exact diff, and the
//!   answer can never be "always"; commands the agents run cannot write them at all (Landlock: see `carve`), so an
//!   agent that wants to change one uses the file tools, which ask;
//! - "livre": changes go ahead; each one is still recorded in the timeline with undo, marked so the agents center
//!   tells the person a startup file changed (a command's changes too: see `snapshot`).
//!
//! The choice is read here, by kalangod, from the person's settings file under `~/.config/kalango`, a protected
//! folder no agent reaches (`policy::KALANGO_PROTECTED`): never from what an agent sends.

use crate::peer::Peer;
use crate::policy::{expand, is_within, normalize, Config};
use crate::sandbox::resolve_existing;
use kalango_i18n::{tr, trf};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Ask,
    Allow,
}

/// The person's choice, from their settings file. Anything but an explicit "livre" asks: a missing, unreadable or
/// broken file never opens the door.
pub fn mode(cfg: &Config, peer: &Peer) -> Mode {
    mode_from_file(&expand(&cfg.startup_setting, &peer.home))
}

pub fn mode_from_file(p: &Path) -> Mode {
    // (read again only when the file changed: every file tool call asks)
    use std::os::unix::fs::MetadataExt;
    type Seen = std::collections::HashMap<PathBuf, ((u64, u64, i64, i64, u64), Mode)>;
    static CACHE: std::sync::Mutex<Option<Seen>> = std::sync::Mutex::new(None);
    let key = std::fs::symlink_metadata(p).ok().map(|m| (m.dev(), m.ino(), m.mtime(), m.mtime_nsec(), m.len()));
    if let Some(k) = key {
        if let Some((_, m)) = CACHE.lock().unwrap().as_ref().and_then(|c| c.get(p)).filter(|(kk, _)| *kk == k) {
            return *m;
        }
    }
    let m = read_mode(p);
    if let Some(k) = key {
        CACHE.lock().unwrap().get_or_insert_with(Default::default).insert(p.to_path_buf(), (k, m));
    }
    m
}

fn read_mode(p: &Path) -> Mode {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    // (not through a link: the file is the person's own, in a protected folder)
    let Ok(f) = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW).open(p) else { return Mode::Ask };
    let mut text = String::new();
    if f.take(1 << 20).read_to_string(&mut text).is_err() {
        return Mode::Ask;
    }
    let v: Value = serde_json::from_str(&text).unwrap_or_default();
    if v["arquivos_de_inicio"].as_str() == Some("livre") { Mode::Allow } else { Mode::Ask }
}

/// The startup paths of a person: the configured list (`policy::STARTUP_PATHS`, where a `*` part stands for every
/// folder there, e.g. each node version of nvm) and every folder of their PATH that is inside their home (a program
/// written there under a common name runs the next time they type it).
pub fn paths(cfg: &Config, peer: &Peer) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = Vec::new();
    for s in &cfg.startup_paths {
        for p in expand_glob(&normalize(&expand(s, &peer.home))) {
            if !v.contains(&p) {
                v.push(p);
            }
        }
    }
    for d in login_path_dirs(cfg, peer) {
        if !v.contains(&d) {
            v.push(d);
        }
    }
    v
}

/// A path whose parts may be `*` (any folder there): the paths it names now.
pub fn expand_glob(p: &Path) -> Vec<PathBuf> {
    let mut out = vec![PathBuf::new()];
    for c in p.components() {
        if c.as_os_str() == "*" {
            out = out.iter().flat_map(|d| std::fs::read_dir(d).into_iter().flatten().flatten().map(|e| e.path()).filter(|x| x.is_dir()).collect::<Vec<_>>()).collect();
        } else {
            out.iter_mut().for_each(|d| d.push(c.as_os_str()));
        }
    }
    out
}

/// The folders of the person's PATH inside their home: from the config's `login_path` (tests), else the union of what
/// their own processes outside the agents' cgroup have (`processes_path`) and what their shell gives at login and in a
/// terminal (`shell_path`). Only ever adds: nothing an agent does removes a folder from the list.
pub fn login_path_dirs(cfg: &Config, peer: &Peer) -> Vec<PathBuf> {
    let path = match &cfg.login_path {
        Some(p) => p.clone(),
        None => format!("{}:{}", processes_path(peer.uid, Path::new("/proc")), shell_path(peer)),
    };
    dirs_in_home(&path, &peer.home)
}

/// The folders of a PATH value that are inside `home` (with `~` and `$HOME` expanded).
pub fn dirs_in_home(path: &str, home: &Path) -> Vec<PathBuf> {
    let mut v = Vec::new();
    for d in path.split(':').filter(|d| !d.is_empty()) {
        let d = d.replace("$HOME", &home.display().to_string());
        let d = normalize(&expand(&d, home));
        if d.is_absolute() && is_within(&d, home) && d != normalize(home) && !v.contains(&d) {
            v.push(d);
        }
    }
    v
}

/// The union of the PATH of every process the person owns outside the agents' cgroup: their user manager's
/// (user@UID.service: the terminals and the apps they started, where nvm, conda and similar have added theirs) and
/// their login sessions'. Read at most every ten seconds; what was once seen stays (only added to, until restart).
pub fn processes_path(uid: u32, proc_root: &Path) -> String {
    type Seen = std::collections::HashMap<(u32, PathBuf), (std::time::Instant, Vec<String>, bool)>;
    static SEEN: std::sync::Mutex<Option<Seen>> = std::sync::Mutex::new(None);
    let key = (uid, proc_root.to_path_buf());
    let add = |key: &(u32, PathBuf), found: Vec<String>| {
        let mut g = SEEN.lock().unwrap();
        let e = g.get_or_insert_with(Default::default).entry(key.clone()).or_insert_with(|| (std::time::Instant::now(), vec![], false));
        for d in found {
            if !e.1.contains(&d) {
                e.1.push(d);
            }
        }
        e.0 = std::time::Instant::now();
        e.2 = false;
    };
    let state = SEEN.lock().unwrap().as_ref().and_then(|m| m.get(&key)).map(|(t, d, r)| (t.elapsed(), d.join(":"), *r));
    match state {
        // the first time: read now
        None => {
            add(&key, scan_processes_path(uid, proc_root));
        }
        // read again in the background every ten seconds (a scan of /proc can take a while), the union meanwhile
        Some((age, _, false)) if age > std::time::Duration::from_secs(10) => {
            if let Some(e) = SEEN.lock().unwrap().as_mut().and_then(|m| m.get_mut(&key)) {
                e.2 = true;
            }
            let k = key.clone();
            std::thread::spawn(move || add(&k, scan_processes_path(k.0, &k.1)));
        }
        _ => {}
    }
    SEEN.lock().unwrap().as_ref().and_then(|m| m.get(&key)).map(|(_, d, _)| d.join(":")).unwrap_or_default()
}

fn scan_processes_path(uid: u32, proc_root: &Path) -> Vec<String> {
    use std::os::unix::fs::MetadataExt;
    let mut dirs: Vec<String> = Vec::new();
    let mine = format!("/user@{uid}.service/");
    for e in std::fs::read_dir(proc_root).into_iter().flatten().flatten() {
        let p = e.path();
        if std::fs::metadata(&p).map(|m| m.uid() != uid).unwrap_or(true) {
            continue;
        }
        let cg = std::fs::read_to_string(p.join("cgroup")).unwrap_or_default();
        // (an agent's process never counts: what it sets in its own environment is not the person's PATH)
        if cg.contains("kalango-agentes.slice") || !(cg.contains(&mine) || cg.contains("/session-")) {
            continue;
        }
        let env = std::fs::read(p.join("environ")).unwrap_or_default();
        if let Some(v) = env.split(|b| *b == 0).find_map(|kv| kv.strip_prefix(b"PATH=")) {
            for d in String::from_utf8_lossy(v).split(':') {
                if !d.is_empty() && !dirs.iter().any(|x| x == d) {
                    dirs.push(d.to_string());
                }
            }
        }
    }
    dirs
}

/// The files a login or terminal shell reads (the person's and the system's): when one of them changes, what the
/// person's shell gives as PATH is evaluated again.
fn rc_files(home: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = [".bashrc", ".bash_profile", ".bash_login", ".profile", ".bash_aliases", ".zshrc", ".zshenv", ".zprofile", ".zlogin",
        ".config/fish/config.fish", ".pam_environment", ".bashrc.d", ".config/environment.d", ".config/fish/conf.d", ".config/zsh"]
        .iter().map(|f| home.join(f)).collect();
    v.extend(["/etc/profile", "/etc/bash.bashrc", "/etc/profile.d", "/etc/environment", "/etc/zsh"].iter().map(PathBuf::from));
    // the files of those folders too (a folder's mtime moves when an entry is added, not when one is edited)
    let dirs: Vec<PathBuf> = v.iter().filter(|p| p.is_dir()).cloned().collect();
    for d in dirs {
        v.extend(std::fs::read_dir(&d).into_iter().flatten().flatten().map(|e| e.path()));
    }
    v
}

/// The rc files' state: when it differs from the last evaluation's, the shell is evaluated again (as soon as a file
/// changes — the next check notices it — not after a time).
fn rc_fingerprint(home: &Path) -> Vec<(PathBuf, Option<(i64, i64, u64, u64)>)> {
    use std::os::unix::fs::MetadataExt;
    rc_files(home).into_iter().map(|f| {
        let m = std::fs::metadata(&f).ok().map(|m| (m.mtime(), m.mtime_nsec(), m.len(), m.ino()));
        (f, m)
    }).collect()
}

/// The PATH the person's own shell gives at login (`-lic`) and in a terminal (`-ic`): their shell from passwd, run as
/// them by kalangod (outside the agents' cgroup), with a clean environment, no input and a five-second limit; kept
/// until one of the rc files changes. (These files run at the person's every login anyway; a file that execs another
/// shell or waits just gives nothing here, and the other sources still count.)
pub fn shell_path(peer: &Peer) -> String {
    type Fp = Vec<(PathBuf, Option<(i64, i64, u64, u64)>)>;
    type Kept = std::collections::HashMap<(u32, PathBuf), (Fp, String, bool)>;
    static KEPT: std::sync::Mutex<Option<Kept>> = std::sync::Mutex::new(None);
    let fp = rc_fingerprint(&peer.passwd_home);
    let evaluate = {
        let peer = peer.clone();
        move || {
            let shell = nix::unistd::User::from_uid(nix::unistd::Uid::from_raw(peer.uid)).ok().flatten().map(|u| u.shell).unwrap_or_else(|| "/bin/sh".into());
            [&["-lic"][..], &["-ic"][..]].iter().map(|flags| run_shell_for_path(&shell, flags, &peer)).collect::<Vec<_>>().join(":")
        }
    };
    let key = (peer.uid, peer.passwd_home.clone());
    let mut g = KEPT.lock().unwrap();
    let k = g.get_or_insert_with(Default::default);
    match k.get_mut(&key) {
        Some((f, p, _)) if *f == fp => p.clone(),
        // an rc file changed: evaluated again in the background (the shell can take seconds), the last value meanwhile
        // — still a startup file change itself, asked for while agents are asked, so nothing slips in through the gap
        Some((f, p, refreshing)) => {
            if !*refreshing {
                *refreshing = true;
                let (key2, fp2) = (key.clone(), fp.clone());
                std::thread::spawn(move || {
                    let new = evaluate();
                    if let Some((f, p, r)) = KEPT.lock().unwrap().as_mut().and_then(|k| k.get_mut(&key2)) {
                        // (only added to: what a folder was stays)
                        *p = format!("{p}:{new}");
                        *f = fp2;
                        *r = false;
                    }
                });
            }
            let _ = f;
            p.clone()
        }
        // the first time: evaluated now
        None => {
            drop(g);
            let p = evaluate();
            KEPT.lock().unwrap().get_or_insert_with(Default::default).insert(key, (fp, p.clone(), false));
            p
        }
    }
}

/// Runs `shell FLAGS 'printf …$PATH'` as the person and reads the PATH it prints after a marker (an rc file may print
/// anything before it). Its whole process group is killed after (nothing an rc file starts stays behind).
pub fn run_shell_for_path(shell: &Path, flags: &[&str], peer: &Peer) -> String {
    use std::io::Read;
    use std::os::unix::process::CommandExt;
    const MARK: &str = "__KALANGO_PATH__";
    let mut cmd = std::process::Command::new(shell);
    cmd.args(flags).arg(format!("printf '\\n{MARK}%s\\n' \"$PATH\""))
        .env_clear().env("HOME", &peer.passwd_home).env("USER", &peer.user).env("LOGNAME", &peer.user).env("SHELL", shell)
        .env("TERM", "dumb").env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .current_dir(&peer.passwd_home).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null())
        .process_group(0);
    if nix::unistd::geteuid().is_root() && peer.uid != 0 {
        cmd.uid(peer.uid).gid(peer.gid);
    }
    let Ok(mut child) = cmd.spawn() else { return String::new() };
    let pgid = child.id() as i32;
    let mut out = child.stdout.take();
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(o) = out.as_mut() {
            let _ = o.take(1 << 20).read_to_string(&mut s);
        }
        s
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        if let Ok(Some(_)) = child.try_wait() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let _ = nix::sys::signal::killpg(nix::unistd::Pid::from_raw(pgid), nix::sys::signal::Signal::SIGKILL);
    let _ = child.wait();
    let s = reader.join().unwrap_or_default();
    s.lines().rev().find_map(|l| l.strip_prefix(MARK)).unwrap_or("").to_string()
}

/// The startup path that `raw` is in, leads to (through `..` or links, also dangling ones) or holds (a folder that
/// holds `~/.config/autostart`, moved away and back, would carry new entries in). None: not a startup file.
pub fn startup_base(raw: &str, peer: &Peer, cfg: &Config) -> Option<PathBuf> {
    let p = expand(raw, &peer.home);
    if !p.is_absolute() {
        return None;
    }
    let p = normalize(&p);
    let resolved = resolve_existing(&p);
    let by_path = resolved_paths(cfg, peer).into_iter().find(|(base, rbase)| {
        is_within(&p, base) || is_within(&resolved, base) || is_within(&resolved, rbase)
            || is_within(base, &p) || is_within(base, &resolved) || is_within(rbase, &resolved)
    }).map(|(base, _)| base);
    // ("*" entries as patterns too: a version folder made a moment ago counts at once)
    let by_pattern = || cfg.startup_paths.iter().filter(|s| s.contains('*')).map(|s| normalize(&expand(s, &peer.home)))
        .find(|pat| [&p, &resolved].iter().any(|q| matches_pattern(q, pat) || holds_pattern(q, pat)));
    by_path.or_else(by_pattern).or_else(|| hard_linked_file(&resolved))
}

/// `p` is a folder above what `pattern` names (moving it would carry them).
fn holds_pattern(p: &Path, pattern: &Path) -> bool {
    let (pc, qc): (Vec<_>, Vec<_>) = (p.components().collect(), pattern.components().collect());
    pc.len() < qc.len() && pc.iter().zip(&qc).all(|(a, b)| b.as_os_str() == "*" || a == b)
}

/// `p` is, or is inside, the path `pattern` names, where a `*` part matches any one folder.
fn matches_pattern(p: &Path, pattern: &Path) -> bool {
    let mut pc = p.components();
    pattern.components().all(|c| pc.next().is_some_and(|x| c.as_os_str() == "*" || x == c))
}

/// The startup paths with where they really are (links followed), kept for a few seconds: every file tool call checks
/// against all of them. (Only the person changes where they lead while agents are asked: the folders holding them are
/// read-only for agents' commands, and the file tools ask before touching them.)
fn resolved_paths(cfg: &Config, peer: &Peer) -> Vec<(PathBuf, PathBuf)> {
    type Seen = std::collections::HashMap<(u32, PathBuf), (std::time::Instant, Vec<(PathBuf, PathBuf)>)>;
    static CACHE: std::sync::Mutex<Option<Seen>> = std::sync::Mutex::new(None);
    let key = (peer.uid, peer.home.clone());
    if let Some((t, v)) = CACHE.lock().unwrap().as_ref().and_then(|c| c.get(&key)) {
        if t.elapsed() < std::time::Duration::from_secs(5) {
            return v.clone();
        }
    }
    let v: Vec<(PathBuf, PathBuf)> = paths(cfg, peer).into_iter().map(|b| { let r = resolve_existing(&b); (b, r) }).collect();
    CACHE.lock().unwrap().get_or_insert_with(Default::default).insert(key, (std::time::Instant::now(), v.clone()));
    v
}

/// A file with more than one name (a hard link): it may be a startup file under another name, wherever that is —
/// no search can be complete, so any change to such a file goes through the gate (asked, never "always").
fn hard_linked_file(p: &Path) -> Option<PathBuf> {
    use std::os::unix::fs::MetadataExt;
    std::fs::symlink_metadata(p).ok().filter(|m| m.is_file() && m.nlink() > 1).map(|_| p.to_path_buf())
}

/// The files there are now under the startup paths, all of them: (startup path, file).
pub fn startup_files(peer: &Peer, cfg: &Config) -> Vec<(PathBuf, PathBuf)> {
    let mut out = Vec::new();
    for base in paths(cfg, peer) {
        let mut pending = vec![(resolve_existing(&base), 0)];
        while let Some((f, depth)) = pending.pop() {
            let Ok(md) = std::fs::symlink_metadata(&f) else { continue };
            if md.is_dir() && depth < 32 {
                pending.extend(std::fs::read_dir(&f).into_iter().flatten().flatten().map(|e| (e.path(), depth + 1)));
            } else if md.is_file() {
                out.push((base.clone(), f));
            }
        }
    }
    out
}

/// A startup file with more than one name (a hard link somewhere else): a command could write it through the other
/// name, which Landlock (by path) does not see. While the person wants to be asked, commands are refused until the
/// extra name is gone.
pub fn hardlinked(peer: &Peer, cfg: &Config) -> Option<PathBuf> {
    use std::os::unix::fs::MetadataExt;
    // (not the program folders: they hold hard links by design — conda's are its package cache's — and they stay
    // read-only for commands anyway; the file tools ask before writing any file with another name)
    let programs: Vec<PathBuf> = crate::policy::PROGRAM_FOLDERS.iter().flat_map(|s| expand_glob(&normalize(&expand(s, &peer.home))))
        .chain(login_path_dirs(cfg, peer)).collect();
    startup_files(peer, cfg).into_iter().filter(|(base, _)| !programs.iter().any(|p| is_within(base, p)))
        .map(|(_, f)| f).find(|f| std::fs::symlink_metadata(f).map(|m| m.nlink() > 1).unwrap_or(false))
}

/// The paths a tool call would change on this computer (only the tools that change the person's files).
pub fn changed_paths(name: &str, args: &Value) -> Vec<String> {
    let get = |k: &str| args.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
    let mut out: Vec<String> = match name {
        "arquivos.escrever" | "arquivos.lixeira" | "arquivos.restaurar" | "arquivos.apagar_criado" | "vm.pastas" => get("caminho").into_iter().collect(),
        "arquivos.mover" => {
            let mut v: Vec<String> = [get("de"), get("para")].into_iter().flatten().collect();
            // moving into an existing folder lands at folder/name
            if let (Some(de), Some(para)) = (get("de"), get("para")) {
                if let Some(n) = Path::new(&de).file_name() {
                    v.push(Path::new(&para).join(n).display().to_string());
                }
            }
            v
        }
        "vm.copiar_arquivo" if args["sentido"].as_str() == Some("da_maquina") => get("para").into_iter().collect(),
        _ => vec![],
    };
    out.dedup();
    out
}

/// The startup files a call would change: (the path as given, the startup path it touches).
pub fn touched(name: &str, args: &Value, peer: &Peer, cfg: &Config) -> Vec<(String, PathBuf)> {
    changed_paths(name, args).into_iter().filter_map(|raw| startup_base(&raw, peer, cfg).map(|b| (raw, b))).collect()
}

/// A path as the person reads it: `~/.bashrc`.
pub fn tilde(p: &Path, home: &Path) -> String {
    match p.strip_prefix(home) {
        Ok(r) if r.as_os_str().is_empty() => "~".into(),
        Ok(r) => format!("~/{}", r.display()),
        Err(_) => p.display().to_string(),
    }
}

/// A touched path as the approval screen and the timeline name it: the startup file it leads to too, when it is
/// reached another way (`~/Documentos/notas.txt → ~/.bashrc`, through a link).
pub fn shown(raw: &str, base: &Path, home: &Path) -> String {
    let p = normalize(&expand(raw, home));
    if is_within(&p, base) || is_within(base, &p) { tilde(&p, home) } else { format!("{} → {}", tilde(&p, home), tilde(base, home)) }
}

// ------------------------------------------------------------------ the diff

/// One line of a diff: ' ' kept, '-' removed, '+' added, '@' a gap of unchanged lines left out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: char,
    pub text: String,
}

/// The lines that change from `old` to `new`, with `context` unchanged lines around each change (at most `max`
/// lines; a longer diff ends with a gap marker). Lines are compared whole.
pub fn line_diff(old: &str, new: &str, context: usize, max: usize) -> Vec<DiffLine> {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let mut pre = 0;
    while pre < a.len() && pre < b.len() && a[pre] == b[pre] {
        pre += 1;
    }
    let mut suf = 0;
    while suf < a.len() - pre && suf < b.len() - pre && a[a.len() - 1 - suf] == b[b.len() - 1 - suf] {
        suf += 1;
    }
    let (am, bm) = (&a[pre..a.len() - suf], &b[pre..b.len() - suf]);
    // the changed middle: a longest common subsequence when it is small enough, else all of it removed and added
    let mut ops: Vec<(char, &str)> = a[..pre].iter().map(|l| (' ', *l)).collect();
    if am.len() * bm.len() <= 4_000_000 {
        let (n, m) = (am.len(), bm.len());
        let mut lcs = vec![0u32; (n + 1) * (m + 1)];
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                lcs[i * (m + 1) + j] = if am[i] == bm[j] { lcs[(i + 1) * (m + 1) + j + 1] + 1 } else { lcs[(i + 1) * (m + 1) + j].max(lcs[i * (m + 1) + j + 1]) };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < n || j < m {
            if i < n && j < m && am[i] == bm[j] {
                ops.push((' ', am[i]));
                i += 1;
                j += 1;
            } else if i < n && (j == m || lcs[(i + 1) * (m + 1) + j] >= lcs[i * (m + 1) + j + 1]) {
                // (what goes away before what comes in, as diff shows it)
                ops.push(('-', am[i]));
                i += 1;
            } else {
                ops.push(('+', bm[j]));
                j += 1;
            }
        }
    } else {
        ops.extend(am.iter().map(|l| ('-', *l)));
        ops.extend(bm.iter().map(|l| ('+', *l)));
    }
    ops.extend(a[a.len() - suf..].iter().map(|l| (' ', *l)));
    // keep the changes and `context` lines around each
    let keep: Vec<bool> = (0..ops.len()).map(|i| {
        let lo = i.saturating_sub(context);
        let hi = (i + context + 1).min(ops.len());
        ops[lo..hi].iter().any(|(k, _)| *k != ' ')
    }).collect();
    let mut out = Vec::new();
    let mut gap = false;
    for (i, (k, l)) in ops.iter().enumerate() {
        if !keep[i] {
            gap = true;
            continue;
        }
        if gap && !out.is_empty() {
            out.push(DiffLine { kind: '@', text: String::new() });
        }
        gap = false;
        if out.len() >= max {
            out.push(DiffLine { kind: '@', text: String::new() });
            break;
        }
        out.push(DiffLine { kind: *k, text: l.to_string() });
    }
    out
}

/// "1 line added, 2 removed"-like summary of a diff.
pub fn diff_summary(d: &[DiffLine]) -> String {
    let add = d.iter().filter(|l| l.kind == '+').count();
    let del = d.iter().filter(|l| l.kind == '-').count();
    match (add, del) {
        (0, 0) => tr("nothing changes").into(),
        (1, 0) => tr("1 line added").into(),
        (0, 1) => tr("1 line removed").into(),
        (1, 1) => tr("1 line changes").into(),
        (a, 0) => trf("{n} lines added", &[("n", a.to_string())]),
        (0, r) => trf("{n} lines removed", &[("n", r.to_string())]),
        (a, r) => trf("{added} lines added, {removed} removed", &[("added", a.to_string()), ("removed", r.to_string())]),
    }
}

/// What the approval screen shows for a call that changes startup files: which ones, what happens to them and, for
/// a write, the exact diff against what is there now (read as the person, confined like every file tool).
pub fn approval_detail(name: &str, args: &Value, touched: &[(String, PathBuf)], peer: &Peer, roots: &[PathBuf], copies: &Path) -> Value {
    let home = &peer.home;
    let files: Vec<String> = touched.iter().map(|(raw, base)| shown(raw, base, home)).collect();
    let mut v = json!({"arquivos": files, "acao": name});
    // (a file with another name: asked because it may be a startup file elsewhere — the card says so)
    use std::os::unix::fs::MetadataExt;
    let linked = touched.iter().any(|(raw, _)| std::fs::symlink_metadata(resolve_existing(&normalize(&expand(raw, home)))).map(|m| m.is_file() && m.nlink() > 1).unwrap_or(false));
    if linked {
        v["outro_nome"] = json!(true);
    }
    // (what the call does, as the approval screen's scope line says it: design page 30, "Escreve um arquivo")
    let action = match name {
        "arquivos.escrever" => tr("Writes a file"),
        "arquivos.mover" => tr("Moves or renames a file"),
        "arquivos.lixeira" => tr("Moves a file to the trash"),
        "arquivos.apagar_criado" => tr("Deletes a file"),
        "arquivos.restaurar" => tr("Puts back an earlier version of a file"),
        "vm.pastas" => tr("Shares a folder with a virtual machine, which may then change it"),
        "vm.copiar_arquivo" => tr("Copies a file from a virtual machine here"),
        _ => tr("Writes a file"),
    };
    v["o_que"] = json!(action);
    if matches!(name, "arquivos.escrever" | "arquivos.restaurar" | "arquivos.apagar_criado") {
        let path = normalize(&expand(args["caminho"].as_str().unwrap_or(""), home));
        let new = match name {
            // an undo's earlier version: kept by kalangod, read only from this person's copies folder
            "arquivos.restaurar" => args["copia"].as_str().map(PathBuf::from).filter(|c| is_within(c, copies))
                .and_then(|c| std::fs::read(c).ok()).map(|b| String::from_utf8_lossy(&b).into_owned()),
            "arquivos.apagar_criado" => Some(String::new()),
            _ if args.get("conteudo_base64").is_some() => None,
            _ => args["conteudo"].as_str().map(str::to_string),
        };
        let old = crate::sandbox::run_as_user(peer, roots, json!({"tipo": "ler", "caminho": path, "max_bytes": 1 << 20})).ok();
        let exists = old.is_some();
        let old_text = old.as_ref().filter(|o| o["binario"] != true && o["cortado"] != true).and_then(|o| o["conteudo"].as_str().map(str::to_string));
        v["novo"] = json!(!exists);
        match (new, old_text, exists) {
            (Some(n), Some(o), _) => {
                let d = line_diff(&o, &n, 3, 400);
                v["resumo"] = json!(diff_summary(&d));
                v["linhas"] = diff_json(&d);
            }
            (Some(n), None, false) => {
                let d = line_diff("", &n, 3, 400);
                v["resumo"] = json!(trf("new file · {summary}", &[("summary", diff_summary(&d))]));
                v["linhas"] = diff_json(&d);
            }
            _ => v["resumo"] = json!(tr("binary or very large content: no line-by-line view")),
        }
    }
    v
}

fn diff_json(d: &[DiffLine]) -> Value {
    Value::Array(d.iter().map(|l| json!({"t": l.kind.to_string(), "l": l.text})).collect())
}

// ------------------------------------------------------------------ commands

/// The Landlock rules of a command run for an agent (`tarefas.executar`): read and write in `roots`, except that
/// nothing at all is reached in `no_access` (the protected folders) and nothing is written in `read_only` (the
/// startup files, when the person wants to be asked). Landlock only grants, so the grant is cut around them: a folder
/// on the way to one is not granted whole; its other entries are, one by one (links are skipped: a link's target is
/// granted where it really is, or not at all). A folder on the way to a protected one is listable only; one on the
/// way to a startup file is readable. So a command cannot create or delete entries directly in such a folder (the
/// home's top level, `~/.config`, `~/.local/share`), only change the files already there and work inside the
/// folders below.
#[derive(Debug, Default, PartialEq, Clone)]
pub struct Carved {
    pub write: Vec<PathBuf>,
    pub read: Vec<PathBuf>,
    pub list: Vec<PathBuf>,
}

pub fn carve(roots: &[PathBuf], no_access: &[PathBuf], read_only: &[PathBuf]) -> Carved {
    // (kept while none of the folders it read changed: their mtime moves when an entry is added, removed or renamed,
    // so a new file at the top of the home is seen at once; file tools ask for it on every call)
    type Entry = (Carved, Vec<(PathBuf, (i64, i64))>);
    static CACHE: std::sync::Mutex<Option<std::collections::HashMap<String, Entry>>> = std::sync::Mutex::new(None);
    let key = format!("{roots:?}{no_access:?}{read_only:?}");
    if let Some((c, seen)) = CACHE.lock().unwrap().as_ref().and_then(|m| m.get(&key)) {
        if seen.iter().all(|(d, t)| dir_mtime(d) == Some(*t)) {
            return c.clone();
        }
    }
    let real = |v: &[PathBuf]| -> Vec<PathBuf> { v.iter().flat_map(|p| [normalize(p), resolve_existing(p)]).collect() };
    let (none, ro) = (real(no_access), real(read_only));
    let mut out = Carved::default();
    let mut seen = Vec::new();
    for r in roots {
        visit(&resolve_existing(r), &none, &ro, &mut out, 0, &mut seen);
    }
    CACHE.lock().unwrap().get_or_insert_with(Default::default).insert(key, (out.clone(), seen));
    out
}

fn dir_mtime(d: &Path) -> Option<(i64, i64)> {
    use std::os::unix::fs::MetadataExt;
    std::fs::symlink_metadata(d).ok().map(|m| (m.mtime(), m.mtime_nsec()))
}

fn visit(p: &Path, none: &[PathBuf], ro: &[PathBuf], out: &mut Carved, depth: usize, seen: &mut Vec<(PathBuf, (i64, i64))>) {
    if none.iter().any(|n| is_within(p, n)) {
        return;
    }
    let holds_none = none.iter().any(|n| is_within(n, p));
    if ro.iter().any(|r| is_within(p, r)) && !holds_none {
        out.read.push(p.to_path_buf());
        return;
    }
    let holds_ro = ro.iter().any(|r| is_within(r, p));
    if !holds_none && !holds_ro {
        out.write.push(p.to_path_buf());
        return;
    }
    if holds_none { out.list.push(p.to_path_buf()) } else { out.read.push(p.to_path_buf()) }
    if depth > 16 {
        return;
    }
    if let Some(t) = dir_mtime(p) {
        seen.push((p.to_path_buf(), t));
    }
    let Ok(rd) = std::fs::read_dir(p) else { return };
    for e in rd.flatten() {
        let child = e.path();
        let Ok(md) = std::fs::symlink_metadata(&child) else { continue };
        if md.file_type().is_symlink() {
            continue;
        }
        if md.is_dir() {
            visit(&child, none, ro, out, depth + 1, seen);
        } else if none.iter().any(|n| is_within(&child, n)) {
        } else if ro.iter().any(|r| is_within(&child, r)) {
            if holds_none {
                out.read.push(child);
            }
        } else {
            out.write.push(child);
        }
    }
}

/// The startup files there are now, with their content (read as the person, confined): files of the startup
/// folders too, up to a limit. For a command run with "livre": what it changed is compared afterwards.
pub fn snapshot(peer: &Peer, cfg: &Config, roots: &[PathBuf]) -> std::collections::BTreeMap<String, Option<String>> {
    let v = crate::sandbox::run_as_user(peer, roots, json!({"tipo": "instantaneo", "caminhos": paths(cfg, peer)})).unwrap_or_default();
    v["arquivos"].as_object().map(|m| m.iter().map(|(k, c)| (k.clone(), c.as_str().map(str::to_string))).collect()).unwrap_or_default()
}

/// The files that differ between two snapshots: (path, before, after); None is "did not exist".
pub fn changes(before: &std::collections::BTreeMap<String, Option<String>>, after: &std::collections::BTreeMap<String, Option<String>>) -> Vec<(String, Option<String>, Option<String>)> {
    let keys: std::collections::BTreeSet<&String> = before.keys().chain(after.keys()).collect();
    keys.into_iter().filter_map(|k| {
        let (a, b) = (before.get(k).cloned().flatten(), after.get(k).cloned().flatten());
        (a != b).then(|| (k.clone(), a, b))
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn setup() -> (tempfile::TempDir, Peer, Config) {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("casa");
        for p in ["Documentos", ".config/kalango", ".config/autostart", ".config/gtk-4.0", ".local/share/systemd/user"] {
            std::fs::create_dir_all(home.join(p)).unwrap();
        }
        std::fs::write(home.join(".bashrc"), "alias la='ls -A'\n").unwrap();
        let peer = Peer { pid: 1, uid: 1000, gid: 1000, exe: "/usr/bin/teste".into(), cgroup: String::new(), passwd_home: home.clone(), home, user: "teste".into(), has_tty: false };
        (d, peer, Config::system_default())
    }

    #[test]
    fn startup_files_are_found_also_around_the_corner() {
        let (_d, peer, cfg) = setup();
        let h = peer.home.clone();
        for raw in ["~/.bashrc", "~/.profile", "~/.zshrc", "~/.config/autostart/x.desktop", "~/.config/systemd/user/x.service",
            "~/.config/environment.d/10-x.conf", "~/.pam_environment", "~/.xprofile", "~/.local/share/systemd/user/y.service",
            "~/Documentos/../.bashrc", "~/Documentos/../.config/autostart/z.desktop"] {
            assert!(startup_base(raw, &peer, &cfg).is_some(), "{raw} is a startup file");
        }
        // links, also to a file not created yet
        symlink(h.join(".bashrc"), h.join("Documentos/notas.txt")).unwrap();
        assert_eq!(startup_base("~/Documentos/notas.txt", &peer, &cfg), Some(h.join(".bashrc")));
        assert_eq!(shown("~/Documentos/notas.txt", &h.join(".bashrc"), &h), "~/Documentos/notas.txt → ~/.bashrc");
        assert_eq!(shown("~/Documentos/../.bashrc", &h.join(".bashrc"), &h), "~/.bashrc");
        symlink(h.join(".zprofile"), h.join("Documentos/novo.txt")).unwrap();
        assert!(!h.join(".zprofile").exists());
        assert!(startup_base("~/Documentos/novo.txt", &peer, &cfg).is_some());
        symlink("../.config/autostart", h.join("Documentos/auto")).unwrap();
        assert!(startup_base("~/Documentos/auto/x.desktop", &peer, &cfg).is_some());
        // a folder that holds one
        assert!(startup_base("~/.local/share/systemd", &peer, &cfg).is_some());
        // programs in the PATH and what systemd and D-Bus start
        for raw in ["~/.local/bin/ls", "~/bin/sudo", "~/.cargo/bin/x", "~/.bun/bin/x", "~/.npm-global/bin/x", "~/.bashrc.d/x.sh",
            "~/.config/zsh/.zshrc", "~/.oh-my-zsh/custom/x.zsh", "~/.local/share/dbus-1/services/org.x.service",
            "~/.config/user-tmpfiles.d/x.conf", "~/.local/share/systemd/timers/x", "~/.config/user-dirs.dirs", "~/miniconda3/bin/python",
            "~/.opencode/bin/x", "~/.local/share/fnm/ls"] {
            assert!(startup_base(raw, &peer, &cfg).is_some(), "{raw} is a startup path");
        }
        // a tool manager's version folders ("*")
        std::fs::create_dir_all(h.join(".nvm/versions/node/v24.20.0/bin")).unwrap();
        assert!(startup_base("~/.nvm/versions/node/v24.20.0/bin/ls", &peer, &cfg).is_some());
        // the person's other files are not
        for raw in ["~/Documentos/notas.md", "~/.config/gtk-4.0/gtk.css", "~/.bashrc_old", "~/.config/autostartx/a"] {
            assert!(startup_base(raw, &peer, &cfg).is_none(), "{raw} is not a startup file");
        }
    }

    /// The PATH of the person's processes outside the agents' cgroup (their user manager's and their sessions'), never
    /// an agent's; a fake /proc here.
    #[test]
    fn the_path_comes_from_the_persons_processes_never_the_agents() {
        let d = tempfile::tempdir().unwrap();
        let uid = nix::unistd::getuid().as_raw();
        let proc_ = d.path();
        let fake = |pid: &str, cgroup: String, path: &str| {
            std::fs::create_dir_all(proc_.join(pid)).unwrap();
            std::fs::write(proc_.join(pid).join("cgroup"), format!("0::{cgroup}\n")).unwrap();
            std::fs::write(proc_.join(pid).join("environ"), format!("HOME=/h\0PATH={path}\0")).unwrap();
        };
        fake("101", format!("/user.slice/user-{uid}.slice/user@{uid}.service/app.slice/vte-spawn-1.scope"), "/h/.nvm/versions/node/v24/bin:/usr/bin");
        fake("102", format!("/user.slice/user-{uid}.slice/session-3.scope"), "/h/.local/bin:/usr/bin");
        fake("103", format!("/user.slice/user-{uid}.slice/user@{uid}.service/kalango.slice/kalango-agentes.slice/t.scope"), "/h/evil:/usr/bin");
        fake("104", "/system.slice/x.service".into(), "/h/system");
        let p = processes_path(uid, proc_);
        assert!(p.contains("/h/.nvm/versions/node/v24/bin") && p.contains("/h/.local/bin"), "{p}");
        assert!(!p.contains("/h/evil") && !p.contains("/h/system"), "{p}");
        // only ever added to: a process gone keeps its folders in the list
        std::fs::remove_dir_all(proc_.join("101")).unwrap();
        assert!(processes_path(uid, proc_).contains("/h/.nvm/versions/node/v24/bin"));
    }

    /// The PATH the person's own shell gives, from their rc files (a fake home): at login (-lic, .profile) and in a
    /// terminal (-ic, .bashrc); evaluated again when an rc file changes.
    #[test]
    fn the_path_the_persons_shell_gives() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().to_path_buf();
        let rc = |extra: &str| {
            std::fs::write(home.join(".profile"), "export PATH=\"$HOME/no-login/bin:$PATH\"\n").unwrap();
            for f in [".bashrc", ".zshrc"] {
                std::fs::write(home.join(f), format!("export PATH=\"$HOME/no-terminal/bin:{extra}$PATH\"\necho barulho\n")).unwrap();
            }
            std::fs::write(home.join(".zprofile"), "export PATH=\"$HOME/no-login/bin:$PATH\"\n").unwrap();
        };
        rc("");
        let peer = Peer { pid: 1, uid: nix::unistd::getuid().as_raw(), gid: nix::unistd::getgid().as_raw(), exe: "/x".into(), cgroup: String::new(),
            passwd_home: home.clone(), home: home.clone(), user: "t".into(), has_tty: false };
        let login = run_shell_for_path(Path::new("/bin/bash"), &["-lic"], &peer);
        let term = run_shell_for_path(Path::new("/bin/bash"), &["-ic"], &peer);
        assert!(login.contains(&format!("{}/no-login/bin", home.display())), "{login}");
        assert!(term.contains(&format!("{}/no-terminal/bin", home.display())), "{term}");
        // the person's shell from passwd, cached until an rc file changes
        let first = shell_path(&peer);
        assert!(first.contains("no-terminal/bin"), "{first}");
        std::thread::sleep(std::time::Duration::from_millis(20));
        rc(&format!("{}/novo/bin:", home.display()));
        // (evaluated again in the background: the new folder shows in a moment)
        let mut second = shell_path(&peer);
        for _ in 0..200 {
            if second.contains("novo/bin") {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
            second = shell_path(&peer);
        }
        assert!(second.contains("novo/bin"), "a changed rc file is read again: {second}");
        assert!(second.contains("no-terminal/bin"), "only ever added to: {second}");
    }

    #[test]
    fn star_patterns_match_one_folder() {
        let pat = Path::new("/h/.nvm/versions/node/*/bin");
        assert!(matches_pattern(Path::new("/h/.nvm/versions/node/v24/bin/ls"), pat));
        assert!(matches_pattern(Path::new("/h/.nvm/versions/node/v24/bin"), pat));
        assert!(!matches_pattern(Path::new("/h/.nvm/versions/node/v24/lib/x"), pat));
        assert!(!matches_pattern(Path::new("/h/.nvm/versions/node"), pat));
        assert!(holds_pattern(Path::new("/h/.nvm/versions/node/v24"), pat) && holds_pattern(Path::new("/h/.nvm"), pat));
        assert!(!holds_pattern(Path::new("/h/Documentos"), pat));
    }

    #[test]
    fn login_path_folders_inside_the_home() {
        let home = Path::new("/home/ana");
        let d = dirs_in_home("/usr/bin:/home/ana/.local/bin:$HOME/ferramentas:~/go/bin:/home/ana:/opt/x::/home/ana/../bob/bin", home);
        assert_eq!(d, vec![PathBuf::from("/home/ana/.local/bin"), PathBuf::from("/home/ana/ferramentas"), PathBuf::from("/home/ana/go/bin")]);
    }

    #[test]
    fn the_setting_asks_unless_the_person_chose_otherwise() {
        let (_d, peer, cfg) = setup();
        let f = peer.home.join(".config/kalango/agente-permissoes.json");
        assert_eq!(mode(&cfg, &peer), Mode::Ask, "no file: asks");
        std::fs::write(&f, "{broken").unwrap();
        assert_eq!(mode(&cfg, &peer), Mode::Ask);
        std::fs::write(&f, r#"{"arquivos_de_inicio": "perguntar"}"#).unwrap();
        assert_eq!(mode(&cfg, &peer), Mode::Ask);
        std::fs::write(&f, r#"{"arquivos_de_inicio": "livre", "categorias": {}}"#).unwrap();
        assert_eq!(mode(&cfg, &peer), Mode::Allow);
        // not through a link
        std::fs::remove_file(&f).unwrap();
        std::fs::write(peer.home.join("Documentos/livre.json"), r#"{"arquivos_de_inicio": "livre"}"#).unwrap();
        symlink(peer.home.join("Documentos/livre.json"), &f).unwrap();
        assert_eq!(mode(&cfg, &peer), Mode::Ask);
    }

    #[test]
    fn the_tools_that_change_files_are_covered() {
        let (_d, peer, cfg) = setup();
        let t = |n: &str, a: Value| touched(n, &a, &peer, &cfg).len();
        assert_eq!(t("arquivos.escrever", json!({"caminho": "~/.bashrc", "conteudo": "x"})), 1);
        assert_eq!(t("arquivos.lixeira", json!({"caminho": "~/.bashrc"})), 1);
        assert_eq!(t("arquivos.apagar_criado", json!({"caminho": "~/.bashrc"})), 1);
        assert!(t("arquivos.mover", json!({"de": "~/Documentos/x.desktop", "para": "~/.config/autostart"})) >= 1);
        assert!(t("arquivos.mover", json!({"de": "~/.bashrc", "para": "~/Documentos/b"})) >= 1);
        assert_eq!(t("vm.pastas", json!({"caminho": "~/.config/autostart"})), 1);
        assert_eq!(t("vm.copiar_arquivo", json!({"sentido": "da_maquina", "para": "~/.profile"})), 1);
        assert_eq!(t("vm.copiar_arquivo", json!({"sentido": "para_maquina", "de": "~/.profile"})), 0);
        assert_eq!(t("arquivos.ler", json!({"caminho": "~/.bashrc"})), 0);
        assert_eq!(t("arquivos.escrever", json!({"caminho": "~/Documentos/a.txt", "conteudo": "x"})), 0);
    }

    #[test]
    fn the_diff_shows_exactly_what_changes() {
        let old = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\n";
        let new = "a\nb\nc\nd\ne\nf\nG\nh\ni\nj\nk\n";
        let d = line_diff(old, new, 1, 100);
        let s: Vec<String> = d.iter().map(|l| format!("{}{}", l.kind, l.text)).collect();
        assert_eq!(s, vec![" f", "-g", "+G", " h", "@", " j", "+k"]);
        assert_eq!(diff_summary(&d), trf("{added} lines added, {removed} removed", &[("added", "2".into()), ("removed", "1".into())]));
        assert_eq!(diff_summary(&line_diff("a\n", "b\n", 1, 10)), tr("1 line changes"));
        let d = line_diff("", "alias ll='ls -alF'\n", 3, 100);
        assert_eq!(d, vec![DiffLine { kind: '+', text: "alias ll='ls -alF'".into() }]);
        assert!(line_diff("x\n", "x\n", 3, 100).is_empty());
        // long diffs are cut, with a marker
        let big: String = (0..1000).map(|i| format!("{i}\n")).collect();
        let d = line_diff("", &big, 3, 50);
        assert_eq!(d.len(), 51);
        assert_eq!(d.last().unwrap().kind, '@');
    }

    #[test]
    fn commands_get_the_home_cut_around_protected_and_startup_files() {
        let (_d, peer, _cfg) = setup();
        let h = peer.home.clone();
        std::fs::write(h.join("notas.txt"), "x").unwrap();
        std::fs::write(h.join(".config/gtk-4.0/gtk.css"), "x").unwrap();
        symlink(h.join(".config/kalango"), h.join("Documentos/atalho")).unwrap();
        let c = carve(std::slice::from_ref(&h), &[h.join(".config/kalango")], &[h.join(".bashrc"), h.join(".config/autostart"), h.join(".profile")]);
        // the home itself: listable only (it holds a protected folder); its files, one by one
        assert!(c.list.contains(&h) && !c.write.contains(&h) && !c.read.contains(&h), "{c:?}");
        assert!(c.write.contains(&h.join("notas.txt")) && c.write.contains(&h.join("Documentos")));
        assert!(c.read.contains(&h.join(".bashrc")) && !c.write.contains(&h.join(".bashrc")));
        assert!(c.read.contains(&h.join(".config/autostart")));
        assert!(c.write.contains(&h.join(".config/gtk-4.0")));
        // nothing for the protected folder, and links are not followed
        assert!(!c.write.iter().chain(&c.read).chain(&c.list).any(|p| p.starts_with(h.join(".config/kalango"))));
        assert!(!c.write.contains(&h.join("Documentos/atalho")));
        // only startup files: the folders on the way are readable
        let c = carve(std::slice::from_ref(&h), &[], &[h.join(".bashrc")]);
        assert!(c.read.contains(&h) && c.list.is_empty() && !c.read.contains(&h.join(".bashrc")), "{c:?}");
        // nothing to cut: the root whole
        let c = carve(&[h.join("Documentos")], &[h.join(".config/kalango")], &[h.join(".bashrc")]);
        assert_eq!(c.write, vec![h.join("Documentos")]);
    }

    #[test]
    fn snapshots_compare() {
        use std::collections::BTreeMap;
        let a: BTreeMap<String, Option<String>> = [("/h/.bashrc".to_string(), Some("a".to_string())), ("/h/.profile".to_string(), Some("p".to_string()))].into();
        let b: BTreeMap<String, Option<String>> = [("/h/.bashrc".to_string(), Some("b".to_string())), ("/h/.profile".to_string(), Some("p".to_string())),
            ("/h/.config/autostart/x.desktop".to_string(), Some("n".to_string()))].into();
        let c = changes(&a, &b);
        assert_eq!(c.len(), 2);
        assert!(c.contains(&("/h/.bashrc".into(), Some("a".into()), Some("b".into()))));
        assert!(c.contains(&("/h/.config/autostart/x.desktop".into(), None, Some("n".into()))));
    }
}
