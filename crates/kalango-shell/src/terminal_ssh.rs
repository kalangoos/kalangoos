//! SSH and Mosh sessions in the Terminal's tabs (design "Kalango Remote · SSH abre no Terminal do sistema", and
//! "SSH · primeira conexão (impressão digital)"). `kalango-shell terminal-ssh HOST-ID` opens a tab for a host of
//! Kalango Remote (ssh_hosts.rs), or brings its tab to the front.
//! - The tab runs the system's `ssh` (or `mosh`) in its VTE with the host's options: passwords are typed to ssh, in
//!   the terminal; keys come from the files or the agent. The Terminal keeps no secret.
//! - Its dot: gold while connecting (or while Mosh's network is down), teal connected, coral when it dropped for good.
//!   Badges: "produção" (with the coral stripe and the guard on Enter) and "mosh".
//! - **Auto-reconnect:** ssh's connection dropped (exit 255) — the text dims, "Reconectando… (2/5)", up to five tries
//!   (1, 2, 4, 8, 15 s apart); then "Reconectar".
//! - **Mosh:** mosh-client's own "Last contact 8 seconds ago" becomes the design's notice ("rede caiu há 8 s; a sessão
//!   continua…") and the gold dot; without mosh (here or a mosh-server there) the tab goes back to SSH with a notice.
//! - **Fingerprints:** ssh's own question about a host it does not know is read off the screen and asked in the
//!   design's dialog (the answer is typed to ssh); a changed key is ssh's hard refusal, shown as the design's coral
//!   dialog with no way to accept. The agent has no part in either.
//! - **One connection:** the session is ssh's ControlMaster: SFTP, the second pane and the latency reuse it (no
//!   second login). A Mosh tab has no ssh left after its start: its SFTP connects by itself (keys or the agent).

use super::*;
use crate::ssh_hosts::{changed_key, known_hosts_files, known_key, mosh_last_contact, mosh_server_missing, prompt_dir, unknown_key_prompt, Hosts, Target, UnknownKey};
use std::time::Duration;

const TRIES: u32 = 5;
const WAITS: [u64; 5] = [1, 2, 4, 8, 15];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Connecting,
    Connected,
    Reconnecting,
    Down,
    /// a changed key, or the person said no to an unknown one: no reconnecting by itself
    Blocked,
}

/// An SSH or Mosh session of a tab.
pub(super) struct Remote {
    pub target: Target,
    /// ssh's control socket for this host
    pub control: String,
    pub dot: gtk::Box,
    state: Cell<State>,
    attempts: Cell<u32>,
    mosh_failed: Cell<bool>,
    mosh_running: Cell<bool>,
    /// Mosh: seconds since the last contact, while the network is down
    lost: Cell<Option<u64>>,
    latency: Cell<Option<u32>>,
    key_type: RefCell<Option<String>>,
    asked: RefCell<Option<String>>,
    forget_key: Cell<bool>,
    banner: RefCell<Option<gtk::Box>>,
    /// bumped by each start: timers of an older one stop
    generation: Cell<u32>,
    pub closing: Cell<bool>,
    checking: Cell<bool>,
    pub sftp: RefCell<Option<Rc<sftp::Panel>>>,
    revealer: RefCell<Option<gtk::Revealer>>,
    /// the tab's "mosh" badge (it goes when the tab falls back to SSH)
    mosh_badge: RefCell<Option<gtk::Label>>,
    /// the key files Kalango Remote wrote for this session: deleted when the connection ends
    key_files: RefCell<Vec<PathBuf>>,
    /// the known_hosts its command checks against (Kalango Remote's own)
    known_files: RefCell<Vec<String>>,
    /// the command came from `kalango-remote ssh-command` (it checks host keys strictly: an unknown one is its job)
    from_remote: Cell<bool>,
}

/// Where the control sockets live (the session's runtime folder, only the person's).
fn control_dir() -> PathBuf {
    let d = glib::user_runtime_dir().join("kalango").join("ssh");
    let _ = std::fs::create_dir_all(&d);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o700));
    }
    d
}

/// A program's full path (the tab's argv[0]).
fn program(name: &str) -> Option<String> {
    glib::find_program_in_path(name).map(|p| p.display().to_string())
}

/// With its full path, or the name as is when it is not installed (the spawn says so).
fn absolute(mut argv: Vec<String>) -> Vec<String> {
    if let Some(p) = argv.first().and_then(|n| program(n)) {
        argv[0] = p;
    }
    argv
}

/// Runs a command to its end (no window, no terminal): its output, or None.
fn output(argv: &[String]) -> Option<String> {
    let o = std::process::Command::new(&argv[0]).args(&argv[1..]).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).into_owned())
}

/// The key known_hosts has for the host (type, fingerprint), and the files ssh reads it from.
fn known(t: &Target, given: &[String]) -> (Option<(String, String)>, Vec<String>) {
    // (Kalango Remote's command names its own known_hosts; else the ones ssh would read for these options)
    let files = if given.is_empty() { output(&absolute(t.config_argv())).map(|g| known_hosts_files(&g)).unwrap_or_default() } else { given.to_vec() };
    let key = files.iter().find_map(|f| output(&absolute(vec!["ssh-keygen".into(), "-l".into(), "-F".into(), t.known_hosts_name(), "-f".into(), f.clone()])).and_then(|o| known_key(&o)));
    (key, files)
}

