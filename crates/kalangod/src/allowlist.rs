//! Where agents' commands may write (the allowlist).
//!
//! A command an agent runs (`tarefas.executar`) reads the person's home, except the protected folders, and writes
//! only in:
//! - the work folders (Documentos, Downloads, Área de Trabalho, Imagens, Músicas, Vídeos, Modelos), as the XDG user
//!   dirs name them. kalangod reads `~/.config/user-dirs.dirs` itself. Commands can't write that file (it is outside
//!   the work folders), and file tools ask before changing it (it is a startup path);
//! - the project folders the person added or approved, kept by kalangod in its own state (no agent reaches it);
//! - a folder the person approved for one task only (kept in memory for the asking process while it lives);
//! - its own temporary folder (`limits.rs`, TMPDIR).
//!
//! Everything else in the home is read-only for commands: the files that run at login, the PATH folders, the
//! language managers' scripts (~/.cargo/env, ~/.nvm/nvm.sh, conda's), whatever their name. Installing for the person
//! (`pip install --user`, `npm -g`, `cargo install`…) asks first, one command at a time (`installs_for_user`).

use crate::peer::Peer;
use crate::policy::{is_within, normalize};
use kalango_i18n::{tr, trf};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The XDG keys of the work folders, with the folder used when the key is missing.
pub const WORK_KEYS: &[(&str, &str)] = &[
    ("XDG_DOCUMENTS_DIR", "Documentos"),
    ("XDG_DOWNLOAD_DIR", "Downloads"),
    ("XDG_DESKTOP_DIR", "Área de Trabalho"),
    ("XDG_PICTURES_DIR", "Imagens"),
    ("XDG_MUSIC_DIR", "Músicas"),
    ("XDG_VIDEOS_DIR", "Vídeos"),
    ("XDG_TEMPLATES_DIR", "Modelos"),
];

/// The values of `user-dirs.dirs` (`XDG_X_DIR="$HOME/Pasta"` or an absolute path), with `$HOME` expanded.
pub fn parse_user_dirs(text: &str, home: &Path) -> Vec<(String, PathBuf)> {
    text.lines().filter_map(|l| {
        let l = l.trim();
        if l.starts_with('#') {
            return None;
        }
        let (k, v) = l.split_once('=')?;
        let v = v.trim().trim_matches('"');
        let p = if let Some(rest) = v.strip_prefix("$HOME") {
            home.join(rest.trim_start_matches('/'))
        } else if v.starts_with('/') {
            PathBuf::from(v)
        } else {
            return None;
        };
        Some((k.trim().to_string(), normalize(&p)))
    }).collect()
}

/// A folder in the home that no allowlist entry may be or hold: the home itself, a hidden folder (where programs keep
/// configuration and scripts), or one holding a protected folder. Outside the home: only removable and data mounts.
fn refused(p: &Path, home: &Path, protected: &[PathBuf]) -> Option<String> {
    let inside_home = is_within(p, home) && p != home;
    let mount = ["/media", "/mnt", "/run/media", "/srv"].iter().any(|m| is_within(p, Path::new(m)) && p != Path::new(m));
    if !inside_home && !mount {
        return Some(tr("only a folder inside your home (or on a disk under /media, /mnt or /srv) can be a project folder").into());
    }
    if inside_home && p.strip_prefix(home).map(|r| r.components().any(|c| c.as_os_str().to_string_lossy().starts_with('.'))).unwrap_or(true) {
        return Some(tr("hidden folders hold programs' settings and scripts: they cannot be a project folder").into());
    }
    if let Some(x) = protected.iter().find(|x| is_within(p, x) || is_within(x, p)) {
        return Some(trf("{path} is or holds the protected folder {protected}", &[("path", p.display().to_string()), ("protected", x.display().to_string())]));
    }
    None
}

fn real_dir(p: &Path) -> Option<PathBuf> {
    let md = std::fs::symlink_metadata(p).ok()?;
    (md.is_dir() && !md.file_type().is_symlink()).then(|| p.to_path_buf())
}

/// The work folders that exist, as `user-dirs.dirs` names them (read here, never from what an agent passes). An entry
/// that is the home, a hidden folder or holds a protected one is left out (`XDG_DESKTOP_DIR="$HOME"` is common when
/// there is no desktop folder: that would be the whole home).
pub fn work_folders(home: &Path, protected: &[PathBuf]) -> Vec<PathBuf> {
    let file = home.join(".config/user-dirs.dirs");
    let text = match std::fs::symlink_metadata(&file) {
        Ok(m) if m.is_file() => std::fs::read_to_string(&file).unwrap_or_default(),
        _ => String::new(),
    };
    let named = parse_user_dirs(&text, home);
    let mut out: Vec<PathBuf> = Vec::new();
    for (key, fallback) in WORK_KEYS {
        let p = named.iter().find(|(k, _)| k == key).map(|(_, p)| p.clone()).unwrap_or_else(|| home.join(fallback));
        if !is_within(&p, home) || refused(&p, home, protected).is_some() {
            continue;
        }
        if let Some(d) = real_dir(&p) {
            if !out.contains(&d) {
                out.push(d);
            }
        }
    }
    out
}

/// The home as it really is (a link to it, as `/home` → `/var/home`, resolved): the base every allowlist path is
/// opened from.
pub fn real_home(home: &Path) -> PathBuf {
    std::fs::canonicalize(home).unwrap_or_else(|_| normalize(home))
}

/// A folder asked to become a project folder, with its identity (device, inode) — the folder itself, never where a
/// link would lead: a path that goes through a link is refused, and so is a folder holding a file that runs at login
/// or a PATH folder (those stay read only, and a project folder is written whole).
pub fn check_project(raw: &str, home: &Path, protected: &[PathBuf], startup: &[PathBuf]) -> Result<(PathBuf, (u64, u64)), String> {
    let home = real_home(home);
    let p = normalize(&crate::policy::expand(raw, &home));
    let real = std::fs::canonicalize(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    if real != p {
        return Err(trf("{path} goes through a link: give the folder itself ({real})", &[("path", raw.to_string()), ("real", real.display().to_string())]));
    }
    if !real.is_dir() {
        return Err(trf("{path} is not a folder", &[("path", raw.to_string())]));
    }
    let prot: Vec<PathBuf> = protected.iter().flat_map(|x| [normalize(x), crate::sandbox::resolve_existing(x)]).collect();
    if let Some(e) = refused(&real, &home, &prot) {
        return Err(e);
    }
    if let Some(x) = startup.iter().flat_map(|x| [normalize(x), crate::sandbox::resolve_existing(x)]).find(|x| is_within(x, &real)) {
        return Err(trf("{path} holds {startup}, which runs when you log in or is on your PATH: it cannot be a project folder", &[("path", raw.to_string()), ("startup", crate::startup::tilde(&x, &home))]));
    }
    let id = identity(&real, &home).ok_or_else(|| trf("{path} goes through a link: give the folder itself ({real})", &[("path", raw.to_string()), ("real", real.display().to_string())]))?;
    Ok((real, id))
}

/// Where a folder of the allowlist is opened from: the home, or a disk under /media, /mnt, /run/media, /srv.
fn base_of(p: &Path, home: &Path) -> Option<PathBuf> {
    [home, Path::new("/media"), Path::new("/mnt"), Path::new("/run/media"), Path::new("/srv")].iter()
        .filter(|b| is_within(p, b)).max_by_key(|b| b.as_os_str().len()).map(|b| b.to_path_buf())
}

/// Opens `p` beneath `base` without following any link on the way (`openat2`, RESOLVE_NO_SYMLINKS |
/// RESOLVE_BENEATH): a folder swapped for a link does not open.
pub fn open_nofollow(base: &Path, p: &Path) -> Option<std::os::fd::OwnedFd> {
    use nix::fcntl::{openat2, OFlag, OpenHow, ResolveFlag};
    use std::os::fd::{AsRawFd, FromRawFd};
    let rel = p.strip_prefix(base).ok()?;
    let b = std::fs::File::open(base).ok()?;
    let fd = if rel.as_os_str().is_empty() {
        openat2(b.as_raw_fd(), ".", OpenHow::new().flags(OFlag::O_PATH | OFlag::O_CLOEXEC).resolve(ResolveFlag::RESOLVE_NO_SYMLINKS))
    } else {
        openat2(b.as_raw_fd(), rel, OpenHow::new().flags(OFlag::O_PATH | OFlag::O_CLOEXEC).resolve(ResolveFlag::RESOLVE_NO_SYMLINKS | ResolveFlag::RESOLVE_BENEATH))
    }.ok()?;
    Some(unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) })
}

