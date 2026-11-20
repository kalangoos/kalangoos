//! The SSH and Mosh hosts the Terminal opens (`kalango-shell terminal-ssh HOST-ID`), as Kalango Remote keeps them in
//! `~/.config/kalango/remote/hosts.json` (schema in docs/ssh-hosts.md), and the command lines built from them.
//!
//! The Terminal never handles passwords or keys: it runs the system's `ssh`, `mosh` and `sftp` with the host's
//! options, and those ask in the terminal itself or use the agent. The reader is tolerant: unknown fields are ignored,
//! a port may be a number or a string, and the list may be the whole file or its "hosts" (or "servers") key.
//! Everything here is pure (no GTK), so it is tested as plain functions.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Where Kalango Remote keeps the hosts (`$XDG_CONFIG_HOME/kalango/remote/hosts.json`).
pub fn hosts_path() -> PathBuf {
    if let Some(p) = std::env::var_os("KALANGO_SSH_HOSTS") {
        return PathBuf::from(p);
    }
    gtk::glib::user_config_dir().join("kalango").join("remote").join("hosts.json")
}

/// A number or a string ("22").
fn de_port<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<u16>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum P {
        N(u64),
        S(String),
        Null,
    }
    Ok(match Option::<P>::deserialize(d)? {
        Some(P::N(n)) => u16::try_from(n).ok(),
        Some(P::S(s)) => s.trim().parse().ok(),
        _ => None,
    })
}

/// One jump host or many ("a,b" or ["a", "b"]).
fn de_list<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum L {
        One(String),
        Many(Vec<String>),
        Null,
    }
    Ok(match Option::<L>::deserialize(d)? {
        Some(L::One(s)) => s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect(),
        Some(L::Many(v)) => v.into_iter().map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect(),
        _ => Vec::new(),
    })
}

/// A port forward the session opens (`-L`, `-R`, `-D`): in a host's own "forwards", or in the file's "forwards"
/// with the host's id in "host" and "with_host" on (Kalango Remote's).
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Forward {
    /// "local", "remote" or "dynamic"
    #[serde(alias = "type")]
    pub kind: String,
    /// "[address:]port" where it listens
    #[serde(alias = "from")]
    pub listen: String,
    /// "host:port" it goes to (none for dynamic)
    #[serde(alias = "target", alias = "destination")]
    pub to: String,
    /// the host it goes through (the file's list)
    pub host: String,
    /// opened with that host's session (the file's list)
    pub with_host: bool,
    /// listening only on 127.0.0.1
    pub loopback_only: bool,
    /// switched off
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

/// What a host sets, or inherits from its groups when it is null: the user, the key, the jump, the keepalive, the
/// agent forwarding.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Settings {
    #[serde(alias = "username")]
    pub user: Option<String>,
    /// an id of the file's "identities" (Kalango Remote's vault or the agent), or a key file's path
    pub identity: Option<String>,
    /// a key file's path
    #[serde(alias = "key_file", alias = "identityfile")]
    pub identity_file: Option<String>,
    /// a host's id (or `user@address:port`), or several
    #[serde(alias = "jump_hosts", alias = "proxy_jump", alias = "jumps", deserialize_with = "de_list")]
    pub jump: Vec<String>,
    /// seconds between keepalives (0: off)
    #[serde(alias = "keepalive_interval")]
    pub keepalive: Option<u32>,
    pub forward_agent: Option<bool>,
    pub production: Option<bool>,
}

/// A host as written in hosts.json (docs/ssh-hosts.md; Kalango Remote's docs/HOSTS.md).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Host {
    pub id: String,
    pub name: String,
    #[serde(alias = "hostname", alias = "host")]
    pub address: String,
    #[serde(deserialize_with = "de_port")]
    pub port: Option<u16>,
    /// "ssh" or "mosh"
    #[serde(rename = "type", alias = "kind", alias = "connection", alias = "protocol")]
    pub kind: Option<String>,
    /// "agent", "key" (the identity file) or "password" (ssh asks in the terminal)
    pub auth: Option<String>,
    #[serde(flatten)]
    pub own: Settings,
    pub group: Option<String>,
    #[serde(deserialize_with = "de_list")]
    pub tags: Vec<String>,
    /// ask before connecting (Kalango Remote asks); the Terminal guards commands that erase or restart
    pub confirm: Option<bool>,
    pub forwards: Vec<Forward>,
    /// extra `-o Key=Value` options
    pub options: BTreeMap<String, String>,
    /// an ssh_config to use instead of ~/.ssh/config (`-F`)
    #[serde(alias = "ssh_config")]
    pub config_file: Option<String>,
    /// the mosh-server on the host, when it is not in its PATH (`mosh --server=`)
    pub mosh_server: Option<String>,
}

/// A group: what its hosts inherit ("defaults", or the same fields on the group itself) and its parent group.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Group {
    pub id: String,
    pub name: String,
    pub parent: Option<String>,
    pub defaults: Settings,
    #[serde(flatten)]
    pub own: Settings,
}

impl Group {
    /// Its settings: "defaults" first, then the same fields written on the group.
    fn get<T>(&self, f: impl Fn(&Settings) -> Option<T>) -> Option<T> {
        f(&self.defaults).or_else(|| f(&self.own))
    }
}

/// A key or password of Kalango Remote's (the secret itself is never in the file).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Identity {
    pub id: String,
    /// "vault-key", "vault-password" or "agent"
    pub kind: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct File {
    #[serde(alias = "servers")]
    hosts: Vec<Host>,
    groups: Vec<Group>,
    identities: Vec<Identity>,
    forwards: Vec<Forward>,
}

/// All the hosts and groups of hosts.json.
#[derive(Clone, Debug, Default)]
pub struct Hosts {
    pub hosts: Vec<Host>,
    pub groups: Vec<Group>,
    pub identities: Vec<Identity>,
    pub forwards: Vec<Forward>,
}

fn some(s: &Option<String>) -> Option<String> {
    s.clone().map(|x| x.trim().to_string()).filter(|x| !x.is_empty())
}

