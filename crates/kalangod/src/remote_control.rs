//! Controle remoto da tela: quem pode capturar a tela e injetar teclado e ponteiro no niri.
//!
//! O niri do KalangoOS (patch 0008) só oferece wlr-screencopy, o ponteiro virtual e o teclado virtual aos
//! processos que o arquivo `controle_remoto` permite. Este módulo é o único que escreve esse arquivo (o kalangod
//! roda como root; um programa da sessão não consegue se dar a permissão):
//!
//! - `exe <caminho> <capacidades>`: sempre, para os programas do sistema na configuração (o teclado na tela do
//!   shell usa o teclado virtual);
//! - `pid <pid> <início> <capacidades>`: uma sessão do Kalango Remote aprovada pela pessoa. Só o próprio app
//!   (`controle_remoto_apps`) pede, só para o seu processo do núcleo (`kalango-remote-core`, filho dele, do mesmo
//!   usuário), e a permissão sai quando ele pede ou quando o processo termina.

use crate::peer::Peer;
use kalango_i18n::{tr, trf};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// `privacidade`: the local monitors are covered in black while the session lasts (niri patch 0012); the app asks
/// for it only for a session nobody approved on this computer.
pub const CAPABILITIES: [&str; 4] = ["captura", "ponteiro", "teclado", "privacidade"];

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Grant {
    pub pid: i32,
    pub start: u64,
    pub capabilities: Vec<String>,
    pub uid: u32,
    /// A Kalango Remote core: the grant lasts only while the process stays non-dumpable and untraced (see
    /// [`hardened`]); kalangod's own helpers are not checked this way.
    #[serde(skip)]
    pub strict: bool,
    /// What a strict grant is looked at against every second (set by [`RemoteControl::allow`]).
    #[serde(skip)]
    pub guard: Option<Guard>,
}

/// The configuration a core is held to for as long as it has the permission.
#[derive(Debug, Clone, PartialEq)]
pub struct Guard {
    pub env: Vec<String>,
    pub libs: Vec<String>,
    /// The file the core ran when the permission was given (device, inode, owner, mode): what it must still run
    /// every second after. An `exec` into another program keeps the pid and the start time, so only this tells.
    pub exe_id: Option<FileId>,
}

impl Guard {
    /// The files the core may be running, as remembered when it was given the permission, never looked up again by path.
    pub fn trusted_exe(&self) -> Vec<Option<FileId>> {
        vec![self.exe_id]
    }
}

/// Called with a grant that was taken away because its core stopped being safe, and why (the Timeline records it).
pub type Hook = Box<dyn Fn(&Grant, &str) + Send + Sync>;

/// What kalangod's configuration says about who may be given the screen.
pub struct Rules<'a> {
    /// the app's executables (`controle_remoto_apps`)
    pub apps: &'a [String],
    /// loader variables the app and its core may carry (what the image's script sets)
    pub env: &'a [String],
    /// folders the executable code mapped into them may come from (as root: root's own, nobody else's to write)
    pub libs: &'a [String],
}

pub struct RemoteControl {
    file: PathBuf,
    always: Vec<String>,
    grants: Arc<Mutex<Vec<Grant>>>,
    hook: Arc<Mutex<Option<Hook>>>,
}

/// What the kernel says about a process: parent, user, executable and start time (field 22 of /proc/pid/stat).
#[derive(Debug, Clone, PartialEq)]
pub struct Process {
    pub pid: i32,
    pub ppid: i32,
    pub uid: u32,
    pub exe: String,
    pub start: u64,
}

pub fn process(pid: i32) -> Option<Process> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let fields: Vec<&str> = stat.rsplit_once(')')?.1.split_whitespace().collect();
    let ppid = fields.get(1)?.parse().ok()?;
    let start = fields.get(19)?.parse().ok()?;
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let uid = status.lines().find_map(|l| l.strip_prefix("Uid:"))?.split_whitespace().next()?.parse().ok()?;
    let exe = std::fs::read_link(format!("/proc/{pid}/exe")).ok()?.display().to_string();
    Some(Process { pid, ppid, uid, exe, start })
}

/// May the app `caller` (identified by the kernel) give the permission to the process `target`?
pub fn validate(caller: &Peer, apps: &[String], target: &Process, capabilities: &[String]) -> Result<(), String> {
    if !apps.iter().any(|a| a == &caller.exe) {
        return Err(trf("{exe} is not Kalango Remote", &[("exe", caller.exe.clone())]));
    }
    if target.ppid != caller.pid || target.uid != caller.uid {
        return Err(trf("process {pid} is not this Kalango Remote's", &[("pid", target.pid.to_string())]));
    }
    let core = Path::new(&caller.exe).with_file_name("kalango-remote-core");
    if Path::new(&target.exe) != core {
        return Err(trf("{exe} is not Kalango Remote's core", &[("exe", target.exe.clone())]));
    }
    if capabilities.is_empty() || capabilities.iter().any(|c| !CAPABILITIES.contains(&c.as_str())) {
        return Err(tr("capabilities must be captura, ponteiro, teclado or privacidade").to_string());
    }
    Ok(())
}

/// A file as the kernel names it: which one (device and inode), whose, and who may write it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileId {
    pub dev: u64,
    pub ino: u64,
    pub uid: u32,
    pub mode: u32,
}

impl FileId {
    /// Only root can change it: owned by root, no write for group or others.
    pub fn root_only(&self) -> bool {
        self.uid == 0 && self.mode & 0o022 == 0
    }
}

/// The file behind a path or a /proc link (symbolic links followed): `/proc/<pid>/exe` and `/proc/<pid>/map_files/<range>`
/// name the very file the process runs or has mapped, whatever path text it is shown under in its own mount namespace.
pub fn file_id(path: &str) -> Option<FileId> {
    use std::os::unix::fs::MetadataExt;
    let m = std::fs::metadata(path).ok()?;
    Some(FileId { dev: m.dev(), ino: m.ino(), uid: m.uid(), mode: m.mode() })
}

/// The mount and user namespaces of a process, as the kernel names them (None: could not be read).
fn namespaces(pid: &str) -> Vec<Option<String>> {
    ["mnt", "user"].iter().map(|n| std::fs::read_link(format!("/proc/{pid}/ns/{n}")).ok().map(|p| p.display().to_string())).collect()
}

/// A process in another mount or user namespace (`bwrap`, `unshare`, a container) can show kalangod's trusted paths
/// bound over files of its own: its /proc path text, its exe link and its maps all read as the system's. Only a
/// process in kalangod's own namespaces is judged by what it shows. Anything that could not be read is not the same.
pub fn same_namespaces(own: &[Option<String>], theirs: &[Option<String>]) -> bool {
    own.len() == theirs.len() && own.iter().zip(theirs).all(|(a, b)| a.is_some() && a == b)
}