/// (device, inode) of `p`, reached without links.
pub fn identity(p: &Path, home: &Path) -> Option<(u64, u64)> {
    use std::os::fd::AsRawFd;
    let base = base_of(p, home)?;
    let fd = open_nofollow(&base, p)?;
    let st = nix::sys::stat::fstat(fd.as_raw_fd()).ok()?;
    Some((st.st_dev, st.st_ino))
}

/// A write rule pinned to what kalangod saw: the trampoline opens it again the same way and checks the identity
/// (`limits.rs`), so a folder swapped in between gets no rule at all.
pub fn pin(p: &Path, home: &Path) -> Option<crate::limits::Fixa> {
    let base = base_of(p, home)?;
    let (dev, ino) = identity(p, home)?;
    Some(crate::limits::Fixa { base, caminho: p.to_path_buf(), dev, ino })
}

/// What the approval card shows of a folder: how many files (counted up to a limit), their size, the git branch and
/// the first entries. Links are not followed.
pub fn summary(dir: &Path, home: &Path) -> Value {
    const LIMIT: usize = 20_000;
    let (mut files, mut bytes, mut more) = (0usize, 0u64, false);
    let mut stack = vec![dir.to_path_buf()];
    'walk: while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let Ok(md) = e.metadata() else { continue };
            if md.is_dir() {
                stack.push(e.path());
            } else {
                files += 1;
                bytes += md.len();
                if files >= LIMIT {
                    more = true;
                    break 'walk;
                }
            }
        }
    }
    let git = std::fs::read_to_string(dir.join(".git/HEAD")).ok().map(|h| h.trim().strip_prefix("ref: refs/heads/").unwrap_or("").to_string());
    let mut entries: Vec<String> = std::fs::read_dir(dir).map(|rd| rd.flatten().map(|e| {
        let n = e.file_name().to_string_lossy().into_owned();
        if e.file_type().map(|t| t.is_dir()).unwrap_or(false) { format!("{n}/") } else { n }
    }).filter(|n| n != ".git/").collect()).unwrap_or_default();
    entries.sort_by_key(|n| (!n.ends_with('/'), n.to_lowercase()));
    let total = entries.len();
    entries.truncate(6);
    json!({"caminho": crate::startup::tilde(dir, home), "arquivos": files, "mais": more, "bytes": bytes, "git": git, "entradas": entries, "outras": total.saturating_sub(6)})
}

/// A project folder the person added or approved with "always": the folder itself, by its identity.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Project {
    pub uid: u32,
    pub caminho: PathBuf,
    /// "aprovada" (an agent asked and the person answered "always") or "adicionada" (by the person, in Permissões)
    pub origem: String,
    pub criado: u64,
    #[serde(default)]
    pub dev: u64,
    #[serde(default)]
    pub ino: u64,
}

/// How long "only this task" lasts at most: the asking process may live much longer (an MCP client), and a grant
/// the person cannot see ending must not stay. Shown in Permissões while it lasts.
pub const TASK_GRANT: std::time::Duration = std::time::Duration::from_secs(30 * 60);

/// A folder approved for one task: for the process that asked, while it lives (its start time tells a reused pid)
/// and for at most `TASK_GRANT`.
#[derive(Clone)]
pub struct TaskGrant {
    pub uid: u32,
    pub pid: i32,
    start: u64,
    pub exe: String,
    pub caminho: PathBuf,
    dev: u64,
    ino: u64,
    pub ate: u64,
}

fn start_time(pid: i32) -> Option<u64> {
    let s = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // (the name, in parentheses, may hold spaces: the fields after its closing one)
    s.rsplit_once(')')?.1.split_whitespace().nth(19)?.parse().ok()
}

#[derive(Default)]
pub struct Store {
    file: Option<PathBuf>,
    projects: Mutex<Vec<Project>>,
    grants: Mutex<Vec<TaskGrant>>,
}

impl Store {
    /// With the project folders kept in `state_dir/pastas-de-projeto.json`.
    pub fn open(state_dir: &Path) -> Store {
        let file = state_dir.join("pastas-de-projeto.json");
        let projects = std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        Store { file: Some(file), projects: Mutex::new(projects), grants: Default::default() }
    }

    fn save(&self, all: &[Project]) {
        if let Some(p) = &self.file {
            let tmp = p.with_extension("json.tmp");
            if std::fs::write(&tmp, serde_json::to_string_pretty(all).unwrap_or_default()).is_ok() {
                let _ = std::fs::rename(&tmp, p);
            }
        }
    }

    pub fn list(&self, uid: u32) -> Vec<Project> {
        self.projects.lock().unwrap().iter().filter(|p| p.uid == uid).cloned().collect()
    }

    /// Keeps a project folder. One inside another project folder adds nothing (skipped); one holding others takes
    /// their place.
    pub fn add(&self, uid: u32, caminho: PathBuf, id: (u64, u64), origem: &str) {
        let mut g = self.projects.lock().unwrap();
        if g.iter().any(|p| p.uid == uid && is_within(&caminho, &p.caminho)) {
            return;
        }
        g.retain(|p| !(p.uid == uid && is_within(&p.caminho, &caminho)));
        g.push(Project { uid, caminho, origem: origem.into(), criado: crate::timeline::now(), dev: id.0, ino: id.1 });
        let all = g.clone();
        drop(g);
        self.save(&all);
    }

    /// Takes a folder out — a project folder and the task grants for it (only removes permission): whether there
    /// was one.
    pub fn remove(&self, uid: u32, caminho: &Path) -> bool {
        let mut g = self.projects.lock().unwrap();
        let n = g.len();
        g.retain(|p| !(p.uid == uid && p.caminho == caminho));
        let changed = g.len() != n;
        let all = g.clone();
        drop(g);
        if changed {
            self.save(&all);
        }
        let mut t = self.grants.lock().unwrap();
        let m = t.len();
        t.retain(|x| !(x.uid == uid && x.caminho == caminho));
        changed || t.len() != m
    }

    pub fn grant_task(&self, peer: &Peer, caminho: PathBuf, id: (u64, u64)) {
        let Some(start) = start_time(peer.pid) else { return };
        let mut g = self.grants.lock().unwrap();
        g.push(TaskGrant { uid: peer.uid, pid: peer.pid, start, exe: peer.exe.clone(), caminho, dev: id.0, ino: id.1, ate: crate::timeline::now() + TASK_GRANT.as_secs() });
    }

    /// The task grants still in force (the person's, for Permissões).
    pub fn task_grants(&self, uid: u32) -> Vec<TaskGrant> {
        let now = crate::timeline::now();
        let mut g = self.grants.lock().unwrap();
        g.retain(|t| t.ate > now && start_time(t.pid) == Some(t.start));
        g.iter().filter(|t| t.uid == uid).cloned().collect()
    }