impl Hosts {
    pub fn parse(text: &str) -> Result<Hosts, String> {
        let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let f: File = if v.is_array() {
            File { hosts: serde_json::from_value(v).map_err(|e| e.to_string())?, ..File::default() }
        } else {
            serde_json::from_value(v).map_err(|e| e.to_string())?
        };
        // (a host without an id answers by its name)
        let hosts = f.hosts.into_iter().map(|mut h| {
            if h.id.is_empty() {
                h.id = h.name.clone();
            }
            if h.name.is_empty() {
                h.name = h.id.clone();
            }
            h
        }).filter(|h| !h.id.is_empty()).collect();
        Ok(Hosts { hosts, groups: f.groups, identities: f.identities, forwards: f.forwards })
    }

    pub fn load(path: &Path) -> Result<Hosts, String> {
        Hosts::parse(&std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?)
    }

    pub fn find(&self, id: &str) -> Option<&Host> {
        self.hosts.iter().find(|h| h.id == id).or_else(|| self.hosts.iter().find(|h| h.name == id))
    }

    /// The groups a host is in, its own first, then the ones above (by id or name; a loop stops).
    fn chain(&self, group: Option<&str>) -> Vec<&Group> {
        let mut out: Vec<&Group> = Vec::new();
        let mut g = group;
        while let Some(name) = g {
            let Some(x) = self.groups.iter().find(|x| x.id == name).or_else(|| self.groups.iter().find(|x| x.name.eq_ignore_ascii_case(name))) else { break };
            if out.iter().any(|y| std::ptr::eq(*y, x)) {
                break;
            }
            out.push(x);
            g = x.parent.as_deref();
        }
        out
    }

    /// A setting of the host, else of its nearest group that has it.
    fn inherited<T>(&self, h: &Host, f: impl Fn(&Settings) -> Option<T>) -> Option<T> {
        f(&h.own).or_else(|| self.chain(h.group.as_deref()).iter().find_map(|g| g.get(&f)))
    }

    /// The key file to pass with `-i`: a path; an identity of the agent (or of Kalango Remote's vault, which only
    /// `kalango-remote ssh-command` can hand out) gives none.
    fn key_file(&self, h: &Host) -> Option<String> {
        if h.auth.as_deref().is_some_and(|a| a.eq_ignore_ascii_case("agent")) {
            return None;
        }
        if let Some(p) = self.inherited(h, |s| some(&s.identity_file)) {
            return Some(expand_home(&p));
        }
        let id = self.inherited(h, |s| some(&s.identity))?;
        let is_path = id.contains('/') || id.starts_with('~');
        (is_path && !self.identities.iter().any(|i| i.id == id)).then(|| expand_home(&id))
    }

    /// The host with what it inherits filled in, its jump hosts as `user@address:port`, checked to be safe as
    /// arguments.
    pub fn resolve(&self, id: &str) -> Result<Target, String> {
        let h = self.find(id).ok_or_else(|| format!("no host {id:?} in hosts.json"))?;
        let groups = self.chain(h.group.as_deref());
        let user = self.inherited(h, |s| some(&s.user));
        let jump_ids = self.inherited(h, |s| Some(s.jump.clone()).filter(|j| !j.is_empty())).unwrap_or_default();
        let keepalive = self.inherited(h, |s| s.keepalive);
        let forward_agent = self.inherited(h, |s| s.forward_agent).unwrap_or(false);
        let tags_say = h.tags.iter().any(|t| matches!(t.to_lowercase().as_str(), "produção" | "producao" | "production" | "prod"));
        let production = h.own.production.or_else(|| groups.iter().find_map(|g| g.get(|s| s.production))).unwrap_or(false) || tags_say;
        let mut jumps = Vec::new();
        let mut jump_names = Vec::new();
        let mut seen = vec![h.id.clone()];
        for j in &jump_ids {
            self.expand_jump(j, &mut jumps, &mut jump_names, &mut seen)?;
        }
        let mut forwards: Vec<Forward> = h.forwards.iter().filter(|f| f.enabled).cloned().collect();
        forwards.extend(self.forwards.iter().filter(|f| f.enabled && f.with_host && f.host == h.id).cloned());
        for f in &mut forwards {
            // (listening only on this computer: the address goes in front of a bare port)
            if f.loopback_only && !f.listen.contains(':') {
                f.listen = format!("127.0.0.1:{}", f.listen);
            }
        }
        let t = Target {
            id: h.id.clone(),
            name: h.name.clone(),
            address: h.address.trim().to_string(),
            port: h.port.unwrap_or(22),
            user,
            mosh: h.kind.as_deref().is_some_and(|k| k.eq_ignore_ascii_case("mosh")),
            identity_file: self.key_file(h),
            jumps,
            jump_names,
            production,
            confirm: h.confirm.unwrap_or(false),
            keepalive,
            forward_agent,
            forwards,
            options: h.options.clone(),
            config_file: h.config_file.as_deref().map(expand_home),
            mosh_server: h.mosh_server.clone(),
            tags: h.tags.clone(),
        };
        t.check()?;
        Ok(t)
    }

    /// A jump given by a host's id (with its own jumps before it) or as `[user@]address[:port]`.
    fn expand_jump(&self, j: &str, out: &mut Vec<String>, names: &mut Vec<String>, seen: &mut Vec<String>) -> Result<(), String> {
        match self.find(j) {
            Some(h) => {
                if seen.contains(&h.id) {
                    return Err(format!("jump hosts loop through {:?}", h.id));
                }
                seen.push(h.id.clone());
                let own = self.inherited(h, |s| Some(s.jump.clone()).filter(|j| !j.is_empty())).unwrap_or_default();
                for k in own {
                    self.expand_jump(&k, out, names, seen)?;
                }
                let user = self.inherited(h, |s| some(&s.user));
                let addr = h.address.trim();
                let host = if addr.contains(':') { format!("[{addr}]") } else { addr.to_string() };
                out.push(format!("{}{host}:{}", user.map(|u| format!("{u}@")).unwrap_or_default(), h.port.unwrap_or(22)));
                names.push(h.name.clone());
            }
            None => {
                out.push(j.to_string());
                names.push(j.to_string());
            }
        }
        Ok(())
    }
}