/// The process runs one of the trusted files themselves (same device and inode as a file kalangod sees, not a file
/// that only carries the same name), and that file is root's alone.
pub fn runs_trusted_file(exe: Option<FileId>, trusted: &[Option<FileId>]) -> bool {
    exe.is_some_and(|e| e.root_only() && trusted.iter().flatten().any(|t| t.dev == e.dev && t.ino == e.ino))
}

/// What to do with what looking at the environment found. `root`: only root can read a protected process's environment,
/// so for a root kalangod "could not be checked" is a refusal; a user's kalangod (development) says so in the log and
/// goes on. It is never "clean".
pub fn judge_check(root: bool, exe: &str, pid: i32, check: crate::allowlist::Check) -> Result<(), String> {
    use crate::allowlist::Check;
    match check {
        Check::Clean => Ok(()),
        Check::Tampered(why) => Err(trf("{exe} runs with something else loaded into it or under a tracer ({why})", &[("exe", exe.to_owned()), ("why", why)])),
        Check::Unchecked(why) if root => Err(trf("{exe} could not be checked ({why})", &[("exe", exe.to_owned()), ("why", why)])),
        Check::Unchecked(why) => {
            eprintln!("kalangod: {exe} (pid {pid}): ambiente não conferido ({why}); o kalangod não é root");
            Ok(())
        }
    }
}

/// The trusted file and the one a process runs are the same file (device and inode). A user's kalangod cannot read the
/// link of a protected process (`running` is None): there it is not asked, a root kalangod always is.
pub fn same_file(trusted: Option<FileId>, running: Option<FileId>, root: bool) -> bool {
    match (trusted, running) {
        (Some(t), Some(r)) => t.dev == r.dev && t.ino == r.ino,
        (Some(_), None) => !root,
        _ => false,
    }
}

/// The process still runs the trusted file (checked by device and inode of /proc/<pid>/exe).
pub fn exe_is_trusted(who: &str, pid: i32, trusted: &[Option<FileId>]) -> Result<(), String> {
    if runs_trusted_file(file_id(&format!("/proc/{pid}/exe")), trusted) {
        Ok(())
    } else {
        Err(trf("{exe} is not running the file kalangod trusts", &[("exe", who.to_owned())]))
    }
}

/// Everything about a running Remote process that does not need the other one, as root: the environment, the
/// namespaces, the file it runs, and the code mapped into it. Used when the permission is given and again every second.
pub fn process_is_clean(who: &str, pid: i32, trusted_exe: &[Option<FileId>], env: &[String], libs: &[String], anon_exec_ok: bool) -> Result<(), String> {
    judge_check(true, who, pid, crate::allowlist::inspect(pid, env, true))?;
    if !same_namespaces(&namespaces("self"), &namespaces(&pid.to_string())) {
        return Err(trf("{exe} runs in another mount or user namespace", &[("exe", who.to_owned())]));
    }
    exe_is_trusted(who, pid, trusted_exe)?;
    let maps = std::fs::read_to_string(format!("/proc/{pid}/maps")).map_err(|e| trf("{exe} could not be checked ({why})", &[("exe", who.to_owned()), ("why", e.to_string())]))?;
    if let Some(path) = foreign_mapping(&maps, libs, anon_exec_ok, &|range| file_id(&format!("/proc/{pid}/map_files/{range}")), &file_id) {
        return Err(trf("{exe} has executable code from {path} mapped into it", &[("exe", who.to_owned()), ("path", path)]));
    }
    Ok(())
}

/// The names alone are not enough: the same program run with another library loaded into it (`LD_PRELOAD`, `LD_AUDIT`),
/// or under a tracer, is whatever its user made of it, and it would get the screen, the keyboard and the privacy cover
/// without the person's approval. So neither the app nor its core may carry loader variables beyond `rules.env`
/// (what the image's script sets) or be traced, and both must be out of reach of the user's other processes
/// ([`hardened`]). As root, also ([`process_is_clean`]): the same mount and user namespaces as kalangod, the very trusted
/// file as the executable (device and inode, not path text), and no executable code mapped that is not a root-only file of
/// `rules.libs` (checked through /proc/<pid>/map_files, so the file is the one that is mapped, and nothing deleted).
/// The app may have anonymous executable memory (GL shader compilers and GObject closures use it); its core has none.
/// A user's kalangod (a development session) cannot read a protected process: it says so in the log and goes on.
pub fn untampered(caller: &Peer, target: &Process, rules: &Rules) -> Result<(), String> {
    let root = nix::unistd::geteuid().is_root();
    for (who, pid) in [(&caller.exe, caller.pid), (&target.exe, target.pid)] {
        judge_check(root, who, pid, crate::allowlist::inspect(pid, rules.env, true))?;
    }
    // the app asking and the core getting the rights must both be out of reach of the user's other processes
    for (who, pid) in [(&caller.exe, caller.pid), (&target.exe, target.pid)] {
        hardened(pid).map_err(|why| trf("{exe} can be traced or read by the user's other processes ({why}); it must call prctl(PR_SET_DUMPABLE, 0) first", &[("exe", who.clone()), ("why", why)]))?;
    }
    if root {
        let core_path = Path::new(&caller.exe).with_file_name("kalango-remote-core").display().to_string();
        if !crate::approval::trusted_approver(&caller.exe, &rules.apps.iter().map(PathBuf::from).collect::<Vec<_>>()) || !crate::approval::trusted_approver(&target.exe, &[PathBuf::from(&core_path)]) {
            return Err(trf("{exe} or its folder can be replaced by another account", &[("exe", caller.exe.clone())]));
        }
        let apps: Vec<Option<FileId>> = rules.apps.iter().map(|p| file_id(p)).collect();
        process_is_clean(&caller.exe, caller.pid, &apps, rules.env, rules.libs, true)?;
        process_is_clean(&target.exe, target.pid, &[file_id(&core_path)], rules.env, rules.libs, false)?;
    }
    Ok(())
}