    /// The folders this caller's commands write in besides the work folders — the person's project folders and the
    /// ones approved for this very process — each still the very folder approved: one swapped (for a link, or for
    /// another folder) is dropped, and returned apart so it can be recorded.
    pub fn for_peer(&self, peer: &Peer, home: &Path) -> (Vec<PathBuf>, Vec<PathBuf>) {
        let (mut ok, mut gone) = (Vec::new(), Vec::new());
        let same = |p: &Path, dev: u64, ino: u64| identity(p, home) == Some((dev, ino));
        let mut g = self.projects.lock().unwrap();
        let n = g.len();
        g.retain(|p| {
            if p.uid != peer.uid {
                return true;
            }
            if same(&p.caminho, p.dev, p.ino) {
                ok.push(p.caminho.clone());
                true
            } else {
                gone.push(p.caminho.clone());
                false
            }
        });
        if g.len() != n {
            let all = g.clone();
            drop(g);
            self.save(&all);
        } else {
            drop(g);
        }
        let start = start_time(peer.pid);
        let now = crate::timeline::now();
        let mut t = self.grants.lock().unwrap();
        t.retain(|x| x.ate > now && start_time(x.pid) == Some(x.start));
        t.retain(|x| {
            if !(x.uid == peer.uid && x.pid == peer.pid && Some(x.start) == start) {
                return true;
            }
            if same(&x.caminho, x.dev, x.ino) {
                ok.push(x.caminho.clone());
                true
            } else {
                gone.push(x.caminho.clone());
                false
            }
        });
        (ok, gone)
    }
}

/// The folders a command may write in, narrowed to the caller's allowed folders (an agent allowed only ~/Documentos
/// writes only there, even with project folders elsewhere).
pub fn writable(candidates: &[PathBuf], allowed: &[PathBuf]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for w in candidates {
        for r in allowed {
            let pick = if is_within(w, r) { Some(w) } else if is_within(r, w) { Some(r) } else { None };
            if let Some(p) = pick {
                if !out.iter().any(|o| is_within(p, o)) {
                    out.retain(|o| !is_within(o, p));
                    out.push(p.clone());
                }
            }
        }
    }
    out
}

/// The commands of a shell script, word by word, as far as installing goes: split at `;`, `&&`, `||`, `|` and
/// newlines, quotes taken off, `env VAR=x` and `VAR=x` in front skipped (enough to find `pip install --user` in it;
/// not a shell parser — what it misses still runs inside the allowlist, where an install outside the folders fails).
fn shell_commands(script: &str) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    for part in script.split([';', '\n', '|', '&']) {
        let mut words: Vec<String> = part.split_whitespace().map(|w| w.trim_matches(|c| c == '"' || c == '\'').to_string()).filter(|w| !w.is_empty()).collect();
        while words.first().is_some_and(|w| w == "env" || w == "sudo" || w == "exec" || w == "command" || (w.contains('=') && !w.starts_with('-'))) {
            words.remove(0);
        }
        if !words.is_empty() {
            out.push(words);
        }
    }
    out
}

/// A command that installs for the person, outside the work folders: what it is and where it writes (the folders
/// that exist). Asked once, for that command only.
pub struct Install {
    pub what: &'static str,
    /// where it writes, as the card shows it (made, as the person, when missing and approved)
    pub writes: Vec<PathBuf>,
    /// the cache variables left out for it, so it installs where the card says (`cargo install` into ~/.cargo, `go
    /// install` into ~/go/bin), not into the agents' cache
    pub unset: &'static [&'static str],
    /// the files there that must stay as they are (sourced by the shell, read by the tool at every run)
    pub keep: Vec<PathBuf>,
}

pub fn installs_for_user(prog: &str, args: &[String], home: &Path) -> Option<Install> {
    let base = Path::new(prog).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    // a shell's script: each command in it (`sh -c "cd x && npm i -g y"`)
    if matches!(base.as_str(), "sh" | "bash" | "dash" | "zsh") {
        let script = args.iter().position(|a| a == "-c" || a == "-lc" || a == "-ic").and_then(|i| args.get(i + 1))?;
        return shell_commands(script).into_iter().find_map(|words| {
            let (p, rest) = words.split_first()?;
            installs_for_user(p, rest, home)
        });
    }
    let has = |a: &str| args.iter().any(|x| x == a);
    let h = |p: &str| home.join(p);
    let local = |v: &[&str]| -> Vec<PathBuf> { v.iter().map(|p| h(p)).collect() };
    let is_python = base.starts_with("python");
    let pip_args: Option<&[String]> = if base.starts_with("pip") {
        Some(args)
    } else if is_python && args.len() >= 2 && args[0] == "-m" && args[1] == "pip" {
        Some(&args[2..])
    } else {
        None
    };
    let mut unset: &'static [&'static str] = &[];
    let (what, writes, keep): (&'static str, Vec<PathBuf>, Vec<PathBuf>) = if let Some(pa) = pip_args {
        if !(pa.iter().any(|a| a == "install") && pa.iter().any(|a| a == "--user")) {
            return None;
        }
        // (the python3.X folders that exist, else the folder they go in; never ~/.local/share, where the launchers are)
        let mut libs: Vec<PathBuf> = std::fs::read_dir(h(".local/lib")).map(|rd| rd.flatten().map(|e| e.path())
            .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("python"))).collect()).unwrap_or_default();
        libs.sort();
        if libs.is_empty() {
            libs.push(h(".local/lib"));
        }
        libs.push(h(".local/bin"));
        ("pip install --user", libs, vec![])
    } else if base == "pipx" && args.first().is_some_and(|a| a == "install") {
        ("pipx install", local(&[".local/pipx", ".local/share/pipx", ".local/state/pipx", ".local/bin"]), vec![])
    } else if matches!(base.as_str(), "npm" | "pnpm" | "yarn" | "bun") && (has("-g") || has("--global") || has("--global=true") || has("--location=global")
        || args.windows(2).any(|w| w[0] == "--location" && w[1] == "global") || args.first().is_some_and(|a| a == "global")) {
        ("npm -g", local(&[".npm-global", ".local/lib/node_modules", ".local/bin", ".local/share/pnpm", ".config/yarn/global", ".bun/install/global", ".bun/bin"]), vec![])
    } else if base == "cargo" && args.first().is_some_and(|a| a == "install") {
        unset = &["CARGO_HOME"];
        ("cargo install", local(&[".cargo"]), local(&[".cargo/env", ".cargo/env.fish", ".cargo/env.nu", ".cargo/config", ".cargo/config.toml", ".cargo/credentials", ".cargo/credentials.toml"]))
    } else if base == "go" && args.first().is_some_and(|a| a == "install") {
        unset = &["GOPATH"];
        ("go install", local(&["go/bin"]), vec![])
    } else if base == "gem" && args.first().is_some_and(|a| a == "install") && has("--user-install") {
        ("gem install --user-install", local(&[".local/share/gem", ".gem"]), vec![])
    } else {
        return None;
    };
    // (pip's python3.X and npm's prefix: only the ones there are; ~/go/bin and the like are made when approved)
    let writes: Vec<PathBuf> = writes.into_iter().enumerate().filter(|(i, p)| real_dir(p).is_some() || (*i == 0 && unset.contains(&"GOPATH"))).map(|(_, p)| p).collect();
    Some(Install { what, writes, unset, keep })
}

/// The protected folders, as written and where links lead (no allowlist entry may be or hold one).
pub fn protected(cfg: &crate::policy::Config, home: &Path) -> Vec<PathBuf> {
    cfg.protected_paths.iter().map(|p| crate::policy::expand(p, home)).flat_map(|p| [normalize(&p), crate::sandbox::resolve_existing(&p)]).collect()
}