fn expand_home(p: &str) -> String {
    match p.strip_prefix("~/") {
        Some(rest) => gtk::glib::home_dir().join(rest).display().to_string(),
        None => p.to_string(),
    }
}

/// A host ready to connect to.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Target {
    pub id: String,
    pub name: String,
    pub address: String,
    pub port: u16,
    pub user: Option<String>,
    pub mosh: bool,
    pub identity_file: Option<String>,
    /// `user@address:port` of each jump, in order
    pub jumps: Vec<String>,
    /// their names, for the status line ("via bastiao-trabalho")
    pub jump_names: Vec<String>,
    pub production: bool,
    /// "Pedir confirmação antes de conectar": its commands that erase or restart ask first, as production's do
    pub confirm: bool,
    pub keepalive: Option<u32>,
    pub forward_agent: bool,
    pub forwards: Vec<Forward>,
    pub options: BTreeMap<String, String>,
    pub config_file: Option<String>,
    pub mosh_server: Option<String>,
    pub tags: Vec<String>,
}

/// A word that cannot be read by ssh as an option, nor break a line or a quote.
fn safe_word(s: &str) -> bool {
    !s.is_empty() && !s.starts_with('-') && !s.chars().any(|c| c.is_whitespace() || c.is_control() || matches!(c, '\'' | '"' | '`' | '$' | ';' | '|' | '&' | '\\'))
}

impl Target {
    /// What would let a hosts.json turn into another command: an address or user that ssh would take as an option,
    /// spaces, quotes, new lines; option names that are not plain words.
    pub fn check(&self) -> Result<(), String> {
        if !safe_word(&self.address) {
            return Err(format!("unsafe address {:?}", self.address));
        }
        if let Some(u) = &self.user {
            if !safe_word(u) || u.contains('@') {
                return Err(format!("unsafe user {u:?}"));
            }
        }
        for j in &self.jumps {
            if !safe_word(j) || j.contains(',') {
                return Err(format!("unsafe jump host {j:?}"));
            }
        }
        for (k, v) in &self.options {
            if k.is_empty() || !k.chars().all(|c| c.is_ascii_alphanumeric()) || v.chars().any(|c| c.is_control() || c == '\'') {
                return Err(format!("unsafe option {k:?}"));
            }
        }
        for p in [&self.identity_file, &self.config_file, &self.mosh_server].into_iter().flatten() {
            if p.starts_with('-') || p.chars().any(|c| c.is_control() || c == '\'') {
                return Err(format!("unsafe path {p:?}"));
            }
        }
        for f in &self.forwards {
            if !safe_word(&f.listen) || (!f.to.is_empty() && !safe_word(&f.to)) {
                return Err(format!("unsafe forward {:?}", f.listen));
            }
        }
        Ok(())
    }

    /// `user@address` (or the address alone).
    pub fn destination(&self) -> String {
        match &self.user {
            Some(u) => format!("{u}@{}", self.address),
            None => self.address.clone(),
        }
    }

    /// "deploy@app-prod-01" for the status line.
    pub fn label(&self) -> String {
        match &self.user {
            Some(u) => format!("{u}@{}", self.name),
            None => self.name.clone(),
        }
    }

    /// The options every ssh of this host takes (the session's, mosh's bootstrap and sftp's), `port_flag` being `-p`
    /// for ssh and `-P` for sftp. The control socket lets sftp and a second pane reuse the session's connection: the
    /// password, when there is one, is typed once, in the terminal, to ssh.
    fn common(&self, port_flag: &str, control: Option<&str>, master: bool) -> Vec<String> {
        let mut a: Vec<String> = Vec::new();
        if let Some(f) = &self.config_file {
            a.extend(["-F".into(), f.clone()]);
        }
        if self.port != 22 {
            a.extend([port_flag.into(), self.port.to_string()]);
        }
        if let Some(i) = &self.identity_file {
            a.extend(["-i".into(), i.clone()]);
        }
        if !self.jumps.is_empty() {
            a.extend(["-J".into(), self.jumps.join(",")]);
        }
        let mut o: BTreeMap<String, String> = BTreeMap::new();
        // (ssh asks about a host it does not know; a changed key is refused)
        o.insert("StrictHostKeyChecking".into(), "ask".into());
        // (a dead connection is noticed in about 30 s, and the tab reconnects)
        let ka = self.keepalive.unwrap_or(15);
        if ka > 0 {
            o.insert("ServerAliveInterval".into(), ka.to_string());
            o.insert("ServerAliveCountMax".into(), "2".into());
        }
        if let Some(c) = control {
            o.insert("ControlPath".into(), c.to_string());
            o.insert("ControlMaster".into(), if master { "auto" } else { "no" }.into());
            if master {
                o.insert("ControlPersist".into(), "600".into());
            }
        }
        // (the host's own options win)
        for (k, v) in &self.options {
            o.retain(|x, _| !x.eq_ignore_ascii_case(k));
            o.insert(k.clone(), v.clone());
        }
        for (k, v) in o {
            a.extend(["-o".into(), format!("{k}={v}")]);
        }
        a
    }

    /// The session's own options: agent forwarding and the port forwards.
    fn session_only(&self) -> Vec<String> {
        let mut a = Vec::new();
        if self.forward_agent {
            a.push("-A".into());
        }
        for f in &self.forwards {
            match f.kind.to_lowercase().as_str() {
                "local" | "l" => a.extend(["-L".into(), format!("{}:{}", f.listen, f.to)]),
                "remote" | "r" => a.extend(["-R".into(), format!("{}:{}", f.listen, f.to)]),
                "dynamic" | "d" | "socks" => a.extend(["-D".into(), f.listen.clone()]),
                _ => {}
            }
        }
        a
    }

