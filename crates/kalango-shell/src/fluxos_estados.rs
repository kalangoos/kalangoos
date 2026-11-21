//! The empty, loading and error states of the agents and automations screens (design "Estados · vazio, carregando,
//! erro"), as reusable widgets. Every screen that reads kalango-fluxos uses them the same way:
//!
//! - [`empty`]: nothing there yet — a glyph tile, a title, what to do, its buttons ([`action_button`]) and a
//!   footnote. The caller connects the buttons it gets back.
//! - [`Loading`]: an honest text of what is happening (with an optional fraction for a progress bar), over skeleton
//!   cards that pulse. `Loading::new(text, rows)`, then `set_text` / `set_fraction` while it waits.
//! - [`notice`]: something stopped or is missing — [`Tone::Error`] (coral, the card outlined), [`Tone::Warning`]
//!   (gold glyph) or [`Tone::Info`]; with an optional code block (the machine's own words) and a row for the
//!   caller's buttons (returned).
//! - [`service_offline`]: the notice for "the service is not answering" with "Restart the service"
//!   (`systemctl --user restart kalango-fluxos.socket`; not offered for a service on another socket,
//!   `KALANGO_FLUXOS_SOCKET`) and "Try again"; both call `retry` afterwards.
//! - [`Screen`]: a stack with the four states (content, loading, empty, offline/error) a page switches between,
//!   so each page only says which one is current.
//! - [`call_all`]: several kalango-fluxos calls at once, the answers together on the main loop (in order).

use crate::conversa::ui_icon;
use crate::superficies::{caixa, estrela, rotulo};
use gtk::prelude::*;
use gtk::{glib, Align, Orientation};
use kalango_i18n::{tr, trf};
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// A button as the states draw them: primary (accent), secondary (control) or a text button (`"texto"`).
pub fn action_button(label: &str, glyph: &str, kind: &str) -> gtk::Button {
    let b = gtk::Button::new();
    b.add_css_class("fx-botao");
    if !kind.is_empty() {
        b.add_css_class(kind);
    }
    let c = caixa(Orientation::Horizontal, 6, "");
    c.set_halign(Align::Center);
    let cor = match kind {
        "principal" => "#1C1D24",
        "texto" => "#8E8F99",
        "perigo" => "#EFA2A6",
        _ => "#E2E2E6",
    };
    match glyph {
        "" => {}
        "spark" => c.append(&estrela(11, cor)),
        g => c.append(&ui_icon(g, cor, 14)),
    }
    c.append(&rotulo(label, ""));
    b.set_child(Some(&c));
    b
}

/// Changes the text of a button made by [`action_button`] (its glyph stays).
pub fn set_button_label(b: &gtk::Button, label: &str) {
    if let Some(l) = b.child().and_then(|c| c.last_child()).and_downcast::<gtk::Label>() {
        l.set_text(label);
    }
}

fn left(t: &str, class: &str) -> gtk::Label {
    let l = rotulo(t, class);
    l.set_xalign(0.0);
    l
}

/// "Nothing here yet": returns the block (centred where it is put) and its row of buttons.
pub fn empty(glyph: &str, title: &str, text: &str, note: &str) -> (gtk::Box, gtk::Box) {
    let b = caixa(Orientation::Vertical, 0, "fx-vazio");
    b.set_halign(Align::Center);
    b.set_valign(Align::Center);
    let tile = caixa(Orientation::Horizontal, 0, "fx-vazio-icone");
    tile.set_size_request(64, 64);
    tile.set_halign(Align::Center);
    let g = if glyph == "spark" { estrela(16, "#A5A1E2").upcast::<gtk::Widget>() } else { ui_icon(glyph, "#A5A1E2", 20).upcast() };
    g.set_hexpand(true);
    tile.append(&g);
    b.append(&tile);
    let t = rotulo(title, "fx-vazio-titulo");
    t.set_margin_top(14);
    b.append(&t);
    let x = rotulo(text, "fx-vazio-texto");
    x.set_wrap(true);
    x.set_justify(gtk::Justification::Center);
    x.set_max_width_chars(52);
    x.set_margin_top(8);
    b.append(&x);
    let acoes = caixa(Orientation::Horizontal, 8, "");
    acoes.set_halign(Align::Center);
    acoes.set_margin_top(14);
    b.append(&acoes);
    if !note.is_empty() {
        let n = rotulo(note, "fx-vazio-nota");
        n.set_wrap(true);
        n.set_justify(gtk::Justification::Center);
        n.set_margin_top(12);
        b.append(&n);
    }
    (b, acoes)
}