/// The person's own interface (config `interface_da_pessoa`: the shell), and not an agent: not in the agents' slice,
/// and the very program listed (as the kernel reports it). Run as root, the program must also be one no other user
/// can replace (as the approval screen: `approval::trusted_approver`).
pub fn is_person_ui(peer: &Peer, listed: &[PathBuf], allowed_env: &[String]) -> bool {
    if peer.cgroup.contains("/kalango-agentes.slice") {
        eprintln!("kalangod: interface da pessoa recusada (cgroup dos agentes): pid {}", peer.pid);
        return false;
    }
    let exe = Path::new(&peer.exe);
    if !listed.iter().any(|l| std::fs::canonicalize(l).is_ok_and(|c| c == exe)) {
        eprintln!("kalangod: interface da pessoa recusada (programa {}): pid {}", peer.exe, peer.pid);
        return false;
    }
    // (the right program, but run with something else loaded into it, or under a tracer, is not the person's)
    if let Some(why) = tampered(peer.pid, allowed_env) {
        eprintln!("kalangod: interface da pessoa recusada ({why}): pid {}", peer.pid);
        return false;
    }
    let trusted = !nix::unistd::geteuid().is_root() || crate::approval::trusted_approver(&peer.exe, listed);
    if !trusted {
        eprintln!("kalangod: interface da pessoa recusada (o programa ou a pasta dele pode ser trocado por outra conta): {}", peer.exe);
    }
    trusted
}

/// Every folder this caller's commands may write in, before narrowing to its allowed folders (all of them as
/// reached from the real home). An approved folder that is no longer the one approved is dropped here and the
/// timeline records it.
pub fn candidates(st: &crate::server::State, peer: &Peer) -> Vec<PathBuf> {
    let home = real_home(&peer.home);
    let mut v = work_folders(&home, &protected(&st.cfg, &home));
    let (ok, gone) = st.folders.for_peer(peer, &home);
    for p in gone {
        let shown = crate::startup::tilde(&p, &home);
        eprintln!("kalangod: {shown}: a pasta aprovada foi trocada; a aprovação caiu");
        st.timeline.append(crate::timeline::Entry {
            id: 0, time: 0, caller: format!("kalangod ({})", peer.exe), uid: peer.uid, pid: peer.pid, tool: "kalango/pastas".into(), level: "usuario".into(),
            args: json!({"pasta": shown}), status: "negado".into(),
            summary: trf("{path} is no longer the folder you approved (it was moved, or swapped for a link): commands do not write there until you approve it again", &[("path", shown)]),
            inverse: None, restore_point: None, miniatura: None, startup: None, d1: None,
        });
    }
    let extra: Vec<PathBuf> = ok.into_iter().filter(|p| !v.iter().any(|w| is_within(p, w))).collect();
    v.extend(extra);
    v
}

/// Asks the person on the approval screen whether commands may write in a folder (the card shows its path, a
/// summary and the first entries). An agent's request: "only this task" (for the asking process) or "always" (a
/// project folder). The person's own request (Permissões › Adicionar): a project folder once approved there, so an
/// agent can't add one by calling the same method. Returns the folder and whether it became a project folder.
pub fn ask_folder(st: &crate::server::State, peer: &Peer, caller: &str, tool: &str, raw: &str, from_person: bool) -> Result<(PathBuf, bool), String> {
    let startup = crate::startup::paths(&st.cfg, peer);
    let (real, id) = check_project(raw, &peer.home, &protected(&st.cfg, &peer.home), &startup)?;
    // (inside a work folder or a folder already allowed: nothing to add)
    let home = real_home(&peer.home);
    let work = work_folders(&home, &protected(&st.cfg, &home));
    if work.iter().chain(&st.folders.list(peer.uid).into_iter().map(|p| p.caminho).collect::<Vec<_>>()).any(|c| is_within(&real, c))
        || (!from_person && candidates(st, peer).iter().any(|c| is_within(&real, c))) {
        return Ok((real, false));
    }
    let mut card = summary(&real, &home);
    card["pessoa"] = json!(from_person);
    let shown = crate::startup::tilde(&real, &home);
    let p = crate::approval::Pending {
        id: 0, tool: tool.into(), level: "pasta".into(), args: json!({"pasta": raw}), caller: caller.into(), uid: peer.uid, pid: peer.pid,
        reason: trf("let commands write in {path}", &[("path", shown.clone())]),
        descricao: tr("Lets commands create, change and delete files in a folder").into(), efeito: "escreve".into(), reversivel: true,
        criado: 0, prazo_s: 0, exe: peer.exe.clone(), decision: None, startup: None, no_always: from_person, cartao: Some(card),
    };
    match st.approvals.request_with_always(p, std::time::Duration::from_secs(st.cfg.approval_timeout_s)) {
        (crate::approval::Outcome::Approved, always) => {
            if always || from_person {
                st.folders.add(peer.uid, real.clone(), id, if from_person { "adicionada" } else { "aprovada" });
            } else {
                st.folders.grant_task(peer, real.clone(), id);
            }
            // (a folder granted to a creative app that is now inside this one leaves its permissions: an agent could swap it)
            crate::tools::craft::reapply_installed(&st.cfg, peer, crate::tools::craft::GrantContext::of(st), None);
            Ok((real, always || from_person))
        }
        (crate::approval::Outcome::Denied, _) => Err(trf("the person did not let commands write in {path}", &[("path", shown)])),
        (crate::approval::Outcome::Timeout, _) => Err(tr("no approval within the time limit").into()),
    }
}

/// The caches agents' commands build with (crates, npm packages, Go modules, pip wheels…): their own, never the
/// person's — a cache feeds later builds, and the person's `~/.cargo/registry` or Go build cache changed by an agent
/// would be compiled into the person's next build. And one per grant: a caller and the exact set of folders it may
/// write in (`scope`). A cache written under one grant only ever feeds commands with that same grant — which could
/// write the same projects themselves — so a narrow caller cannot plant something (a `rustc-wrapper`, a poisoned
/// wheel) that a broader one then runs with its access. Their settings files are read only for commands (`CACHE_KEEP`).
/// Nothing on the PATH, nothing sourced at login, is in them.
pub const CACHE_VARS: &[(&str, &str)] = &[
    ("XDG_CACHE_HOME", "xdg"), ("CARGO_HOME", "cargo"), ("npm_config_cache", "npm"), ("YARN_CACHE_FOLDER", "yarn"),
    ("BUN_INSTALL_CACHE_DIR", "bun"), ("GOCACHE", "go-build"), ("GOMODCACHE", "go-mod"), ("GOPATH", "go"), ("PIP_CACHE_DIR", "pip"), ("UV_CACHE_DIR", "uv"),
    ("POETRY_CACHE_DIR", "poetry"), ("DENO_DIR", "deno"), ("GRADLE_USER_HOME", "gradle"),
];

/// In a grant's cache, what tools read as settings or run: kalangod writes them (or nothing), commands only read.
pub const CACHE_KEEP: &[&str] = &["cargo/config.toml", "cargo/config", "cargo/credentials.toml", "cargo/credentials", "cargo/env", "cargo/bin",
    "gradle/init.d", "gradle/init.gradle", "gradle/init.gradle.kts", "gradle/gradle.properties", "go/bin"];

/// cargo's own files in its home it must be able to change (locks, the global cache tracker): made beforehand, since
/// the home itself stays read only (it holds the settings).
const CARGO_FILES: &[&str] = &[".package-cache", ".package-cache-mutate", ".global-cache"];

/// A grant's name for its cache folder: the caller and the folders it may write (sorted), hashed (FNV-1a, stable).
pub fn cache_scope(caller: &str, roots: &[PathBuf]) -> String {
    let mut r: Vec<String> = roots.iter().map(|p| p.display().to_string()).collect();
    r.sort();
    r.dedup();
    let mut h: u64 = 0xcbf29ce484222325;
    for b in std::iter::once(caller.to_string()).chain(r).collect::<Vec<_>>().join("\n").bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100000001b3);
    }
    let name: String = caller.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).take(32).collect();
    format!("{name}-{h:016x}")
}

