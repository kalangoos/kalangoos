//! Quem está chamando: identificado pelo kernel (SO_PEERCRED + /proc), nunca pelo que o chamador diz.

use nix::sys::socket::{getsockopt, sockopt::PeerCredentials};
use serde::Serialize;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize)]
pub struct Peer {
    pub pid: i32,
    pub uid: u32,
    pub gid: u32,
    pub exe: String,
    pub cgroup: String,
    /// The home for the caller's own paths ("~" in an agent's arguments): from passwd, or the test session's folder
    /// (config `home`).
    pub home: PathBuf,
    /// The home from passwd, always: where the person's programs are (~/.local/bin) and what kalangod's helpers run
    /// with. A test session's `home` must not move those.
    pub passwd_home: PathBuf,
    pub user: String,
    /// O processo tem terminal de controle (usado para aprovação presencial).
    pub has_tty: bool,
}

impl Peer {
    pub fn from_stream(s: &UnixStream) -> std::io::Result<Peer> {
        let cred = getsockopt(s, PeerCredentials).map_err(std::io::Error::from)?;
        Ok(Peer::from_ids(cred.pid(), cred.uid(), cred.gid()))
    }

    pub fn from_ids(pid: i32, uid: u32, gid: u32) -> Peer {
        let exe = std::fs::read_link(format!("/proc/{pid}/exe")).map(|p| p.display().to_string()).unwrap_or_default();
        let cgroup = std::fs::read_to_string(format!("/proc/{pid}/cgroup"))
            .ok()
            .and_then(|t| t.lines().find_map(|l| l.strip_prefix("0::").map(str::to_string)))
            .unwrap_or_default();
        let (user, home) = match nix::unistd::User::from_uid(nix::unistd::Uid::from_raw(uid)) {
            Ok(Some(u)) => (u.name, u.dir),
            _ => (uid.to_string(), PathBuf::from("/nonexistent")),
        };
        Peer { pid, uid, gid, exe, cgroup, passwd_home: home.clone(), home, user, has_tty: has_tty(pid) }
    }
}

/// tty_nr (7º campo de /proc/pid/stat) diferente de zero: há terminal de controle.
fn has_tty(pid: i32) -> bool {
    let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else { return false };
    // o nome do processo pode ter espaços: os campos começam depois do último ')'
    let Some(rest) = stat.rsplit_once(')').map(|(_, r)| r) else { return false };
    rest.split_whitespace().nth(4).and_then(|v| v.parse::<i64>().ok()).map(|v| v != 0).unwrap_or(false)
}