/// A spinner drawn in the accent (GTK's own needs an icon theme the shell does not carry).
fn spinner() -> gtk::DrawingArea {
    let d = gtk::DrawingArea::new();
    d.add_css_class("fx-girando");
    d.set_size_request(14, 14);
    d.set_valign(Align::Center);
    let angulo = Rc::new(Cell::new(0.0f64));
    let a = angulo.clone();
    d.set_draw_func(move |w, cr, wd, ht| {
        let c = w.color();
        let (cx, cy, r) = (wd as f64 / 2.0, ht as f64 / 2.0, (wd.min(ht) as f64 / 2.0) - 1.5);
        cr.set_line_width(1.6);
        cr.set_source_rgba(c.red() as f64, c.green() as f64, c.blue() as f64, 0.25);
        cr.arc(cx, cy, r, 0.0, std::f64::consts::TAU);
        let _ = cr.stroke();
        cr.set_source_rgba(c.red() as f64, c.green() as f64, c.blue() as f64, 1.0);
        cr.set_line_cap(gtk::cairo::LineCap::Round);
        cr.arc(cx, cy, r, a.get(), a.get() + 4.2);
        let _ = cr.stroke();
    });
    d.add_tick_callback(move |w, clock| {
        angulo.set((clock.frame_time() as f64 / 1e6 * 5.0) % std::f64::consts::TAU);
        w.queue_draw();
        glib::ControlFlow::Continue
    });
    d
}

/// "Loading": what is happening, a progress bar when the fraction is known, and skeleton cards.
pub struct Loading {
    pub root: gtk::Box,
    text: gtk::Label,
    bar: gtk::Box,
    fill: gtk::Box,
    fraction: Cell<Option<f64>>,
}

impl Loading {
    pub fn new(text: &str, rows: usize) -> Rc<Loading> {
        let root = caixa(Orientation::Vertical, 0, "fx-carregando");
        let h = caixa(Orientation::Horizontal, 8, "");
        h.append(&spinner());
        let t = left(text, "fx-carregando-texto");
        t.set_wrap(true);
        t.set_hexpand(true);
        h.append(&t);
        root.append(&h);
        let bar = caixa(Orientation::Horizontal, 0, "fx-barra");
        bar.set_margin_top(12);
        let fill = caixa(Orientation::Horizontal, 0, "fx-barra-cheia");
        fill.set_hexpand(false);
        bar.append(&fill);
        bar.set_visible(false);
        root.append(&bar);
        for i in 0..rows {
            let c = caixa(Orientation::Vertical, 0, "fx-esqueleto");
            c.set_margin_top(if i == 0 { 16 } else { 14 });
            let top = caixa(Orientation::Horizontal, 10, "");
            let sq = caixa(Orientation::Horizontal, 0, "fx-osso");
            sq.set_size_request(36, 36);
            sq.set_valign(Align::Start);
            top.append(&sq);
            let col = caixa(Orientation::Vertical, 6, "");
            col.set_margin_top(4);
            for w in [180, 110] {
                let l = caixa(Orientation::Horizontal, 0, "fx-osso");
                l.set_size_request(w, if w == 180 { 10 } else { 7 });
                l.set_halign(Align::Start);
                col.append(&l);
            }
            top.append(&col);
            c.append(&top);
            let long = caixa(Orientation::Horizontal, 0, "fx-osso");
            long.set_size_request(-1, 7);
            long.set_margin_top(10);
            long.set_margin_end(80);
            c.append(&long);
            root.append(&c);
        }
        let l = Rc::new(Loading { root, text: t, bar: bar.clone(), fill, fraction: Cell::new(None) });
        // the filled part follows the bar's width
        let w = Rc::downgrade(&l);
        let medidor = gtk::DrawingArea::new();
        medidor.set_hexpand(true);
        medidor.set_can_target(false);
        bar.append(&medidor);
        medidor.connect_resize(move |_, _, _| {
            if let Some(l) = w.upgrade() {
                let l2 = l.clone();
                glib::idle_add_local_once(move || l2.resize_fill());
            }
        });
        l
    }