/// Opens (making it when missing) a folder `name` in `parent`, as the person, without following links; a folder of
/// someone else is refused (root's own, just made, is given to the person).
fn mkdir_as(parent: &std::os::fd::OwnedFd, name: &str, uid: u32, gid: u32, mode: u32) -> Option<std::os::fd::OwnedFd> {
    use nix::fcntl::{openat2, OFlag, OpenHow, ResolveFlag};
    use std::os::fd::{AsRawFd, FromRawFd};
    match nix::sys::stat::mkdirat(Some(parent.as_raw_fd()), name, nix::sys::stat::Mode::from_bits_truncate(mode)) {
        Ok(()) | Err(nix::errno::Errno::EEXIST) => {}
        Err(_) => return None,
    }
    let fd = openat2(parent.as_raw_fd(), name, OpenHow::new().flags(OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC)
        .resolve(ResolveFlag::RESOLVE_NO_SYMLINKS | ResolveFlag::RESOLVE_BENEATH)).ok()?;
    let fd = unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) };
    let st = nix::sys::stat::fstat(fd.as_raw_fd()).ok()?;
    if st.st_uid != uid {
        if !nix::unistd::geteuid().is_root() || st.st_uid != 0 {
            return None;
        }
        nix::unistd::fchown(fd.as_raw_fd(), Some(nix::unistd::Uid::from_raw(uid)), Some(nix::unistd::Gid::from_raw(gid))).ok()?;
    }
    Some(fd)
}

/// Makes `p` (a folder in the home) and the folders on the way, as the person, without following links.
pub fn ensure_dir(home: &Path, p: &Path, uid: u32, gid: u32) -> Option<PathBuf> {
    let home = real_home(home);
    let rel = p.strip_prefix(&home).ok()?;
    let mut fd: std::os::fd::OwnedFd = std::fs::File::open(&home).ok()?.into();
    for c in rel.components() {
        let name = c.as_os_str().to_str()?;
        if name == ".." || name == "." {
            return None;
        }
        fd = mkdir_as(&fd, name, uid, gid, 0o755)?;
    }
    Some(p.to_path_buf())
}

/// A copy of the person's cargo settings with only what says where crates come from (registries, mirrors, network
/// and http settings): never what runs something (`build.rustc-wrapper`, a target's `runner` or `linker`, aliases) nor
/// tokens. Empty when there is nothing to copy.
pub fn cargo_settings(person: &str) -> String {
    let Ok(v) = person.parse::<toml::Table>() else { return String::new() };
    let mut out = toml::Table::new();
    for key in ["registries", "source", "net", "http"] {
        if let Some(t) = v.get(key) {
            out.insert(key.to_string(), t.clone());
        }
    }
    // (no tokens, even inside a registry's entry; and no credential helpers, which run programs)
    if let Some(toml::Value::Table(regs)) = out.get_mut("registries") {
        for (_, r) in regs.iter_mut() {
            if let toml::Value::Table(r) = r {
                r.remove("token");
                r.remove("credential-provider");
            }
        }
    }
    if let Some(toml::Value::Table(reg)) = v.get("registry") {
        if let Some(d) = reg.get("default") {
            let mut t = toml::Table::new();
            t.insert("default".into(), d.clone());
            out.insert("registry".into(), toml::Value::Table(t));
        }
    }
    if let Some(toml::Value::Table(http)) = out.get_mut("http") {
        // (a CA file the person set is a path read, not run: kept; anything else that names a program is not)
        http.retain(|k, _| matches!(k as &str, "proxy" | "timeout" | "cainfo" | "check-revoke" | "multiplexing" | "low-speed-limit" | "debug" | "ssl-version"));
    }
    if out.is_empty() { String::new() } else { toml::to_string(&out).unwrap_or_default() }
}

/// Makes a grant's cache folder (`scope`) and one folder per tool, as the person, without following links, writes its
/// cargo settings (only where crates come from: `cargo_settings`) and returns it. None: it could not be made safely,
/// so commands get no cache.
pub fn agents_cache(home: &Path, scope: &str, uid: u32, gid: u32) -> Option<PathBuf> {
    use std::io::Write;
    use std::os::fd::{AsRawFd, FromRawFd};
    let home = real_home(home);
    let h: std::os::fd::OwnedFd = std::fs::File::open(&home).ok()?.into();
    let cache = mkdir_as(&h, ".cache", uid, gid, 0o700)?;
    let ours = mkdir_as(&cache, "kalango-agentes", uid, gid, 0o700)?;
    let grant = mkdir_as(&ours, scope, uid, gid, 0o700)?;
    for (_, sub) in CACHE_VARS {
        mkdir_as(&grant, sub, uid, gid, 0o700)?;
    }
    let cargo = mkdir_as(&grant, "cargo", uid, gid, 0o700)?;
    for sub in ["registry", "git"] {
        mkdir_as(&cargo, sub, uid, gid, 0o700)?;
    }
    // (the folders that hold settings are read only at their top, so what the tools make there is made here first)
    let gradle = mkdir_as(&grant, "gradle", uid, gid, 0o700)?;
    for sub in ["caches", "wrapper", "daemon", "native", "notifications", "jdks", "kotlin-profile", ".tmp"] {
        mkdir_as(&gradle, sub, uid, gid, 0o700)?;
    }
    let go = mkdir_as(&grant, "go", uid, gid, 0o700)?;
    mkdir_as(&go, "pkg", uid, gid, 0o700)?;
    // the files cargo changes in its home (made empty), and its settings: replaced each time, never followed
    let open_new = |name: &str, trunc: bool| -> Option<std::fs::File> {
        use nix::fcntl::{openat2, OFlag, OpenHow, ResolveFlag};
        let flags = OFlag::O_WRONLY | OFlag::O_CREAT | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW | if trunc { OFlag::O_TRUNC } else { OFlag::empty() };
        let fd = openat2(cargo.as_raw_fd(), name, OpenHow::new().flags(flags).mode(nix::sys::stat::Mode::from_bits_truncate(0o600))
            .resolve(ResolveFlag::RESOLVE_NO_SYMLINKS | ResolveFlag::RESOLVE_BENEATH)).ok()?;
        let f = unsafe { std::fs::File::from_raw_fd(fd) };
        if nix::unistd::geteuid().is_root() {
            nix::unistd::fchown(f.as_raw_fd(), Some(nix::unistd::Uid::from_raw(uid)), Some(nix::unistd::Gid::from_raw(gid))).ok()?;
        }
        Some(f)
    };
    for f in CARGO_FILES {
        open_new(f, false)?;
    }
    let person = std::fs::read_to_string(home.join(".cargo/config.toml")).or_else(|_| std::fs::read_to_string(home.join(".cargo/config"))).unwrap_or_default();
    open_new("config.toml", true)?.write_all(cargo_settings(&person).as_bytes()).ok()?;
    Some(home.join(".cache/kalango-agentes").join(scope))
}

/// Where kalangod keeps track of the agents' grant caches (in its own state folder, out of agents' reach): whose
/// they are, the folders of the grant and when a command last used them — so a cache goes when its grant goes, and
/// the rest by age and size (`prune_caches`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CacheEntry {
    pub uid: u32,
    pub home: PathBuf,
    pub scope: String,
    pub roots: Vec<PathBuf>,
    pub last: u64,
}

/// Caches unused this long are removed.
pub const CACHE_MAX_AGE_S: u64 = 30 * 24 * 3600;
/// All of a person's agents' caches together stay under this (the least recently used go first).
pub const CACHE_CAP_BYTES: u64 = 10 * 1024 * 1024 * 1024;

#[derive(Default)]
pub struct CacheIndex {
    file: Option<PathBuf>,
    entries: Mutex<Vec<CacheEntry>>,
}

impl CacheIndex {
    pub fn open(state_dir: &Path) -> CacheIndex {
        let file = state_dir.join("caches-dos-agentes.json");
        let entries = std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        CacheIndex { file: Some(file), entries: Mutex::new(entries) }
    }

    fn save(&self, all: &[CacheEntry]) {
        if let Some(p) = &self.file {
            let tmp = p.with_extension("json.tmp");
            if std::fs::write(&tmp, serde_json::to_string_pretty(all).unwrap_or_default()).is_ok() {
                let _ = std::fs::rename(&tmp, p);
            }
        }
    }

