//! KalangoOS's Terminal (design "Terminal", app id os.kalango.Terminal; `kalango-shell terminal [FOLDER]`), in place of
//! GNOME's. The emulator underneath is VTE (the cells, the PTY, selection, scrollback); the window around it is
//! KalangoOS's:
//! - **Tabs:** one shell each, named by the folder it is in or by the command at work ("htop"); "+", Ctrl+Shift+T and
//!   Ctrl+Shift+W, Ctrl+PageUp/PageDown. Closing one while a command runs asks first (design "Terminal · fechar com
//!   processo rodando").
//! - **The person's shell files are left alone:** the Terminal wraps them (zsh: its ZDOTDIR; bash: --rcfile) and adds
//!   two hooks that write when each command starts and how it ended — so the Terminal knows the folder, the command
//!   at work and its exit status without reading the screen.
//! - **The agent:** a failed command gets one hint from the local model under the prompt ("Explicar" and "Corrigir"
//!   send it to the agent's bar); "Pedir um comando" (Ctrl+Shift+K — Ctrl+K is the shell's) turns a sentence into a
//!   command line, put at the prompt for the person to read and run.
//! - **Status:** the shell, the encoding, the size in cells and the git branch of the folder.
//! - **Two instances:** the window (`kalango-shell terminal`) and the drop-down one (terminal_dropdown.rs, F12), each
//!   with its own tabs; both follow the Terminal's preferences (terminal_prefs.rs) live.
//! - **SSH and Mosh** (design "Kalango Remote · SSH abre no Terminal do sistema"; `kalango-shell terminal-ssh HOST`):
//!   a tab per host of Kalango Remote, with its state, auto-reconnect, the fingerprint question, production's guard and
//!   the SFTP panel (terminal_ssh.rs, terminal_sftp.rs); any tab splits in two, searches, and explains a selection
//!   (terminal_tools.rs).

use crate::conversa::ui_icon;
use crate::superficies::{caixa, rotulo};
use crate::Shell;
use gtk::prelude::*;
use gtk::{gdk, gio, glib, pango, Align, Orientation};
use kalango_i18n::{tr, trf};
use std::cell::{Cell, RefCell};
use std::ffi::{c_char, CString};
use std::io::{Read, Seek};
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::time::Instant;
use vte4::prelude::*;

#[path = "terminal_ssh.rs"]
mod ssh;
#[path = "terminal_sftp.rs"]
mod sftp;
#[path = "terminal_tools.rs"]
mod tools;

pub use ssh::open_host;

extern "C" {
    fn gdk_wayland_toplevel_set_application_id(toplevel: *mut gdk::ffi::GdkToplevel, id: *const c_char);
    fn kill(pid: i32, sig: i32) -> i32;
}

fn debug() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

/// The content's padding, as drawn (the first cell at 24, 24).
const PAD: i32 = 24;

/// The zsh the Terminal starts: the person's .zshenv, then back here for .zshrc — theirs, then the hooks.
const ZSHENV: &str = r#"# KalangoOS's Terminal: the person's own .zshenv, then back here for .zshrc (nothing of theirs is changed)
ZDOTDIR="${KALANGO_ZDOTDIR:-$HOME}"
[ -f "$ZDOTDIR/.zshenv" ] && . "$ZDOTDIR/.zshenv"
KALANGO_ZDOTDIR="$ZDOTDIR"
ZDOTDIR="$KALANGO_TERMINAL_DIR/zsh"
"#;
const ZSHRC: &str = r#"# KalangoOS's Terminal: the person's own .zshrc, then two hooks that tell the Terminal when each command starts and
# how it ended (its status and the folder)
ZDOTDIR="$KALANGO_ZDOTDIR"
if [ -f "$ZDOTDIR/.zshrc" ]; then . "$ZDOTDIR/.zshrc"; else
  # (a prompt like the design's, for someone with none of their own)
  PROMPT='%B%F{6}➜%f%b %F{5}%~%f '
fi
__kalango_preexec() { print -r -- "inicio	${1//$'\n'/ }" >> "$KALANGO_TERMINAL_ESTADO"; }
__kalango_precmd() { local s=$?; print -r -- "fim	$s	$PWD" >> "$KALANGO_TERMINAL_ESTADO"; }
preexec_functions=(__kalango_preexec $preexec_functions)
precmd_functions=(__kalango_precmd $precmd_functions)
"#;
const BASHRC: &str = r#"# KalangoOS's Terminal: the person's own .bashrc, then two hooks that tell the Terminal when each command starts and
# how it ended (its status and the folder)
if [ -f "$HOME/.bashrc" ]; then . "$HOME/.bashrc"; else
  PS1='\[\e[1;36m\]➜\[\e[0m\] \[\e[35m\]\w\[\e[0m\] '
fi
__kalango_precmd() { local s=$?; printf 'fim\t%s\t%s\n' "$s" "$PWD" >> "$KALANGO_TERMINAL_ESTADO"; }
__kalango_pronto() { __kalango_espera=1; }
__kalango_preexec() {
  [ -n "${__kalango_espera:-}" ] || return; [ -n "${COMP_LINE:-}" ] && return
  __kalango_espera=
  printf 'inicio\t%s\n' "$(HISTTIMEFORMAT= history 1 | sed 's/^ *[0-9]* *//')" >> "$KALANGO_TERMINAL_ESTADO"
}
PROMPT_COMMAND="__kalango_precmd${PROMPT_COMMAND:+; $PROMPT_COMMAND}; __kalango_pronto"
trap '__kalango_preexec' DEBUG
"#;

/// Where the wrappers and each tab's record of commands live (the session's runtime folder: gone at logout).
fn integration_dir() -> PathBuf {
    let d = glib::user_runtime_dir().join("kalango").join(format!("terminal-{}", std::process::id()));
    let _ = std::fs::create_dir_all(d.join("zsh"));
    let _ = std::fs::write(d.join("zsh/.zshenv"), ZSHENV);
    let _ = std::fs::write(d.join("zsh/.zshrc"), ZSHRC);
    let _ = std::fs::write(d.join("bashrc"), BASHRC);
    d
}

/// The person's shell: the one chosen in the preferences, else $SHELL, else bash.
fn user_shell() -> String {
    let chosen = crate::terminal_prefs::load().shell;
    if !chosen.is_empty() && Path::new(&chosen).is_file() {
        return chosen;
    }
    account_shell()
}

/// $SHELL (the account's), else bash.
pub fn account_shell() -> String {
    std::env::var("SHELL").ok().filter(|s| Path::new(s).is_file()).unwrap_or_else(|| "/bin/bash".into())
}