    pub fn set_text(&self, t: &str) {
        self.text.set_text(t);
    }

    /// Some(0..1) shows the bar; None hides it (the time is unknown).
    pub fn set_fraction(&self, f: Option<f64>) {
        self.fraction.set(f.map(|x| x.clamp(0.0, 1.0)));
        self.bar.set_visible(f.is_some());
        self.resize_fill();
    }

    fn resize_fill(&self) {
        let w = self.bar.width();
        let f = self.fraction.get().unwrap_or(0.0);
        self.fill.set_size_request((w as f64 * f).round() as i32, -1);
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tone {
    Error,
    Warning,
    Info,
}

/// A notice card: returns the card and the row for the caller's buttons (hidden while empty).
pub fn notice(tone: Tone, glyph: &str, title: &str, text: &str, code: Option<&str>) -> (gtk::Box, gtk::Box) {
    let c = caixa(Orientation::Vertical, 8, "fx-aviso");
    c.add_css_class(match tone {
        Tone::Error => "erro",
        Tone::Warning => "atencao",
        Tone::Info => "info",
    });
    let h = caixa(Orientation::Horizontal, 8, "");
    let cor = match tone {
        Tone::Error => "#EFA2A6",
        Tone::Warning => "#E5C98C",
        Tone::Info => "#8E8F99",
    };
    h.append(&ui_icon(glyph, cor, 15));
    let t = left(title, "fx-aviso-titulo");
    t.set_wrap(true);
    t.set_hexpand(true);
    h.append(&t);
    c.append(&h);
    if !text.is_empty() {
        let x = left(text, "fx-aviso-texto");
        x.set_wrap(true);
        x.set_max_width_chars(1);
        x.set_hexpand(true);
        c.append(&x);
    }
    if let Some(code) = code.filter(|c| !c.is_empty()) {
        let k = left(code, "fx-aviso-codigo");
        k.set_wrap(true);
        k.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        k.set_selectable(true);
        c.append(&k);
    }
    let acoes = caixa(Orientation::Horizontal, 8, "");
    acoes.set_margin_top(4);
    // shown once the caller puts a button in it (add_action)
    acoes.set_visible(false);
    c.append(&acoes);
    (c, acoes)
}

/// Puts a button in a notice's row (and shows the row).
pub fn add_action(row: &gtk::Box, b: &gtk::Button) {
    row.append(b);
    row.set_visible(true);
}

/// Where the flows service writes what it does.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum LogSource {
    /// the user unit's journal (kalango-fluxos.service under systemd)
    Journal,
    /// a file (a test session's service, `KALANGO_FLUXOS_LOG`)
    File(std::path::PathBuf),
}

/// The log to open: a service of another socket (a test session) only when its log file is known; the user unit's
/// journal otherwise.
pub(crate) fn log_source(custom_socket: bool, log_file: Option<std::path::PathBuf>) -> Option<LogSource> {
    match (custom_socket, log_file) {
        (_, Some(f)) => Some(LogSource::File(f)),
        (true, None) => None,
        (false, None) => Some(LogSource::Journal),
    }
}

/// Opens the service's log in the KalangoOS Editor (the journal is copied to a file first: its last 500 lines).
fn open_log(src: &LogSource, w: &gtk::Widget) {
    let path = match src {
        LogSource::File(f) => f.clone(),
        LogSource::Journal => {
            let dir = glib::user_runtime_dir().join("kalango");
            let _ = std::fs::create_dir_all(&dir);
            let f = dir.join("kalango-fluxos-registro.txt");
            let out = std::process::Command::new("journalctl").args(["--user", "-u", "kalango-fluxos.service", "-n", "500", "--no-pager", "-o", "short-iso"]).output();
            let text = match out {
                Ok(o) if !o.stdout.is_empty() => String::from_utf8_lossy(&o.stdout).to_string(),
                Ok(o) => String::from_utf8_lossy(&o.stderr).to_string(),
                Err(e) => e.to_string(),
            };
            let _ = std::fs::write(&f, text);
            f
        }
    };
    // the system's own Editor (the running shell takes the command: `kalango-shell editor PATH`)
    let _ = w;
    if let Ok(me) = std::env::current_exe() {
        let _ = std::process::Command::new(me).arg("editor").arg(&path).spawn();
    }
}

/// "The service is not answering", with what can be done about it. `retry` runs after a restart and on "Try again".
pub fn service_offline(retry: impl Fn() + 'static) -> gtk::Box {
    let (c, acoes) = notice(
        Tone::Warning,
        "alerta",
        tr("Flows service offline"),
        tr("Nothing on this page can be read or changed until it is back. Your agents, flows and keys stay as they were."),
        None,
    );
    let erro = left("", "fx-aviso-texto");
    erro.add_css_class("erro");
    erro.set_wrap(true);
    erro.set_visible(false);
    c.append(&erro);
    let retry = Rc::new(retry);
    let reiniciar = action_button(tr("Restart the service"), "tocar", "");
    let (r, e) = (retry.clone(), erro.clone());
    reiniciar.connect_clicked(move |b| {
        b.set_sensitive(false);
        let (r, e, b) = (r.clone(), e.clone(), b.clone());
        // socket activation brings the service back on the next connection: restarting the socket is enough
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let out = std::process::Command::new("systemctl").args(["--user", "restart", "kalango-fluxos.socket"]).output();
            let _ = tx.send(match out {
                Ok(o) if o.status.success() => Ok(()),
                Ok(o) => Err(String::from_utf8_lossy(&o.stderr).trim().to_string()),
                Err(err) => Err(err.to_string()),
            });
        });
        glib::timeout_add_local(std::time::Duration::from_millis(100), move || match rx.try_recv() {
            Ok(res) => {
                b.set_sensitive(true);
                match res {
                    Ok(()) => {
                        e.set_visible(false);
                        r();
                    }
                    Err(why) => {
                        e.set_text(&trf("Could not restart it: {why}", &[("why", why)]));
                        e.set_visible(true);
                    }
                }
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(_) => glib::ControlFlow::Break,
        });
    });
    // a service of another socket (KALANGO_FLUXOS_SOCKET: a test session) is not the user unit systemctl restarts
    if std::env::var_os("KALANGO_FLUXOS_SOCKET").is_none() {
        add_action(&acoes, &reiniciar);
    }
    let de_novo = action_button(tr("Try again"), "reiniciar", "texto");
    de_novo.connect_clicked(move |_| retry());
    add_action(&acoes, &de_novo);
    // "Ver registro": what the service wrote before it stopped
    if let Some(src) = log_source(std::env::var_os("KALANGO_FLUXOS_SOCKET").is_some(), std::env::var_os("KALANGO_FLUXOS_LOG").map(std::path::PathBuf::from)) {
        let registro = action_button(tr("See log"), "doc", "texto");
        crate::central::posicao_debug_sempre("fluxos ver registro", registro.upcast_ref());
        registro.connect_clicked(move |b| open_log(&src, b.upcast_ref()));
        add_action(&acoes, &registro);
    }
    c
}