    /// A command used this grant's cache now.
    pub fn touch(&self, uid: u32, home: &Path, scope: &str, roots: &[PathBuf]) {
        let mut g = self.entries.lock().unwrap();
        let now = crate::timeline::now();
        match g.iter_mut().find(|e| e.uid == uid && e.scope == scope) {
            Some(e) => e.last = now,
            None => g.push(CacheEntry { uid, home: home.to_path_buf(), scope: scope.into(), roots: roots.to_vec(), last: now }),
        }
        let all = g.clone();
        drop(g);
        self.save(&all);
    }

    pub fn entries(&self) -> Vec<CacheEntry> {
        self.entries.lock().unwrap().clone()
    }

    fn forget(&self, gone: &[CacheEntry]) {
        let mut g = self.entries.lock().unwrap();
        g.retain(|e| !gone.contains(e));
        let all = g.clone();
        drop(g);
        self.save(&all);
    }
}

/// Which caches go: a grant that no longer exists (one of its folders is no longer a work folder, a project folder or
/// a task grant of the person), one unused for `max_age`, and — per person — the least recently used beyond `cap`.
/// `granted(uid, home)`: the person's folders now; `size(entry)`: the cache's size.
pub fn caches_to_prune(entries: &[CacheEntry], now: u64, max_age: u64, cap: u64, granted: &dyn Fn(u32, &Path) -> Vec<PathBuf>,
    size: &dyn Fn(&CacheEntry) -> u64) -> Vec<CacheEntry> {
    let mut out: Vec<CacheEntry> = entries.iter().filter(|e| {
        let g = granted(e.uid, &e.home);
        now.saturating_sub(e.last) > max_age || !e.roots.iter().all(|r| g.iter().any(|w| is_within(r, w)))
    }).cloned().collect();
    let mut uids: Vec<u32> = entries.iter().map(|e| e.uid).collect();
    uids.sort();
    uids.dedup();
    for uid in uids {
        let mut kept: Vec<&CacheEntry> = entries.iter().filter(|e| e.uid == uid && !out.contains(e)).collect();
        kept.sort_by_key(|e| e.last);
        let mut total: u64 = kept.iter().map(|e| size(e)).sum();
        for e in kept {
            if total <= cap {
                break;
            }
            total = total.saturating_sub(size(e));
            out.push(e.clone());
        }
    }
    out
}

/// A folder's size (links not followed).
fn dir_size(p: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![p.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let Ok(md) = e.path().symlink_metadata() else { continue };
            if md.is_dir() { stack.push(e.path()) } else { total += md.len() }
        }
    }
    total
}

/// Removes a folder whose contents may be read only (Go makes its module cache so): write permission back on each
/// folder first, through its file descriptor (opened without following links, so a folder swapped for a link while
/// this runs gives nothing to change), then the folder removed (`remove_dir_all` does not follow links either).
fn remove_tree(p: &Path) -> std::io::Result<()> {
    use nix::fcntl::OFlag;
    let flags = OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC;
    let top = nix::fcntl::open(p, flags, nix::sys::stat::Mode::empty()).map_err(std::io::Error::from)?;
    let top = unsafe { <std::os::fd::OwnedFd as std::os::fd::FromRawFd>::from_raw_fd(top) };
    unlock_tree(top, 0);
    std::fs::remove_dir_all(p)
}

fn unlock_tree(fd: std::os::fd::OwnedFd, depth: usize) {
    use nix::fcntl::OFlag;
    use std::os::fd::{AsRawFd, FromRawFd};
    let Ok(st) = nix::sys::stat::fstat(fd.as_raw_fd()) else { return };
    let _ = nix::sys::stat::fchmod(fd.as_raw_fd(), nix::sys::stat::Mode::from_bits_truncate((st.st_mode & 0o7777) | 0o700));
    if depth > 64 {
        return;
    }
    let Ok(mut dir) = nix::dir::Dir::from_fd(match nix::unistd::dup(fd.as_raw_fd()) { Ok(d) => d, Err(_) => return }) else { return };
    let names: Vec<std::ffi::CString> = dir.iter().flatten().filter(|e| e.file_type() != Some(nix::dir::Type::Symlink))
        .map(|e| e.file_name().to_owned()).filter(|n| n.as_bytes() != b"." && n.as_bytes() != b"..").collect();
    for n in names {
        let flags = OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC;
        if let Ok(child) = nix::fcntl::openat(Some(fd.as_raw_fd()), n.as_c_str(), flags, nix::sys::stat::Mode::empty()) {
            unlock_tree(unsafe { std::os::fd::OwnedFd::from_raw_fd(child) }, depth + 1);
        }
    }
}

/// Removes the caches `caches_to_prune` names (each reached without links, inside the person's own agents' cache
/// folder) and forgets them: at kalangod's start, every hour, and right after a folder is taken out.
pub fn prune_caches(st: &crate::server::State) -> Vec<String> {
    let entries = st.caches.entries();
    let granted = |uid: u32, home: &Path| -> Vec<PathBuf> {
        let h = real_home(home);
        let mut v = work_folders(&h, &protected(&st.cfg, &h));
        v.extend(st.folders.list(uid).into_iter().map(|p| p.caminho));
        v.extend(st.folders.task_grants(uid).into_iter().map(|t| t.caminho));
        v
    };
    let path = |e: &CacheEntry| real_home(&e.home).join(".cache/kalango-agentes").join(&e.scope);
    let gone = caches_to_prune(&entries, crate::timeline::now(), CACHE_MAX_AGE_S, CACHE_CAP_BYTES, &granted, &|e| dir_size(&path(e)));
    let mut removed = Vec::new();
    for e in &gone {
        let p = path(e);
        // (the very folder kalangod made, not a link put in its place)
        if e.scope.contains('/') || e.scope.starts_with('.') || identity(&p, &real_home(&e.home)).is_none() {
            continue;
        }
        if remove_tree(&p).is_ok() {
            removed.push(p.display().to_string());
        }
    }
    st.caches.forget(&gone);
    if !removed.is_empty() {
        eprintln!("kalangod: caches dos agentes removidos: {}", removed.join(", "));
    }
    removed
}

/// What the person's interface may not carry: another program loaded into it (`LD_PRELOAD`, `LD_AUDIT`, libraries or
/// GIO and pixbuf modules from elsewhere — GTK 4 does not load `GTK_MODULES`, which a desktop sets for accessibility)
/// unless listed (a development session preloads its layer-shell library), or a tracer attached.
pub fn tampered(pid: i32, allowed_env: &[String]) -> Option<String> {
    match inspect(pid, allowed_env, false) {
        Check::Tampered(why) => Some(why),
        Check::Clean => None,
        // what could not be looked at is not clean: it is refused (and said)
        Check::Unchecked(why) => {
            eprintln!("kalangod: ambiente do processo {pid} não conferido ({why})");
            Some(format!("unchecked: {why}"))
        }
    }
}

/// What looking at a process found.
#[derive(Debug, Clone, PartialEq)]
pub enum Check {
    Clean,
    Tampered(String),
    /// The environment could not be read (kalangod is not root and the process is not dumpable): neither clean nor
    /// tampered. A caller that must be sure refuses it.
    Unchecked(String),
}

/// Looks at a process for a tracer and for loader variables not in `allowed_env`. `strict` also takes the variables
/// that make a program load code or plugins from elsewhere without being a library path (Vulkan layers, PipeWire and
/// GStreamer plugins, GL drivers, EGL vendors, GTK modules): the Remote runs with none of them, while the person's
/// shell may carry some a desktop sets.
pub fn inspect(pid: i32, allowed_env: &[String], strict: bool) -> Check {
    let Ok(status) = std::fs::read_to_string(format!("/proc/{pid}/status")) else {
        return Check::Unchecked("status".into());
    };
    if traced(&status) {
        return Check::Tampered("TracerPid".into());
    }
    let env = match std::fs::read(format!("/proc/{pid}/environ")) {
        Ok(e) => e,
        Err(e) => return Check::Unchecked(format!("environ: {e}")),
    };
    match loader_vars_in(&env, allowed_env, strict) {
        Some(v) => Check::Tampered(v),
        None => Check::Clean,
    }
}