    /// `ssh … user@address` for the tab (`control`: the socket's path, `%C` and all).
    pub fn ssh_argv(&self, control: Option<&str>) -> Vec<String> {
        let mut a = vec!["ssh".to_string()];
        a.extend(self.common("-p", control, true));
        a.extend(self.session_only());
        a.push(self.destination());
        a
    }

    /// `mosh --ssh='ssh …' -- user@address`: mosh's own bootstrap goes through ssh with the same options (the agent
    /// forwarding and port forwards are not mosh's: they stay with ssh).
    pub fn mosh_argv(&self, control: Option<&str>) -> Vec<String> {
        let mut ssh = vec!["ssh".to_string()];
        ssh.extend(self.common("-p", control, true));
        let mut a = vec!["mosh".to_string(), format!("--ssh={}", ssh.iter().map(|w| shell_quote(w)).collect::<Vec<_>>().join(" "))];
        if let Some(s) = &self.mosh_server {
            a.push(format!("--server={s}"));
        }
        a.push("--".into());
        a.push(self.destination());
        a
    }

    /// `sftp -b BATCH …`: never asks (BatchMode), and goes through the session's connection when it is up.
    pub fn sftp_argv(&self, control: Option<&str>, batch: &str) -> Vec<String> {
        let mut a = vec!["sftp".to_string(), "-b".into(), batch.to_string()];
        a.extend(self.common("-P", control, false));
        a.extend(["-o".into(), "BatchMode=yes".into()]);
        a.push(self.destination());
        a
    }

    /// `ssh -O exit` for the control socket (the last tab of a host closed).
    pub fn control_exit_argv(&self, control: &str) -> Vec<String> {
        let mut a = vec!["ssh".to_string()];
        if let Some(f) = &self.config_file {
            a.extend(["-F".into(), f.clone()]);
        }
        a.extend(["-o".into(), format!("ControlPath={control}"), "-O".into(), "exit".into(), self.destination()]);
        a
    }

    /// `ssh -G …`: the options ssh would use (where its known_hosts are).
    pub fn config_argv(&self) -> Vec<String> {
        let mut a = vec!["ssh".to_string(), "-G".into()];
        a.extend(self.common("-p", None, false));
        a.push(self.destination());
        a
    }

    /// `ssh … true` through the session's connection (never asks): how long a round trip takes.
    pub fn ping_argv(&self, control: &str) -> Vec<String> {
        let mut a = vec!["ssh".to_string()];
        a.extend(self.common("-p", Some(control), false));
        a.extend(["-o".into(), "BatchMode=yes".into(), "-T".into(), self.destination(), "true".into()]);
        a
    }

    /// The name known_hosts keeps the host under: `address`, or `[address]:port`.
    pub fn known_hosts_name(&self) -> String {
        if self.port == 22 { self.address.clone() } else { format!("[{}]:{}", self.address, self.port) }
    }
}

/// `kalango-remote ssh-command HOST-ID`'s answer: a JSON array, the command to run as is (ssh or mosh).
pub fn parse_remote_command(stdout: &str) -> Result<Vec<String>, String> {
    let argv: Vec<String> = serde_json::from_str(stdout.trim()).map_err(|e| e.to_string())?;
    let prog = argv.first().map(|p| Path::new(p).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()).unwrap_or_default();
    if prog != "ssh" && prog != "mosh" {
        return Err(format!("not an ssh or mosh command: {prog:?}"));
    }
    Ok(argv)
}

/// Kalango Remote's command with the session's control socket (SFTP and the second pane reuse the connection):
/// the options after `ssh`, or at the end of mosh's `--ssh=`.
pub fn with_control(mut argv: Vec<String>, control: &str) -> Vec<String> {
    let opts = ["-o".to_string(), "ControlMaster=auto".into(), "-o".into(), format!("ControlPath={control}"), "-o".into(), "ControlPersist=600".into()];
    let prog = argv.first().map(|p| Path::new(p).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()).unwrap_or_default();
    if prog == "ssh" {
        argv.splice(1..1, opts);
    } else if let Some(a) = argv.iter_mut().find(|a| a.starts_with("--ssh=")) {
        a.push(' ');
        a.push_str(&opts.iter().map(|o| shell_quote(o)).collect::<Vec<_>>().join(" "));
    }
    argv
}

/// The key files Kalango Remote wrote for this command (`-i` under its runtime folder, also inside mosh's
/// `--ssh=`): the Terminal deletes them when the connection ends. Nothing outside `keys_dir` is ever named.
pub fn temporary_keys(argv: &[String], keys_dir: &Path) -> Vec<PathBuf> {
    let mut words: Vec<String> = Vec::new();
    for a in argv {
        match a.strip_prefix("--ssh=") {
            Some(rest) => words.extend(rest.split_whitespace().map(|w| w.trim_matches('\'').to_string())),
            None => words.push(a.clone()),
        }
    }
    let mut out = Vec::new();
    for w in words.windows(2) {
        if w[0] == "-i" {
            let p = PathBuf::from(&w[1]);
            if p.starts_with(keys_dir) && !w[1].contains("..") && !out.contains(&p) {
                out.push(p);
            }
        }
    }
    out
}

/// The known_hosts files a command names (`-o UserKnownHostsFile=…`).
pub fn named_known_hosts(argv: &[String]) -> Vec<String> {
    argv.iter().filter_map(|a| a.strip_prefix("UserKnownHostsFile=")).flat_map(|f| f.split_whitespace().map(expand_home)).collect()
}

/// The known_hosts files in `ssh -G`'s output ("userknownhostsfile a b").
pub fn known_hosts_files(ssh_g: &str) -> Vec<String> {
    ssh_g.lines().find_map(|l| l.strip_prefix("userknownhostsfile ")).map(|f| f.split_whitespace().map(expand_home).collect()).unwrap_or_default()
}

/// The key type and fingerprint in `ssh-keygen -l -F NAME`'s output ("NAME ED25519 SHA256:…").
pub fn known_key(keygen: &str) -> Option<(String, String)> {
    keygen.lines().filter(|l| !l.starts_with('#')).find_map(|l| {
        let mut w = l.split_whitespace().skip(1);
        let (kt, fp) = (w.next()?, w.next()?);
        fp.contains(':').then(|| (kt.to_string(), fp.to_string()))
    })
}

