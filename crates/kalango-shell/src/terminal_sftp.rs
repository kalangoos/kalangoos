//! The SFTP panel of an SSH tab (design "Kalango Remote · SSH abre no Terminal do sistema": "Arquivos (SFTP) ·
//! app-prod-01" at the right): the remote folder, "Seguir a pasta do terminal", the files with their sizes, "Solte
//! arquivos aqui para enviar", the transfer queue with its progress and "Tentar de novo", Baixar and Enviar; a remote
//! file opened in the Editor and changed there comes back as "config.yml mudou — Enviar?" (Ver diferenças, Descartar,
//! Enviar; the old one becomes config.yml.bak).
//!
//! It runs the system's `sftp` in batch mode, one run per operation, through the tab's ssh connection (its control
//! socket): no second login, no password here, and ssh's own host checks. Chosen over a Rust SFTP crate (russh-sftp)
//! because that would need its own authentication, agent, jump hosts and known_hosts — all of which ssh already does
//! for the session. Each run gets a pseudo-terminal, so sftp's progress meter ("48%") can be read for the queue.

use super::*;
use crate::ssh_hosts::Target;
use std::collections::VecDeque;
use std::time::Duration;

/// The largest remote file a double click opens in the Editor (a bigger one is downloaded).
const EDIT_MAX: u64 = 8 * 1024 * 1024;

/// A file or folder of the remote listing.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Entry {
    pub name: String,
    pub dir: bool,
    pub size: u64,
}

/// sftp's `ls -la` (the whole path at the end of each line), without "." and "..": folders first, then by name.
pub(super) fn parse_listing(out: &str) -> Vec<Entry> {
    let mut v: Vec<Entry> = out.lines().filter_map(|l| {
        let first = l.chars().next()?;
        if !"d-lcbps".contains(first) || l.starts_with("sftp>") {
            return None;
        }
        // perms, links, owner, group, size, month, day, time-or-year, path
        let mut rest = l;
        let mut fields = Vec::new();
        for _ in 0..8 {
            rest = rest.trim_start();
            let end = rest.find(char::is_whitespace)?;
            fields.push(&rest[..end]);
            rest = &rest[end..];
        }
        let path = rest.trim_start().trim_end_matches(['\r', '\n']);
        let name = path.rsplit('/').next().unwrap_or(path).to_string();
        if name.is_empty() || name == "." || name == ".." {
            return None;
        }
        Some(Entry { name, dir: first == 'd', size: fields[4].parse().unwrap_or(0) })
    }).collect();
    v.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    v
}

/// "Remote working directory: /home/deploy".
pub(super) fn parse_pwd(out: &str) -> Option<String> {
    out.lines().find_map(|l| l.trim().strip_prefix("Remote working directory: ").map(|p| p.trim().to_string()))
}

/// The last "NN%" of sftp's progress meter in a piece of its output.
pub(super) fn last_percent(s: &str) -> Option<u8> {
    let b = s.as_bytes();
    let mut i = s.rfind('%')?;
    loop {
        let mut j = i;
        while j > 0 && b[j - 1].is_ascii_digit() {
            j -= 1;
        }
        if j < i {
            return s[j..i].parse::<u8>().ok().filter(|n| *n <= 100);
        }
        i = s[..i].rfind('%')?;
    }
}

/// A path quoted for sftp's batch lines (its globs escaped: a file named "a*" is that file).
pub(super) fn sftp_quote(p: &str) -> String {
    let mut o = String::from("\"");
    for c in p.chars() {
        if matches!(c, '"' | '\\' | '*' | '?' | '[' | ']') {
            o.push('\\');
        }
        o.push(c);
    }
    o.push('"');
    o
}

/// `/a/b` + `c` → `/a/b/c`.
pub(super) fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') { format!("{dir}{name}") } else { format!("{dir}/{name}") }
}

/// `/a/b/c` → `/a/b` (the root stays).
pub(super) fn parent(dir: &str) -> String {
    let t = dir.trim_end_matches('/');
    match t.rfind('/') {
        Some(0) | None => "/".into(),
        Some(i) => t[..i].to_string(),
    }
}