/// The four states of a page. `content` is the page itself; the others are built here and filled by the caller
/// with [`Screen::show_empty`] / [`Screen::show_error`].
pub struct Screen {
    pub stack: gtk::Stack,
    pub loading: Rc<Loading>,
    empty_slot: gtk::Box,
    error_slot: gtk::Box,
    offline: gtk::Box,
}

impl Screen {
    pub fn new(content: &impl IsA<gtk::Widget>, loading_text: &str, rows: usize, retry: impl Fn() + 'static) -> Screen {
        let stack = gtk::Stack::new();
        stack.set_transition_type(gtk::StackTransitionType::Crossfade);
        stack.set_transition_duration(140);
        stack.set_vexpand(true);
        stack.set_hhomogeneous(false);
        stack.add_named(content, Some("conteudo"));
        let loading = Loading::new(loading_text, rows);
        loading.root.set_margin_top(16);
        stack.add_named(&loading.root, Some("carregando"));
        let empty_slot = caixa(Orientation::Vertical, 0, "");
        empty_slot.set_vexpand(true);
        stack.add_named(&empty_slot, Some("vazio"));
        let error_slot = caixa(Orientation::Vertical, 0, "");
        error_slot.set_margin_top(16);
        stack.add_named(&error_slot, Some("erro"));
        let offline = caixa(Orientation::Vertical, 0, "");
        offline.set_margin_top(16);
        offline.append(&service_offline(retry));
        stack.add_named(&offline, Some("fora"));
        Screen { stack, loading, empty_slot, error_slot, offline }
    }