/// Kalango Remote on its SSH page (servers, keys and known servers: `kalango-remote --page ssh`).
fn open_remote_ssh() {
    detach(&absolute(vec!["kalango-remote".into(), "--page".into(), "ssh".into()]));
}

/// Runs a command in the background, its output thrown away.
fn detach(argv: &[String]) {
    let a: Vec<&std::ffi::OsStr> = argv.iter().map(std::ffi::OsStr::new).collect();
    let _ = gio::Subprocess::newv(&a, gio::SubprocessFlags::STDOUT_SILENCE | gio::SubprocessFlags::STDERR_SILENCE | gio::SubprocessFlags::STDIN_INHERIT);
}

impl Remote {
    /// Kalango Remote's key files for this session go (docs/HOSTS.md: the Terminal deletes them when the connection
    /// ends; a reconnect asks for them again).
    pub fn delete_keys(&self) {
        for k in self.key_files.borrow_mut().drain(..) {
            if std::fs::remove_file(&k).is_ok() && debug() {
                eprintln!("debug terminal: chave temporária apagada ({})", k.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
            }
        }
    }

    pub fn new(target: Target) -> Rc<Remote> {
        let id: String = target.id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).take(40).collect();
        let control = control_dir().join(format!("{id}.sock")).display().to_string();
        let dot = caixa(Orientation::Horizontal, 0, "term-ponto");
        dot.set_size_request(6, 6);
        dot.set_valign(Align::Center);
        let r = Rc::new(Remote {
            target, control, dot, state: Cell::new(State::Connecting), attempts: Cell::new(0), mosh_failed: Cell::new(false),
            mosh_running: Cell::new(false), lost: Cell::new(None), latency: Cell::new(None), key_type: RefCell::default(),
            asked: RefCell::default(), forget_key: Cell::new(false), banner: RefCell::default(), generation: Cell::new(0),
            closing: Cell::new(false), checking: Cell::new(false), sftp: RefCell::default(), revealer: RefCell::default(), mosh_badge: RefCell::default(),
            key_files: RefCell::default(), known_files: RefCell::default(), from_remote: Cell::new(false),
        });
        r.paint();
        r
    }

    /// The tab's badges: "produção", "mosh".
    pub fn badges(&self) -> Vec<gtk::Label> {
        let mut v = Vec::new();
        if self.target.production {
            v.push(rotulo(tr("production"), "term-selo-producao"));
        }
        if self.target.mosh {
            let b = rotulo("mosh", "term-selo-mosh");
            *self.mosh_badge.borrow_mut() = Some(b.clone());
            v.push(b);
        }
        for b in &v {
            b.set_valign(Align::Center);
        }
        v
    }

    pub fn running_mosh(&self) -> bool {
        self.mosh_running.get()
    }

    /// The dot's colour for the state.
    fn paint(&self) {
        for c in ["conectado", "conectando", "caiu"] {
            self.dot.remove_css_class(c);
        }
        self.dot.add_css_class(match self.state.get() {
            State::Connected if self.lost.get().is_none() => "conectado",
            State::Connecting | State::Reconnecting | State::Connected => "conectando",
            State::Down | State::Blocked => "caiu",
        });
    }

    fn set_state(&self, s: State) {
        self.state.set(s);
        self.paint();
    }

    /// "deploy@app-prod-01  via bastiao-trabalho  23 ms  ED25519".
    pub fn status_text(&self) -> String {
        let mut parts = vec![self.target.label()];
        if !self.target.jump_names.is_empty() {
            parts.push(trf("via {host}", &[("host", self.target.jump_names.join(" › "))]));
        }
        match self.state.get() {
            State::Connected => {
                if let Some(ms) = self.latency.get() {
                    parts.push(format!("{ms} ms"));
                }
            }
            State::Connecting => parts.push(tr("connecting…").to_string()),
            State::Reconnecting => parts.push(tr("reconnecting…").to_string()),
            State::Down | State::Blocked => parts.push(tr("disconnected").to_string()),
        }
        if let Some(k) = self.key_type.borrow().as_ref() {
            parts.push(k.clone());
        }
        parts.join("  ")
    }

    /// The command for a second pane: ssh to the same host (through the session's connection).
    pub fn argv(&self) -> Vec<String> {
        absolute(self.target.ssh_argv(Some(&self.control)))
    }
}

/// The coral stripe of a production host (design: "Produção · deploy@app-prod-01 · comandos que apagam ou reiniciam
/// pedem confirmação").
pub(super) fn production_stripe(t: &Target) -> gtk::Box {
    let b = caixa(Orientation::Horizontal, 8, "term-producao");
    b.append(&ui_icon("alerta", "#EFA2A6", 14));
    let l = rotulo(&trf("Production · {host} · commands that erase or restart ask for confirmation", &[("host", t.label())]), "term-producao-texto");
    l.set_ellipsize(pango::EllipsizeMode::End);
    l.set_xalign(0.0);
    b.append(&l);
    b
}