fn traced(status: &str) -> bool {
    status.lines().find_map(|l| l.strip_prefix("TracerPid:")).map(|v| v.trim() != "0").unwrap_or(true)
}

const LOADER_VARS: [&str; 6] = ["LD_PRELOAD=", "LD_AUDIT=", "LD_LIBRARY_PATH=", "GIO_EXTRA_MODULES=", "GIO_MODULE_DIR=", "GDK_PIXBUF_MODULE_FILE="];
const PLUGIN_VARS: [&str; 19] = [
    "VK_LAYER_PATH=", "VK_ADD_LAYER_PATH=", "SPA_PLUGIN_DIR=", "GST_PLUGIN_PATH=", "LIBGL_DRIVERS_PATH=",
    "__EGL_VENDOR_LIBRARY_FILENAMES=", "GTK_PATH=", "GTK_MODULES=", "GIO_MODULE_DIR=",
    "LIBVA_DRIVERS_PATH=", "GST_PLUGIN_PATH_1_0=", "GST_PLUGIN_SYSTEM_PATH=", "GST_PLUGIN_SYSTEM_PATH_1_0=", "GCONV_PATH=",
    "PIPEWIRE_MODULE_DIR=", "VK_ICD_FILENAMES=", "VK_DRIVER_FILES=", "GBM_BACKENDS_PATH=", "GDK_PIXBUF_MODULEDIR=",
];

#[cfg(test)]
fn loader_vars(env: &[u8], allowed_env: &[String]) -> Option<String> {
    loader_vars_in(env, allowed_env, false)
}