    pub fn show_content(&self) {
        self.stack.set_visible_child_name("conteudo");
    }
    pub fn show_loading(&self) {
        self.stack.set_visible_child_name("carregando");
    }
    pub fn show_offline(&self) {
        let _ = &self.offline;
        self.stack.set_visible_child_name("fora");
    }
    pub fn show_empty(&self, w: &impl IsA<gtk::Widget>) {
        while let Some(c) = self.empty_slot.first_child() {
            self.empty_slot.remove(&c);
        }
        self.empty_slot.append(w);
        self.stack.set_visible_child_name("vazio");
    }
    pub fn show_error(&self, w: &impl IsA<gtk::Widget>) {
        while let Some(c) = self.error_slot.first_child() {
            self.error_slot.remove(&c);
        }
        self.error_slot.append(w);
        self.stack.set_visible_child_name("erro");
    }
    pub fn current(&self) -> String {
        self.stack.visible_child_name().map(|s| s.to_string()).unwrap_or_default()
    }
}

/// Several calls at once; `done` gets every answer, in the order asked, on the main loop.
pub fn call_all(calls: Vec<(&'static str, Value)>, done: impl FnOnce(Vec<Result<Value, String>>) + 'static) {
    let n = calls.len();
    let results: Rc<RefCell<Vec<Option<Result<Value, String>>>>> = Rc::new(RefCell::new(vec![None; n]));
    let done = Rc::new(RefCell::new(Some(done)));
    if n == 0 {
        if let Some(d) = done.borrow_mut().take() {
            d(vec![]);
        }
        return;
    }
    for (i, (m, p)) in calls.into_iter().enumerate() {
        let (results, done) = (results.clone(), done.clone());
        crate::fluxos::call(m, p, move |r| {
            results.borrow_mut()[i] = Some(r);
            if results.borrow().iter().all(Option::is_some) {
                let all: Vec<Result<Value, String>> = results.borrow_mut().drain(..).map(|x| x.unwrap()).collect();
                if let Some(d) = done.borrow_mut().take() {
                    d(all);
                }
            }
        });
    }
}

/// Whether an error of `fluxos::call` means the service is not there (no socket, refused) rather than a refusal of
/// the service itself.
pub fn is_offline_error(e: &str) -> bool {
    e.starts_with("kalango-fluxos:")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_is_the_journal_or_a_known_file() {
        assert_eq!(log_source(false, None), Some(LogSource::Journal));
        assert_eq!(log_source(true, None), None);
        assert_eq!(log_source(true, Some("/tmp/x.log".into())), Some(LogSource::File("/tmp/x.log".into())));
    }

    #[test]
    fn offline_errors_are_told_apart() {
        assert!(is_offline_error("kalango-fluxos: No such file or directory (os error 2)"));
        assert!(is_offline_error("kalango-fluxos: Connection refused (os error 111)"));
        assert!(!is_offline_error("invalid secret name: \"a b\""));
    }
}