/// The first executable mapping in the text of /proc/<pid>/maps that is not a root-only file from one of `dirs`, as the
/// kernel names the mapped file itself (`mapped`: its identity by the map_files link of the range; `at`: the file at that
/// path in kalangod's view; they must be the same file). Nothing deleted counts, whatever its old path; nothing
/// anonymous does unless `anon_exec_ok`; the vdso and vsyscall pages are the kernel's; mappings that cannot execute are not code.
pub fn foreign_mapping(maps: &str, dirs: &[String], anon_exec_ok: bool, mapped: &dyn Fn(&str) -> Option<FileId>, at: &dyn Fn(&str) -> Option<FileId>) -> Option<String> {
    for line in maps.lines() {
        let mut f = line.split_whitespace();
        let (Some(range), Some(perms)) = (f.next(), f.next()) else { continue };
        let _ = (f.next(), f.next(), f.next()); // offset, device, inode
        let path: String = f.collect::<Vec<_>>().join(" ");
        if !perms.contains('x') {
            continue; // not code
        }
        if path == "[vdso]" || path == "[vsyscall]" {
            continue;
        }
        if path.is_empty() {
            if anon_exec_ok {
                continue;
            }
            return Some("(memória anônima executável)".into());
        }
        if !path.starts_with('/') || path.ends_with(" (deleted)") || !dirs.iter().any(|d| path.starts_with(d.as_str())) {
            return Some(path);
        }
        match (mapped(range), at(&path)) {
            (Some(m), Some(a)) if m.root_only() && m.dev == a.dev && m.ino == a.ino => {}
            _ => return Some(path),
        }
    }
    None
}

/// Is the core of a grant still what it was given to? Not traced, not dumpable and, for a root kalangod, still clean
/// by `clean` (the environment, the namespaces, the file it runs, the code mapped into it). Kalangod's own helpers are not
/// asked this.
fn still_safe(g: &Grant, root: bool, clean: &dyn Fn(i32, &Guard) -> Result<(), String>) -> Result<(), String> {
    if !g.strict {
        return Ok(());
    }
    hardened(g.pid)?;
    match &g.guard {
        Some(gd) if root => clean(g.pid, gd),
        _ => Ok(()),
    }
}

/// Looks at every grant again and rewrites the file when one is gone.
fn recheck(grants: &Mutex<Vec<Grant>>, hook: &Mutex<Option<Hook>>, file: &Path, always: &[String]) {
    let Ok(mut v) = grants.lock() else { return };
    let before = v.len();
    RemoteControl::prune(&mut v, hook);
    if v.len() != before {
        eprintln!("kalangod: controle remoto: {} permissão(ões) caíram", before - v.len());
        write_file(file, always, &v);
    }
}

/// Ends the core of a grant that was taken away: niri looks at who may capture the screen and type only when a client
/// binds the globals, so taking the line out of the file does not stop a session that is already going. Only the
/// process the grant was for (same pid AND start time) is killed.
fn end_core(g: &Grant) -> bool {
    // (never itself: the tests give a grant to the process they run in)
    if g.pid as u32 == std::process::id() || !process(g.pid).is_some_and(|p| p.start == g.start) {
        return false;
    }
    // SAFETY: kill on a pid that was just checked against its start time.
    unsafe { libc::kill(g.pid, libc::SIGKILL) == 0 }
}

/// The text of the file niri reads.
pub fn file_text(always: &[String], grants: &[Grant]) -> String {
    let mut s = String::new();
    for line in always {
        // "path capabilities"
        if let Some((path, caps)) = line.rsplit_once(' ') {
            s.push_str(&format!("exe {path} {caps}\n"));
        }
    }
    for g in grants {
        s.push_str(&format!("pid {} {} {}\n", g.pid, g.start, g.capabilities.join(",")));
    }
    s
}

/// Writes the whole file (temporary + rename: niri never reads half of it), 0644, owned by kalangod's user.
fn write_file(file: &Path, always: &[String], grants: &[Grant]) {
    use std::os::unix::fs::PermissionsExt;
    if let Some(d) = file.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let tmp = file.with_extension("tmp");
    if std::fs::write(&tmp, file_text(always, grants)).is_ok() {
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644));
        let _ = std::fs::rename(&tmp, file);
    }
}

/// Whether a process is out of reach of the other processes of its user: not traced, and not dumpable. A dumpable
/// process can be ptrace-attached (Yama's ptrace_scope 1 only limits that to its ancestors) and its memory and
/// environment read by anything of the same user; the kernel then makes the files in /proc/<pid> root's, which is how
/// it shows from outside (`status` owned by root). The app and its core call `prctl(PR_SET_DUMPABLE, 0)` as the first
/// thing they do. A root process cannot be told apart this way and is not asked to be.
pub fn hardened(pid: i32) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).map_err(|e| e.to_string())?;
    let owner = std::fs::metadata(format!("/proc/{pid}/status")).map_err(|e| e.to_string())?.uid();
    hardened_from(&status, owner)
}

fn hardened_from(status: &str, status_owner: u32) -> Result<(), String> {
    let field = |k: &str| status.lines().find_map(|l| l.strip_prefix(k)).map(|v| v.split_whitespace().next().unwrap_or("").to_owned());
    if field("TracerPid:").is_none_or(|v| v != "0") {
        return Err("TracerPid".into());
    }
    let uid: u32 = field("Uid:").and_then(|v| v.parse().ok()).ok_or("Uid")?;
    if uid != 0 && status_owner != 0 {
        return Err("dumpable".into());
    }
    Ok(())
}

impl RemoteControl {
    pub fn new(file: PathBuf, always: Vec<String>) -> RemoteControl {
        write_file(&file, &always, &[]);
        let c = RemoteControl { file, always, grants: Arc::new(Mutex::new(Vec::new())), hook: Arc::new(Mutex::new(None)) };
        // a grant does not outlive what made it safe: once a second the processes are looked at again (the thread
        // ends with the RemoteControl)
        let (grants, hook, file, always) = (c.grants.clone(), c.hook.clone(), c.file.clone(), c.always.clone());
        std::thread::spawn(move || {
            while Arc::strong_count(&grants) > 1 {
                std::thread::sleep(std::time::Duration::from_secs(1));
                recheck(&grants, &hook, &file, &always);
            }
        });
        c
    }

    /// Who is told when a core loses its grant for being unsafe (the Timeline).
    pub fn on_revoked(&self, hook: Hook) {
        if let Ok(mut h) = self.hook.lock() {
            *h = Some(hook);
        }
    }

    /// Writes the whole file.
    fn write(&self, grants: &[Grant]) {
        write_file(&self.file, &self.always, grants);
    }

    /// Drops the grants of processes that ended (or whose pid was reused), and those of a core that is no longer
    /// hardened: traced, or dumpable again. That core is ended (SIGKILL) and the hook told, because a session already
    /// going would otherwise go on: niri only looks at the file when a client connects.
    fn prune(v: &mut Vec<Grant>, hook: &Mutex<Option<Hook>>) {
        let mut lost: Vec<(Grant, String)> = Vec::new();
        v.retain(|g| {
            if !process(g.pid).is_some_and(|p| p.start == g.start) {
                return false;
            }
            let state = still_safe(g, nix::unistd::geteuid().is_root(), &|pid, gd| {
                // (the files it runs and has mapped, its namespaces and its environment, again: a library that came in after
                // the permission was given is as foreign as one that was there before)
                let core = process(pid).map(|p| p.exe).unwrap_or_default();
                process_is_clean(&core, pid, &gd.trusted_exe(), &gd.env, &gd.libs, false)
            });
            match state {
                Ok(()) => true,
                Err(why) => {
                    lost.push((g.clone(), why));
                    false
                }
            }
        });
        for (g, why) in lost {
            let ended = end_core(&g);
            eprintln!("kalangod: controle remoto: o núcleo {} perdeu a proteção ({why}); {}", g.pid, if ended { "encerrado" } else { "não foi preciso encerrar" });
            if let Ok(h) = hook.lock() {
                if let Some(f) = h.as_ref() {
                    f(&g, &why);
                }
            }
        }
    }