/// "~/KalangoOS" for a folder in the home, the path otherwise (long ones by their last part).
fn short_dir(p: &Path) -> String {
    let home = glib::home_dir();
    let s = match p.strip_prefix(&home) {
        Ok(r) if r.as_os_str().is_empty() => "~".to_string(),
        Ok(r) => format!("~/{}", r.display()),
        Err(_) => p.display().to_string(),
    };
    if s.chars().count() > 28 {
        format!("…/{}", p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
    } else {
        s
    }
}

/// The git branch of a folder (or of the one it is in), from .git/HEAD.
fn git_branch(dir: &Path) -> Option<String> {
    let mut d = Some(dir);
    while let Some(x) = d {
        let git = x.join(".git");
        let head = if git.is_dir() {
            std::fs::read_to_string(git.join("HEAD")).ok()
        } else if git.is_file() {
            // (a worktree: ".git" names the real folder)
            let real = std::fs::read_to_string(&git).ok()?.trim().strip_prefix("gitdir:")?.trim().to_string();
            std::fs::read_to_string(x.join(real).join("HEAD")).ok()
        } else {
            None
        };
        if let Some(h) = head {
            let h = h.trim();
            return Some(h.strip_prefix("ref: refs/heads/").map(str::to_string).unwrap_or_else(|| h.chars().take(7).collect()));
        }
        d = x.parent();
    }
    None
}

/// "há 40 s", "há 2 min", "há 1 h 05 min".
fn since(t: Instant) -> String {
    let s = t.elapsed().as_secs();
    if s < 60 {
        trf("running for {n} s", &[("n", s.to_string())])
    } else if s < 3600 {
        trf("running for {n} min", &[("n", (s / 60).to_string())])
    } else {
        trf("running for {h} h {m} min", &[("h", (s / 3600).to_string()), ("m", format!("{:02}", s % 3600 / 60))])
    }
}

/// The KalangoOS palette for the 16 ANSI colours (the prompt's ➜ is cyan = teal, the folder magenta = lavender).
const DARK_PALETTE: [&str; 16] = [
    "#16171B", "#EFA2A6", "#9FD8A4", "#E5C98C", "#8FB4EC", "#A5A1E2", "#8ED2C3", "#C9CAD3",
    "#5C5D6A", "#F4B8BB", "#B6E4BA", "#EED7A6", "#A9C6F2", "#BDB9EE", "#A6E0D3", "#E2E2E6",
];
/// On the light theme: the light tokens (dark ink on white), black being the text's colour.
const LIGHT_PALETTE: [&str; 16] = [
    "k_text", "k_coral", "k_accent_text", "k_gold", "k_blue", "k_purple", "k_teal", "k_muted",
    "k_dim", "k_red", "k_accent", "k_cream", "k_bt", "k_wifi", "k_teal", "k_text2",
];

/// The terminal's text, background and 16 colours: the dark ones, or the light theme's when the Terminal follows it.
fn colors(p: &crate::terminal_prefs::Prefs) -> (gdk::RGBA, gdk::RGBA, Vec<gdk::RGBA>) {
    let rgba = |c: &str| gdk::RGBA::parse(c).unwrap_or(gdk::RGBA::BLACK);
    if p.follow_theme && crate::tema::e_claro() {
        let light = |t: &str| crate::tema::token_claro(t).unwrap_or_else(|| "#322153".into());
        (rgba(&light("k_text")), rgba(&light("k_panel")), LIGHT_PALETTE.iter().map(|t| rgba(&light(t))).collect())
    } else {
        (rgba("#E2E2E6"), rgba("#101114"), DARK_PALETTE.iter().map(|c| rgba(c)).collect())
    }
}

/// A terminal as the preferences say: the font and its size, the colours, the cursor, the lines kept.
fn style_terminal(term: &vte4::Terminal, p: &crate::terminal_prefs::Prefs) {
    term.set_font(Some(&pango::FontDescription::from_string(&format!("JetBrains Mono {}", p.font_size))));
    let (fg, bg, pal) = colors(p);
    term.set_colors(Some(&fg), Some(&bg), &pal.iter().collect::<Vec<_>>());
    term.set_color_cursor(Some(&fg));
    term.set_cursor_shape(match p.cursor.as_str() {
        "bloco" => vte4::CursorShape::Block,
        "sublinhado" => vte4::CursorShape::Underline,
        _ => vte4::CursorShape::Ibeam,
    });
    term.set_cursor_blink_mode(if p.blink { vte4::CursorBlinkMode::On } else { vte4::CursorBlinkMode::Off });
    term.set_scrollback_lines(p.scrollback);
    // (the area around the cells: the terminal's background, also when it stays dark on the light theme)
    if p.follow_theme { term.remove_css_class("sempre-escuro") } else { term.add_css_class("sempre-escuro") }
}

/// Both Terminals (the window and the drop-down one), as they are now.
fn instances() -> Vec<Rc<Terminal>> {
    let mut v: Vec<Rc<Terminal>> = TERMINAL.with(|t| t.borrow().clone()).into_iter().collect();
    v.extend(crate::terminal_dropdown::terminal());
    v
}

/// The preferences changed (or the theme): every tab of both Terminals follows at once.
pub fn apply_prefs() {
    let p = crate::terminal_prefs::load();
    for t in instances() {
        for x in t.tabs.borrow().iter() {
            style_terminal(&x.term, &p);
            if let Some(s) = x.split.borrow().as_ref() {
                style_terminal(s, &p);
            }
        }
        t.update_status();
    }
    if debug() {
        eprintln!("debug terminal: preferências aplicadas (fonte {}, {} linhas, tema {})", p.font_size, p.scrollback,
            if p.follow_theme { "do sistema" } else { "escuro" });
    }
}

/// Which Terminal this is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// the app's window
    Window,
    /// the drop-down one (F12): a layer surface that terminal_dropdown.rs shows and hides
    Dropdown,
}

/// A shell in a tab (or an SSH or Mosh session: `remote`).
struct Tab {
    term: vte4::Terminal,
    /// the terminal (and its split), with the agent's hint, the search and the session's notices over it
    page: gtk::Overlay,
    /// what the tab shows: production's stripe, the page and the SFTP panel
    root: gtk::Box,
    /// the page's two panes (the second one when the tab is split)
    panes: gtk::Paned,
    split: RefCell<Option<vte4::Terminal>>,
    /// the pane the keyboard was in last (copy, paste, search and "Pedir um comando" go there)
    focused: RefCell<vte4::Terminal>,
    tab: gtk::Box,
    name: gtk::Label,
    remote: Option<Rc<ssh::Remote>>,
    /// the SFTP panel's place, at the right of the page
    side: gtk::Box,
    search: RefCell<Option<Rc<tools::Search>>>,
    /// the agent's command is on the line, not run yet (its card is up)
    agent_card: RefCell<Option<gtk::Box>>,
    /// what the shell's hooks write: each command's start, and how it ended
    record: PathBuf,
    read_at: Cell<u64>,
    monitor: RefCell<Option<gio::FileMonitor>>,
    cwd: RefCell<PathBuf>,
    /// the command at work (none: at the prompt), when it started and the row it started on
    running: RefCell<Option<(String, Instant, i64)>>,
    pid: Cell<i32>,
    shell_name: String,
    hint: RefCell<Option<gtk::Box>>,
    /// a command to run once the prompt is up (Files' "Abrir com → Terminal")
    pending: RefCell<Option<String>>,
    /// bumped by each command: a hint that comes back late for an older one is dropped
    generation: Cell<u32>,
}

impl Tab {
    fn title(&self) -> String {
        if let Some(r) = &self.remote {
            return r.target.name.clone();
        }
        match self.running.borrow().as_ref() {
            Some((cmd, _, _)) => {
                let first = cmd.split_whitespace().find(|w| !w.contains('=') && *w != "sudo").unwrap_or(cmd);
                Path::new(first).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| first.to_string())
            }
            None => short_dir(&self.cwd.borrow()),
        }
    }
    fn hide_hint(&self) {
        if let Some(h) = self.hint.borrow_mut().take() {
            self.page.remove_overlay(&h);
        }
    }
    fn hang_up(&self) {
        if self.pid.get() > 0 {
            unsafe { kill(self.pid.get(), 1) };
        }
        if let Some(s) = self.split.borrow().as_ref() {
            tools::hang_up_pane(s);
        }
        let _ = std::fs::remove_file(&self.record);
    }
    /// The pane in use.
    fn active(&self) -> vte4::Terminal {
        self.focused.borrow().clone()
    }
}

pub struct Terminal {
    pub window: gtk::ApplicationWindow,
    pub kind: Kind,
    /// the end of the toolbar: the window's buttons, or the drop-down's pin and expand
    pub bar_end: gtk::Box,
    tabs_box: gtk::Box,
    stack: gtk::Stack,
    tabs: RefCell<Vec<Rc<Tab>>>,
    current: Cell<usize>,
    ask: gtk::Entry,
    st_shell: gtk::Label,
    /// an SSH tab's host, jump, latency and key type
    st_host: gtk::Label,
    st_size: gtk::Label,
    st_branch: gtk::Label,
    st_hint: gtk::Label,
    shell: Weak<Shell>,
    pub over: gtk::Overlay,
    asking: RefCell<Option<Rc<dyn Fn()>>>,
    /// the toolbar's SFTP button (shown on an SSH tab)
    files_button: gtk::Button,
    dir: PathBuf,
    count: Cell<u32>,
    /// (the drop-down: the content keeps its size while the height animates, and lays its lines out once at the end)
    frozen: Cell<bool>,
    room: Cell<(i32, i32)>,
}