/// Opens a host of Kalango Remote in a tab of the Terminal's window, or brings its tab to the front.
/// `files`: with the SFTP panel open (`terminal-ssh HOST-ID --arquivos`, Kalango Remote's "Arquivos").
pub fn open_host(shell: &Rc<Shell>, id: &str, files: bool) {
    let t = window(shell);
    let found = t.tabs.borrow().iter().position(|x| x.remote.as_ref().is_some_and(|r| r.target.id == id || r.target.name == id));
    if let Some(i) = found {
        t.select(i);
        // (the tab blinks once: it was already open)
        if let Some(x) = t.tabs.borrow().get(i) {
            x.tab.add_css_class("pisca");
            let w = x.tab.downgrade();
            glib::timeout_add_local_once(Duration::from_millis(240), move || {
                if let Some(b) = w.upgrade() {
                    b.remove_css_class("pisca");
                }
            });
        }
        if debug() {
            eprintln!("debug terminal: ssh {id} já aberto (aba {i})");
        }
    } else {
        match Hosts::load(&crate::ssh_hosts::hosts_path()).and_then(|h| h.resolve(id)) {
            Ok(target) => {
                if debug() {
                    eprintln!("debug terminal: ssh {id} → {}", target.ssh_argv(None).join(" "));
                }
                t.add_tab(None, Some(Remote::new(target)));
            }
            Err(e) => {
                eprintln!("kalango-shell: terminal-ssh {id}: {e}");
                if t.tabs.borrow().is_empty() {
                    t.new_tab(None);
                }
                t.st_hint.set_text(&trf("Could not open {host}: {error}", &[("host", id.to_string()), ("error", e)]));
            }
        }
    }
    if files {
        let tab = t.tabs.borrow().get(t.current.get()).cloned();
        if let Some(x) = tab.filter(|x| x.remote.is_some()) {
            let open = x.remote.as_ref().and_then(|r| r.revealer.borrow().clone()).is_some_and(|v| v.reveals_child());
            if !open {
                t.toggle_files(&x);
            }
        }
    }
    t.window.present();
}

impl Terminal {
    /// Starts (or starts again) a tab's session.
    pub(super) fn spawn_remote(self: &Rc<Self>, x: &Rc<Tab>) {
        let Some(r) = x.remote.clone() else { return };
        let gen = r.generation.get() + 1;
        r.generation.set(gen);
        r.set_state(if r.attempts.get() > 0 { State::Reconnecting } else { State::Connecting });
        r.lost.set(None);
        r.asked.borrow_mut().take();
        let want_mosh = r.target.mosh && !r.mosh_failed.get();
        if want_mosh && program("mosh").is_none() {
            r.mosh_failed.set(true);
            self.st_hint.set_text(tr("Mosh is not installed on this computer: the tab uses SSH."));
        }
        let mosh = r.target.mosh && !r.mosh_failed.get();
        r.mosh_running.set(mosh);
        if let Some(b) = r.mosh_badge.borrow().as_ref() {
            b.set_visible(mosh);
        }
        // the command: Kalango Remote's when it is installed (its keys, its known_hosts, its jump chain), else built
        // here from hosts.json
        let own = absolute(if mosh { r.target.mosh_argv(Some(&r.control)) } else { r.target.ssh_argv(Some(&r.control)) });
        if program("kalango-remote").is_none() || std::env::var_os("KALANGO_TERMINAL_SEM_REMOTE").is_some() {
            self.start_remote(x, own, gen);
            return;
        }
        let (id, control, want) = (r.target.id.clone(), r.control.clone(), if mosh { "mosh" } else { "ssh" });
        let (w, xw) = (Rc::downgrade(self), Rc::downgrade(x));
        crate::lateral::em_segundo_plano(move || {
            let o = std::process::Command::new("kalango-remote").args(["ssh-command", &id]).stdin(std::process::Stdio::null()).output();
            match o {
                Ok(o) if o.status.success() => crate::ssh_hosts::parse_remote_command(&String::from_utf8_lossy(&o.stdout)).map(Some),
                // (an id it does not know: the Terminal's own command)
                Ok(o) if o.status.code() == Some(2) => Ok(None),
                Ok(o) => Err(String::from_utf8_lossy(&o.stderr).trim().trim_start_matches("kalango-remote: ").to_string()),
                Err(_) => Ok(None),
            }
        }, move |res: Result<Option<Vec<String>>, String>| {
            let (Some(t), Some(x)) = (w.upgrade(), xw.upgrade()) else { return };
            let Some(r) = x.remote.clone() else { return };
            if r.generation.get() != gen || r.closing.get() {
                return;
            }
            match res {
                Ok(Some(argv)) if Path::new(&argv[0]).file_name().is_some_and(|n| n == want) => {
                    let keys = glib::user_runtime_dir().join("kalango-remote").join("keys");
                    *r.key_files.borrow_mut() = crate::ssh_hosts::temporary_keys(&argv, &keys);
                    *r.known_files.borrow_mut() = crate::ssh_hosts::named_known_hosts(&argv);
                    r.from_remote.set(true);
                    t.start_remote(&x, absolute(crate::ssh_hosts::with_control(argv, &control)), gen);
                }
                // (Kalango Remote's is Mosh, and the tab went back to SSH; or it does not know the host)
                Ok(_) => t.start_remote(&x, own, gen),
                Err(e) => {
                    eprintln!("kalango-shell: kalango-remote ssh-command: {e}");
                    r.set_state(State::Down);
                    t.remote_banner(&x, "caiu", &trf("Kalango Remote could not prepare {host}: {error}", &[("host", r.target.name.clone()), ("error", e)]), true);
                    t.update_status();
                }
            }
        });
    }

