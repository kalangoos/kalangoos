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
use std::sync::Mutex;

pub const CAPABILITIES: [&str; 3] = ["captura", "ponteiro", "teclado"];

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Grant {
    pub pid: i32,
    pub start: u64,
    pub capabilities: Vec<String>,
    pub uid: u32,
}

pub struct RemoteControl {
    file: PathBuf,
    always: Vec<String>,
    grants: Mutex<Vec<Grant>>,
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
        return Err(tr("capabilities must be captura, ponteiro or teclado").to_string());
    }
    Ok(())
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

impl RemoteControl {
    pub fn new(file: PathBuf, always: Vec<String>) -> RemoteControl {
        let c = RemoteControl { file, always, grants: Mutex::new(Vec::new()) };
        c.write(&[]);
        c
    }

    /// Writes the whole file (temporary + rename: niri never reads half of it), 0644, owned by kalangod's user.
    fn write(&self, grants: &[Grant]) {
        use std::os::unix::fs::PermissionsExt;
        if let Some(d) = self.file.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let tmp = self.file.with_extension("tmp");
        if std::fs::write(&tmp, file_text(&self.always, grants)).is_ok() {
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644));
            let _ = std::fs::rename(&tmp, &self.file);
        }
    }

    /// Drops the grants of processes that ended (or whose pid was reused).
    fn prune(v: &mut Vec<Grant>) {
        v.retain(|g| process(g.pid).is_some_and(|p| p.start == g.start));
    }

    pub fn allow(&self, caller: &Peer, apps: &[String], pid: i32, capabilities: Vec<String>) -> Result<Grant, String> {
        let target = process(pid).ok_or_else(|| trf("process {pid} does not exist", &[("pid", pid.to_string())]))?;
        validate(caller, apps, &target, &capabilities)?;
        let g = Grant { pid, start: target.start, capabilities, uid: target.uid };
        let mut v = self.grants.lock().unwrap();
        Self::prune(&mut v);
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
        Self::prune(&mut v);
        self.write(&v);
        Ok(())
    }

    /// kalangod's own screen helper (an agent's screen session, `screen_control.rs`): kalangod started it, so no
    /// caller is checked here; `agente` marks the grant as an agent's (niri patch 0011 then reports the person's own
    /// input, and the session yields).
    pub fn allow_own(&self, pid: i32, capabilities: &[&str]) -> Result<Grant, String> {
        let target = process(pid).ok_or_else(|| trf("process {pid} does not exist", &[("pid", pid.to_string())]))?;
        let g = Grant { pid, start: target.start, capabilities: capabilities.iter().map(|c| c.to_string()).collect(), uid: target.uid };
        let mut v = self.grants.lock().unwrap();
        Self::prune(&mut v);
        v.retain(|x| x.pid != pid);
        v.push(g.clone());
        self.write(&v);
        Ok(g)
    }

    /// Takes the helper's grant away (its session ended).
    pub fn revoke_own(&self, pid: i32) {
        let mut v = self.grants.lock().unwrap();
        v.retain(|g| g.pid != pid);
        Self::prune(&mut v);
        self.write(&v);
    }

    pub fn list(&self, uid: u32) -> Vec<Grant> {
        let mut v = self.grants.lock().unwrap();
        Self::prune(&mut v);
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
        assert!(validate(&app, &apps, &core, &[]).is_err());
    }

    #[test]
    fn the_file_niri_reads() {
        let t = file_text(
            &["/usr/bin/kalango-shell teclado".into()],
            &[Grant { pid: 7, start: 9, capabilities: vec!["captura".into(), "ponteiro".into()], uid: 1000 }],
        );
        assert_eq!(t, "exe /usr/bin/kalango-shell teclado\npid 7 9 captura,ponteiro\n");
    }

    #[test]
    fn reads_its_own_process() {
        let p = process(std::process::id() as i32).unwrap();
        assert!(p.start > 0 && !p.exe.is_empty());
    }
}