/// A name that is not taken in `dir`: "api.log", else "api (1).log".
fn free_name(dir: &Path, name: &str) -> PathBuf {
    let p = dir.join(name);
    if !p.exists() {
        return p;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    (1..).map(|n| dir.join(format!("{stem} ({n}){ext}"))).find(|p| !p.exists()).unwrap_or(p)
}

/// Runs `sftp -b` with these lines, in a pseudo-terminal (for the progress meter): `progress` with each "NN%",
/// `done` with whether it worked and what it printed.
fn run_sftp(target: &Target, control: &str, lines: &[String], progress: impl Fn(u8) + 'static, done: impl FnOnce(bool, String) + 'static) {
    thread_local! {
        static N: Cell<u32> = const { Cell::new(0) };
    }
    let n = N.with(|c| {
        c.set(c.get() + 1);
        c.get()
    });
    let batch = glib::user_runtime_dir().join("kalango").join(format!("sftp-{}-{n}.txt", std::process::id()));
    let _ = std::fs::create_dir_all(batch.parent().unwrap_or(Path::new("/tmp")));
    if let Err(e) = std::fs::write(&batch, lines.join("\n") + "\n") {
        done(false, e.to_string());
        return;
    }
    let mut argv = target.sftp_argv(Some(control), &batch.display().to_string());
    match glib::find_program_in_path("sftp") {
        Some(p) => argv[0] = p.display().to_string(),
        None => {
            done(false, tr("sftp is not installed on this computer.").to_string());
            return;
        }
    }
    // (a bandwidth limit in Kbit/s, to watch the queue move in a test)
    if let Ok(l) = std::env::var("KALANGO_SFTP_LIMIT") {
        argv.splice(1..1, ["-l".to_string(), l]);
    }
    let pty = match vte4::Pty::new_sync(vte4::PtyFlags::DEFAULT, gio::Cancellable::NONE) {
        Ok(p) => p,
        Err(e) => {
            done(false, e.to_string());
            return;
        }
    };
    let _ = pty.set_size(24, 200);
    let fd = match pty.fd().try_clone_to_owned() {
        Ok(f) => f,
        Err(e) => {
            done(false, e.to_string());
            return;
        }
    };
    let out: Rc<RefCell<String>> = Rc::default();
    let reader = std::fs::File::from(fd);
    use std::os::fd::AsRawFd;
    let raw = reader.as_raw_fd();
    // (non-blocking: everything there is is read, at each wake-up and once more at the end)
    unsafe { glib::ffi::g_unix_set_fd_nonblocking(raw, glib::ffi::GTRUE, std::ptr::null_mut()) };
    let reader = RefCell::new(reader);
    let o2 = out.clone();
    // reads what is there; false at the end of the output
    let drain: Rc<dyn Fn() -> bool> = Rc::new(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.borrow_mut().read(&mut buf) {
                Ok(0) => return false,
                Ok(k) => {
                    let s = String::from_utf8_lossy(&buf[..k]).into_owned();
                    if let Some(p) = last_percent(&s) {
                        progress(p);
                    }
                    o2.borrow_mut().push_str(&s);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return true,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => return false,
            }
        }
    });
    let source: Rc<RefCell<Option<glib::SourceId>>> = Rc::default();
    let (s2, d2) = (source.clone(), drain.clone());
    let id = glib::unix_fd_add_local(raw, glib::IOCondition::IN | glib::IOCondition::HUP | glib::IOCondition::ERR, move |_, _| {
        if d2() {
            return glib::ControlFlow::Continue;
        }
        // (ended: the source goes by itself)
        s2.borrow_mut().take();
        glib::ControlFlow::Break
    });
    *source.borrow_mut() = Some(id);
    let argv_refs: Vec<&str> = argv.iter().map(String::as_str).collect();
    let keep = pty.clone();
    pty.spawn_async(None, &argv_refs, &[], glib::SpawnFlags::DO_NOT_REAP_CHILD, || {}, -1, gio::Cancellable::NONE, move |r| {
        match r {
            Ok(pid) => {
                let mut done = Some(done);
                glib::child_watch_add_local(pid, move |_, status| {
                    let _keep = &keep;
                    // (what is still in the terminal, then the end)
                    let (out, source, batch) = (out.clone(), source.clone(), batch.clone());
                    let done = done.take();
                    let drain = drain.clone();
                    glib::timeout_add_local_once(Duration::from_millis(60), move || {
                        drain();
                        if let Some(s) = source.borrow_mut().take() {
                            s.remove();
                        }
                        let _ = std::fs::remove_file(&batch);
                        let text = out.borrow().replace('\r', "\n");
                        if let Some(d) = done {
                            d(status == 0, text);
                        }
                    });
                });
            }
            Err(e) => {
                if let Some(s) = source.borrow_mut().take() {
                    s.remove();
                }
                done(false, e.to_string());
            }
        }
    });
}

/// Runs `sftp -b` with these lines and pipes (a listing: no progress to read, and every line of its output comes —
/// the pseudo-terminal can lose the last ones when sftp ends).
fn run_sftp_plain(target: &Target, control: &str, lines: &[String], done: impl FnOnce(bool, String) + 'static) {
    let batch = glib::user_runtime_dir().join("kalango").join(format!("sftp-{}-lista-{}.txt", std::process::id(), glib::random_int()));
    let _ = std::fs::create_dir_all(batch.parent().unwrap_or(Path::new("/tmp")));
    if let Err(e) = std::fs::write(&batch, lines.join("\n") + "\n") {
        done(false, e.to_string());
        return;
    }
    let mut argv = target.sftp_argv(Some(control), &batch.display().to_string());
    match glib::find_program_in_path("sftp") {
        Some(p) => argv[0] = p.display().to_string(),
        None => {
            done(false, tr("sftp is not installed on this computer.").to_string());
            return;
        }
    }
    let a: Vec<&std::ffi::OsStr> = argv.iter().map(std::ffi::OsStr::new).collect();
    let sp = match gio::Subprocess::newv(&a, gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_MERGE | gio::SubprocessFlags::STDIN_INHERIT) {
        Ok(p) => p,
        Err(e) => {
            let _ = std::fs::remove_file(&batch);
            done(false, e.to_string());
            return;
        }
    };
    let sp2 = sp.clone();
    sp.communicate_utf8_async(None, gio::Cancellable::NONE, move |r| {
        let _ = std::fs::remove_file(&batch);
        let out = r.ok().and_then(|(o, _)| o).map(|g| g.to_string()).unwrap_or_default();
        done(sp2.is_successful(), out);
    });
}