/// A word quoted for a POSIX shell (and for mosh's shellwords).
pub fn shell_quote(w: &str) -> String {
    if !w.is_empty() && w.chars().all(|c| c.is_ascii_alphanumeric() || "-_./=:@%,+".contains(c)) {
        w.to_string()
    } else {
        format!("'{}'", w.replace('\'', "'\\''"))
    }
}

/// ssh's question about a host it does not know yet.
#[derive(Clone, Debug, PartialEq)]
pub struct UnknownKey {
    /// as ssh names it ("[127.0.0.1]:2222")
    pub host: String,
    pub key_type: String,
    pub fingerprint: String,
}

/// "The authenticity of host 'H (IP)' can't be established. / ED25519 key fingerprint is SHA256:… / … Are you sure
/// you want to continue connecting (yes/no/[fingerprint])?" — the last such question on the screen, if it is the last
/// thing there (not answered yet).
pub fn unknown_key_prompt(screen: &str) -> Option<UnknownKey> {
    let at = screen.rfind("The authenticity of host '")?;
    let rest = &screen[at..];
    let ask = rest.find("Are you sure you want to continue connecting")?;
    // (answered: something typed after the question's end)
    let after = &rest[ask..];
    let tail = after.split_once('?').map(|(_, t)| t).unwrap_or("");
    if !tail.trim().is_empty() {
        return None;
    }
    let host_part = &rest["The authenticity of host '".len()..];
    let host = host_part.split('\'').next()?.split(" (").next()?.to_string();
    let line = rest.lines().find(|l| l.contains(" key fingerprint is "))?;
    let (kt, fp) = line.split_once(" key fingerprint is ")?;
    Some(UnknownKey { host, key_type: kt.trim().to_string(), fingerprint: fp.trim().trim_end_matches('.').to_string() })
}

/// "WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!": the key type and the new fingerprint ssh saw.
pub fn changed_key(screen: &str) -> Option<(String, String)> {
    let at = screen.rfind("REMOTE HOST IDENTIFICATION HAS CHANGED")?;
    let rest = &screen[at..];
    let i = rest.find("The fingerprint for the ")?;
    let after = &rest[i + "The fingerprint for the ".len()..];
    let kt = after.split_whitespace().next()?.to_string();
    let fp = after.split_whitespace().find(|w| w.starts_with("SHA256:") || w.starts_with("MD5:"))?.trim_end_matches('.').to_string();
    Some((kt, fp))
}

/// mosh-client's notice on the top row ("mosh: Last contact 8 seconds ago. [To quit: Ctrl-^ .]"): how long ago.
pub fn mosh_last_contact(row: &str) -> Option<u64> {
    let i = row.find("Last contact ").map(|i| i + 13).or_else(|| row.find("Last reply ").map(|i| i + 11))?;
    let rest = &row[i..];
    let word = rest.split_whitespace().next()?;
    if rest.contains(" seconds ago") || rest.contains(" second ago") {
        return word.parse::<f64>().ok().map(|s| s as u64);
    }
    // "1:05 ago", "1:02:03 ago"
    let parts: Vec<u64> = word.split(':').map(|p| p.parse().ok()).collect::<Option<_>>()?;
    Some(parts.iter().fold(0, |acc, p| acc * 60 + p))
}

/// mosh could not start mosh-server on the host (none there): the tab goes back to SSH.
pub fn mosh_server_missing(screen: &str) -> bool {
    screen.contains("Did not find mosh server startup message") || (screen.contains("mosh-server") && (screen.contains("command not found") || screen.contains("No such file")))
}

/// A command that erases or restarts something: on a production host it asks before Enter sends it.
pub fn is_destructive(line: &str) -> bool {
    let l = line.to_lowercase();
    let words: Vec<&str> = l.split(|c: char| c.is_whitespace() || matches!(c, ';' | '|' | '&' | '(' | ')' | '`')).filter(|w| !w.is_empty()).collect();
    let has = |w: &str| words.contains(&w);
    let rm_force = words.windows(2).any(|p| p[0] == "rm" && p[1].starts_with('-') && (p[1].contains('r') || p[1].contains('f')))
        || words.iter().enumerate().any(|(i, w)| *w == "rm" && words[i..].iter().any(|x| x.starts_with('-') && (x.contains('r') || x.contains('f')) && !x.starts_with("--")));
    rm_force
        || has("reboot") || has("shutdown") || has("poweroff") || has("halt") || has("mkfs") || words.iter().any(|w| w.starts_with("mkfs."))
        || has("dd") && l.contains("of=")
        || has("truncate") || has("shred") || has("wipefs") || has("fdisk") || has("parted")
        || (has("systemctl") && (has("restart") || has("stop") || has("reboot") || has("poweroff") || has("disable") || has("mask")))
        || (has("service") && (has("restart") || has("stop")))
        || (has("docker") && (has("rm") || has("rmi") || has("prune") || has("restart") || has("stop") || has("kill")))
        || (has("kubectl") && (has("delete") || has("drain") || has("rollout") && has("restart")))
        || has("kill") || has("killall") || has("pkill")
        || l.contains("drop table") || l.contains("drop database") || l.contains("truncate table") || (l.contains("delete from") && !l.contains(" where "))
        || (has("git") && (has("reset") && l.contains("--hard") || has("clean") || (has("push") && (l.contains("--force") || l.contains(" -f")))))
        || l.contains("> /dev/sd") || l.contains(">/dev/sd") || l.contains(":(){")
}