    /// Runs the session's command in the tab, and watches for the login.
    fn start_remote(self: &Rc<Self>, x: &Rc<Tab>, argv: Vec<String>, gen: u32) {
        let Some(r) = x.remote.clone() else { return };
        let argv_refs: Vec<&str> = argv.iter().map(String::as_str).collect();
        let program_name = argv.first().cloned().unwrap_or_default();
        let env = ["COLORTERM=truecolor", "TERM_PROGRAM=KalangoOS-Terminal"];
        if debug() {
            eprintln!("debug terminal: ssh conectando {} (tentativa {}): {}", r.target.id, r.attempts.get(), argv.join(" "));
        }
        let (w, xw) = (Rc::downgrade(self), Rc::downgrade(x));
        x.term.spawn_async(vte4::PtyFlags::DEFAULT, Some(&glib::home_dir().display().to_string()), &argv_refs, &env, glib::SpawnFlags::DEFAULT, || {}, -1,
            gio::Cancellable::NONE, move |res| {
                let (Some(t), Some(x)) = (w.upgrade(), xw.upgrade()) else { return };
                match res {
                    Ok(pid) => x.pid.set(pid.0),
                    Err(e) => {
                        eprintln!("kalango-shell: terminal: ssh: {e}");
                        if let Some(r) = &x.remote {
                            r.set_state(State::Down);
                        }
                        t.remote_banner(&x, "caiu", &trf("Could not start {program}: {error}", &[("program", program_name.clone()), ("error", e.to_string())]), true);
                    }
                }
            });
        self.update_status();
        // connected: ssh's control socket is up (after the login)
        let (w, xw) = (Rc::downgrade(self), Rc::downgrade(x));
        glib::timeout_add_local(Duration::from_millis(300), move || {
            let (Some(t), Some(x)) = (w.upgrade(), xw.upgrade()) else { return glib::ControlFlow::Break };
            let Some(r) = x.remote.clone() else { return glib::ControlFlow::Break };
            if r.generation.get() != gen || r.closing.get() || !matches!(r.state.get(), State::Connecting | State::Reconnecting) {
                return glib::ControlFlow::Break;
            }
            // (Mosh: its script becomes mosh-client once mosh-server answered — no ssh stays up to tell)
            let mosh_up = r.mosh_running.get() && x.pid.get() > 0
                && std::fs::read_to_string(format!("/proc/{}/comm", x.pid.get())).is_ok_and(|c| c.trim() == "mosh-client");
            if (Path::new(&r.control).exists() || mosh_up) && x.pid.get() > 0 {
                t.remote_connected(&x);
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
    }

    fn remote_connected(self: &Rc<Self>, x: &Rc<Tab>) {
        let Some(r) = x.remote.clone() else { return };
        r.attempts.set(0);
        r.set_state(State::Connected);
        self.hide_remote_banner(x);
        x.term.set_opacity(1.0);
        // (Mosh: its ssh is gone once mosh-client runs — Kalango Remote's key file is not needed any more)
        if r.mosh_running.get() {
            r.delete_keys();
        }
        if debug() {
            eprintln!("debug terminal: ssh conectado {}", r.target.id);
        }
        // "Lembrar este servidor" off: the key ssh just wrote goes again
        if r.forget_key.replace(false) {
            let (t, given) = (r.target.clone(), r.known_files.borrow().clone());
            crate::lateral::em_segundo_plano(move || {
                for f in known(&t, &given).1 {
                    let _ = output(&absolute(vec!["ssh-keygen".into(), "-R".into(), t.known_hosts_name(), "-f".into(), f]));
                }
            }, |_| {
                if debug() {
                    eprintln!("debug terminal: ssh chave não lembrada (tirada do known_hosts)");
                }
            });
        }
        self.measure(x);
        if let Some(p) = r.sftp.borrow().clone() {
            p.refresh();
        }
        // the latency again every 30 s while connected
        let (w, xw, gen) = (Rc::downgrade(self), Rc::downgrade(x), r.generation.get());
        glib::timeout_add_local(Duration::from_secs(30), move || {
            let (Some(t), Some(x)) = (w.upgrade(), xw.upgrade()) else { return glib::ControlFlow::Break };
            let alive = x.remote.as_ref().is_some_and(|r| r.generation.get() == gen && r.state.get() == State::Connected && !r.closing.get());
            if !alive {
                return glib::ControlFlow::Break;
            }
            t.measure(&x);
            glib::ControlFlow::Continue
        });
    }

    /// The key type (from known_hosts) and a round trip's time, for the status line.
    fn measure(self: &Rc<Self>, x: &Rc<Tab>) {
        let Some(r) = x.remote.clone() else { return };
        let (t, control, given) = (r.target.clone(), r.control.clone(), r.known_files.borrow().clone());
        let (w, xw) = (Rc::downgrade(self), Rc::downgrade(x));
        crate::lateral::em_segundo_plano(move || {
            let key = known(&t, &given).0.map(|k| k.0);
            let start = Instant::now();
            let ok = output(&absolute(t.ping_argv(&control))).is_some();
            (key, ok.then(|| start.elapsed().as_millis() as u32))
        }, move |(key, ms)| {
            let (Some(t), Some(x)) = (w.upgrade(), xw.upgrade()) else { return };
            let Some(r) = &x.remote else { return };
            if key.is_some() {
                *r.key_type.borrow_mut() = key;
            }
            if ms.is_some() {
                r.latency.set(ms);
            }
            if t.tabs.borrow().get(t.current.get()).is_some_and(|c| Rc::ptr_eq(c, &x)) {
                t.update_status();
            }
        });
    }

    /// The session's program ended: the person's "exit" closes the tab; a dropped connection reconnects.
    pub(super) fn remote_exited(self: &Rc<Self>, x: &Rc<Tab>, status: i32) {
        let Some(r) = x.remote.clone() else { return };
        r.delete_keys();
        if r.closing.get() {
            return;
        }
        // (what ssh said last, read before deciding: a changed key, no mosh-server)
        self.check_screen(x);
        let code = if status & 0x7f == 0 { (status >> 8) & 0xff } else { -1 };
        let was_connected = r.state.get() == State::Connected;
        if debug() {
            eprintln!("debug terminal: ssh {} saiu com {code} ({:?}, tentativa {})", r.target.id, r.state.get(), r.attempts.get());
        }
        if r.state.get() == State::Blocked {
            self.update_status();
            return;
        }
        if r.mosh_running.get() && r.mosh_failed.get() {
            // no mosh-server on the host: SSH, with a notice
            self.st_hint.set_text(&trf("There is no mosh-server on {host}: the tab went back to SSH.", &[("host", r.target.name.clone())]));
            x.term.feed(b"\r\n");
            self.spawn_remote(x);
            return;
        }
        let dropped = code == 255 || code < 0 || (r.mosh_running.get() && code != 0);
        if !dropped {
            if let Some(i) = self.index_of(x) {
                self.close_tab_now(i);
            }
            return;
        }
        if !was_connected && r.attempts.get() == 0 {
            // never got in (refused, no route, wrong key): the person reconnects when ready
            r.set_state(State::Down);
            self.remote_banner(x, "caiu", &trf("Could not connect to {host}.", &[("host", r.target.name.clone())]), true);
            self.update_status();
            return;
        }
        let n = r.attempts.get() + 1;
        if n > TRIES {
            r.attempts.set(0);
            r.set_state(State::Down);
            x.term.set_opacity(1.0);
            self.remote_banner(x, "caiu", &trf("The connection to {host} dropped.", &[("host", r.target.name.clone())]), true);
            self.update_status();
            return;
        }
        r.attempts.set(n);
        r.set_state(State::Reconnecting);
        x.term.set_opacity(0.5);
        self.remote_banner(x, "conectando", &trf("Reconnecting… ({n}/{total})", &[("n", n.to_string()), ("total", TRIES.to_string())]), false);
        self.update_status();
        let (w, xw, gen) = (Rc::downgrade(self), Rc::downgrade(x), r.generation.get());
        glib::timeout_add_local_once(Duration::from_secs(WAITS[(n - 1) as usize]), move || {
            let (Some(t), Some(x)) = (w.upgrade(), xw.upgrade()) else { return };
            if x.remote.as_ref().is_some_and(|r| r.generation.get() == gen && !r.closing.get()) {
                x.term.feed(b"\r\n");
                t.spawn_remote(&x);
            }
        });
    }

    /// A notice over the top of the panes: reconnecting, dropped (with "Reconectar"), Mosh's network down.
    fn remote_banner(self: &Rc<Self>, x: &Rc<Tab>, dot: &str, text: &str, reconnect: bool) {
        self.remote_banner_inner(x, dot, text, reconnect, None);
    }

    /// The notice with another button (opening Kalango Remote).
    fn remote_banner_with(self: &Rc<Self>, x: &Rc<Tab>, dot: &str, text: &str, action: Option<(&str, fn())>) {
        self.remote_banner_inner(x, dot, text, false, action);
    }

    fn remote_banner_inner(self: &Rc<Self>, x: &Rc<Tab>, dot: &str, text: &str, reconnect: bool, action: Option<(&str, fn())>) {
        self.hide_remote_banner(x);
        let Some(r) = &x.remote else { return };
        let b = caixa(Orientation::Horizontal, 8, "term-sessao-aviso");
        b.set_halign(Align::Center);
        b.set_valign(Align::Start);
        b.set_margin_top(10);
        let d = caixa(Orientation::Horizontal, 0, "term-ponto");
        d.add_css_class(dot);
        d.set_size_request(6, 6);
        d.set_valign(Align::Center);
        b.append(&d);
        let l = rotulo(text, "term-sessao-aviso-texto");
        l.set_ellipsize(pango::EllipsizeMode::End);
        b.append(&l);
        if reconnect {
            let again = gtk::Button::with_label(tr("Reconnect"));
            again.add_css_class("term-sessao-aviso-botao");
            let (w, xw) = (Rc::downgrade(self), Rc::downgrade(x));
            again.connect_clicked(move |_| {
                let (Some(t), Some(x)) = (w.upgrade(), xw.upgrade()) else { return };
                if let Some(r) = &x.remote {
                    r.attempts.set(0);
                }
                t.hide_remote_banner(&x);
                x.term.feed(b"\r\n");
                t.spawn_remote(&x);
            });
            crate::central::posicao_debug("terminal reconectar", again.upcast_ref());
            b.append(&again);
        }
        if let Some((label, f)) = action {
            let go = gtk::Button::with_label(label);
            go.add_css_class("term-sessao-aviso-botao");
            go.connect_clicked(move |_| f());
            crate::central::posicao_debug("terminal abrir kalango remote", go.upcast_ref());
            b.append(&go);
        }
        x.page.add_overlay(&b);
        *r.banner.borrow_mut() = Some(b);
        if debug() {
            eprintln!("debug terminal: aviso «{text}»");
        }
    }

    fn hide_remote_banner(&self, x: &Rc<Tab>) {
        if let Some(b) = x.remote.as_ref().and_then(|r| r.banner.borrow_mut().take()) {
            x.page.remove_overlay(&b);
        }
    }

    /// The session's screen changed: read it a moment later (ssh's questions, Mosh's notice, the folder).
    pub(super) fn screen_changed(self: &Rc<Self>, x: &Rc<Tab>) {
        let Some(r) = &x.remote else { return };
        if r.checking.replace(true) {
            return;
        }
        let (w, xw) = (Rc::downgrade(self), Rc::downgrade(x));
        glib::timeout_add_local_once(Duration::from_millis(120), move || {
            let (Some(t), Some(x)) = (w.upgrade(), xw.upgrade()) else { return };
            if let Some(r) = &x.remote {
                r.checking.set(false);
            }
            t.check_screen(&x);
        });
    }

    fn check_screen(self: &Rc<Self>, x: &Rc<Tab>) {
        let Some(r) = x.remote.clone() else { return };
        let text = x.term.text_format(vte4::Format::Text).map(|s| s.to_string()).unwrap_or_default();
        if matches!(r.state.get(), State::Connecting | State::Reconnecting) {
            if let Some(k) = unknown_key_prompt(&text) {
                // (one question per host: a jump host's and then the server's may show the same key)
                let id = format!("{}|{}", k.host, k.fingerprint);
                let asked = r.asked.borrow().clone();
                if asked.as_deref() != Some(id.as_str()) {
                    *r.asked.borrow_mut() = Some(id);
                    self.ask_fingerprint(x, k);
                }
            }
            if let Some((kt, fp)) = changed_key(&text) {
                r.set_state(State::Blocked);
                x.term.set_opacity(1.0);
                self.remote_banner(x, "caiu", &trf("Blocked: the key of {host} changed.", &[("host", r.target.name.clone())]), false);
                self.key_changed(x, kt, fp);
            }
            // Kalango Remote's command checks keys strictly: a host it does not know yet is its own dialog's job
            if r.from_remote.get() && changed_key(&text).is_none() && (text.contains("and you have requested strict checking") || text.contains("Host key verification failed")) {
                r.set_state(State::Blocked);
                x.term.set_opacity(1.0);
                self.remote_banner_with(x, "caiu", &trf("Kalango Remote does not know the key of {host} yet.", &[("host", r.target.name.clone())]), Some((tr("Open Kalango Remote"), open_remote_ssh as fn())));
            }
            if r.mosh_running.get() && mosh_server_missing(&text) {
                r.mosh_failed.set(true);
            }
        }
        if r.mosh_running.get() && r.state.get() == State::Connected {
            let lost = text.lines().next().and_then(mosh_last_contact);
            if lost != r.lost.get() {
                r.lost.set(lost);
                r.paint();
                match lost {
                    Some(s) => self.remote_banner(x, "conectando", &trf("mosh · the network dropped {n} s ago; the session goes on and syncs when it is back", &[("n", s.to_string())]), false),
                    None => self.hide_remote_banner(x),
                }
            }
        }
        let panel = r.sftp.borrow().clone();
        if let (Some(p), Some(dir)) = (panel, r.remote_dir_of(&x.term)) {
            p.follow(&dir);
        }
    }

    /// "Primeira conexão com app-prod-01": the fingerprint ssh asks about, to compare before trusting.
    fn ask_fingerprint(self: &Rc<Self>, x: &Rc<Tab>, k: UnknownKey) {
        let Some(r) = x.remote.clone() else { return };
        let card = caixa(Orientation::Vertical, 14, "arq-dialogo");
        card.add_css_class("term-digital");
        card.set_size_request(480, -1);
        let head = caixa(Orientation::Horizontal, 10, "");
        head.append(&ui_icon("escudo", "#A5A1E2", 16));
        let title = rotulo(&trf("First connection to {host}", &[("host", r.target.name.clone())]), "arq-dialogo-titulo");
        title.set_xalign(0.0);
        head.append(&title);
        card.append(&head);
        let p = rotulo(tr("KalangoOS does not know this server yet. Check the fingerprint with whoever runs the server (or in the hosting panel) before trusting it."), "term-digital-texto");
        p.set_wrap(true);
        p.set_max_width_chars(1);
        p.set_hexpand(true);
        p.set_xalign(0.0);
        card.append(&p);
        let server = format!("{}:{}", r.target.destination(), r.target.port);
        let fp = k.fingerprint.strip_prefix("SHA256:").unwrap_or(&k.fingerprint).to_string();
        let mut rows = vec![(tr("Server").to_string(), server, "")];
        if !r.target.jump_names.is_empty() {
            rows.push((tr("Through the jump").to_string(), r.target.jump_names.join(" › "), ""));
        }
        // (a question about a jump host: its name)
        if !k.host.contains(&r.target.address) {
            rows.push((tr("Asked for").to_string(), k.host.clone(), ""));
        }
        rows.push((tr("Key").to_string(), k.key_type.clone(), ""));
        rows.push(("SHA256".to_string(), fp, "realce"));
        card.append(&key_table(&rows));
        let remember = gtk::CheckButton::with_label(tr("Remember this server (keep it in known_hosts)"));
        remember.add_css_class("arq-dialogo-check");
        remember.set_active(true);
        card.append(&remember);
        let buttons = caixa(Orientation::Horizontal, 8, "");
        buttons.set_homogeneous(true);
        let cancel = gtk::Button::with_label(tr("Cancel"));
        cancel.add_css_class("arq-acao");
        let trust = gtk::Button::with_label(tr("Trust and connect"));
        trust.add_css_class("arq-acao-principal");
        buttons.append(&cancel);
        buttons.append(&trust);
        card.append(&buttons);
        let close = self.show_dialog(&card, &cancel);
        let (c, xw) = (close.clone(), Rc::downgrade(x));
        cancel.connect_clicked(move |_| {
            c();
            if let Some(x) = xw.upgrade() {
                // (the answer goes to ssh, which then refuses; no reconnecting by itself)
                if let Some(r) = &x.remote {
                    r.set_state(State::Blocked);
                }
                x.term.feed_child(b"no\n");
            }
        });
        let (c, xw, rw) = (close.clone(), Rc::downgrade(x), remember.downgrade());
        trust.connect_clicked(move |_| {
            c();
            let Some(x) = xw.upgrade() else { return };
            if let Some(r) = &x.remote {
                r.forget_key.set(!rw.upgrade().is_some_and(|b| b.is_active()));
            }
            x.term.feed_child(b"yes\n");
            x.term.grab_focus();
        });
        crate::central::posicao_debug("terminal confiar", trust.upcast_ref());
        crate::central::posicao_debug("terminal lembrar", remember.upcast_ref());
        crate::central::posicao_debug("terminal cancelar digital", cancel.upcast_ref());
        if debug() {
            eprintln!("debug terminal: impressão digital de {} ({} {})", k.host, k.key_type, k.fingerprint);
        }
    }

    /// "A impressão digital MUDOU": ssh refused; nothing here accepts the new key.
    fn key_changed(self: &Rc<Self>, x: &Rc<Tab>, key_type: String, fingerprint: String) {
        let Some(r) = x.remote.clone() else { return };
        let card = caixa(Orientation::Vertical, 14, "arq-dialogo");
        card.add_css_class("term-digital");
        card.add_css_class("mudou");
        card.set_size_request(500, -1);
        let head = caixa(Orientation::Horizontal, 10, "");
        head.append(&ui_icon("alerta", "#EFA2A6", 16));
        let title = rotulo(tr("The fingerprint CHANGED"), "arq-dialogo-titulo");
        title.set_xalign(0.0);
        title.set_hexpand(true);
        head.append(&title);
        head.append(&rotulo(tr("high risk"), "term-selo-producao"));
        card.append(&head);
        let p = rotulo(&trf("The key of the server {host} is not the same as last time. It may be a reinstall, or someone in the middle of the way. For safety, the connection was blocked: neither you here nor the agent accept the new key in this dialog.", &[("host", r.target.name.clone())]), "term-digital-texto");
        p.set_wrap(true);
        p.set_max_width_chars(1);
        p.set_xalign(0.0);
        card.append(&p);
        let table_slot = caixa(Orientation::Vertical, 0, "");
        card.append(&table_slot);
        let how = caixa(Orientation::Vertical, 4, "term-digital-como");
        let how_title = rotulo(tr("How to solve it"), "term-digital-como-titulo");
        how_title.set_xalign(0.0);
        how.append(&how_title);
        let steps = rotulo(tr("1. Confirm the new fingerprint with whoever runs the server.  2. In SSH › Keys › Known servers, remove the old key.  3. Connect again: the first connection dialog comes up."), "term-digital-como-texto");
        steps.set_wrap(true);
        steps.set_max_width_chars(1);
        steps.set_xalign(0.0);
        how.append(&steps);
        card.append(&how);
        let buttons = caixa(Orientation::Horizontal, 8, "");
        buttons.set_homogeneous(true);
        let known_btn = gtk::Button::with_label(tr("Open known servers"));
        known_btn.add_css_class("arq-acao");
        let no = gtk::Button::with_label(tr("Do not connect"));
        no.add_css_class("arq-acao-principal");
        buttons.append(&known_btn);
        buttons.append(&no);
        card.append(&buttons);
        let close = self.show_dialog(&card, &no);
        let c = close.clone();
        no.connect_clicked(move |_| c());
        // before / now / the key: known_hosts' entry, read in the background
        let (t, given) = (r.target.clone(), r.known_files.borrow().clone());
        let (slot, now_fp, now_kt) = (table_slot.downgrade(), fingerprint.clone(), key_type.clone());
        let files: Rc<RefCell<Vec<String>>> = Rc::default();
        let f2 = files.clone();
        crate::lateral::em_segundo_plano(move || known(&t, &given), move |(before, fs)| {
            *f2.borrow_mut() = fs;
            let Some(slot) = slot.upgrade() else { return };
            let (bkt, bfp) = before.unwrap_or_else(|| ("?".into(), "—".into()));
            let kt = if bkt.eq_ignore_ascii_case(&now_kt) { now_kt.clone() } else { format!("{bkt} → {now_kt}") };
            slot.append(&key_table(&[(tr("Before").to_string(), bfp, ""), (tr("Now").to_string(), now_fp, "perigo"), (tr("Key").to_string(), kt, "")]));
        });
        let (c, sw) = (close.clone(), self.shell.clone());
        known_btn.connect_clicked(move |_| {
            c();
            // Kalango Remote's known servers when it is installed; else the file itself, in the Editor
            if program("kalango-remote").is_some() {
                open_remote_ssh();
            } else if let (Some(s), Some(f)) = (sw.upgrade(), files.borrow().first().cloned()) {
                crate::editor::open(&s, Some(PathBuf::from(f)));
            }
        });
        crate::central::posicao_debug("terminal nao conectar", no.upcast_ref());
        if debug() {
            eprintln!("debug terminal: impressão digital MUDOU em {} ({key_type} {fingerprint})", r.target.id);
        }
    }

    /// The tab closed: its timers stop, its SFTP edits go, and the host's connection ends with its last tab.
    pub(super) fn forget_remote(&self, x: &Rc<Tab>) {
        let Some(r) = &x.remote else { return };
        r.closing.set(true);
        r.generation.set(r.generation.get() + 1);
        r.delete_keys();
        if let Some(p) = r.sftp.borrow_mut().take() {
            p.cleanup();
        }
        let others = instances().iter().any(|t| t.tabs.borrow().iter().any(|y| !Rc::ptr_eq(y, x) && y.remote.as_ref().is_some_and(|o| o.control == r.control && !o.closing.get())));
        if !others && Path::new(&r.control).exists() {
            detach(&absolute(r.target.control_exit_argv(&r.control)));
        }
    }

    /// The SFTP panel of an SSH tab (the toolbar's folder): in from the right, the terminal giving way.
    pub(super) fn toggle_files(self: &Rc<Self>, x: &Rc<Tab>) {
        let Some(r) = x.remote.clone() else { return };
        let existing = r.revealer.borrow().clone();
        let rev = match existing {
            Some(v) => v,
            None => {
                let rev = gtk::Revealer::new();
                rev.set_transition_type(gtk::RevealerTransitionType::SlideLeft);
                rev.set_transition_duration(220);
                rev.set_hexpand(false);
                let p = sftp::Panel::new(self, x);
                rev.set_child(Some(&p.root));
                x.side.append(&rev);
                *r.sftp.borrow_mut() = Some(p);
                *r.revealer.borrow_mut() = Some(rev.clone());
                rev
            }
        };
        let open = !rev.reveals_child();
        rev.set_reveal_child(open);
        if open {
            if let Some(p) = r.sftp.borrow().clone() {
                p.refresh();
            }
        }
        if debug() {
            eprintln!("debug terminal: sftp {}", if open { "aberto" } else { "fechado" });
        }
    }
}

impl Remote {
    /// The remote folder the terminal is in: the shell's own report (OSC 7), else the prompt ("user@host:~/app$").
    pub fn remote_dir_of(&self, term: &vte4::Terminal) -> Option<String> {
        if let Some(uri) = term.current_directory_uri() {
            if let Ok((p, _)) = glib::filename_from_uri(&uri) {
                return Some(p.display().to_string());
            }
        }
        let row = term.cursor_position().1;
        let (text, _) = term.text_range_format(vte4::Format::Text, row, 0, row, term.column_count());
        prompt_dir(text.as_deref().unwrap_or(""))
    }
}

impl Tab {
    pub(super) fn remote_dir(&self) -> Option<String> {
        self.remote.as_ref().and_then(|r| r.remote_dir_of(&self.term))
    }
}

/// The fingerprint dialogs' table: a label and a value in mono (accent or coral).
fn key_table(rows: &[(String, String, &str)]) -> gtk::Grid {
    let g = gtk::Grid::new();
    g.add_css_class("term-digital-tabela");
    g.set_column_spacing(24);
    g.set_row_spacing(8);
    for (i, (k, v, class)) in rows.iter().enumerate() {
        let a = rotulo(k, "term-digital-rotulo");
        a.set_xalign(0.0);
        g.attach(&a, 0, i as i32, 1, 1);
        let b = rotulo(v, "term-digital-valor");
        if !class.is_empty() {
            b.add_css_class(class);
        }
        b.set_xalign(0.0);
        b.set_selectable(true);
        b.set_wrap(true);
        b.set_wrap_mode(pango::WrapMode::Char);
        b.set_hexpand(true);
        g.attach(&b, 1, i as i32, 1, 1);
    }
    g
}