/// The last line of sftp's output that says what went wrong.
fn error_line(out: &str) -> String {
    out.lines().map(str::trim).rfind(|l| !l.is_empty() && !l.starts_with("sftp>") && last_percent(l).is_none() && !l.starts_with("Progress meter"))
        .unwrap_or("").to_string()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum JobKind {
    Download,
    Upload,
    /// to the editing folder, then into the Editor
    Edit,
    /// the edited file back (the old one to .bak)
    SendBack,
}

/// A transfer of the queue, and its row.
struct Job {
    kind: JobKind,
    remote: String,
    local: PathBuf,
    row: gtk::Box,
    text: gtk::Label,
    pct: gtk::Label,
    bar: gtk::ProgressBar,
    last: Cell<u8>,
    resume: Cell<bool>,
}

/// A remote file open in the Editor.
struct Edit {
    remote: String,
    local: PathBuf,
    /// as it came (and as sent last)
    original: RefCell<Vec<u8>>,
    /// a version already asked about (Descartar: not again until it changes)
    asked: RefCell<Vec<u8>>,
    _monitor: gio::FileMonitor,
}

pub(super) struct Panel {
    pub root: gtk::Box,
    t: Weak<Terminal>,
    x: Weak<Tab>,
    target: Target,
    control: String,
    path: RefCell<String>,
    home: RefCell<Option<String>>,
    path_label: gtk::Label,
    follow: gtk::Switch,
    list: gtk::ListBox,
    note: gtk::Label,
    entries: RefCell<Vec<Entry>>,
    change_slot: gtk::Box,
    queue_box: gtk::Box,
    queue: RefCell<VecDeque<Rc<Job>>>,
    busy: Cell<bool>,
    edits: RefCell<Vec<Rc<Edit>>>,
    edit_dir: PathBuf,
    listing: Cell<u32>,
    /// the folder being listed
    wanted: RefCell<String>,
}

impl Panel {
    pub fn new(t: &Rc<Terminal>, x: &Rc<Tab>) -> Rc<Panel> {
        let r = x.remote.clone().expect("an SSH tab");
        let root = caixa(Orientation::Vertical, 10, "term-sftp");
        root.set_size_request(344, -1);
        // (its own width: the labels inside that expand do not take the terminal's room)
        root.set_hexpand(false);
        // the title
        let head = caixa(Orientation::Horizontal, 8, "");
        head.append(&ui_icon("pasta", "#B9BAC4", 15));
        let title = rotulo(&trf("Files (SFTP) · {host}", &[("host", r.target.name.clone())]), "term-sftp-titulo");
        title.set_hexpand(true);
        title.set_xalign(0.0);
        title.set_ellipsize(pango::EllipsizeMode::End);
        head.append(&title);
        let close = gtk::Button::new();
        close.add_css_class("arq-dialogo-fechar");
        close.set_child(Some(&ui_icon("fechar", "#8E8F99", 11)));
        close.set_tooltip_text(Some(tr("Close")));
        head.append(&close);
        root.append(&head);
        // the folder (back to the one above)
        let up = gtk::Button::new();
        up.add_css_class("term-sftp-caminho");
        let ub = caixa(Orientation::Horizontal, 8, "");
        ub.append(&ui_icon("seta-esquerda", "#B9BAC4", 12));
        let path_label = rotulo("", "term-sftp-caminho-texto");
        path_label.set_ellipsize(pango::EllipsizeMode::Start);
        path_label.set_xalign(0.0);
        path_label.set_hexpand(true);
        ub.append(&path_label);
        up.set_child(Some(&ub));
        up.set_tooltip_text(Some(tr("The folder above")));
        root.append(&up);
        let fr = caixa(Orientation::Horizontal, 8, "");
        let fl = rotulo(tr("Follow the terminal's folder"), "term-sftp-seguir");
        fl.set_hexpand(true);
        fl.set_xalign(0.0);
        fr.append(&fl);
        let follow = gtk::Switch::new();
        follow.set_valign(Align::Center);
        follow.set_active(true);
        fr.append(&follow);
        root.append(&fr);
        // the files
        let list = gtk::ListBox::new();
        list.add_css_class("term-sftp-lista");
        list.set_selection_mode(gtk::SelectionMode::Single);
        // (a click selects; a double click opens the folder or edits the file)
        list.set_activate_on_single_click(false);
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
        scroll.set_vexpand(true);
        let lb = caixa(Orientation::Vertical, 0, "");
        lb.append(&list);
        let note = rotulo("", "term-sftp-nota");
        note.set_wrap(true);
        note.set_max_width_chars(1);
        note.set_xalign(0.0);
        note.set_visible(false);
        lb.append(&note);
        scroll.set_child(Some(&lb));
        root.append(&scroll);
        // a remote file changed in the Editor
        let change_slot = caixa(Orientation::Vertical, 0, "");
        root.append(&change_slot);
        // drop to send
        let drop = caixa(Orientation::Vertical, 6, "term-sftp-soltar");
        let di = ui_icon("acima", "#8E8F99", 14);
        drop.append(&di);
        drop.append(&rotulo(tr("Drop files here to send them"), "term-sftp-soltar-texto"));
        root.append(&drop);
        let queue_box = caixa(Orientation::Vertical, 8, "");
        root.append(&queue_box);
        let buttons = caixa(Orientation::Horizontal, 8, "");
        buttons.set_homogeneous(true);
        let get = gtk::Button::new();
        get.add_css_class("arq-acao");
        let gb = caixa(Orientation::Horizontal, 8, "");
        gb.set_halign(Align::Center);
        gb.append(&ui_icon("baixar", "#E2E2E6", 14));
        gb.append(&rotulo(tr("Download"), ""));
        get.set_child(Some(&gb));
        let send = gtk::Button::new();
        send.add_css_class("arq-acao");
        let sb = caixa(Orientation::Horizontal, 8, "");
        sb.set_halign(Align::Center);
        sb.append(&ui_icon("compartilhar", "#E2E2E6", 14));
        sb.append(&rotulo(tr("Send"), ""));
        send.set_child(Some(&sb));
        buttons.append(&get);
        buttons.append(&send);
        root.append(&buttons);
        let edit_dir = glib::user_runtime_dir().join("kalango").join(format!("sftp-{}", std::process::id())).join(&r.target.id);
        let p = Rc::new(Panel {
            root, t: Rc::downgrade(t), x: Rc::downgrade(x), target: r.target.clone(), control: r.control.clone(), path: RefCell::default(),
            home: RefCell::default(), path_label, follow, list, note, entries: RefCell::default(), change_slot, queue_box,
            queue: RefCell::default(), busy: Cell::new(false), edits: RefCell::default(), edit_dir, listing: Cell::new(0), wanted: RefCell::default(),
        });
        let (tw, xw) = (Rc::downgrade(t), Rc::downgrade(x));
        close.connect_clicked(move |_| {
            if let (Some(t), Some(x)) = (tw.upgrade(), xw.upgrade()) {
                t.toggle_files(&x);
            }
        });
        let w = Rc::downgrade(&p);
        up.connect_clicked(move |_| {
            if let Some(p) = w.upgrade() {
                let above = parent(&p.path.borrow());
                p.follow.set_active(false);
                p.list_dir(Some(above));
            }
        });
        let w = Rc::downgrade(&p);
        p.list.connect_row_activated(move |_, row| {
            let Some(p) = w.upgrade() else { return };
            let Some(e) = p.entries.borrow().get(row.index() as usize).cloned() else { return };
            let path = join(&p.path.borrow(), &e.name);
            if e.dir {
                p.follow.set_active(false);
                p.list_dir(Some(path));
            } else if e.size > EDIT_MAX {
                // (too big for the Editor: it is downloaded)
                p.download(&path);
            } else {
                p.edit(&path);
            }
        });
        let w = Rc::downgrade(&p);
        get.connect_clicked(move |_| {
            let Some(p) = w.upgrade() else { return };
            let Some(e) = p.selected().filter(|e| !e.dir) else { return };
            let path = join(&p.path.borrow(), &e.name);
            p.download(&path);
        });
        let w = Rc::downgrade(&p);
        send.connect_clicked(move |_| {
            let Some(p) = w.upgrade() else { return };
            // (the system's chooser, called directly: the shell serves the file chooser portal itself, and asking it
            // through GTK's FileDialog from inside the shell waits on itself — the chooser shows but takes no input)
            let w = Rc::downgrade(&p);
            crate::arquivos::seletor::abrir_arquivo(tr("Terminal"), &trf("Send to {host}", &[("host", p.target.name.clone())]), Some(tr("Send")), Vec::new(), None, move |path| {
                if let Some(p) = w.upgrade() {
                    p.upload(&path);
                }
            });
        });
        // files dropped on the panel
        let target = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
        let (w, dw) = (Rc::downgrade(&p), drop.downgrade());
        target.connect_drop(move |_, v, _, _| {
            let (Some(p), Ok(files)) = (w.upgrade(), v.get::<gdk::FileList>()) else { return false };
            for f in files.files() {
                if let Some(path) = f.path().filter(|x| x.is_file()) {
                    p.upload(&path);
                }
            }
            if let Some(d) = dw.upgrade() {
                d.remove_css_class("sobre");
            }
            true
        });
        let dw = drop.downgrade();
        target.connect_enter(move |_, _, _| {
            if let Some(d) = dw.upgrade() {
                d.add_css_class("sobre");
            }
            gdk::DragAction::COPY
        });
        let dw = drop.downgrade();
        target.connect_leave(move |_| {
            if let Some(d) = dw.upgrade() {
                d.remove_css_class("sobre");
            }
        });
        p.root.add_controller(target);
        crate::central::posicao_debug("terminal sftp baixar", get.upcast_ref());
        crate::central::posicao_debug("terminal sftp enviar", send.upcast_ref());
        crate::central::posicao_debug("terminal sftp acima", up.upcast_ref());
        crate::central::posicao_debug("terminal sftp seguir", p.follow.upcast_ref());
        crate::central::posicao_debug("terminal sftp lista", p.list.upcast_ref());
        p
    }

    fn selected(&self) -> Option<Entry> {
        let row = self.list.selected_row()?;
        self.entries.borrow().get(row.index() as usize).cloned()
    }

    /// The folder again (or the home, the first time).
    pub fn refresh(self: &Rc<Self>) {
        let path = self.path.borrow().clone();
        if path.is_empty() && self.follow.is_active() {
            // the first time: where the terminal is ("~/app" is "app" from sftp's home)
            if let Some(d) = self.x.upgrade().and_then(|x| x.remote_dir()) {
                let rel = d.strip_prefix('~').map(|r| r.trim_start_matches('/').to_string()).unwrap_or(d);
                self.list_dir(Some(if rel.is_empty() { ".".into() } else { rel }));
                return;
            }
        }
        self.list_dir((!path.is_empty()).then_some(path));
    }

    /// The terminal moved to `dir`: the panel follows (when "Seguir" is on).
    pub fn follow(self: &Rc<Self>, dir: &str) {
        if debug() {
            eprintln!("debug terminal: sftp seguir {dir} ({}, {})", self.follow.is_active(), self.root.is_mapped());
        }
        if !self.follow.is_active() || !self.root.is_mapped() {
            return;
        }
        let home = self.home.borrow().clone();
        let full = match (dir.strip_prefix('~'), home) {
            (Some(rest), Some(h)) => format!("{}{}", h.trim_end_matches('/'), rest),
            (Some(rest), None) => Some(rest.trim_start_matches('/')).filter(|r| !r.is_empty()).unwrap_or(".").to_string(),
            (None, _) => dir.to_string(),
        };
        // (the screen changes often: once per folder)
        if full != *self.path.borrow() && full != *self.wanted.borrow() {
            self.list_dir(Some(full));
        }
    }

    fn list_dir(self: &Rc<Self>, dir: Option<String>) {
        let n = self.listing.get() + 1;
        self.listing.set(n);
        *self.wanted.borrow_mut() = dir.clone().unwrap_or_default();
        self.note.set_text(tr("Loading…"));
        self.note.set_visible(self.entries.borrow().is_empty());
        let mut lines = vec!["pwd".to_string()];
        match &dir {
            Some(d) => lines.push(format!("ls -la {}", sftp_quote(d))),
            None => lines.push("ls -la .".into()),
        }
        let w = Rc::downgrade(self);
        run_sftp_plain(&self.target, &self.control, &lines, move |ok, out| {
            let Some(p) = w.upgrade() else { return };
            if p.listing.get() != n {
                return;
            }
            if !ok {
                let e = error_line(&out);
                p.note.set_text(&trf("Could not list the folder: {error}", &[("error", if e.is_empty() { tr("the connection is not up").to_string() } else { e })]));
                p.note.set_visible(true);
                if debug() {
                    eprintln!("debug terminal: sftp ls falhou: {}", out.trim());
                }
                return;
            }
            if p.home.borrow().is_none() {
                *p.home.borrow_mut() = parse_pwd(&out);
            }
            let home = p.home.borrow().clone();
            // (a relative folder is under sftp's home)
            let path = match dir.clone() {
                Some(d) if d == "." => home.unwrap_or_else(|| "/".into()),
                Some(d) if !d.starts_with('/') => join(&home.unwrap_or_else(|| "/".into()), &d),
                Some(d) => d,
                None => home.unwrap_or_else(|| "/".into()),
            };
            p.show(&path, parse_listing(&out));
        });
    }

    fn show(&self, path: &str, entries: Vec<Entry>) {
        *self.path.borrow_mut() = path.to_string();
        let home = self.home.borrow().clone().unwrap_or_default();
        let shown = match path.strip_prefix(home.as_str()) {
            Some(rest) if !home.is_empty() && (rest.is_empty() || rest.starts_with('/')) => format!("~{rest}"),
            _ => path.to_string(),
        };
        self.path_label.set_text(&shown);
        while let Some(c) = self.list.first_child() {
            self.list.remove(&c);
        }
        for e in &entries {
            let row = caixa(Orientation::Horizontal, 10, "term-sftp-item");
            row.append(&ui_icon(if e.dir { "pasta" } else { "doc" }, "#8E8F99", 14));
            let n = rotulo(&e.name, "term-sftp-nome");
            n.set_hexpand(true);
            n.set_xalign(0.0);
            n.set_ellipsize(pango::EllipsizeMode::Middle);
            row.append(&n);
            let s = rotulo(&if e.dir { "—".to_string() } else { crate::arquivos::tamanho(e.size) }, "term-sftp-tamanho");
            row.append(&s);
            self.list.append(&row);
        }
        self.note.set_visible(entries.is_empty());
        self.note.set_text(tr("This folder is empty."));
        if debug() {
            eprintln!("debug terminal: sftp {path}: {}", entries.iter().map(|e| if e.dir { format!("{}/", e.name) } else { format!("{} {}", e.name, e.size) }).collect::<Vec<_>>().join(", "));
        }
        *self.entries.borrow_mut() = entries;
    }

    pub fn download(self: &Rc<Self>, remote: &str) {
        let name = remote.rsplit('/').next().unwrap_or(remote).to_string();
        let dir = crate::pastas::downloads();
        let _ = std::fs::create_dir_all(&dir);
        let local = free_name(&dir, &name);
        let folder = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.enqueue(JobKind::Download, remote, local, &trf("{name} → {folder}", &[("name", name.clone()), ("folder", folder)]));
    }

    pub fn upload(self: &Rc<Self>, local: &Path) {
        let name = local.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let remote = join(&self.path.borrow(), &name);
        let folder = self.path_label.text().to_string();
        self.enqueue(JobKind::Upload, &remote, local.to_path_buf(), &trf("{name} → {folder}", &[("name", name.clone()), ("folder", folder)]));
    }

    /// A remote file to the Editor (a copy in the session's runtime folder, watched).
    pub fn edit(self: &Rc<Self>, remote: &str) {
        let name = remote.rsplit('/').next().unwrap_or(remote).to_string();
        // (each file in its own folder: two "config.yml" of two folders do not meet)
        let n = self.edits.borrow().len() + 1;
        let dir = self.edit_dir.join(n.to_string());
        let _ = std::fs::create_dir_all(&dir);
        let local = dir.join(&name);
        self.enqueue(JobKind::Edit, remote, local, &trf("{name} → Text Editor", &[("name", name.clone())]));
    }

    fn enqueue(self: &Rc<Self>, kind: JobKind, remote: &str, local: PathBuf, label: &str) {
        let row = caixa(Orientation::Vertical, 6, "term-sftp-transferencia");
        let top = caixa(Orientation::Horizontal, 8, "");
        top.append(&ui_icon(if kind == JobKind::Upload || kind == JobKind::SendBack { "acima" } else { "baixar" }, "#B9BAC4", 14));
        let text = rotulo(label, "term-sftp-transferencia-texto");
        text.set_hexpand(true);
        text.set_xalign(0.0);
        text.set_ellipsize(pango::EllipsizeMode::Middle);
        top.append(&text);
        let pct = rotulo(tr("waiting"), "term-sftp-pct");
        top.append(&pct);
        row.append(&top);
        let bar = gtk::ProgressBar::new();
        bar.add_css_class("term-sftp-barra");
        row.append(&bar);
        self.queue_box.append(&row);
        let job = Rc::new(Job { kind, remote: remote.to_string(), local, row, text, pct, bar, last: Cell::new(0), resume: Cell::new(false) });
        self.queue.borrow_mut().push_back(job);
        self.next();
    }

    /// The queue's next transfer (one at a time).
    fn next(self: &Rc<Self>) {
        if self.busy.get() {
            return;
        }
        let Some(job) = self.queue.borrow_mut().pop_front() else { return };
        self.busy.set(true);
        let (l, r) = (sftp_quote(&job.local.display().to_string()), sftp_quote(&job.remote));
        let resume = job.resume.get();
        let mut lines = vec!["progress".to_string()];
        match job.kind {
            JobKind::Download | JobKind::Edit => lines.push(format!("{} {r} {l}", if resume { "reget" } else { "get" })),
            JobKind::Upload => lines.push(format!("{} {l} {r}", if resume { "reput" } else { "put" })),
            JobKind::SendBack => {
                let bak = sftp_quote(&format!("{}.bak", job.remote));
                lines.push(format!("-rm {bak}"));
                lines.push(format!("rename {r} {bak}"));
                lines.push(format!("put {l} {r}"));
            }
        }
        job.pct.set_text("0%");
        if debug() {
            eprintln!("debug terminal: sftp {:?} {} ↔ {}", job.kind, job.remote, job.local.display());
        }
        let jw = Rc::downgrade(&job);
        let w = Rc::downgrade(self);
        run_sftp(&self.target, &self.control, &lines, move |n| {
            if let Some(j) = jw.upgrade() {
                j.last.set(n);
                j.pct.set_text(&format!("{n}%"));
                j.bar.set_fraction(n as f64 / 100.0);
            }
        }, move |ok, out| {
            let Some(p) = w.upgrade() else { return };
            p.busy.set(false);
            p.finished(&job, ok, &out);
            p.next();
        });
    }

    fn finished(self: &Rc<Self>, job: &Rc<Job>, ok: bool, out: &str) {
        if debug() {
            eprintln!("debug terminal: sftp {:?} {} {}", job.kind, job.remote, if ok { "ok".to_string() } else { format!("falhou em {}%: {}", job.last.get(), error_line(out)) });
        }
        if !ok {
            // "backup.tar.gz · conexão caiu em 71%" and "Tentar de novo" (it resumes where it stopped)
            job.row.add_css_class("falhou");
            let name = job.remote.rsplit('/').next().unwrap_or("").to_string();
            job.text.set_text(&if job.last.get() > 0 { trf("{name} · the connection dropped at {n}%", &[("name", name), ("n", job.last.get().to_string())]) } else { trf("{name} · {error}", &[("name", name), ("error", error_line(out))]) });
            job.text.set_wrap(true);
            job.text.set_ellipsize(pango::EllipsizeMode::None);
            job.text.set_max_width_chars(1);
            job.pct.set_visible(false);
            job.bar.set_visible(false);
            if let Some(first) = job.row.first_child().and_downcast::<gtk::Box>() {
                if let Some(icon) = first.first_child().and_downcast::<gtk::Image>() {
                    crate::conversa::set_icon(&icon, "alerta", "#EFA2A6");
                }
                let again = gtk::Button::with_label(tr("Try again"));
                again.add_css_class("term-sftp-de-novo");
                // (the button keeps the failed transfer: nothing else holds it now)
                let (w, j) = (Rc::downgrade(self), job.clone());
                again.connect_clicked(move |b| {
                    let Some(p) = w.upgrade() else { return };
                    let j = j.clone();
                    b.set_visible(false);
                    j.row.remove_css_class("falhou");
                    j.resume.set(j.kind != JobKind::SendBack && j.last.get() > 0);
                    j.pct.set_visible(true);
                    j.bar.set_visible(true);
                    j.pct.set_text(tr("waiting"));
                    p.queue.borrow_mut().push_back(j);
                    p.next();
                });
                crate::central::posicao_debug("terminal sftp de novo", again.upcast_ref());
                first.append(&again);
            }
            return;
        }
        job.pct.set_text("100%");
        job.bar.set_fraction(1.0);
        match job.kind {
            JobKind::Edit => self.watch(job),
            JobKind::Upload | JobKind::SendBack => self.refresh(),
            JobKind::Download => {}
        }
        // (done: it leaves the queue a moment later)
        let (qw, rw) = (self.queue_box.downgrade(), job.row.downgrade());
        glib::timeout_add_local_once(Duration::from_secs(5), move || {
            if let (Some(q), Some(r)) = (qw.upgrade(), rw.upgrade()) {
                if r.parent().is_some() {
                    q.remove(&r);
                }
            }
        });
    }

    /// The downloaded file into the Editor, and watched: a change there asks to send it back.
    fn watch(self: &Rc<Self>, job: &Rc<Job>) {
        let Ok(original) = std::fs::read(&job.local) else { return };
        let dir = job.local.parent().map(Path::to_path_buf).unwrap_or_default();
        let Ok(monitor) = gio::File::for_path(&dir).monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE) else { return };
        let edit = Rc::new(Edit { remote: job.remote.clone(), local: job.local.clone(), original: RefCell::new(original.clone()), asked: RefCell::new(original), _monitor: monitor.clone() });
        let (w, ew) = (Rc::downgrade(self), Rc::downgrade(&edit));
        let pending = Rc::new(Cell::new(false));
        monitor.connect_changed(move |_, f, other, _| {
            let Some(e) = ew.upgrade() else { return };
            let ours = |g: &gio::File| g.path().is_some_and(|p| p == e.local);
            if !ours(f) && !other.is_some_and(ours) {
                return;
            }
            // (an editor writes in steps: once it is quiet)
            if pending.replace(true) {
                return;
            }
            let (w, ew, pending) = (w.clone(), ew.clone(), pending.clone());
            glib::timeout_add_local_once(Duration::from_millis(300), move || {
                pending.set(false);
                if let (Some(p), Some(e)) = (w.upgrade(), ew.upgrade()) {
                    p.edited(&e);
                }
            });
        });
        self.edits.borrow_mut().push(edit);
        if let Some(s) = self.t.upgrade().and_then(|t| t.shell.upgrade()) {
            crate::editor::open(&s, Some(job.local.clone()));
        }
    }

    /// The Editor saved: if it is new, "config.yml mudou no Editor de Texto — Enviar?".
    fn edited(self: &Rc<Self>, e: &Rc<Edit>) {
        let Ok(now) = std::fs::read(&e.local) else { return };
        if now == *e.original.borrow() || now == *e.asked.borrow() {
            return;
        }
        *e.asked.borrow_mut() = now;
        while let Some(c) = self.change_slot.first_child() {
            self.change_slot.remove(&c);
        }
        let name = e.remote.rsplit('/').next().unwrap_or("").to_string();
        let card = caixa(Orientation::Vertical, 8, "term-sftp-mudou");
        let top = caixa(Orientation::Horizontal, 8, "");
        top.append(&ui_icon("doc", "#B9BAC4", 14));
        let t = rotulo(&trf("{name} changed in the Text Editor", &[("name", name.clone())]), "term-sftp-mudou-titulo");
        t.set_xalign(0.0);
        t.set_ellipsize(pango::EllipsizeMode::Middle);
        top.append(&t);
        card.append(&top);
        let body = rotulo(&trf("Opened from {path}. Send the new version to {host}? (the old one becomes {name}.bak)", &[("path", e.remote.clone()), ("host", self.target.name.clone()), ("name", name.clone())]), "term-sftp-mudou-texto");
        body.set_wrap(true);
        body.set_max_width_chars(1);
        body.set_xalign(0.0);
        card.append(&body);
        let buttons = caixa(Orientation::Horizontal, 6, "");
        let diff = gtk::Button::with_label(tr("See the differences"));
        diff.add_css_class("arq-acao-texto");
        diff.set_hexpand(true);
        diff.set_halign(Align::Start);
        let discard = gtk::Button::with_label(tr("Discard"));
        discard.add_css_class("arq-acao-texto");
        let send = gtk::Button::new();
        send.add_css_class("arq-acao-principal");
        let sb = caixa(Orientation::Horizontal, 6, "");
        sb.append(&ui_icon("check", "#1C1D24", 12));
        sb.append(&rotulo(tr("Send"), ""));
        send.set_child(Some(&sb));
        buttons.append(&diff);
        buttons.append(&discard);
        buttons.append(&send);
        card.append(&buttons);
        self.change_slot.append(&card);
        let (w, ew) = (Rc::downgrade(self), Rc::downgrade(e));
        send.connect_clicked(move |_| {
            let (Some(p), Some(e)) = (w.upgrade(), ew.upgrade()) else { return };
            p.clear_change();
            if let Ok(now) = std::fs::read(&e.local) {
                *e.original.borrow_mut() = now;
            }
            let name = e.remote.rsplit('/').next().unwrap_or("").to_string();
            p.enqueue(JobKind::SendBack, &e.remote, e.local.clone(), &trf("{name} → {host}", &[("name", name), ("host", p.target.name.clone())]));
        });
        let w = Rc::downgrade(self);
        discard.connect_clicked(move |_| {
            if let Some(p) = w.upgrade() {
                p.clear_change();
            }
        });
        let (w, ew) = (Rc::downgrade(self), Rc::downgrade(e));
        diff.connect_clicked(move |_| {
            if let (Some(p), Some(e)) = (w.upgrade(), ew.upgrade()) {
                p.show_diff(&e);
            }
        });
        crate::central::posicao_debug("terminal sftp mudou enviar", send.upcast_ref());
        crate::central::posicao_debug("terminal sftp mudou diferencas", diff.upcast_ref());
        crate::central::posicao_debug("terminal sftp mudou descartar", discard.upcast_ref());
        if debug() {
            eprintln!("debug terminal: sftp {} mudou no editor", e.remote);
        }
    }

    fn clear_change(&self) {
        while let Some(c) = self.change_slot.first_child() {
            self.change_slot.remove(&c);
        }
    }

    /// "Ver diferenças": `diff -u` of the remote one (as it came) and the edited one.
    fn show_diff(&self, e: &Rc<Edit>) {
        let Some(t) = self.t.upgrade() else { return };
        let old = e.local.with_extension("kalango-original");
        let _ = std::fs::write(&old, &*e.original.borrow());
        let out = std::process::Command::new("diff").arg("-u").arg("--label").arg(format!("{}:{}", self.target.name, e.remote)).arg("--label")
            .arg(tr("edited")).arg(&old).arg(&e.local).output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
        let _ = std::fs::remove_file(&old);
        let card = caixa(Orientation::Vertical, 12, "arq-dialogo");
        card.set_size_request(640, 420);
        let title = rotulo(&trf("Differences in {path}", &[("path", e.remote.clone())]), "arq-dialogo-titulo");
        title.set_xalign(0.0);
        title.set_ellipsize(pango::EllipsizeMode::Middle);
        title.set_max_width_chars(60);
        card.append(&title);
        let view = gtk::TextView::new();
        view.set_editable(false);
        view.set_monospace(true);
        view.add_css_class("term-sftp-diff");
        let buf = view.buffer();
        let (add, del) = (buf.create_tag(Some("mais"), &[("foreground", &"#9FD8A4")]), buf.create_tag(Some("menos"), &[("foreground", &"#EFA2A6")]));
        for line in out.lines() {
            let mut end = buf.end_iter();
            let tag = if line.starts_with('+') && !line.starts_with("+++") { add.as_ref() } else if line.starts_with('-') && !line.starts_with("---") { del.as_ref() } else { None };
            match tag {
                Some(tag) => buf.insert_with_tags(&mut end, &format!("{line}\n"), &[tag]),
                None => buf.insert(&mut end, &format!("{line}\n")),
            }
        }
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_vexpand(true);
        scroll.set_child(Some(&view));
        card.append(&scroll);
        let ok = gtk::Button::with_label(tr("Close"));
        ok.add_css_class("arq-acao");
        ok.set_halign(Align::End);
        card.append(&ok);
        let close = t.show_dialog(&card, &ok);
        ok.connect_clicked(move |_| close());
    }

    /// The tab closed: the edited copies go.
    pub fn cleanup(&self) {
        self.edits.borrow_mut().clear();
        let _ = std::fs::remove_dir_all(&self.edit_dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_sftp_output() {
        let out = "sftp> pwd\nRemote working directory: /home/deploy\nsftp> ls -la \"/var/log/app\"\n\
drwxr-xr-x    ? deploy   deploy       4096 Nov 19 22:03 /var/log/app/.\n\
drwxr-xr-x    ? root     root         4096 Nov 19 22:03 /var/log/app/..\n\
-rw-r--r--    ? deploy   deploy 2100000000 Nov 19 22:03 /var/log/app/api.log\n\
drwxr-xr-x    ? deploy   deploy       4096 Nov 19  2025 /var/log/app/arquivo antigo\n\
-rw-r--r--    ? deploy   deploy   88000000 Nov 19 22:03 /var/log/app/Worker.log\n";
        assert_eq!(parse_pwd(out), Some("/home/deploy".into()));
        let e = parse_listing(out);
        assert_eq!(e.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), vec!["arquivo antigo", "api.log", "Worker.log"]);
        assert!(e[0].dir && !e[1].dir);
        assert_eq!(e[1].size, 2_100_000_000);
        assert_eq!(last_percent("\rgrande.bin   43% 1275KB 279.4KB/s   00:05 ETA\rgrande.bin   52% 1530KB"), Some(52));
        assert_eq!(last_percent("nada aqui"), None);
        assert_eq!(last_percent("100% 2930KB"), Some(100));
    }

    #[test]
    fn paths_and_quoting() {
        assert_eq!(sftp_quote("/srv/a \"b\"*.yml"), "\"/srv/a \\\"b\\\"\\*.yml\"");
        assert_eq!(join("/srv/app", "x"), "/srv/app/x");
        assert_eq!(join("/", "x"), "/x");
        assert_eq!(parent("/srv/app/"), "/srv");
        assert_eq!(parent("/srv"), "/");
        assert_eq!(parent("/"), "/");
        let d = std::env::temp_dir().join(format!("kalango-sftp-teste-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("api.log"), "x").unwrap();
        assert_eq!(free_name(&d, "api.log"), d.join("api (1).log"));
        assert_eq!(free_name(&d, "novo.txt"), d.join("novo.txt"));
        std::fs::remove_dir_all(&d).unwrap();
    }
}