    pub fn allow(&self, caller: &Peer, rules: &Rules, pid: i32, capabilities: Vec<String>) -> Result<Grant, String> {
        let target = process(pid).ok_or_else(|| trf("process {pid} does not exist", &[("pid", pid.to_string())]))?;
        validate(caller, rules.apps, &target, &capabilities)?;
        untampered(caller, &target, rules)?;
        // what is remembered is the TRUSTED file (the one `untampered` looked at by path), and it must be the one the core runs
        // at this very moment: an exec between the check and here would otherwise be remembered as the trusted file
        let core_path = Path::new(&caller.exe).with_file_name("kalango-remote-core").display().to_string();
        let trusted = file_id(&core_path);
        let running = file_id(&format!("/proc/{pid}/exe"));
        if !same_file(trusted, running, nix::unistd::geteuid().is_root()) {
            return Err(trf("{exe} is not running the file kalangod trusts", &[("exe", target.exe.clone())]));
        }
        let g = Grant { pid, start: target.start, capabilities, uid: target.uid, strict: true, guard: Some(Guard { env: rules.env.to_vec(), libs: rules.libs.to_vec(), exe_id: trusted }) };
        let mut v = self.grants.lock().unwrap();
        Self::prune(&mut v, &self.hook);
        v.retain(|x| x.pid != pid);
        v.push(g.clone());
        self.write(&v);
        Ok(g)
    }

    pub fn revoke(&self, caller: &Peer, pid: i32) -> Result<(), String> {
        let mut v = self.grants.lock().unwrap();
        // only the user who got it (or root) takes it away
        if caller.uid != 0 && v.iter().any(|g| g.pid == pid && g.uid != caller.uid) {
            return Err(tr("the permission belongs to another user").to_string());
        }
        v.retain(|g| g.pid != pid);
        Self::prune(&mut v, &self.hook);
        self.write(&v);
        Ok(())
    }

    /// kalangod's own screen helper (an agent's screen session, `screen_control.rs`): kalangod started it, so no
    /// caller is checked here; `agente` marks the grant as an agent's (niri patch 0011 then reports the person's own
    /// input, and the session yields).
    pub fn allow_own(&self, pid: i32, capabilities: &[&str]) -> Result<Grant, String> {
        let target = process(pid).ok_or_else(|| trf("process {pid} does not exist", &[("pid", pid.to_string())]))?;
        let g = Grant { pid, start: target.start, capabilities: capabilities.iter().map(|c| c.to_string()).collect(), uid: target.uid, strict: false, guard: None };
        let mut v = self.grants.lock().unwrap();
        Self::prune(&mut v, &self.hook);
        v.retain(|x| x.pid != pid);
        v.push(g.clone());
        self.write(&v);
        Ok(g)
    }

    /// Takes the helper's grant away (its session ended).
    pub fn revoke_own(&self, pid: i32) {
        let mut v = self.grants.lock().unwrap();
        v.retain(|g| g.pid != pid);
        Self::prune(&mut v, &self.hook);
        self.write(&v);
    }