/// The secrets the agent never reads: lines of a private key, and the value after password=, token: and such.
pub fn redact_secrets(text: &str) -> String {
    let mut out = Vec::new();
    let mut in_key = false;
    for line in text.lines() {
        if line.contains("-----BEGIN") && line.contains("PRIVATE KEY") {
            in_key = true;
            out.push("[private key removed]".to_string());
            continue;
        }
        if in_key {
            if line.contains("-----END") {
                in_key = false;
            }
            continue;
        }
        let lower = line.to_lowercase();
        let mut l = line.to_string();
        for k in ["password", "passwd", "senha", "secret", "token", "api_key", "apikey", "authorization", "private_key", "access_key"] {
            if let Some(i) = lower.find(k) {
                // the value after the key's "=" or ":" (to the end of the line)
                let after = i + k.len();
                if let Some(j) = line[after..].find(['=', ':']).map(|j| after + j) {
                    if line[after..j].trim().chars().all(|c| c.is_alphanumeric() || c == '_' || c == '"' || c == '\'') {
                        l = format!("{}{} [removed]", &line[..j], &line[j..=j]);
                        break;
                    }
                }
            }
        }
        out.push(l);
    }
    out.join("\n")
}

/// The folder in a prompt like "deploy@app-prod-01:~/app$ " (Debian's and Ubuntu's default) on the cursor's row.
pub fn prompt_dir(row: &str) -> Option<String> {
    let at = row.find('@')?;
    let rest = &row[at + 1..];
    let colon = rest.find(':')?;
    let host = &rest[..colon];
    if host.is_empty() || host.contains(char::is_whitespace) {
        return None;
    }
    let after = &rest[colon + 1..];
    let end = after.find(['$', '#'])?;
    let dir = after[..end].trim();
    (dir.starts_with('/') || dir.starts_with('~')).then(|| dir.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "version": 1,
      "groups": [
        {"name": "Trabalho", "user": "deploy", "jump": ["bastiao-trabalho"], "keepalive": 30},
        {"name": "Produção", "parent": "Trabalho", "production": true}
      ],
      "hosts": [
        {"id": "bastiao-trabalho", "name": "bastiao-trabalho", "address": "bastiao.exemplo.com.br", "port": 2222, "user": "ops", "jump": []},
        {"id": "app-prod-01", "name": "app-prod-01", "address": "203.0.113.24", "port": "22", "group": "Produção",
         "tags": ["trabalho", "produção"], "identity_file": "/chaves/deploy-ed25519", "forward_agent": false,
         "forwards": [{"kind": "local", "listen": "127.0.0.1:5433", "to": "localhost:5432"}],
         "favorite_color": "teal", "last_used": "2026-11-15T02:00:00Z"},
        {"id": "raspberry-sala", "name": "raspberry-sala", "address": "raspberry.local", "user": "pi", "type": "MOSH",
         "auth": "agent", "identity_file": "/nao/usar", "mosh_server": "/opt/mosh/bin/mosh-server"},
        {"name": "sem-id", "hostname": "10.0.2.15", "username": "x", "jump_hosts": "a@b:22, c"}
      ]
    }"#;

    #[test]
    fn reads_hosts_json_tolerantly() {
        let h = Hosts::parse(SAMPLE).unwrap();
        assert_eq!(h.hosts.len(), 4);
        let p = h.find("app-prod-01").unwrap();
        assert_eq!(p.port, Some(22), "a port as a string");
        // (a host without an id answers by its name; aliases of the fields)
        let s = h.find("sem-id").unwrap();
        assert_eq!((s.address.as_str(), s.own.user.as_deref()), ("10.0.2.15", Some("x")));
        assert_eq!(s.own.jump, vec!["a@b:22", "c"]);
        // the whole file may be just the list
        let l = Hosts::parse(r#"[{"id": "a", "address": "1.2.3.4"}]"#).unwrap();
        assert_eq!(l.hosts[0].name, "a");
        assert!(Hosts::parse("{").is_err());
        assert!(Hosts::parse(r#"{"servers": [{"id": "z", "host": "h"}]}"#).unwrap().find("z").is_some());
    }

    #[test]
    fn inherits_from_groups_and_resolves_jumps() {
        let h = Hosts::parse(SAMPLE).unwrap();
        let t = h.resolve("app-prod-01").unwrap();
        assert_eq!(t.user.as_deref(), Some("deploy"), "from the group above");
        assert_eq!(t.keepalive, Some(30));
        assert!(t.production);
        assert_eq!(t.jumps, vec!["ops@bastiao.exemplo.com.br:2222"]);
        assert_eq!(t.jump_names, vec!["bastiao-trabalho"]);
        assert_eq!(t.label(), "deploy@app-prod-01");
        let r = h.resolve("raspberry-sala").unwrap();
        assert!(r.mosh && !r.production);
        assert_eq!(r.identity_file, None, "auth by the agent: no identity file");
        assert!(h.resolve("nenhum").is_err());
    }

    /// Kalango Remote's own file (its docs/HOSTS.md): groups with "defaults", an identity of the vault, a jump by id,
    /// forwards in the file's list.
    #[test]
    fn reads_kalango_remote_hosts_json() {
        let f = r#"{
          "version": 1,
          "groups": [
            { "id": "trabalho", "name": "Trabalho", "parent": null,
              "defaults": { "user": "deploy", "identity": "deploy-ed25519", "jump": "bastiao-trabalho", "keepalive": 30, "forward_agent": null } },
            { "id": "producao", "name": "produção", "parent": "trabalho", "defaults": { "keepalive": 15 } }
          ],
          "hosts": [
            { "id": "bastiao-trabalho", "name": "bastiao-trabalho", "address": "bastiao.example.com", "port": 2222, "group": null,
              "tags": [], "type": "ssh", "user": "ops", "identity": null, "jump": null, "keepalive": null, "forward_agent": null,
              "confirm": false, "last_used": 0 },
            { "id": "app-prod-01", "name": "app-prod-01", "address": "203.0.113.24", "port": 22,
              "group": "producao", "tags": ["trabalho", "produção"], "type": "ssh",
              "user": null, "identity": null, "jump": null, "keepalive": null, "forward_agent": null,
              "confirm": true, "last_used": 1795140000 },
            { "id": "pi", "name": "raspberry-sala", "address": "raspberry.local", "port": 22, "group": null, "tags": [], "type": "mosh",
              "user": "pi", "identity": "agente", "jump": null, "keepalive": null, "forward_agent": true, "confirm": false, "last_used": 0 }
          ],
          "identities": [
            { "id": "deploy-ed25519", "kind": "vault-key", "public_key": "ssh-ed25519 AAAA", "fingerprint": "SHA256:x",
              "key_type": "ED25519", "origin": "generated here", "confirm_each_use": true, "created": 1795130000 },
            { "id": "agente", "kind": "agent" }
          ],
          "forwards": [
            { "id": "banco", "name": "Banco", "kind": "local", "listen": "5433", "target": "db:5432", "host": "app-prod-01",
              "with_host": true, "loopback_only": true },
            { "id": "outro", "kind": "dynamic", "listen": "1080", "host": "app-prod-01", "with_host": false }
          ]
        }"#;
        let h = Hosts::parse(f).unwrap();
        let t = h.resolve("app-prod-01").unwrap();
        assert_eq!(t.user.as_deref(), Some("deploy"), "from the group above, by its defaults");
        assert_eq!(t.keepalive, Some(15), "the nearest group's");
        assert_eq!(t.identity_file, None, "a vault key: only kalango-remote hands it out");
        assert_eq!(t.jumps, vec!["ops@bastiao.example.com:2222"]);
        assert!(t.production && t.confirm);
        assert_eq!(t.forwards.len(), 1, "only the forwards that go with the host");
        assert_eq!(t.forwards[0].listen, "127.0.0.1:5433");
        assert!(t.ssh_argv(None).join(" ").contains("-L 127.0.0.1:5433:db:5432"));
        let p = h.resolve("pi").unwrap();
        assert!(p.mosh && p.forward_agent && p.identity_file.is_none());
        assert_eq!(p.name, "raspberry-sala");
    }

    #[test]
    fn uses_kalango_remotes_command() {
        let keys = Path::new("/run/user/1000/kalango-remote/keys");
        let out = r#"["ssh","-F","/dev/null","-o","UserKnownHostsFile=/home/x/.config/kalango/remote/known_hosts","-o","StrictHostKeyChecking=yes","-i","/run/user/1000/kalango-remote/keys/deploy-ed25519","-o","IdentitiesOnly=yes","-p","22","deploy@203.0.113.24"]"#;
        let argv = parse_remote_command(out).unwrap();
        let c = with_control(argv.clone(), "/run/k/ssh/app.sock");
        assert_eq!(&c[..7], ["ssh", "-o", "ControlMaster=auto", "-o", "ControlPath=/run/k/ssh/app.sock", "-o", "ControlPersist=600"]);
        assert_eq!(c.last().unwrap(), "deploy@203.0.113.24");
        assert_eq!(temporary_keys(&argv, keys), vec![PathBuf::from("/run/user/1000/kalango-remote/keys/deploy-ed25519")]);
        assert_eq!(named_known_hosts(&argv), vec!["/home/x/.config/kalango/remote/known_hosts"]);
        // mosh: the key inside --ssh=, the control options at its end
        let m = vec!["mosh".to_string(), "--ssh=ssh -F /dev/null -i /run/user/1000/kalango-remote/keys/pi -o IdentitiesOnly=yes -p 22".into(), "pi@raspberry.local".into()];
        assert_eq!(temporary_keys(&m, keys), vec![PathBuf::from("/run/user/1000/kalango-remote/keys/pi")]);
        assert!(with_control(m, "/c").get(1).unwrap().ends_with("-p 22 -o ControlMaster=auto -o ControlPath=/c -o ControlPersist=600"));
        // never a file outside its folder, nor another program
        assert!(temporary_keys(&["ssh".into(), "-i".into(), "/home/x/.ssh/id_ed25519".into(), "h".into()], keys).is_empty());
        assert!(temporary_keys(&["ssh".into(), "-i".into(), "/run/user/1000/kalango-remote/keys/../../x".into()], keys).is_empty());
        assert!(parse_remote_command(r#"["sh","-c","rm -rf ~"]"#).is_err());
        assert!(parse_remote_command("kalango-remote: unknown server").is_err());
    }

    #[test]
    fn builds_ssh_arguments() {
        let h = Hosts::parse(SAMPLE).unwrap();
        let t = h.resolve("app-prod-01").unwrap();
        let a = t.ssh_argv(Some("/run/k/ssh/%C"));
        let s = a.join(" ");
        assert_eq!(a[0], "ssh");
        assert_eq!(a.last().unwrap(), "deploy@203.0.113.24");
        assert!(!a.contains(&"-p".to_string()), "port 22 is the default");
        assert!(s.contains("-i /chaves/deploy-ed25519"));
        assert!(s.contains("-J ops@bastiao.exemplo.com.br:2222"));
        assert!(s.contains("-o StrictHostKeyChecking=ask"));
        assert!(s.contains("-o ServerAliveInterval=30"));
        assert!(s.contains("-o ControlPath=/run/k/ssh/%C") && s.contains("-o ControlMaster=auto"));
        assert!(s.contains("-L 127.0.0.1:5433:localhost:5432"));
        let b = h.resolve("bastiao-trabalho").unwrap().ssh_argv(None);
        assert_eq!(b, vec!["ssh", "-p", "2222", "-o", "ServerAliveCountMax=2", "-o", "ServerAliveInterval=15", "-o", "StrictHostKeyChecking=ask", "ops@bastiao.exemplo.com.br"]);
        // the host's own options win over the Terminal's
        let mut o = t.clone();
        o.options.insert("StrictHostKeyChecking".into(), "yes".into());
        o.options.insert("UserKnownHostsFile".into(), "/teste/known_hosts".into());
        let s = o.ssh_argv(None).join(" ");
        assert!(s.contains("StrictHostKeyChecking=yes") && !s.contains("=ask") && s.contains("UserKnownHostsFile=/teste/known_hosts"));
    }

    #[test]
    fn builds_mosh_and_sftp_arguments() {
        let h = Hosts::parse(SAMPLE).unwrap();
        let mut r = h.resolve("raspberry-sala").unwrap();
        r.port = 2222;
        let a = r.mosh_argv(Some("/run/k/%C"));
        assert_eq!(a[0], "mosh");
        assert!(a[1].starts_with("--ssh=ssh -p 2222 -o "), "{}", a[1]);
        assert!(a.contains(&"--server=/opt/mosh/bin/mosh-server".to_string()));
        assert_eq!(&a[a.len() - 2..], ["--", "pi@raspberry.local"]);
        let t = h.resolve("app-prod-01").unwrap();
        let s = t.sftp_argv(Some("/run/k/%C"), "/tmp/lote").join(" ");
        assert!(s.starts_with("sftp -b /tmp/lote "));
        assert!(s.contains("ControlMaster=no") && s.contains("BatchMode=yes") && !s.contains("-L ") && !s.contains("ControlPersist"));
        let mut p = t.clone();
        p.port = 2200;
        assert!(p.sftp_argv(None, "b").join(" ").contains("-P 2200"), "sftp's port flag is -P");
        assert_eq!(p.known_hosts_name(), "[203.0.113.24]:2200");
        assert_eq!(shell_quote("a b'c"), "'a b'\\''c'");
        let g = "hostname 203.0.113.24\nuserknownhostsfile /teste/known_hosts /teste/known_hosts2\n";
        assert_eq!(known_hosts_files(g), vec!["/teste/known_hosts", "/teste/known_hosts2"]);
        let k = "# Host [127.0.0.1]:2222 found: line 1 \n[127.0.0.1]:2222 ED25519 SHA256:j/ZGTIVN\n";
        assert_eq!(known_key(k), Some(("ED25519".into(), "SHA256:j/ZGTIVN".into())));
        assert_eq!(known_key(""), None);
        let p = t.ping_argv("/c").join(" ");
        assert!(p.ends_with("-T deploy@203.0.113.24 true") && p.contains("BatchMode=yes"));
    }

    #[test]
    fn refuses_hosts_that_would_be_options_or_commands() {
        let bad = [
            r#"[{"id": "a", "address": "-oProxyCommand=touch /tmp/x"}]"#,
            r#"[{"id": "a", "address": "h; rm -rf ~"}]"#,
            r#"[{"id": "a", "address": "h", "user": "-x"}]"#,
            r#"[{"id": "a", "address": "h", "options": {"Proxy Command": "x"}}]"#,
            r#"[{"id": "a", "address": "h", "jump": ["-oProxyCommand=x"]}]"#,
            r#"[{"id": "a", "address": "h", "jump": ["b"]}, {"id": "b", "address": "h2", "jump": ["a"]}]"#,
        ];
        for b in bad {
            assert!(Hosts::parse(b).unwrap().resolve("a").is_err(), "{b}");
        }
    }

    #[test]
    fn reads_ssh_and_mosh_screens() {
        let s = "The authenticity of host '[127.0.0.1]:2222 ([127.0.0.1]:2222)' can't be established.\nED25519 key fingerprint is SHA256:Qx3pV9tL0aB7abc+wK2M.\nThis key is not known by any other names.\nAre you sure you want to continue connecting (yes/no/[fingerprint])? ";
        let k = unknown_key_prompt(s).unwrap();
        assert_eq!(k, UnknownKey { host: "[127.0.0.1]:2222".into(), key_type: "ED25519".into(), fingerprint: "SHA256:Qx3pV9tL0aB7abc+wK2M".into() });
        assert!(unknown_key_prompt(&format!("{s}yes\nWarning: Permanently added")).is_none(), "answered");
        let c = "@@@@@@\n@    WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!     @\n@@@@\nIT IS POSSIBLE THAT SOMEONE IS DOING SOMETHING NASTY!\nSomeone could be eavesdropping on you right now (man-in-the-middle attack)!\nIt is also possible that a host key has just been changed.\nThe fingerprint for the ED25519 key sent by the remote host is\nSHA256:c2NabcVv9A.\nPlease contact your system administrator.";
        assert_eq!(changed_key(c), Some(("ED25519".into(), "SHA256:c2NabcVv9A".into())));
        assert_eq!(mosh_last_contact("mosh: Last contact 8 seconds ago. [To quit: Ctrl-^ .]"), Some(8));
        assert_eq!(mosh_last_contact("mosh: Last contact 1:05 ago. [To quit: Ctrl-^ .]"), Some(65));
        assert_eq!(mosh_last_contact("deploy@app:~$ "), None);
        assert!(mosh_server_missing("bash: line 1: mosh-server: command not found\n/usr/bin/mosh: Did not find mosh server startup message."));
    }

    #[test]
    fn destructive_commands_and_secrets() {
        for c in ["rm -rf /var/log/app", "sudo rm -f x", "sudo reboot", "systemctl restart nginx", "docker rm -f web", "kubectl delete pod x",
                  "psql -c 'DROP TABLE pedidos'", "dd if=/dev/zero of=/dev/sda", "find /var/log -mtime +7 -exec rm -f {} ;", "git push --force"] {
            assert!(is_destructive(c), "{c}");
        }
        for c in ["df -h /var", "du -sh /var/log/* | sort -h", "systemctl status nginx", "rm arquivo.txt", "docker ps", "cat format.txt", "ls -la"] {
            assert!(!is_destructive(c), "{c}");
        }
        let r = redact_secrets("DB_PASSWORD=hunter2\nok line\ntoken: abc123\n-----BEGIN OPENSSH PRIVATE KEY-----\nAAAA\n-----END OPENSSH PRIVATE KEY-----\nfim");
        assert!(!r.contains("hunter2") && !r.contains("abc123") && !r.contains("AAAA"), "{r}");
        assert!(r.contains("ok line") && r.contains("fim") && r.contains("DB_PASSWORD="));
        assert_eq!(prompt_dir("deploy@app-prod-01:~/app$ df -h"), Some("~/app".into()));
        assert_eq!(prompt_dir("root@h:/var/log# "), Some("/var/log".into()));
        assert_eq!(prompt_dir("14:02:11 INFO GET /health"), None);
    }
}