thread_local! {
    static TERMINAL: RefCell<Option<Rc<Terminal>>> = const { RefCell::new(None) };
}

/// The Terminal's shells and the folder each one is in (the System Monitor names a command by its tab's folder).
pub fn shells() -> Vec<(i32, PathBuf)> {
    instances().iter().flat_map(|t| t.tabs.borrow().iter().filter(|x| x.pid.get() > 0).map(|x| (x.pid.get(), x.cwd.borrow().clone())).collect::<Vec<_>>())
        .collect()
}

/// A new tab in `folder` that runs `command` once its prompt is up (as typed: it stays in the shell's history).
pub fn run(shell: &Rc<Shell>, folder: Option<PathBuf>, command: &str) {
    open(shell, folder.or_else(|| Some(glib::home_dir())));
    TERMINAL.with(|t| {
        if let Some(x) = t.borrow().as_ref().and_then(|t| t.tabs.borrow().last().cloned()) {
            *x.pending.borrow_mut() = Some(command.to_string());
        }
    });
}

/// The Terminal's window (made the first time; sized to the screen when it was closed).
fn window(shell: &Rc<Shell>) -> Rc<Terminal> {
    let t = TERMINAL.with(|t| t.borrow().clone()).unwrap_or_else(|| {
        let app = gio::Application::default().and_downcast::<gtk::Application>().expect("app");
        let t = Terminal::new(&app, shell, Kind::Window);
        TERMINAL.with(|x| *x.borrow_mut() = Some(t.clone()));
        t
    });
    // (closed and opened again: the screen may have changed meanwhile)
    if !t.window.is_visible() {
        let (w, h) = crate::superficies::tamanho_de_janela(1400, 880);
        t.window.set_default_size(w, h);
    }
    t
}

/// Opens the Terminal: a new tab in `folder`, or (none given) the window as it is — a first tab in the home.
pub fn open(shell: &Rc<Shell>, folder: Option<PathBuf>) {
    let t = window(shell);
    match folder {
        Some(f) => t.new_tab(Some(f)),
        None if t.tabs.borrow().is_empty() => t.new_tab(None),
        None => {}
    }
    t.window.present();
    if debug() {
        eprintln!("debug terminal: aberto ({} abas)", t.tabs.borrow().len());
    }
}