    pub fn list(&self, uid: u32) -> Vec<Grant> {
        let mut v = self.grants.lock().unwrap();
        Self::prune(&mut v, &self.hook);
        v.iter().filter(|g| uid == 0 || g.uid == uid).cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(pid: i32, exe: &str) -> Peer {
        Peer { pid, uid: 1000, gid: 1000, exe: exe.into(), cgroup: String::new(), home: "/home/x".into(), passwd_home: "/home/x".into(), user: "x".into(), has_tty: false }
    }

    #[test]
    fn only_the_app_for_its_own_core() {
        let apps = vec!["/usr/bin/kalango-remote".to_string()];
        let app = peer(100, "/usr/bin/kalango-remote");
        let core = Process { pid: 101, ppid: 100, uid: 1000, exe: "/usr/bin/kalango-remote-core".into(), start: 5 };
        let all: Vec<String> = CAPABILITIES.iter().map(|s| s.to_string()).collect();
        assert_eq!(validate(&app, &apps, &core, &all), Ok(()));
        assert!(validate(&peer(100, "/usr/bin/grim"), &apps, &core, &all).is_err(), "another program asking");
        assert!(validate(&app, &apps, &Process { ppid: 99, ..core.clone() }, &all).is_err(), "another app's core");
        assert!(validate(&app, &apps, &Process { exe: "/usr/bin/grim".into(), ..core.clone() }, &all).is_err(), "a child that is not the core");
        assert!(validate(&app, &apps, &Process { uid: 0, ..core.clone() }, &all).is_err(), "another user's");
        assert!(validate(&app, &apps, &core, &["tudo".to_string()]).is_err());
        assert!(validate(&app, &apps, &core, &["agente".to_string()]).is_err(), "only kalangod's own helper marks an agent");
        assert_eq!(validate(&app, &apps, &core, &["captura".to_string(), "privacidade".to_string()]), Ok(()));
        assert!(validate(&app, &apps, &core, &[]).is_err());
    }

    fn spawn_with(env: &[(&str, &str)], traced: bool) -> std::process::Child {
        use std::os::unix::process::CommandExt;
        let mut c = std::process::Command::new("/usr/bin/sleep");
        c.arg("30").env_clear().envs(env.iter().copied());
        if traced {
            // SAFETY: only ptrace(PTRACE_TRACEME), async-signal-safe, between fork and exec.
            unsafe { c.pre_exec(|| { libc::ptrace(libc::PTRACE_TRACEME, 0, 0, 0); Ok(()) }); }
        }
        let child = c.spawn().unwrap();
        // (until it has exec'd, /proc/pid/environ is still the parent's)
        for _ in 0..200 {
            let env_ready = env.is_empty() || std::fs::read(format!("/proc/{}/environ", child.id())).is_ok_and(|b| b.starts_with(env[0].0.as_bytes()));
            if env_ready && std::fs::read_link(format!("/proc/{}/exe", child.id())).is_ok_and(|e| e.ends_with("sleep")) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        child
    }

    fn me(pid: i32, exe: &str) -> Peer {
        peer(pid, exe)
    }

    #[test]
    fn a_library_loaded_into_the_app_or_a_tracer_stops_the_permission() {
        let allowed = vec!["LD_LIBRARY_PATH=/usr/lib/kalango/remote".to_string()];
        let apps = vec!["/usr/bin/kalango-remote".to_string()];
        let libs = vec!["/usr/lib/".to_string()];
        let rules = Rules { apps: &apps, env: &allowed, libs: &libs };
        let target = |pid| Process { pid, ppid: 1, uid: 1000, exe: "/usr/lib/kalango/remote/kalango-remote-core".into(), start: 1 };
        let reap = |mut c: std::process::Child| { let _ = c.kill(); let _ = c.wait(); };
        // the image's own environment (the script sets exactly this) passes the loader checks...
        let ok = spawn_with(&[("LD_LIBRARY_PATH", "/usr/lib/kalango/remote")], false);
        assert_eq!(crate::allowlist::tampered(ok.id() as i32, &allowed), None);
        // ...an LD_PRELOAD, an LD_AUDIT, another LD_LIBRARY_PATH, or a tracer do not
        for (k, v) in [("LD_PRELOAD", "/tmp/x.so"), ("LD_AUDIT", "/tmp/a.so"), ("LD_LIBRARY_PATH", "/tmp/lib")] {
            let c = spawn_with(&[(k, v)], false);
            let (app, core) = (me(c.id() as i32, "/usr/bin/kalango-remote"), target(ok.id() as i32));
            // (the loader check comes first and says what it found: the dumpable check would refuse these children too)
            assert!(untampered(&app, &core, &rules).is_err_and(|e| e.contains(&format!("{k}="))), "{k} in the app");
            let (app, core) = (me(ok.id() as i32, "/usr/bin/kalango-remote"), target(c.id() as i32));
            assert!(untampered(&app, &core, &rules).is_err_and(|e| e.contains(&format!("{k}="))), "{k} in the core");
            reap(c);
        }
        // the image's own environment, untraced, but still dumpable: refused until the program calls prctl itself
        let why = untampered(&me(ok.id() as i32, "/usr/bin/kalango-remote"), &target(ok.id() as i32), &rules);
        assert!(why.is_err_and(|e| e.contains("dumpable")), "a dumpable app and core");
        let t = spawn_with(&[("LD_LIBRARY_PATH", "/usr/lib/kalango/remote")], true);
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(untampered(&me(t.id() as i32, "/usr/bin/kalango-remote"), &target(ok.id() as i32), &rules).is_err_and(|e| e.contains("(TracerPid)")), "a traced app");
        assert!(untampered(&me(ok.id() as i32, "/usr/bin/kalango-remote"), &target(t.id() as i32), &rules).is_err_and(|e| e.contains("(TracerPid)")), "a traced core");
        reap(t);
        reap(ok);
    }

    #[test]
    fn hardened_means_untraced_and_not_dumpable() {
        let st = |tracer: u32, uid: u32| format!("Name:\tx\nUid:\t{uid}\t{uid}\t{uid}\t{uid}\nTracerPid:\t{tracer}\n");
        assert_eq!(hardened_from(&st(0, 1000), 0), Ok(()), "non-dumpable: /proc/pid/status is root's");
        assert_eq!(hardened_from(&st(0, 1000), 1000), Err("dumpable".into()));
        assert_eq!(hardened_from(&st(4321, 1000), 0), Err("TracerPid".into()));
        assert_eq!(hardened_from("Uid:\t1000\n", 0), Err("TracerPid".into()), "no TracerPid line: not believed");
        assert_eq!(hardened_from(&st(0, 0), 0), Ok(()), "root cannot be told apart and is not asked");
    }

    // The process itself, made non-dumpable and dumpable again: one test, so nothing else in this process sees it flip.
    #[test]
    fn a_grant_ends_when_the_core_stops_being_hardened() {
        let me = std::process::id() as i32;
        let dir = std::env::temp_dir().join(format!("kalangod-rc-{me}"));
        let file = dir.join("controle");
        let rc = RemoteControl::new(file.clone(), vec![]);
        let start = process(me).unwrap().start;
        let set = |on: bool| unsafe { libc::prctl(libc::PR_SET_DUMPABLE, on as libc::c_ulong, 0, 0, 0) };
        let grants = |strict| vec![Grant { pid: me, start, capabilities: vec!["captura".into()], uid: process(me).unwrap().uid, strict, guard: None }];
        let has = |text: &str| text.contains(&format!("pid {me} {start} captura"));
        let read = || std::fs::read_to_string(&file).unwrap_or_default();

        set(false);
        assert_eq!(hardened(me), Ok(()), "a non-dumpable process is hardened");
        *rc.grants.lock().unwrap() = grants(true);
        rc.write(&rc.grants.lock().unwrap());
        recheck(&rc.grants, &rc.hook, &file, &[]);
        assert!(has(&read()), "still hardened: the grant stays");
        // a root kalangod also looks at what is mapped and loaded every second: the check is asked, and its no is the end
        let guarded = Grant { guard: Some(Guard { env: vec![], libs: vec![], exe_id: None }), ..grants(true).remove(0) };
        assert_eq!(still_safe(&guarded, true, &|_, _| Err("foreign".into())), Err("foreign".into()), "root: a core with foreign code in it loses the grant");
        assert_eq!(still_safe(&guarded, true, &|_, _| Ok(())), Ok(()));
        assert_eq!(still_safe(&guarded, false, &|_, _| Err("foreign".into())), Ok(()), "a user's kalangod cannot look, and says so elsewhere");

        set(true);
        assert_eq!(hardened(me), Err("dumpable".into()));
        // a helper of kalangod's own (not strict) is not subject to it
        *rc.grants.lock().unwrap() = grants(false);
        recheck(&rc.grants, &rc.hook, &file, &[]);
        assert_eq!(rc.grants.lock().unwrap().len(), 1, "kalangod's own helpers are not checked");
        *rc.grants.lock().unwrap() = grants(true);
        recheck(&rc.grants, &rc.hook, &file, &[]);
        assert!(rc.grants.lock().unwrap().is_empty() && !has(&read()), "dumpable again: the grant is revoked and the file rewritten");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_traced_core_loses_its_grant() {
        let t = spawn_with(&[("LD_LIBRARY_PATH", "/usr/lib/kalango/remote")], true);
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert_eq!(hardened(t.id() as i32), Err("TracerPid".into()));
        let dir = std::env::temp_dir().join(format!("kalangod-rc-t-{}", t.id()));
        let file = dir.join("controle");
        let rc = RemoteControl::new(file.clone(), vec![]);
        let p = process(t.id() as i32).unwrap();
        *rc.grants.lock().unwrap() = vec![Grant { pid: p.pid, start: p.start, capabilities: vec!["captura".into()], uid: p.uid, strict: true, guard: None }];
        recheck(&rc.grants, &rc.hook, &file, &[]);
        assert!(rc.grants.lock().unwrap().is_empty(), "traced: revoked");
        let mut t = t;
        let _ = t.kill();
        let _ = t.wait();
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn id(dev: u64, ino: u64, uid: u32, mode: u32) -> Option<FileId> {
        Some(FileId { dev, ino, uid, mode })
    }

    #[test]
    fn code_mapped_from_outside_the_system_libraries_is_refused() {
        let dirs = vec!["/usr/lib/".to_string(), "/lib/".to_string(), "/usr/lib/kalango/remote/".to_string()];
        // every mapped file is root's, 0644, and the same file by its map_files link and by its path: dev 8:1 = 2049
        let good = |_: &str| id(2049, 7, 0, 0o100644);
        let clean = "55d0c0000000-55d0c0100000 r-xp 00000000 08:01 12 /usr/lib/kalango/remote/kalango-remote\n\
            7f0000000000-7f0000200000 r-xp 00000000 08:01 7 /usr/lib/x86_64-linux-gnu/libc.so.6\n\
            7f0000300000-7f0000400000 rw-p 00000000 00:00 0 \n\
            7ffc0000000-7ffc0021000 rw-p 00000000 00:00 0 [stack]\n\
            7ffc0100000-7ffc0102000 r-xp 00000000 00:00 0 [vdso]\n\
            ffffffffff600000-ffffffffff601000 --xp 00000000 00:00 0 [vsyscall]\n\
            7f0000500000-7f0000600000 r--p 00000000 08:01 9 /home/x/dados.bin\n";
        assert_eq!(foreign_mapping(clean, &dirs, false, &good, &good), None, "system libraries, data, the vdso and vsyscall pages");
        let with = |extra: &str| format!("{clean}{extra}\n");
        let refused = |extra: &str, anon: bool| foreign_mapping(&with(extra), &dirs, anon, &good, &good);
        // a library that overwrote its own LD_PRELOAD string is invisible in environ, not here
        assert_eq!(refused("7f0000700000-7f0000800000 r-xp 00000000 08:01 33 /tmp/evil.so", true).as_deref(), Some("/tmp/evil.so"));
        // nothing deleted is trusted, even from a system folder (it is not the file that is there now)
        assert_eq!(refused("7f0000700000-7f0000800000 r-xp 0 08:01 33 /usr/lib/x.so (deleted)", true).as_deref(), Some("/usr/lib/x.so (deleted)"));
        assert_eq!(refused("7f0000700000-7f0000800000 r-xp 0 00:05 33 /memfd:payload (deleted)", true).as_deref(), Some("/memfd:payload (deleted)"));
        assert_eq!(refused("7f0000700000-7f0000800000 r-xp 0 08:01 33 /tmp/my lib.so", true).as_deref(), Some("/tmp/my lib.so"));
        assert_eq!(refused("7f0000700000-7f0000800000 r-xp 0 08:01 33 /usr/libevil/x.so", true).as_deref(), Some("/usr/libevil/x.so"));
        assert!(refused("7f0000700000-7f0000800000 rwxp 0 08:01 33 /tmp/x", true).is_some(), "writable and executable");
        assert!(refused("7f0000700000-7f0000800000 r-xp 0 00:00 0 [heap]", true).is_some(), "a heap that executes");
        // anonymous executable memory: refused for a core (it has none), allowed for the app (GL and closures use it)
        assert_eq!(refused("7f0000700000-7f0000800000 rwxp 00000000 00:00 0 ", false).as_deref(), Some("(memória anônima executável)"));
        assert_eq!(refused("7f0000700000-7f0000800000 rwxp 00000000 00:00 0 ", true), None);
        // the file mapped is not the file at the path (the path was bound over in another namespace, or replaced)
        let other_inode = |r: &str| if r.starts_with("7f0000000000") { id(2049, 999, 0, 0o100644) } else { id(2049, 7, 0, 0o100644) };
        assert_eq!(foreign_mapping(clean, &dirs, false, &other_inode, &good).as_deref(), Some("/usr/lib/x86_64-linux-gnu/libc.so.6"));
        let other_dev = |r: &str| if r.starts_with("7f0000000000") { id(777, 7, 0, 0o100644) } else { id(2049, 7, 0, 0o100644) };
        assert!(foreign_mapping(clean, &dirs, false, &other_dev, &good).is_some(), "same inode, another device");
        // the mapped file is somebody else's, or writable by others, or cannot be looked at at all
        assert!(foreign_mapping(clean, &dirs, false, &|_| id(2049, 7, 1000, 0o100644), &good).is_some(), "owned by a user");
        assert!(foreign_mapping(clean, &dirs, false, &|_| id(2049, 7, 0, 0o100666), &good).is_some(), "writable by others");
        assert!(foreign_mapping(clean, &dirs, false, &|_| id(2049, 7, 0, 0o100664), &good).is_some(), "writable by the group");
        assert!(foreign_mapping(clean, &dirs, false, &|_| None, &good).is_some(), "no map_files link");
    }

    #[test]
    fn only_a_process_in_kalangods_own_namespaces_is_judged_by_what_it_shows() {
        let own = vec![Some("mnt:[4026531841]".to_string()), Some("user:[4026531837]".to_string())];
        assert!(same_namespaces(&own, &own.clone()));
        let bwrap = vec![Some("mnt:[4026532999]".to_string()), Some("user:[4026531837]".to_string())];
        assert!(!same_namespaces(&own, &bwrap), "another mount namespace (bwrap, a container)");
        let userns = vec![Some("mnt:[4026531841]".to_string()), Some("user:[4026533000]".to_string())];
        assert!(!same_namespaces(&own, &userns), "another user namespace (unshare -r)");
        assert!(!same_namespaces(&own, &[None, None]) && !same_namespaces(&own, &own[..1]), "not readable, or not all there");
        // two unreadable are not "the same": that nothing could be read says nothing
        assert!(!same_namespaces(&[None, None], &[None, None]), "kalangod's own could not be read either");
        // the kernel's own answer for a process that really is in other namespaces, when this machine lets a user make them
        let mut child = match std::process::Command::new("unshare").args(["-rm", "/usr/bin/sleep", "30"]).stderr(std::process::Stdio::null()).spawn() {
            Ok(c) => c,
            Err(_) => return,
        };
        std::thread::sleep(std::time::Duration::from_millis(500));
        if child.try_wait().ok().flatten().is_some() {
            return; // unshare refused (unprivileged user namespaces off): nothing to compare
        }
        // the child is `unshare` that exec'd sleep inside the new namespaces
        let theirs = namespaces(&child.id().to_string());
        let mine = namespaces("self");
        assert!(theirs.iter().all(Option::is_some), "readable for a process of the same user");
        assert!(!same_namespaces(&mine, &theirs), "a process made by `unshare -rm` is in other namespaces: {mine:?} vs {theirs:?}");
        assert!(same_namespaces(&mine, &namespaces(&std::process::id().to_string())), "and this process is in its own");
        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn the_file_run_is_the_trusted_file_not_one_with_the_same_name() {
        let trusted = vec![id(2049, 100, 0, 0o100755)];
        assert!(runs_trusted_file(id(2049, 100, 0, 0o100755), &trusted));
        assert!(!runs_trusted_file(id(2049, 101, 0, 0o100755), &trusted), "another file (a fake bound over the path)");
        assert!(!runs_trusted_file(id(777, 100, 0, 0o100755), &trusted), "same inode number on another device");
        assert!(!runs_trusted_file(id(2049, 100, 1000, 0o100755), &trusted), "the right file, but a user's");
        assert!(!runs_trusted_file(id(2049, 100, 0, 0o100775), &trusted), "the right file, but writable by the group");
        assert!(!runs_trusted_file(None, &trusted) && !runs_trusted_file(id(2049, 100, 0, 0o100755), &[None]), "nothing known is not trusted");
        // /proc/<pid>/exe of this very process is this very file, whatever it is called
        let me = file_id("/proc/self/exe").unwrap();
        let by_path = file_id(&std::env::current_exe().unwrap().display().to_string()).unwrap();
        assert_eq!((me.dev, me.ino), (by_path.dev, by_path.ino));
    }

    #[test]
    fn a_core_that_execs_into_another_program_is_no_longer_trusted() {
        // the file the core ran when the permission was given is remembered; its pid and start time survive an exec, its file does not
        let mut child = spawn_with(&[], false);
        let pid = child.id() as i32;
        let granted = file_id(&format!("/proc/{pid}/exe"));
        let guard = Guard { env: vec![], libs: vec![], exe_id: granted };
        assert_eq!(guard.trusted_exe(), vec![granted], "what is trusted is what was stored, not what the pid runs now");
        // (root-only is part of "trusted": /usr/bin/sleep is root's)
        assert_eq!(exe_is_trusted("/x", pid, &guard.trusted_exe()), Ok(()), "still the file it was given the permission as");
        let other = Guard { exe_id: file_id("/usr/bin/true").or(file_id("/bin/true")), ..guard.clone() };
        assert!(exe_is_trusted("/x", pid, &other.trusted_exe()).is_err(), "it now runs a different file than the stored one");
        assert!(exe_is_trusted("/x", pid, &Guard { exe_id: None, ..guard }.trusted_exe()).is_err(), "nothing stored, nothing trusted");
        let _ = child.kill();
        let _ = child.wait();
    }

    // The wiring cannot run without root (the maps and the environment of another process), so the source is checked: the
    // grant remembers the file of the core when it is given, and every later look compares against that, never against
    // what the pid runs by then (which, after an exec, is the other program and always passes).
    #[test]
    fn the_grant_remembers_the_core_file_and_the_recheck_uses_it() {
        let src = include_str!("remote_control.rs");
        let fn_body = |name: &str| {
            let from = src.find(&format!("fn {name}")).unwrap_or_else(|| panic!("{name}"));
            let rest = &src[from..];
            // a method (indented) ends at its closing brace at four spaces, a free function at one in column 0
            let indented = src[..from].rsplit('\n').next().is_some_and(|l| l.trim_start_matches("pub ").starts_with(' ') || l.starts_with("    "));
            &rest[..rest.find(if indented { "\n    }\n" } else { "\n}\n" }).unwrap_or(rest.len())]
        };
        let allow = fn_body("allow(");
        assert!(allow.contains("exe_id: trusted") && allow.contains("same_file(trusted, running"), "allow() stores the trusted file, and only if the core runs it right now");
        // the three guards of process_is_clean and who gets what: the very file, the namespaces, no anonymous code for a core
        let clean = fn_body("process_is_clean(");
        assert!(clean.contains("exe_is_trusted(") && clean.contains("same_namespaces(") && clean.contains("foreign_mapping("), "process_is_clean looks at the file, the namespaces and the mappings");
        assert!(clean.contains("anon_exec_ok"), "and passes the anonymous-code rule on");
        let unt = fn_body("untampered(");
        assert!(unt.contains("&apps, rules.env, rules.libs, true)") && unt.contains("rules.env, rules.libs, false)"), "the app may have anonymous executable memory, the core may not (at the grant)");
        assert!(fn_body("prune(").contains("&gd.libs, false)"), "and neither at the recheck");
        let prune = fn_body("prune(");
        assert!(prune.contains("gd.trusted_exe()") && !prune.contains("file_id("), "prune compares with the stored file, not with a lookup of the current one");
    }

    #[test]
    fn the_trusted_file_must_be_the_one_running_right_now() {
        let a = id(2049, 100, 0, 0o100755);
        assert!(same_file(a, id(2049, 100, 0, 0o100755), true));
        assert!(!same_file(a, id(2049, 101, 0, 0o100755), true) && !same_file(a, id(777, 100, 0, 0o100755), false), "another file");
        assert!(!same_file(a, None, true), "a root kalangod can always read it: no answer is no");
        assert!(same_file(a, None, false), "a user's kalangod (development) cannot read a protected process");
        assert!(!same_file(None, a, false) && !same_file(None, None, false), "nothing trusted is not trusted");
    }

    #[test]
    fn a_pid_that_is_somebody_elses_now_is_not_killed() {
        // the grant is for a process that ended; its pid now belongs to this child, which started at another time
        let mut child = spawn_with(&[], false);
        let p = process(child.id() as i32).unwrap();
        let stale = Grant { pid: p.pid, start: p.start + 1, capabilities: vec![], uid: p.uid, strict: true, guard: None };
        assert!(!end_core(&stale), "pid reuse: the start time differs, nothing is killed");
        assert!(child.try_wait().unwrap().is_none(), "and the process is still there");
        let right = Grant { start: p.start, ..stale };
        assert!(end_core(&right), "the same pid and start time is the process the grant was for");
        assert_eq!(std::os::unix::process::ExitStatusExt::signal(&child.wait().unwrap()), Some(libc::SIGKILL));
    }

    // kalangod reads /proc/<pid>/ns/mnt of the Remote and compares it with its own: that is only a statement about the system
    // if kalangod runs in the host's mount namespace. A unit that sets up its own (PrivateTmp, ProtectSystem, ProtectHome,
    // PrivateDevices, ReadOnlyPaths, BindPaths, ...) would make every process of the person differ from it and refuse them all.
    #[test]
    fn the_service_unit_does_not_make_kalangod_a_mount_namespace_of_its_own() {
        let unit = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/dist/kalangod.service")).unwrap();
        let directives: Vec<&str> = unit.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with(';')).collect();
        for banned in [
            "PrivateTmp", "PrivateMounts", "PrivateUsers", "PrivateDevices", "ProtectSystem", "ProtectHome", "ProtectKernelTunables", "ProtectKernelModules",
            "ProtectKernelLogs", "ProtectControlGroups", "ProtectProc", "ProcSubset", "ReadOnlyPaths", "ReadWritePaths", "InaccessiblePaths",
            "TemporaryFileSystem", "BindPaths", "BindReadOnlyPaths", "MountAPIVFS", "RootDirectory", "RootImage", "MountImages", "ExtensionImages",
            "NoExecPaths", "ExecPaths", "MountFlags", "PrivateIPC", "PrivateNetwork",
        ] {
            assert!(!directives.iter().any(|d| d.split('=').next().is_some_and(|k| k.trim() == banned)), "kalangod.service must not set {banned}: it would put kalangod in another mount namespace than the person's processes");
        }
    }

    #[test]
    fn what_could_not_be_checked_is_never_clean() {
        use crate::allowlist::Check;
        assert!(judge_check(true, "/x", 1, Check::Unchecked("environ".into())).is_err(), "root kalangod: refused");
        assert!(judge_check(false, "/x", 1, Check::Unchecked("environ".into())).is_ok(), "a user's kalangod (development): said in the log, goes on");
        assert!(judge_check(true, "/x", 1, Check::Tampered("LD_PRELOAD=/x".into())).is_err() && judge_check(false, "/x", 1, Check::Tampered("TracerPid".into())).is_err());
        assert!(judge_check(true, "/x", 1, Check::Clean).is_ok() && judge_check(false, "/x", 1, Check::Clean).is_ok());
    }

    #[test]
    fn a_core_that_loses_its_protection_is_ended_and_the_timeline_told() {
        let dir = std::env::temp_dir().join(format!("kalangod-rc-k-{}", std::process::id()));
        let file = dir.join("controle");
        let rc = RemoteControl::new(file.clone(), vec![]);
        let told: Arc<Mutex<Vec<(i32, String)>>> = Arc::default();
        let t2 = told.clone();
        rc.on_revoked(Box::new(move |g, why| t2.lock().unwrap().push((g.pid, why.to_owned()))));
        // a strict core that is dumpable (nobody called prctl), and one of kalangod's own helpers that is too
        let mut core = spawn_with(&[("LD_LIBRARY_PATH", "/usr/lib/kalango/remote")], false);
        let mut helper = spawn_with(&[], false);
        let grant = |c: &std::process::Child, strict| {
            let p = process(c.id() as i32).unwrap();
            Grant { pid: p.pid, start: p.start, capabilities: vec!["captura".into()], uid: p.uid, strict, guard: None }
        };
        *rc.grants.lock().unwrap() = vec![grant(&core, true), grant(&helper, false)];
        recheck(&rc.grants, &rc.hook, &file, &[]);
        let core_end = core.wait().unwrap();
        assert_eq!(std::os::unix::process::ExitStatusExt::signal(&core_end), Some(libc::SIGKILL), "the core of the lost grant is killed");
        assert!(helper.try_wait().unwrap().is_none(), "kalangod's own helper is left alone");
        assert_eq!(told.lock().unwrap().as_slice(), &[(core.id() as i32, "dumpable".to_string())], "and the Timeline hook is told why");
        // a grant whose process is gone (the pid may be someone else's now) kills nothing and tells nothing
        let gone = Grant { pid: core.id() as i32, start: 1, capabilities: vec![], uid: 0, strict: true, guard: None };
        *rc.grants.lock().unwrap() = vec![gone];
        recheck(&rc.grants, &rc.hook, &file, &[]);
        assert_eq!(told.lock().unwrap().len(), 1);
        let _ = helper.kill();
        let _ = helper.wait();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_whole_permission_is_refused_for_a_dumpable_core() {
        // the app is this process (the core's parent); the core is a copy of a program placed beside the app's name
        let dir = std::env::temp_dir().join(format!("kalangod-rc-a-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let core_path = dir.join("kalango-remote-core");
        std::fs::copy("/usr/bin/sleep", &core_path).unwrap();
        // (a file just written can be "busy" to exec while another test thread still holds the descriptor of a fork: retry)
        let mut core = (0..100)
            .find_map(|_| {
                std::process::Command::new(&core_path).arg("30").env_clear().env("LD_LIBRARY_PATH", "/usr/lib/kalango/remote").spawn().map_err(|_| std::thread::sleep(std::time::Duration::from_millis(20))).ok()
            })
            .expect("the copy of the core starts");
        for _ in 0..200 {
            if std::fs::read_link(format!("/proc/{}/exe", core.id())).is_ok_and(|e| e == core_path) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let me = std::process::id() as i32;
        let app_exe = dir.join("kalango-remote").display().to_string();
        let own = process(me).unwrap();
        let caller = Peer { pid: me, uid: own.uid, gid: 0, exe: app_exe.clone(), cgroup: String::new(), home: "/home/x".into(), passwd_home: "/home/x".into(), user: "x".into(), has_tty: false };
        let apps = vec![app_exe];
        // everything this process was started with is allowed (the test's environment is cargo's, not the image's), so the
        // loader checks pass for both and the one thing left that can refuse the permission is the dumpable check
        // (from the process's own variables, not /proc/self/environ: another test makes this process non-dumpable for a moment,
        // and then that file cannot be read)
        let mut env: Vec<String> = std::env::vars().map(|(k, v)| format!("{k}={v}")).collect();
        env.push("LD_LIBRARY_PATH=/usr/lib/kalango/remote".into());
        let libs = vec!["/usr/lib/".to_string()];
        let rules = Rules { apps: &apps, env: &env, libs: &libs };
        let rc = RemoteControl::new(dir.join("controle"), vec![]);
        let r = rc.allow(&caller, &rules, core.id() as i32, vec!["captura".into()]);
        assert!(r.as_ref().is_err_and(|e| e.contains("dumpable")), "no grant for an app and a core that can be debugged: {r:?}");
        assert!(rc.list(0).is_empty() && !std::fs::read_to_string(dir.join("controle")).unwrap_or_default().contains("pid "), "and nothing was written");
        let _ = core.kill();
        let _ = core.wait();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_file_niri_reads() {
        let t = file_text(
            &["/usr/bin/kalango-shell teclado".into()],
            &[Grant { pid: 7, start: 9, capabilities: vec!["captura".into(), "ponteiro".into()], uid: 1000, strict: true, guard: None }],
        );
        assert_eq!(t, "exe /usr/bin/kalango-shell teclado\npid 7 9 captura,ponteiro\n");
    }

    #[test]
    fn reads_its_own_process() {
        let p = process(std::process::id() as i32).unwrap();
        assert!(p.start > 0 && !p.exe.is_empty());
    }
}