fn loader_vars_in(env: &[u8], allowed_env: &[String], strict: bool) -> Option<String> {
    env.split(|b| *b == 0).map(|kv| String::from_utf8_lossy(kv).into_owned()).find(|kv| {
        (LOADER_VARS.iter().chain(if strict { PLUGIN_VARS.iter() } else { [].iter() })).any(|k| kv.starts_with(k)) && !kv.ends_with('=') && !allowed_env.contains(kv)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_dirs_are_read_and_bad_entries_left_out() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("casa");
        for p in ["Documentos", "Baixados", "Imagens", ".config", "Músicas", "Vídeos", "Modelos", ".ssh"] {
            std::fs::create_dir_all(home.join(p)).unwrap();
        }
        std::fs::write(home.join(".config/user-dirs.dirs"), "# comentário\nXDG_DOCUMENTS_DIR=\"$HOME/Documentos\"\nXDG_DOWNLOAD_DIR=\"$HOME/Baixados\"\n\
            XDG_DESKTOP_DIR=\"$HOME/\"\nXDG_PICTURES_DIR=\"$HOME/.config\"\nXDG_MUSIC_DIR=\"/etc\"\nXDG_VIDEOS_DIR=\"$HOME/../outra\"\n").unwrap();
        let got = work_folders(&home, &[home.join(".ssh")]);
        // the home itself, a hidden folder, a folder outside the home: never; Modelos by its default name
        assert_eq!(got, vec![home.join("Documentos"), home.join("Baixados"), home.join("Modelos")]);
        // a link in place of a work folder is not followed
        std::fs::remove_dir(home.join("Modelos")).unwrap();
        std::os::unix::fs::symlink(home.join(".config"), home.join("Modelos")).unwrap();
        assert!(!work_folders(&home, &[]).contains(&home.join("Modelos")));
        // a file that is a link (to something an agent could write) is not read: the defaults are used
        std::fs::rename(home.join(".config/user-dirs.dirs"), home.join("Documentos/x")).unwrap();
        std::os::unix::fs::symlink(home.join("Documentos/x"), home.join(".config/user-dirs.dirs")).unwrap();
        assert_eq!(work_folders(&home, &[]), vec![home.join("Documentos"), home.join("Imagens"), home.join("Músicas"), home.join("Vídeos")]);
    }

    #[test]
    fn project_folders_are_checked() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("casa");
        for p in ["Projetos/site", "Projetos/ferramentas/bin", ".config/x", ".ssh", "dados/.ssh"] {
            std::fs::create_dir_all(home.join(p)).unwrap();
        }
        let prot = vec![home.join(".ssh"), home.join("dados/.ssh")];
        let home = real_home(&home);
        let startup = vec![home.join("Projetos/ferramentas/bin")];
        let c = |raw: &str| check_project(raw, &home, &prot, &startup);
        let (p, id) = c("~/Projetos/site").unwrap();
        assert_eq!(p, home.join("Projetos/site"));
        assert_eq!(Some(id), identity(&p, &home));
        assert!(c("~").is_err());
        assert!(c("~/.config/x").is_err());
        assert!(c("~/dados").is_err());
        assert!(c("/etc").is_err());
        assert!(c("~/não-existe").is_err());
        // through a link: refused, even to an ordinary folder (the folder itself is approved, never where a link leads)
        std::os::unix::fs::symlink(home.join(".config/x"), home.join("Projetos/atalho")).unwrap();
        assert!(c("~/Projetos/atalho").is_err());
        std::os::unix::fs::symlink(home.join("Projetos/site"), home.join("atalho-site")).unwrap();
        assert!(c("~/atalho-site").is_err());
        // a folder holding a PATH folder
        assert!(c("~/Projetos").is_err());
        // swapped for a link after being approved: the identity no longer opens
        std::fs::rename(home.join("Projetos/site"), home.join("Projetos/site.bak")).unwrap();
        std::os::unix::fs::symlink("..", home.join("Projetos/site")).unwrap();
        assert_eq!(identity(&home.join("Projetos/site"), &home), None);
    }

    #[test]
    fn writable_is_narrowed_to_the_callers_folders() {
        let p = |s: &str| PathBuf::from(s);
        assert_eq!(writable(&[p("/h/Documentos"), p("/h/Projetos/a")], &[p("/h")]), vec![p("/h/Documentos"), p("/h/Projetos/a")]);
        assert_eq!(writable(&[p("/h/Documentos"), p("/h/Projetos/a")], &[p("/h/Documentos/notas")]), vec![p("/h/Documentos/notas")]);
        assert!(writable(&[p("/h/Documentos")], &[p("/h/Imagens")]).is_empty());
    }

    #[test]
    fn caches_are_per_grant_and_settings_keep_only_where_crates_come_from() {
        let p = |s: &str| PathBuf::from(s);
        let a = cache_scope("agente", &[p("/h/Documentos")]);
        assert_eq!(a, cache_scope("agente", &[p("/h/Documentos"), p("/h/Documentos")]));
        assert_ne!(a, cache_scope("agente", &[p("/h/Documentos"), p("/h/Projetos/site")]));
        assert_ne!(a, cache_scope("outro", &[p("/h/Documentos")]));
        assert!(!a.contains('/'));
        let person = "[build]\nrustc-wrapper = \"/tmp/evil\"\n[target.x86_64-unknown-linux-gnu]\nrunner = \"evil\"\nlinker = \"evil\"\n[alias]\nb = \"build\"\n\
            [registries.empresa]\nindex = \"sparse+https://crates.empresa.local/\"\ntoken = \"segredo\"\ncredential-provider = \"evil\"\n\
            [source.crates-io]\nreplace-with = \"espelho\"\n[source.espelho]\nregistry = \"sparse+https://espelho.local/\"\n[net]\noffline = false\n\
            [registry]\ndefault = \"empresa\"\ntoken = \"segredo\"\n[http]\nproxy = \"http://proxy:3128\"\n";
        let out = cargo_settings(person);
        for bad in ["rustc-wrapper", "runner", "linker", "alias", "segredo", "credential-provider", "evil"] {
            assert!(!out.contains(bad), "{bad}: {out}");
        }
        for good in ["crates.empresa.local", "replace-with", "espelho.local", "proxy:3128", "default = \"empresa\""] {
            assert!(out.contains(good), "{good}: {out}");
        }
        assert_eq!(cargo_settings("[build]\nrustc-wrapper = \"x\"\n"), "");
    }

    #[test]
    fn removing_a_tree_unlocks_read_only_folders_and_follows_no_link() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let outside = d.path().join("fora");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::set_permissions(&outside, std::fs::Permissions::from_mode(0o555)).unwrap();
        let t = d.path().join("cache");
        std::fs::create_dir_all(t.join("go-mod/x/y")).unwrap();
        std::fs::write(t.join("go-mod/x/y/f"), "x").unwrap();
        std::os::unix::fs::symlink(&outside, t.join("go-mod/link")).unwrap();
        for p in ["go-mod/x/y", "go-mod/x", "go-mod"] {
            std::fs::set_permissions(t.join(p), std::fs::Permissions::from_mode(0o555)).unwrap();
        }
        remove_tree(&t).unwrap();
        assert!(!t.exists());
        // the folder a link pointed to keeps its mode
        assert_eq!(std::fs::metadata(&outside).unwrap().permissions().mode() & 0o777, 0o555);
        std::fs::set_permissions(&outside, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn caches_go_with_their_grant_by_age_and_by_size() {
        let p = |s: &str| PathBuf::from(s);
        let e = |scope: &str, roots: &[&str], last: u64| CacheEntry { uid: 1000, home: p("/h"), scope: scope.into(), roots: roots.iter().map(|r| p(r)).collect(), last };
        let day = 24 * 3600;
        let now = 100 * day;
        let all = vec![e("docs", &["/h/Documentos"], now), e("site", &["/h/Documentos", "/h/Projetos/site"], now), e("velho", &["/h/Documentos"], now - 31 * day),
            e("grande-1", &["/h/Documentos/a"], now - 2 * day), e("grande-2", &["/h/Documentos/b"], now - day)];
        let granted = |_: u32, _: &Path| vec![p("/h/Documentos")];
        let sizes = |x: &CacheEntry| if x.scope.starts_with("grande") { 6 } else { 1 };
        let names = |v: Vec<CacheEntry>| { let mut n: Vec<String> = v.into_iter().map(|x| x.scope).collect(); n.sort(); n };
        // the project's grant is gone; one cache is too old; over a cap of 10 the oldest big one goes
        assert_eq!(names(caches_to_prune(&all, now, 30 * day, 10, &granted, &sizes)), vec!["grande-1", "site", "velho"]);
        // nothing to remove
        assert!(caches_to_prune(&all[..1], now, 30 * day, 10, &granted, &sizes).is_empty());
    }

    #[test]
    fn the_persons_interface_carries_nothing_loaded_into_it() {
        assert!(traced("Name:\tx\nTracerPid:\t1234\n"));
        assert!(!traced("Name:\tx\nTracerPid:\t0\n"));
        let env = b"HOME=/h\0LD_PRELOAD=/tmp/evil.so\0PATH=/usr/bin\0";
        assert_eq!(loader_vars(env, &[]).as_deref(), Some("LD_PRELOAD=/tmp/evil.so"));
        assert_eq!(loader_vars(env, &["LD_PRELOAD=/tmp/evil.so".into()]), None);
        assert_eq!(loader_vars(b"HOME=/h\0LD_PRELOAD=\0", &[]), None);
        assert!(loader_vars(b"GIO_EXTRA_MODULES=/tmp/evil\0", &[]).is_some());
        assert_eq!(loader_vars(b"GTK_MODULES=gail:atk-bridge\0", &[]), None);
        // the Remote's list also has the variables that load plugins, layers and drivers from elsewhere
        for v in ["LIBVA_DRIVERS_PATH=/tmp/x", "GST_PLUGIN_PATH_1_0=/tmp/x", "GST_PLUGIN_SYSTEM_PATH=/tmp/x", "GST_PLUGIN_SYSTEM_PATH_1_0=/tmp/x", "GCONV_PATH=/tmp/x", "PIPEWIRE_MODULE_DIR=/tmp/x", "VK_ICD_FILENAMES=/tmp/x.json", "VK_DRIVER_FILES=/tmp/x.json", "GBM_BACKENDS_PATH=/tmp/x", "GDK_PIXBUF_MODULEDIR=/tmp/x", "VK_LAYER_PATH=/tmp/x", "VK_ADD_LAYER_PATH=/tmp/x", "SPA_PLUGIN_DIR=/tmp/x", "GST_PLUGIN_PATH=/tmp/x", "LIBGL_DRIVERS_PATH=/tmp/x", "__EGL_VENDOR_LIBRARY_FILENAMES=/tmp/x.json", "GTK_PATH=/tmp/x", "GTK_MODULES=evil", "GIO_MODULE_DIR=/tmp/x"] {
            let env = format!("HOME=/h\0{v}\0");
            assert_eq!(loader_vars_in(env.as_bytes(), &[], true).as_deref(), Some(v), "{v}");
            assert_eq!(loader_vars_in(env.as_bytes(), &[v.to_string()], true), None, "{v} allowed");
        }
        assert_eq!(loader_vars_in(b"GTK_MODULES=gail\0", &[], false), None, "the person's shell may carry what a desktop sets");
        // a process that cannot be looked at (gone, or its environment is not readable) is not "clean" for anyone
        assert!(tampered(i32::MAX, &[]).is_some_and(|w| w.starts_with("unchecked")));
        assert!(matches!(inspect(i32::MAX, &[], false), Check::Unchecked(_)));
    }

    #[test]
    fn installs_are_recognized() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path();
        for p in [".local/lib", ".local/bin", ".cargo"] {
            std::fs::create_dir_all(home.join(p)).unwrap();
        }
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let i = installs_for_user("pip", &a(&["install", "--user", "requests"]), home).unwrap();
        assert_eq!(i.writes, vec![home.join(".local/lib"), home.join(".local/bin")]);
        std::fs::create_dir_all(home.join(".local/lib/python3.12")).unwrap();
        let i = installs_for_user("pip", &a(&["install", "--user", "requests"]), home).unwrap();
        assert_eq!(i.writes, vec![home.join(".local/lib/python3.12"), home.join(".local/bin")]);
        assert!(installs_for_user("/usr/bin/python3", &a(&["-m", "pip", "install", "--user", "x"]), home).is_some());
        assert!(installs_for_user("pip", &a(&["install", "-r", "req.txt"]), home).is_none());
        assert!(installs_for_user("npm", &a(&["install", "-g", "x"]), home).is_some());
        assert!(installs_for_user("npm", &a(&["install", "x"]), home).is_none());
        for g in [&["install", "--location=global", "x"][..], &["i", "--global=true", "x"], &["install", "--location", "global", "x"]] {
            assert!(installs_for_user("npm", &a(g), home).is_some(), "{g:?}");
        }
        // inside a shell's script
        assert!(installs_for_user("/bin/sh", &a(&["-c", "cd proj && FOO=1 pip install --user requests; echo ok"]), home).is_some());
        assert!(installs_for_user("bash", &a(&["-lc", "npm i -g x | tee log"]), home).is_some());
        assert!(installs_for_user("sh", &a(&["-c", "pip install -r req.txt && cargo build"]), home).is_none());
        let c = installs_for_user("cargo", &a(&["install", "ripgrep"]), home).unwrap();
        assert!(c.keep.contains(&home.join(".cargo/env")));
        assert_eq!(c.unset, &["CARGO_HOME"]);
        // ~/go/bin does not exist yet: still shown, made when approved
        let g = installs_for_user("go", &a(&["install", "x@latest"]), home).unwrap();
        assert_eq!(g.writes, vec![home.join("go/bin")]);
        assert_eq!(g.unset, &["GOPATH"]);
        assert!(installs_for_user("cargo", &a(&["build"]), home).is_none());
    }
}