impl Terminal {
    pub fn new(app: &gtk::Application, shell: &Rc<Shell>, kind: Kind) -> Rc<Terminal> {
        let window = match kind {
            Kind::Window => {
                // the design's 1400 × 880; on a smaller screen almost all of it
                let (w, h) = crate::superficies::tamanho_de_janela(1400, 880);
                let window = gtk::ApplicationWindow::builder().application(app).title(tr("Terminal")).default_width(w).default_height(h).build();
                // (its app id as its name: the state written by `kalango-shell estado` finds it by it)
                window.set_widget_name("os.kalango.Terminal");
                // (closed for a while, the window lets go of its surface and renderer and keeps its widgets: see
                // superficies::release_resources_when_hidden — closing it hangs up its shells first, see connect_close_request)
                crate::superficies::release_resources_when_hidden(&window, || true);
                window.add_css_class("ed-janela");
                let sem_barra = gtk::Box::new(Orientation::Horizontal, 0);
                sem_barra.set_visible(false);
                window.set_titlebar(Some(&sem_barra));
                window
            }
            Kind::Dropdown => {
                let window = crate::superficies::camada(app, "kalango-terminal-suspenso", gtk4_layer_shell::Layer::Top, &[gtk4_layer_shell::Edge::Top]);
                window.add_css_class("term-suspenso-janela");
                window
            }
        };
        // the toolbar: the app, its tabs, "+", "Pedir um comando", the menu, the window's buttons (the drop-down's: pin
        // and expand)
        let bar = caixa(Orientation::Horizontal, 10, "ed-barra");
        if let Some(a) = shell.todos.iter().find(|a| a.id == "os.kalango.Terminal") {
            bar.append(&crate::apps::imagem(a, 24));
        }
        let tabs_box = caixa(Orientation::Horizontal, 4, "");
        tabs_box.set_valign(Align::Center);
        // (many tabs on a narrow screen: they scroll instead of widening the window)
        let tabs_scroll = gtk::ScrolledWindow::new();
        tabs_scroll.set_policy(gtk::PolicyType::External, gtk::PolicyType::Never);
        tabs_scroll.set_propagate_natural_width(true);
        tabs_scroll.set_valign(Align::Center);
        tabs_scroll.set_child(Some(&tabs_box));
        bar.append(&tabs_scroll);
        let new_tab = gtk::Button::new();
        new_tab.add_css_class("ed-nova-aba");
        new_tab.set_child(Some(&ui_icon("mais", "#8E8F99", 14)));
        new_tab.set_tooltip_text(Some(tr("New tab (Ctrl+Shift+T)")));
        new_tab.set_valign(Align::Center);
        bar.append(&new_tab);
        let gap = caixa(Orientation::Horizontal, 0, "");
        gap.set_hexpand(true);
        bar.append(&gap);
        let ask = gtk::Entry::new();
        ask.add_css_class("ed-pedir");
        ask.add_css_class("term-pedir");
        ask.set_size_request(300, 32);
        ask.set_valign(Align::Center);
        ask.set_placeholder_text(Some(tr("Ask for a command…")));
        ask.set_primary_icon_paintable(ui_icon("spark", "#A5A1E2", 13).paintable().as_ref());
        let ask_box = gtk::Overlay::new();
        ask_box.set_child(Some(&ask));
        let key = rotulo("Ctrl ⇧ K", "tb-atalho");
        key.set_halign(Align::End);
        key.set_valign(Align::Center);
        key.set_margin_end(8);
        key.set_can_target(false);
        ask_box.add_overlay(&key);
        bar.append(&ask_box);
        // split, and the SFTP panel (an SSH tab's)
        let split_button = gtk::Button::new();
        split_button.add_css_class("calc-menu");
        split_button.set_child(Some(&ui_icon("colunas", "#8E8F99", 16)));
        split_button.set_tooltip_text(Some(tr("Split (Ctrl+Shift+D)")));
        split_button.set_valign(Align::Center);
        bar.append(&split_button);
        let files_button = gtk::Button::new();
        files_button.add_css_class("calc-menu");
        files_button.set_child(Some(&ui_icon("pasta", "#8E8F99", 16)));
        files_button.set_tooltip_text(Some(tr("Files (SFTP)")));
        files_button.set_valign(Align::Center);
        files_button.set_visible(false);
        bar.append(&files_button);
        let menu = gtk::MenuButton::new();
        menu.add_css_class("calc-menu");
        menu.set_child(Some(&ui_icon("menu", "#8E8F99", 16)));
        menu.set_valign(Align::Center);
        let pop = gtk::Popover::new();
        pop.add_css_class("menu");
        let items = caixa(Orientation::Vertical, 2, "menu-lista");
        pop.set_child(Some(&items));
        menu.set_popover(Some(&pop));
        bar.append(&menu);
        let bar_end = caixa(Orientation::Horizontal, 10, "");
        bar_end.set_valign(Align::Center);
        bar.append(&bar_end);
        let top: gtk::Widget = match kind {
            Kind::Window => {
                bar_end.append(&crate::configuracoes::controles_janela());
                let handle = gtk::WindowHandle::new();
                handle.set_child(Some(&bar));
                handle.upcast()
            }
            Kind::Dropdown => bar.clone().upcast(),
        };
        let stack = gtk::Stack::new();
        stack.set_vexpand(true);
        stack.add_css_class("term-conteudo");
        // the drop-down's content sits in a frame that clips it: while the height animates, the tabs keep their size
        // (no reflow at every frame) and take the new one at the end
        let content: gtk::Widget = match kind {
            Kind::Window => stack.clone().upcast(),
            Kind::Dropdown => {
                let fixed = gtk::Fixed::new();
                fixed.put(&stack, 0.0, 0.0);
                let clip = gtk::ScrolledWindow::new();
                clip.set_policy(gtk::PolicyType::External, gtk::PolicyType::External);
                clip.add_css_class("term-conteudo");
                clip.set_child(Some(&fixed));
                clip.set_vexpand(true);
                clip.set_hexpand(true);
                let o = gtk::Overlay::new();
                o.set_child(Some(&clip));
                o.set_vexpand(true);
                o.upcast()
            }
        };
        // the status line: the shell, the encoding, the size in cells, the branch; and a word from the agent
        let status = caixa(Orientation::Horizontal, 14, "term-status");
        let st_shell = rotulo("", "term-status-texto");
        let st_host = rotulo("", "term-status-texto");
        st_host.set_visible(false);
        let st_enc = rotulo("UTF-8", "term-status-texto");
        let st_size = rotulo("", "term-status-texto");
        let st_branch = rotulo("", "term-status-texto");
        let st_hint = rotulo("", "term-status-dica");
        st_hint.set_hexpand(true);
        st_hint.set_xalign(1.0);
        st_hint.set_ellipsize(pango::EllipsizeMode::End);
        for l in [&st_shell, &st_host, &st_enc, &st_size, &st_branch, &st_hint] {
            l.set_valign(Align::Center);
            status.append(l);
        }
        let root = caixa(Orientation::Vertical, 0, "");
        root.append(&top);
        root.append(&content);
        root.append(&status);
        let over = gtk::Overlay::new();
        over.set_child(Some(&root));
        // (measures the window as it changes: "Pedir um comando" gives way on a narrow one)
        let meter = gtk::DrawingArea::new();
        meter.set_can_target(false);
        over.add_overlay(&meter);
        if kind == Kind::Window {
            window.set_child(Some(&over));
        }

        let t = Rc::new(Terminal {
            window, kind, bar_end, tabs_box, stack, tabs: RefCell::default(), current: Cell::new(0), ask, st_shell, st_host, st_size, st_branch, st_hint,
            files_button: files_button.clone(),
            shell: Rc::downgrade(shell), over, asking: RefCell::default(), dir: integration_dir(), count: Cell::new(0),
            frozen: Cell::new(false), room: Cell::new((0, 0)),
        });
        if let Ok(o) = content.clone().downcast::<gtk::Overlay>() {
            let room = gtk::DrawingArea::new();
            room.set_can_target(false);
            o.add_overlay(&room);
            let w = Rc::downgrade(&t);
            room.connect_resize(move |_, width, height| {
                let Some(t) = w.upgrade() else { return };
                t.room.set((width, height));
                if !t.frozen.get() {
                    // (not inside the allocation: the next frame)
                    let w = Rc::downgrade(&t);
                    glib::idle_add_local_once(move || {
                        if let Some(t) = w.upgrade().filter(|t| !t.frozen.get()) {
                            let (w, h) = t.room.get();
                            t.stack.set_size_request(w, h);
                        }
                    });
                }
            });
        }
        let w = Rc::downgrade(&t);
        meter.connect_resize(move |_, width, _| {
            if let Some(t) = w.upgrade() {
                // (the drop-down has two more buttons at the end and no window controls: about the same room)
                t.ask.set_size_request((width - 600).clamp(160, 300), 32);
                // (a very narrow one: "Pedir um comando" goes, the tabs keep the room)
                if let Some(b) = t.ask.parent() {
                    b.set_visible(width >= 560);
                }
            }
        });
        let w = Rc::downgrade(&t);
        new_tab.connect_clicked(move |_| {
            if let Some(t) = w.upgrade() {
                t.new_tab(None);
            }
        });
        let w = Rc::downgrade(&t);
        split_button.connect_clicked(move |_| {
            if let Some(t) = w.upgrade() {
                t.action("dividir");
            }
        });
        let w = Rc::downgrade(&t);
        files_button.connect_clicked(move |_| {
            if let Some(t) = w.upgrade() {
                t.action("arquivos");
            }
        });
        crate::central::posicao_debug(&format!("terminal dividir {}", if kind == Kind::Window { "janela" } else { "suspenso" }), split_button.upcast_ref());
        crate::central::posicao_debug(&format!("terminal sftp {}", if kind == Kind::Window { "janela" } else { "suspenso" }), files_button.upcast_ref());
        for (label, action) in [(tr("New tab"), "aba"), (tr("Copy"), "copiar"), (tr("Paste"), "colar"), (tr("Search"), "buscar"), (tr("Split"), "dividir"), (tr("Clear"), "limpar"), (tr("Preferences"), "preferencias")] {
            let b = gtk::Button::with_label(label);
            b.add_css_class("menu-item");
            let (w, p) = (Rc::downgrade(&t), pop.downgrade());
            b.connect_clicked(move |_| {
                if let Some(p) = p.upgrade() {
                    p.popdown();
                }
                if let Some(t) = w.upgrade() {
                    t.action(action);
                }
            });
            if action == "preferencias" {
                crate::central::posicao_debug(&format!("terminal preferencias {}", if kind == Kind::Window { "janela" } else { "suspenso" }), b.upcast_ref());
            }
            items.append(&b);
        }
        crate::central::posicao_debug(&format!("terminal menu {}", if kind == Kind::Window { "janela" } else { "suspenso" }), menu.upcast_ref());
        let w = Rc::downgrade(&t);
        t.ask.connect_activate(move |a| {
            if let Some(t) = w.upgrade() {
                let q = a.text().trim().to_string();
                if !q.is_empty() {
                    t.ask_for_command(&q);
                    a.set_text("");
                }
            }
        });
        crate::central::posicao_debug("terminal pedir", t.ask.upcast_ref());
        // the keys (Ctrl+Shift: the plain Ctrl ones belong to the shell and the programs in it)
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let w = Rc::downgrade(&t);
        keys.connect_key_pressed(move |_, k, _, m| {
            let Some(t) = w.upgrade() else { return glib::Propagation::Proceed };
            let asking = t.asking.borrow().clone();
            if let Some(close) = asking {
                if k == gdk::Key::Escape {
                    close();
                }
                return if k == gdk::Key::Escape || m.contains(gdk::ModifierType::CONTROL_MASK) { glib::Propagation::Stop } else { glib::Propagation::Proceed };
            }
            let ctrl = m.contains(gdk::ModifierType::CONTROL_MASK);
            let shift = m.contains(gdk::ModifierType::SHIFT_MASK);
            if !ctrl {
                return glib::Propagation::Proceed;
            }
            match (shift, k.to_lower()) {
                (true, gdk::Key::t) => t.new_tab(None),
                (true, gdk::Key::w) => t.close_tab(t.current.get()),
                (true, gdk::Key::c) => t.action("copiar"),
                (true, gdk::Key::v) => t.action("colar"),
                (true, gdk::Key::k) => {
                    t.ask.grab_focus();
                }
                (true, gdk::Key::f) => t.action("buscar"),
                (true, gdk::Key::d) => t.action("dividir"),
                (false, gdk::Key::Page_Down) => t.select((t.current.get() + 1) % t.tabs.borrow().len().max(1)),
                (false, gdk::Key::Page_Up) => {
                    let n = t.tabs.borrow().len().max(1);
                    t.select((t.current.get() + n - 1) % n);
                }
                (false, gdk::Key::plus | gdk::Key::equal | gdk::Key::KP_Add) => t.zoom(1.1),
                (false, gdk::Key::minus | gdk::Key::KP_Subtract) => t.zoom(1.0 / 1.1),
                (false, gdk::Key::_0 | gdk::Key::KP_0) => t.zoom(0.0),
                (false, gdk::Key::comma) => t.action("preferencias"),
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        t.window.add_controller(keys);
        if kind == Kind::Window {
            let app_id = |j: &gtk::ApplicationWindow| {
                if let Some(s) = j.surface().filter(|s| s.type_().name().contains("Wayland")) {
                    let id = CString::new("os.kalango.Terminal").unwrap();
                    unsafe { gdk_wayland_toplevel_set_application_id(s.as_ptr() as *mut gdk::ffi::GdkToplevel, id.as_ptr()) };
                }
            };
            t.window.connect_realize(move |j| app_id(j));
            t.window.connect_map(move |j| app_id(j));
        }
        // closing the window: a tab with a command at work asks first; then every shell is hung up
        let w = Rc::downgrade(&t);
        t.window.connect_close_request(move |win| {
            let Some(t) = w.upgrade() else { return glib::Propagation::Proceed };
            let busy = t.tabs.borrow().iter().position(|x| x.running.borrow().is_some());
            if let Some(i) = busy {
                t.select(i);
                t.ask_to_close(i, true);
                return glib::Propagation::Stop;
            }
            let gone: Vec<Rc<Tab>> = t.tabs.borrow_mut().drain(..).collect();
            for x in gone {
                x.hang_up();
                t.remove_tab_widget(&x);
                t.stack.remove(&x.root);
                t.forget_remote(&x);
            }
            win.set_visible(false);
            if debug() {
                eprintln!("debug terminal: fechado");
            }
            glib::Propagation::Stop
        });
        // the theme changes: the terminals follow (once for both instances)
        thread_local! {
            static FOLLOWING_THEME: Cell<bool> = const { Cell::new(false) };
        }
        if !FOLLOWING_THEME.with(|f| f.replace(true)) {
            crate::tema::ao_mudar(apply_prefs);
        }
        t
    }

    /// The drop-down's height animates: the content keeps the size it has (`true`) until it is let go (`false`).
    pub fn set_frozen(&self, frozen: bool) {
        self.frozen.set(frozen);
        if !frozen {
            let (w, h) = self.room.get();
            if w > 0 && h > 0 {
                self.stack.set_size_request(w, h);
            }
        }
    }

    pub fn has_tabs(&self) -> bool {
        !self.tabs.borrow().is_empty()
    }

    /// The keyboard to the tab in view (the drop-down, on its first frame).
    pub fn focus_current(&self) {
        if let Some(x) = self.tabs.borrow().get(self.current.get()) {
            x.term.grab_focus();
        }
    }

    pub fn open_tab(self: &Rc<Self>) {
        self.new_tab(None);
    }

    fn new_tab(self: &Rc<Self>, folder: Option<PathBuf>) {
        self.add_tab(folder, None);
    }

    /// A new tab: the person's shell in `folder`, or (`remote`) an SSH or Mosh session.
    fn add_tab(self: &Rc<Self>, folder: Option<PathBuf>, remote: Option<Rc<ssh::Remote>>) -> Rc<Tab> {
        // (a new tab opens where the one in view is)
        let dir = folder.filter(|d| d.is_dir())
            .or_else(|| self.tabs.borrow().get(self.current.get()).filter(|x| x.remote.is_none()).map(|x| x.cwd.borrow().clone()))
            .unwrap_or_else(glib::home_dir);
        let term = tools::new_vte();
        let page = gtk::Overlay::new();
        let panes = gtk::Paned::new(Orientation::Horizontal);
        panes.set_wide_handle(false);
        panes.add_css_class("term-divisao");
        panes.set_start_child(Some(&tools::scrolled(&term)));
        panes.set_resize_start_child(true);
        panes.set_shrink_start_child(false);
        page.set_child(Some(&panes));
        page.set_hexpand(true);
        let meter = gtk::DrawingArea::new();
        meter.set_can_target(false);
        page.add_overlay(&meter);
        // production's stripe over the page and the panel; the SFTP panel at the right
        let root = caixa(Orientation::Vertical, 0, "");
        if let Some(r) = remote.as_ref().filter(|r| r.target.production) {
            root.append(&ssh::production_stripe(&r.target));
        }
        let body = caixa(Orientation::Horizontal, 0, "");
        body.set_vexpand(true);
        body.append(&page);
        let side = caixa(Orientation::Horizontal, 0, "");
        side.set_hexpand(false);
        body.append(&side);
        root.append(&body);
        let n = self.count.get() + 1;
        self.count.set(n);
        let record = self.dir.join(format!("aba-{n}.tsv"));
        let _ = std::fs::write(&record, "");
        let shell = user_shell();
        let shell_name = match &remote {
            Some(r) => if r.target.mosh { "mosh".to_string() } else { "ssh".to_string() },
            None => Path::new(&shell).file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "sh".into()),
        };
        let tab = caixa(Orientation::Horizontal, 8, "ed-aba");
        if let Some(r) = &remote {
            tab.append(&r.dot);
        }
        let name = rotulo("", "ed-aba-nome");
        tab.append(&name);
        if let Some(r) = &remote {
            for b in r.badges() {
                tab.append(&b);
            }
        }
        let close = gtk::Button::new();
        close.add_css_class("ed-aba-fechar");
        close.set_child(Some(&ui_icon("fechar", "#7A7B88", 10)));
        close.set_tooltip_text(Some(tr("Close tab (Ctrl+Shift+W)")));
        tab.append(&close);
        let x = Rc::new(Tab {
            term: term.clone(), page: page.clone(), root: root.clone(), panes, split: RefCell::default(), focused: RefCell::new(term.clone()),
            tab: tab.clone(), name, remote: remote.clone(), side, search: RefCell::default(), agent_card: RefCell::default(),
            record: record.clone(), read_at: Cell::new(0),
            monitor: RefCell::default(), cwd: RefCell::new(dir.clone()), running: RefCell::default(), pid: Cell::new(0),
            shell_name: shell_name.clone(), hint: RefCell::default(), pending: RefCell::default(), generation: Cell::new(0),
        });
        let click = gtk::GestureClick::new();
        let (w, xw) = (Rc::downgrade(self), Rc::downgrade(&x));
        click.connect_released(move |_, _, _, _| {
            if let (Some(t), Some(x)) = (w.upgrade(), xw.upgrade()) {
                if let Some(i) = t.index_of(&x) {
                    t.select(i);
                }
            }
        });
        tab.add_controller(click);
        let (w, xw) = (Rc::downgrade(self), Rc::downgrade(&x));
        close.connect_clicked(move |_| {
            if let (Some(t), Some(x)) = (w.upgrade(), xw.upgrade()) {
                if let Some(i) = t.index_of(&x) {
                    t.close_tab(i);
                }
            }
        });
        // what the hooks write (a local shell's)
        if remote.is_none() {
            if let Ok(m) = gio::File::for_path(&record).monitor_file(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE) {
                let (w, xw) = (Rc::downgrade(self), Rc::downgrade(&x));
                m.connect_changed(move |_, _, _, ev| {
                    if matches!(ev, gio::FileMonitorEvent::Changed | gio::FileMonitorEvent::ChangesDoneHint) {
                        if let (Some(t), Some(x)) = (w.upgrade(), xw.upgrade()) {
                            t.read_record(&x);
                        }
                    }
                });
                *x.monitor.borrow_mut() = Some(m);
            }
        }
        // the selection's menu, the keys of the agent's card and production's guard, the pane in use
        self.wire_pane(&x, &term);
        // typing at the prompt: the hint has been read
        let xw = Rc::downgrade(&x);
        term.connect_commit(move |_, _, _| {
            if let Some(x) = xw.upgrade() {
                x.hide_hint();
            }
        });
        // the shell ended ("exit"): its tab goes; a session that dropped reconnects
        let (w, xw) = (Rc::downgrade(self), Rc::downgrade(&x));
        term.connect_child_exited(move |_, status| {
            if let (Some(t), Some(x)) = (w.upgrade(), xw.upgrade()) {
                x.pid.set(0);
                if x.remote.is_some() {
                    t.remote_exited(&x, status);
                } else if let Some(i) = t.index_of(&x) {
                    t.close_tab_now(i);
                }
            }
        });
        let w = Rc::downgrade(self);
        meter.connect_resize(move |_, _, _| {
            if let Some(t) = w.upgrade() {
                let w = Rc::downgrade(&t);
                glib::idle_add_local_once(move || {
                    if let Some(t) = w.upgrade() {
                        t.update_status();
                    }
                });
            }
        });
        // (design: the new tab grows in from its left edge — the quick spring)
        let grow = kalango_movimento::Animavel::new(&tab);
        grow.origem(0.0, 0.5);
        self.tabs_box.append(&grow);
        if !kalango_movimento::reduzido() {
            grow.definir(kalango_movimento::Canal::Escala, 0.2).definir(kalango_movimento::Canal::Opacidade, 0.0);
            grow.ir(kalango_movimento::Canal::Escala, 1.0, kalango_movimento::RAPIDA).ir(kalango_movimento::Canal::Opacidade, 1.0, kalango_movimento::RAPIDA);
        }
        self.stack.add_child(&root);
        self.tabs.borrow_mut().push(x.clone());
        let i = self.tabs.borrow().len() - 1;
        self.select(i);
        if remote.is_some() {
            self.spawn_remote(&x);
            return x;
        }
        let env: Vec<String> = vec![
            format!("KALANGO_TERMINAL_ESTADO={}", record.display()),
            format!("KALANGO_TERMINAL_DIR={}", self.dir.display()),
            format!("KALANGO_ZDOTDIR={}", std::env::var("ZDOTDIR").unwrap_or_default()),
            format!("ZDOTDIR={}", self.dir.join("zsh").display()),
            "COLORTERM=truecolor".to_string(),
            "TERM_PROGRAM=KalangoOS-Terminal".to_string(),
        ];
        let env_refs: Vec<&str> = env.iter().map(String::as_str).collect();
        let rc = self.dir.join("bashrc").display().to_string();
        let argv: Vec<String> = if shell_name == "bash" { vec![shell.clone(), "--rcfile".into(), rc] } else { vec![shell.clone()] };
        let argv_refs: Vec<&str> = argv.iter().map(String::as_str).collect();
        let (w, xw) = (Rc::downgrade(self), Rc::downgrade(&x));
        term.spawn_async(vte4::PtyFlags::DEFAULT, Some(&dir.display().to_string()), &argv_refs, &env_refs, glib::SpawnFlags::DEFAULT, || {}, -1,
            gio::Cancellable::NONE, move |r| {
                let Some(x) = xw.upgrade() else { return };
                match r {
                    Ok(pid) => {
                        x.pid.set(pid.0);
                        if debug() {
                            eprintln!("debug terminal: aba {n} em {} ({})", x.cwd.borrow().display(), x.shell_name);
                        }
                    }
                    Err(e) => {
                        eprintln!("kalango-shell: terminal: {}: {e}", x.shell_name);
                        if let Some(t) = w.upgrade() {
                            t.st_hint.set_text(&trf("Could not start {shell}", &[("shell", x.shell_name.clone())]));
                        }
                    }
                }
            });
        x
    }

    /// A tab's widget out of the bar (with the wrapper that animates it in).
    fn remove_tab_widget(&self, x: &Rc<Tab>) {
        if let Some(w) = x.tab.parent().filter(|p| p.parent().as_ref() == Some(self.tabs_box.upcast_ref())) {
            self.tabs_box.remove(&w);
        } else if x.tab.parent().as_ref() == Some(self.tabs_box.upcast_ref()) {
            self.tabs_box.remove(&x.tab);
        }
    }

    fn index_of(&self, x: &Rc<Tab>) -> Option<usize> {
        self.tabs.borrow().iter().position(|y| Rc::ptr_eq(y, x))
    }

    fn select(self: &Rc<Self>, i: usize) {
        let Some(x) = self.tabs.borrow().get(i).cloned() else { return };
        self.current.set(i);
        self.stack.set_visible_child(&x.root);
        for (n, y) in self.tabs.borrow().iter().enumerate() {
            if n == i { y.tab.add_css_class("ativa") } else { y.tab.remove_css_class("ativa") }
        }
        self.files_button.set_visible(x.remote.is_some());
        self.update_titles();
        self.update_status();
        x.active().grab_focus();
    }

    fn update_titles(&self) {
        for x in self.tabs.borrow().iter() {
            x.name.set_text(&x.title());
        }
        let title = self.tabs.borrow().get(self.current.get()).map(|x| x.title()).unwrap_or_default();
        self.window.set_title(Some(&format!("{title} — {}", tr("Terminal"))));
    }

    fn update_status(&self) {
        let Some(x) = self.tabs.borrow().get(self.current.get()).cloned() else { return };
        self.st_shell.set_text(&x.shell_name);
        let term = x.active();
        self.st_size.set_text(&format!("{}×{}", term.column_count(), term.row_count()));
        if let Some(r) = &x.remote {
            // "ssh  deploy@app-prod-01  via bastiao  23 ms  ED25519  UTF-8  96×34  1 porta encaminhada"
            self.st_shell.set_text(if r.running_mosh() { "mosh" } else { "ssh" });
            self.st_host.set_visible(true);
            self.st_host.set_text(&r.status_text());
            let forwards = r.target.forwards.len();
            self.st_branch.set_visible(forwards > 0);
            self.st_branch.set_text(&if forwards == 1 { tr("1 port forwarded").to_string() } else { trf("{n} ports forwarded", &[("n", forwards.to_string())]) });
            if debug() {
                eprintln!("debug terminal: status {} {} UTF-8 {} {}", self.st_shell.text(), self.st_host.text(), self.st_size.text(), self.st_branch.text());
            }
            return;
        }
        self.st_host.set_visible(false);
        let branch = git_branch(&x.cwd.borrow());
        self.st_branch.set_visible(branch.is_some());
        self.st_branch.set_text(&branch.as_deref().map(|b| trf("branch: {name}", &[("name", b.to_string())])).unwrap_or_default());
        if debug() {
            eprintln!("debug terminal: status {} UTF-8 {} {}", self.st_shell.text(), self.st_size.text(), branch.as_deref().unwrap_or("-"));
        }
    }

    /// The new lines the shell's hooks wrote: a command started, or one ended (its status, the folder).
    fn read_record(self: &Rc<Self>, x: &Rc<Tab>) {
        let Ok(mut f) = std::fs::File::open(&x.record) else { return };
        if f.seek(std::io::SeekFrom::Start(x.read_at.get())).is_err() {
            return;
        }
        let mut new = String::new();
        if f.read_to_string(&mut new).is_err() {
            return;
        }
        // (a line still being written waits for the next change)
        let Some(end) = new.rfind('\n') else { return };
        x.read_at.set(x.read_at.get() + end as u64 + 1);
        for line in new[..end].lines() {
            let mut p = line.splitn(3, '\t');
            match (p.next(), p.next(), p.next()) {
                (Some("inicio"), Some(cmd), _) => {
                    x.hide_hint();
                    x.generation.set(x.generation.get() + 1);
                    let row = x.term.cursor_position().1 as i64;
                    *x.running.borrow_mut() = Some((cmd.trim().to_string(), Instant::now(), row));
                    if self.tabs.borrow().get(self.current.get()).is_some_and(|c| Rc::ptr_eq(c, x)) {
                        self.st_hint.set_text("");
                    }
                    if debug() {
                        eprintln!("debug terminal: «{}» começou", cmd.trim());
                    }
                }
                (Some("fim"), Some(status), Some(pwd)) => {
                    *x.cwd.borrow_mut() = PathBuf::from(pwd);
                    // (the prompt is up: a command waiting for it goes in)
                    let pending = x.pending.borrow_mut().take();
                    if let Some(c) = pending {
                        x.term.feed_child(format!("{c}\n").as_bytes());
                    }
                    let status: i32 = status.trim().parse().unwrap_or(0);
                    if let Some((cmd, _, row)) = x.running.borrow_mut().take() {
                        if debug() {
                            eprintln!("debug terminal: «{cmd}» terminou com {status} em {pwd}");
                        }
                        // (130 and 148: the person's own Ctrl+C and Ctrl+Z)
                        if status != 0 && status != 130 && status != 148 {
                            self.on_failure(x, cmd, status, row);
                        }
                    }
                }
                _ => {}
            }
        }
        self.update_titles();
        self.update_status();
    }

    /// A command failed: its output goes to the local model, and its hint comes under the prompt.
    fn on_failure(self: &Rc<Self>, x: &Rc<Tab>, cmd: String, status: i32, row: i64) {
        // (Agents › Settings › General › Suggestions while you work)
        if !crate::agent_prefs::load().suggestions {
            if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                eprintln!("debug terminal: «{cmd}» falhou; sugestões desligadas nos ajustes dos agentes");
            }
            return;
        }
        let end = x.term.cursor_position().1 as i64;
        let (text, _) = x.term.text_range_format(vte4::Format::Text, row as _, 0, end as _, x.term.column_count());
        let lines: Vec<&str> = text.as_deref().unwrap_or("").lines().filter(|l| !l.trim().is_empty()).collect();
        let output = lines[lines.len().saturating_sub(40)..].join("\n");
        let pasta = x.cwd.borrow().display().to_string();
        let generation = x.generation.get();
        let bin = crate::voz::agent_binary();
        let input = serde_json::json!({"comando": cmd, "status": status, "saida": output, "pasta": pasta}).to_string();
        let (w, xw) = (Rc::downgrade(self), Rc::downgrade(x));
        crate::lateral::em_segundo_plano(move || {
            use std::io::Write;
            let mut c = std::process::Command::new(&bin).arg("erro-terminal").stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn().ok()?;
            c.stdin.take()?.write_all(input.as_bytes()).ok()?;
            let o = c.wait_with_output().ok()?;
            let v: serde_json::Value = serde_json::from_slice(&o.stdout).ok()?;
            let dica = v["dica"].as_str()?.trim().to_string();
            (!dica.is_empty()).then(|| (dica, v["corrigivel"].as_bool().unwrap_or(false)))
        }, move |r: Option<(String, bool)>| {
            let (Some(t), Some(x)) = (w.upgrade(), xw.upgrade()) else { return };
            // (another command since, or one at work: this hint is late)
            if x.generation.get() != generation || x.running.borrow().is_some() {
                return;
            }
            if let Some((dica, corrigivel)) = r {
                if debug() {
                    eprintln!("debug terminal: dica «{dica}» ({})", if corrigivel { "corrigível" } else { "não corrigível" });
                }
                t.show_hint(&x, &dica, corrigivel, &cmd, &output, &pasta);
            }
        });
    }

    /// The agent's hint under the prompt (design "Sugestão do agente"): "Explicar", and "Corrigir" when the fix is in the
    /// folder's files — both send the error to the agent's bar.
    fn show_hint(self: &Rc<Self>, x: &Rc<Tab>, dica: &str, corrigivel: bool, cmd: &str, output: &str, pasta: &str) {
        x.hide_hint();
        let card = caixa(Orientation::Horizontal, 10, "term-dica");
        card.set_halign(Align::Start);
        card.set_valign(Align::Start);
        card.append(&ui_icon("spark", "#A5A1E2", 13));
        let text = if corrigivel { format!("{dica} {}", tr("Fix it automatically?")) } else { dica.to_string() };
        let l = rotulo(&text, "term-dica-texto");
        l.set_wrap(true);
        l.set_max_width_chars(70);
        l.set_xalign(0.0);
        card.append(&l);
        let explain = gtk::Button::with_label(tr("Explain"));
        explain.add_css_class("term-dica-explicar");
        explain.set_valign(Align::Center);
        card.append(&explain);
        let fix = gtk::Button::with_label(tr("Fix"));
        fix.add_css_class("term-dica-corrigir");
        fix.set_valign(Align::Center);
        fix.set_visible(corrigivel);
        card.append(&fix);
        // under the prompt's row (above it when the prompt is at the bottom)
        let cell = x.term.char_height().max(1) as i32;
        let top = x.term.vadjustment().map(|a| a.value()).unwrap_or(0.0) as i64;
        let row = x.term.cursor_position().1 as i64 - top;
        let below = PAD + (row as i32 + 1) * cell + 16;
        let room = x.page.height();
        card.set_margin_start(PAD);
        card.set_margin_top(if room > 0 && below + 56 > room { (PAD + row as i32 * cell - 56).max(8) } else { below });
        x.page.add_overlay(&card);
        *x.hint.borrow_mut() = Some(card);
        for (b, what, key) in [(&explain, "explicar", "terminal explicar"), (&fix, "corrigir", "terminal corrigir")] {
            crate::central::posicao_debug(key, b.upcast_ref());
            let (w, xw) = (Rc::downgrade(self), Rc::downgrade(x));
            let (cmd, output, pasta) = (cmd.to_string(), output.to_string(), pasta.to_string());
            b.connect_clicked(move |_| {
                let (Some(t), Some(x)) = (w.upgrade(), xw.upgrade()) else { return };
                x.hide_hint();
                let Some(s) = t.shell.upgrade() else { return };
                let pedido = if what == "explicar" {
                    format!("{}\n$ {cmd}\n{output}", trf("Explain this error from the terminal (in {folder}):", &[("folder", pasta.clone())]))
                } else {
                    format!("{}\n{output}", trf("Fix this error in the project in {folder}. The command `{command}` failed with:", &[("folder", pasta.clone()), ("command", cmd.clone())]))
                };
                s.right_bar().ask_agent(&pedido);
                if debug() {
                    eprintln!("debug terminal: ao agente ({what})");
                }
            });
        }
    }

    /// "Pedir um comando": the local model writes the command line; it goes to the prompt, not run — the person reads
    /// it and presses Enter (Esc erases it). On a production host, or for a command that erases or restarts something,
    /// the person approves it first (terminal_tools.rs). The model gets the request and where it runs, never what is on
    /// the screen.
    fn ask_for_command(self: &Rc<Self>, request: &str) {
        let Some(x) = self.tabs.borrow().get(self.current.get()).cloned() else { return };
        self.st_hint.set_text(tr("The agent is writing the command…"));
        let bin = crate::voz::agent_binary();
        let (folder, shell) = match &x.remote {
            Some(r) => (trf("{folder} on the server {host}", &[("folder", x.remote_dir().unwrap_or_else(|| "~".into())), ("host", r.target.label())]), format!("bash ({})", x.shell_name)),
            None => (x.cwd.borrow().display().to_string(), x.shell_name.clone()),
        };
        let input = serde_json::json!({"pedido": request, "pasta": folder, "shell": shell}).to_string();
        let (w, xw, req) = (Rc::downgrade(self), Rc::downgrade(&x), request.to_string());
        crate::lateral::em_segundo_plano(move || {
            use std::io::Write;
            let mut c = std::process::Command::new(&bin).arg("comando").stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn().ok()?;
            c.stdin.take()?.write_all(input.as_bytes()).ok()?;
            let o = c.wait_with_output().ok()?;
            let v: serde_json::Value = serde_json::from_slice(&o.stdout).ok()?;
            let cmd = v["comando"].as_str()?.trim().to_string();
            (!cmd.is_empty()).then(|| (cmd, v["explicacao"].as_str().unwrap_or("").trim().to_string()))
        }, move |r: Option<(String, String)>| {
            let (Some(t), Some(x)) = (w.upgrade(), xw.upgrade()) else { return };
            match r {
                Some((cmd, why)) => {
                    if debug() {
                        eprintln!("debug terminal: pedido «{req}» → «{cmd}»");
                    }
                    let production = x.remote.as_ref().is_some_and(|r| r.target.production);
                    if production || crate::ssh_hosts::is_destructive(&cmd) {
                        t.st_hint.set_text("");
                        t.approve_agent_command(&x, cmd, why);
                    } else {
                        t.put_agent_command(&x, &cmd, &why);
                    }
                }
                None => t.st_hint.set_text(tr("The agent could not write that command.")),
            }
        });
    }

    /// The agent's command on the line, waiting for Enter, with its card under the prompt.
    fn put_agent_command(self: &Rc<Self>, x: &Rc<Tab>, cmd: &str, why: &str) {
        let term = x.active();
        // (the line as typed so far goes, the command comes as a paste: it waits for Enter)
        term.feed_child(b"\x15");
        term.paste_text(cmd);
        term.grab_focus();
        self.st_hint.set_text(&if why.is_empty() { tr("Enter runs it").to_string() } else { trf("{what} — Enter runs it", &[("what", why.to_string())]) });
        let (w, xw) = (Rc::downgrade(self), Rc::downgrade(x));
        // (after the paste has reached the screen: the card goes under the prompt's row)
        let cmd = cmd.to_string();
        glib::timeout_add_local_once(std::time::Duration::from_millis(120), move || {
            if let (Some(t), Some(x)) = (w.upgrade(), xw.upgrade()) {
                t.show_agent_card(&x, &cmd);
            }
        });
    }

    fn action(self: &Rc<Self>, a: &str) {
        if a == "preferencias" {
            if let Some(s) = self.shell.upgrade() {
                crate::terminal_prefs::open(&s, if self.kind == Kind::Dropdown { "suspenso" } else { "geral" });
            }
            return;
        }
        let Some(x) = self.tabs.borrow().get(self.current.get()).cloned() else { return };
        let term = x.active();
        match a {
            "aba" => self.new_tab(None),
            "copiar" => term.copy_clipboard_format(vte4::Format::Text),
            "colar" => term.paste_clipboard(),
            "limpar" => {
                x.hide_hint();
                term.reset(true, true);
                // (a fresh prompt)
                term.feed_child(b"\x0c");
            }
            "buscar" => self.open_search(&x, None),
            "dividir" => self.toggle_split(&x),
            "arquivos" => self.toggle_files(&x),

            _ => {}
        }
    }

    fn zoom(&self, by: f64) {
        for x in self.tabs.borrow().iter() {
            let scale = if by == 0.0 { 1.0 } else { (x.term.font_scale() * by).clamp(0.5, 3.0) };
            x.term.set_font_scale(scale);
            if let Some(s) = x.split.borrow().as_ref() {
                s.set_font_scale(scale);
            }
        }
    }

    fn close_tab(self: &Rc<Self>, i: usize) {
        let busy = self.tabs.borrow().get(i).is_some_and(|x| x.running.borrow().is_some());
        if busy {
            self.select(i);
            self.ask_to_close(i, false);
        } else {
            self.close_tab_now(i);
        }
    }

    fn close_tab_now(self: &Rc<Self>, i: usize) {
        let x = {
            let mut tabs = self.tabs.borrow_mut();
            if i >= tabs.len() {
                return;
            }
            tabs.remove(i)
        };
        x.hang_up();
        self.remove_tab_widget(&x);
        self.stack.remove(&x.root);
        self.forget_remote(&x);
        if debug() {
            eprintln!("debug terminal: fechou a aba «{}»", x.title());
        }
        let n = self.tabs.borrow().len();
        if n == 0 && self.kind == Kind::Dropdown {
            // the drop-down's last one: it goes up (a new tab when it comes down again)
            crate::terminal_dropdown::hide();
        } else if n == 0 {
            // the last one: the window goes too (a new tab when it opens again)
            self.window.set_visible(false);
            if debug() {
                eprintln!("debug terminal: fechado");
            }
        } else {
            self.select(i.min(n - 1));
        }
    }

    /// "Fechar a aba?" (design "Terminal · fechar com processo rodando"): inside the Terminal's window — the command at
    /// work and for how long; "Fechar a aba" ends it, Cancelar (Esc, or a click outside) goes back.
    fn ask_to_close(self: &Rc<Self>, i: usize, closing_window: bool) {
        let Some(x) = self.tabs.borrow().get(i).cloned() else { return };
        let old = self.asking.borrow_mut().take();
        if let Some(close) = old {
            close();
        }
        let dim = caixa(Orientation::Vertical, 0, "arq-escurecer");
        dim.set_hexpand(true);
        dim.set_vexpand(true);
        let card = caixa(Orientation::Vertical, 14, "arq-dialogo");
        card.set_size_request(520, -1);
        card.set_halign(Align::Center);
        card.set_valign(Align::Center);
        let top = caixa(Orientation::Horizontal, 14, "");
        let alert = caixa(Orientation::Horizontal, 0, "arq-dialogo-alerta");
        alert.set_size_request(44, 44);
        alert.set_valign(Align::Start);
        alert.set_hexpand(false);
        alert.set_halign(Align::Start);
        let icon = ui_icon("alerta", "#E5C98C", 20);
        icon.set_hexpand(true);
        icon.set_halign(Align::Center);
        icon.set_valign(Align::Center);
        alert.append(&icon);
        top.append(&alert);
        let col = caixa(Orientation::Vertical, 4, "");
        col.set_hexpand(true);
        col.set_valign(Align::Center);
        let tab_title = short_dir(&x.cwd.borrow());
        let t = rotulo(&trf("Close the tab “{tab}”?", &[("tab", tab_title.clone())]), "arq-dialogo-titulo");
        t.set_xalign(0.0);
        t.set_wrap(true);
        t.set_max_width_chars(1);
        col.append(&t);
        let sub = rotulo(tr("What is running in it is ended."), "arq-sub");
        sub.set_xalign(0.0);
        col.append(&sub);
        top.append(&col);
        card.append(&top);
        let what = x.running.borrow().as_ref().map(|(cmd, at, _)| format!("{cmd} · {}", since(*at))).unwrap_or_default();
        let running = caixa(Orientation::Horizontal, 0, "ed-mudou");
        let rl = rotulo(&what, "ed-mudou-texto");
        rl.set_xalign(0.0);
        rl.set_ellipsize(pango::EllipsizeMode::End);
        rl.set_hexpand(true);
        running.append(&rl);
        card.append(&running);
        let buttons = caixa(Orientation::Horizontal, 8, "");
        buttons.set_halign(Align::End);
        let cancel = gtk::Button::with_label(tr("Cancel"));
        cancel.add_css_class("arq-acao-texto");
        let end = gtk::Button::with_label(tr("Close the tab"));
        end.add_css_class("arq-acao-perigo");
        buttons.append(&cancel);
        buttons.append(&end);
        card.append(&buttons);
        self.over.add_overlay(&dim);
        self.over.add_overlay(&card);
        let (ow, dw, cw, w) = (self.over.downgrade(), dim.downgrade(), card.downgrade(), Rc::downgrade(self));
        let close: Rc<dyn Fn()> = Rc::new(move || {
            if let (Some(o), Some(dm), Some(c)) = (ow.upgrade(), dw.upgrade(), cw.upgrade()) {
                if c.parent().is_some() {
                    o.remove_overlay(&c);
                    o.remove_overlay(&dm);
                }
            }
            if let Some(t) = w.upgrade() {
                t.asking.borrow_mut().take();
                if let Some(x) = t.tabs.borrow().get(t.current.get()) {
                    x.term.grab_focus();
                }
            }
        });
        *self.asking.borrow_mut() = Some(close.clone());
        let outside = gtk::GestureClick::new();
        let c = close.clone();
        outside.connect_released(move |_, _, _, _| c());
        dim.add_controller(outside);
        let c = close.clone();
        cancel.connect_clicked(move |_| c());
        let (c, w, xw) = (close.clone(), Rc::downgrade(self), Rc::downgrade(&x));
        end.connect_clicked(move |_| {
            c();
            let (Some(t), Some(x)) = (w.upgrade(), xw.upgrade()) else { return };
            // (what was at work no longer holds the window)
            x.running.borrow_mut().take();
            if let Some(i) = t.index_of(&x) {
                t.close_tab_now(i);
            }
            if closing_window && !t.tabs.borrow().is_empty() {
                t.window.close();
            }
        });
        crate::central::posicao_debug("terminal cancelar", cancel.upcast_ref());
        crate::central::posicao_debug("terminal fechar aba", end.upcast_ref());
        cancel.grab_focus();
        if debug() {
            eprintln!("debug terminal: pergunta fechar «{tab_title}» — {what}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{git_branch, short_dir};
    use std::path::Path;

    #[test]
    fn folders_and_branches() {
        let home = gtk::glib::home_dir();
        assert_eq!(short_dir(&home), "~");
        assert_eq!(short_dir(&home.join("KalangoOS")), "~/KalangoOS");
        assert_eq!(short_dir(Path::new("/etc")), "/etc");
        assert!(short_dir(Path::new("/usr/share/um/caminho/bem/comprido/demais")).starts_with("…/"));
        // this repository's branch (from inside a folder of it)
        assert!(git_branch(Path::new(env!("CARGO_MANIFEST_DIR"))).is_some_and(|b| !b.is_empty()));
    }
}
