//! The AI models' source on the installed system (designs "Central de Apps · downloads de modelos", "Ajustes ·
//! Fonte dos modelos" and "Alterar a fonte"): the downloads the Central de Apps shows, the card of Settings, and the
//! dialog that changes the source. The engine (checks, resume, retries, the vault) is `kalango-modelos`; here are the
//! one background queue, what each row says about its model, and the widgets.

use crate::app_store_data::{self as data, ModelEntry};
use crate::conversa::ui_icon;
use crate::fluxos_estados::{action_button, set_button_label as estados_label};
use crate::superficies::caixa;
use gtk::prelude::*;
use gtk::{glib, Align, Orientation};
use kalango_i18n::{tr, trf};
use kalango_modelos::catalog::SLOTS;
use kalango_modelos::config::{Config, SourceKind};
use kalango_modelos::download::{is_verified, Source};
use kalango_modelos::net::{Client, NetError};
use kalango_modelos::probe::{probe_folder, probe_server, ModelState, Probe};
use kalango_modelos::queue::{Failure, Item, Phase, Queue};
use kalango_modelos::{vault, Secret};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

// ------------------------------------------------------------------------------------------------ the queue

static QUEUE: OnceLock<Queue> = OnceLock::new();

/// The models of the five kinds first, then the others the catalog has.
fn catalog() -> Vec<ModelEntry> {
    data::model_catalog()
}

/// The configuration in force: an error (not the public source) when the file is there but not valid.
pub fn config() -> Result<Config, kalango_modelos::config::ConfigError> {
    Config::load_effective()
}

/// Where downloads come from now; an invalid configuration has no source.
pub fn current_source() -> Source {
    config().map(|c| c.source()).unwrap_or(Source::Disabled)
}

/// The one background queue of the session (downloads go one at a time).
pub fn queue() -> &'static Queue {
    QUEUE.get_or_init(|| Queue::start(data::download_dir(), catalog(), current_source(), kalango_modelos::download::Options::default))
}

fn cancelled_file() -> std::path::PathBuf {
    data::download_dir().join(".cancelados")
}

fn cancelled() -> Vec<String> {
    std::fs::read_to_string(cancelled_file()).map(|t| t.lines().map(str::to_string).collect()).unwrap_or_default()
}

fn set_cancelled(id: &str, on: bool) {
    let mut v = cancelled();
    v.retain(|x| x != id);
    if on {
        v.push(id.to_string());
    }
    let _ = std::fs::create_dir_all(data::download_dir());
    let _ = std::fs::write(cancelled_file(), v.join("\n"));
}

/// At the session's start: the installer's token goes into the vault, and what was chosen in the installer and is
/// not installed yet is queued (nothing with "later"; nothing the person cancelled). Off the interface's thread.
pub fn start_background() {
    std::thread::spawn(|| {
        match vault::adopt_staged() {
            Ok(true) => eprintln!("modelos: o token do servidor foi para o cofre"),
            Ok(false) => {}
            Err(e) => eprintln!("modelos: o token ainda não foi para o cofre: {e}"),
        }
        let cfg = match config() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("modelos: a configuração da fonte não é válida ({e:?}): nada será baixado");
                return;
            }
        };
        if cfg.fonte == SourceKind::Depois {
            return;
        }
        let q = queue();
        q.set_source(cfg.source());
        let skip = cancelled();
        for id in &cfg.escolhidos {
            if !skip.contains(id) {
                q.enqueue(id);
            }
        }
    });
}

// ------------------------------------------------------------------------------------------------ testing the source

/// The result of the last "test the source", for the Central and the Settings.
#[derive(Clone)]
pub struct Tested {
    pub at: Instant,
    pub result: Result<Probe, NetError>,
}

static LAST: Mutex<Option<Tested>> = Mutex::new(None);

pub fn last_test() -> Option<Tested> {
    LAST.lock().ok().and_then(|g| g.clone())
}

/// Remembers a test's result (the Central and the Settings show it).
pub fn record_last(r: &Result<Probe, NetError>) {
    if let Ok(mut g) = LAST.lock() {
        *g = Some(Tested { at: Instant::now(), result: r.clone() });
    }
}

/// Tests a source: the server (sums and sizes without downloading) or the folder. `done` runs on the main loop.
pub fn test_source(cfg: Config, token: Option<Secret>, record: bool, done: impl FnOnce(Option<Result<Probe, NetError>>) + 'static) {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let cat = catalog();
        let r = match cfg.fonte {
            SourceKind::Servidor => cfg.server().ok().map(|url| {
                // (the token typed now, else the one kept for this very server)
                let token = token.or_else(|| vault::read_for(&url.origin()));
                probe_server(&Client::new(Some(url.clone()), token), &url, &cat)
            }),
            SourceKind::Pasta => cfg.pasta.as_deref().map(|p| Ok(probe_folder(std::path::Path::new(p), &cat, &Default::default()))),
            _ => None,
        };
        if let (Some(r), true) = (&r, record) {
            record_last(r);
        }
        let _ = tx.send(r);
    });
    let mut done = Some(done);
    glib::timeout_add_local(Duration::from_millis(80), move || match rx.try_recv() {
        Ok(r) => {
            if let Some(d) = done.take() {
                d(r);
            }
            glib::ControlFlow::Break
        }
        Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
        Err(_) => glib::ControlFlow::Break,
    });
}

// ------------------------------------------------------------------------------------------------ what a row says

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    Download,
    DownloadAgain,
    Remove,
    Pause,
    Resume,
    Cancel,
    Retry,
    FreeSpace,
    ChangeSource,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RowView {
    /// the chip: (text, tone: teal, acento, cinza, coral, ouro)
    pub chip: (String, &'static str),
    /// the line under the name
    pub detail: String,
    /// "ok", "ruim", "aviso" or "" (muted)
    pub detail_tone: &'static str,
    /// the bar: how full (0 – 1) and its tone ("acento", "cinza", "ruim")
    pub bar: Option<(f64, &'static str)>,
    pub actions: Vec<Act>,
}

fn speed_text(bps: f64) -> String {
    format!("{:.1} MB/s", bps / 1e6).replace('.', ",")
}

fn eta_text(left: f64, bps: f64) -> String {
    if bps < 1.0 {
        return String::new();
    }
    let s = (left / bps).ceil() as u64;
    if s >= 90 { trf("about {n} min left", &[("n", s.div_ceil(60).to_string())]) } else { trf("about {n} s left", &[("n", s.max(1).to_string())]) }
}

/// One model's row: from what is on the disk, what the queue says, the network and what the source's test said.
pub fn row_view(m: &ModelEntry, installed: bool, verified: bool, removable: bool, item: Option<&Item>, online: bool, probed: Option<ModelState>) -> RowView {
    let size = data::gb(m.bytes() as f64);
    let rv = |chip: &str, tone, detail: String, dt, bar, actions: Vec<Act>| RowView { chip: (chip.to_string(), tone), detail, detail_tone: dt, bar, actions };
    let frac = |i: &Item| if i.total > 0 { (i.done as f64 / i.total as f64).clamp(0.0, 1.0) } else { 0.0 };
    let of = |i: &Item| trf("{done} of {total}", &[("done", data::gb(i.done as f64)), ("total", data::gb(i.total as f64))]);
    match item.map(|i| &i.phase) {
        Some(Phase::Installed { verified: v }) => return installed_view(*v, removable, &rv),
        Some(Phase::Downloading) => {
            let i = item.unwrap();
            let mut t = format!("{} · {}", of(i), speed_text(i.bytes_per_s));
            let eta = eta_text(i.total.saturating_sub(i.done) as f64, i.bytes_per_s);
            if !eta.is_empty() {
                t = format!("{t} · {eta}");
            }
            return rv(&trf("downloading {pct}%", &[("pct", format!("{:.0}", frac(i) * 100.0))]), "acento", t, "", Some((frac(i), "acento")), vec![Act::Pause]);
        }
        Some(Phase::Verifying) => return rv(tr("checking"), "acento", tr("checking the SHA-256 sum before keeping the file…").to_string(), "", Some((1.0, "acento")), vec![]),
        Some(Phase::Paused) => {
            let i = item.unwrap();
            return rv(tr("paused"), "cinza", format!("{} · {}", trf("paused at {done}", &[("done", of(i))]), tr("resumes where it stopped")), "", Some((frac(i), "cinza")), vec![Act::Resume, Act::Cancel]);
        }
        Some(Phase::WaitingNetwork) => {
            let i = item.unwrap();
            return rv(tr("waiting for the network"), "ouro", format!("{} · {}", tr("waiting for the network"), trf("{done} kept", &[("done", of(i))])), "aviso", Some((frac(i), "cinza")), vec![Act::Pause]);
        }
        Some(Phase::Queued) => {
            let t = if online { tr("in the queue · starts when the one before ends") } else { tr("in the queue · starts when the network returns") };
            return rv(tr("in the queue"), "cinza", t.to_string(), "", None, vec![Act::Cancel]);
        }
        Some(Phase::Retrying { attempt, max, wait_s }) => {
            return rv(tr("server unreachable"), "coral", trf("Server unreachable. Next try in {s} s · {n} of {max}.", &[("s", wait_s.to_string()), ("n", attempt.to_string()), ("max", max.to_string())]), "ruim", None, vec![Act::Retry]);
        }
        Some(Phase::Failed(f)) => {
            return match f {
                Failure::Mismatch => rv(tr("sum does not match"), "coral", tr("The sum does not match the catalog: the file was refused and deleted. Nothing was installed.").to_string(), "ruim", Some((1.0, "ruim")), vec![Act::DownloadAgain]),
                Failure::WrongSize => rv(tr("different size"), "coral", tr("The file's size is not the catalog's: it was refused and deleted.").to_string(), "ruim", Some((1.0, "ruim")), vec![Act::DownloadAgain]),
                Failure::NoSpace { missing } => rv(tr("no space"), "ouro", trf("No space: {missing} missing on the disk. Free some space and try again.", &[("missing", data::gb(*missing as f64))]), "aviso", None, vec![Act::FreeSpace]),
                Failure::Timeout => rv(tr("server unreachable"), "coral", tr("Server unreachable (timed out). Check the address and the network.").to_string(), "ruim", None, vec![Act::Retry]),
                Failure::Unreachable(_) => rv(tr("server unreachable"), "coral", tr("Server unreachable or the connection kept dropping. Check the address and the network.").to_string(), "ruim", None, vec![Act::Retry]),
                Failure::Denied(c) => rv(tr("access denied"), "coral", trf("Access denied (error {code}). The server refused the token.", &[("code", c.to_string())]), "ruim", None, vec![Act::ChangeSource]),
                Failure::SourceInvalid => rv(tr("settings not valid"), "coral", tr("The models' source settings are not valid: nothing is downloaded until they are fixed.").to_string(), "ruim", None, vec![Act::ChangeSource]),
                Failure::Server(c) => rv(tr("server error"), "coral", trf("The server answered with an error (HTTP {code}).", &[("code", c.to_string())]), "ruim", None, vec![Act::Retry]),
                Failure::NotFound => rv(tr("missing on the server"), "ouro", tr("This model is not on the source.").to_string(), "aviso", None, vec![Act::ChangeSource]),
                Failure::Other(e) => rv(tr("failed"), "coral", e.clone(), "ruim", None, vec![Act::Retry]),
            };
        }
        None => {}
    }
    if installed {
        return installed_view(verified, removable, &rv);
    }
    match probed {
        Some(ModelState::Absent) => return rv(tr("missing on the server"), "ouro", m.descricao.clone(), "", None, vec![]),
        Some(ModelState::SumMismatch) => return rv(tr("sum does not match"), "coral", tr("The source's sum is not the catalog's: it would be refused.").to_string(), "ruim", None, vec![]),
        Some(ModelState::WrongSize) | Some(ModelState::NoChecksum) => return rv(tr("not usable"), "coral", tr("This source cannot provide it (size or sum).").to_string(), "ruim", None, vec![]),
        _ => {}
    }
    let _ = size;
    rv(tr("available"), "acento", m.descricao.clone(), "", None, vec![Act::Download])
}

fn installed_view(verified: bool, removable: bool, rv: &dyn Fn(&str, &'static str, String, &'static str, Option<(f64, &'static str)>, Vec<Act>) -> RowView) -> RowView {
    if verified {
        rv(tr("verified"), "teal", tr("installed · SHA-256 sum checked against the catalog").to_string(), "ok", None, if removable { vec![Act::Remove] } else { vec![] })
    } else {
        rv(tr("installed"), "cinza", tr("installed · it came with the system or was put here by hand").to_string(), "", None, if removable { vec![Act::Remove] } else { vec![] })
    }
}

/// The row of model `i` of the five kinds, from the system's state now.
pub fn view_of(m: &ModelEntry) -> RowView {
    let dir = data::download_dir();
    let installed = data::model_installed(m);
    let verified = installed && m.arquivos.iter().all(|f| is_verified(&dir, f));
    let probed = last_test().and_then(|t| t.result.ok()).and_then(|p| p.state(&m.id));
    // (only what is in the downloads' own folder can be removed here: a model that came with the system is not ours to delete)
    let removable = !m.arquivos.is_empty() && m.arquivos.iter().all(|f| dir.join(&f.nome).is_file());
    row_view(m, installed, verified, removable, queue().item(&m.id).as_ref(), kalango_modelos::net::network_up(), probed)
}

/// The five models the installer offers, in order.
pub fn slot_models() -> Vec<(&'static str, ModelEntry)> {
    let cat = catalog();
    SLOTS.iter().filter_map(|(k, id)| cat.iter().find(|m| m.id == *id).cloned().map(|m| (*k, m))).collect()
}

pub fn is_slot(id: &str) -> bool {
    SLOTS.iter().any(|(_, i)| *i == id)
}

fn kind_name(kind: &str) -> &'static str {
    match kind {
        "texto" => tr("Text"),
        "voz" => tr("Voice"),
        "visao" => tr("Vision"),
        "omni" => tr("Omni"),
        _ => tr("Decision d1"),
    }
}

fn kind_glyph(kind: &str) -> &'static str {
    match kind {
        "texto" => "cerebro",
        "voz" => "microfone",
        "visao" => "olho",
        "omni" => "spark",
        _ => "ramo",
    }
}

fn source_name(c: &Config) -> &'static str {
    match c.fonte {
        SourceKind::Padrao => tr("default source"),
        SourceKind::Servidor => tr("private server"),
        SourceKind::Pasta => tr("folder or USB drive"),
        SourceKind::Depois => tr("later"),
    }
}

/// Does what a row asks.
pub fn act(a: Act, m: &ModelEntry, from: &gtk::Widget) {
    let q = queue();
    match a {
        Act::Download | Act::DownloadAgain | Act::Retry => {
            set_cancelled(&m.id, false);
            q.set_source(current_source());
            q.enqueue(&m.id);
            q.retry(&m.id);
        }
        Act::Pause => q.pause(&m.id),
        Act::Resume => q.resume(&m.id),
        Act::Cancel => {
            set_cancelled(&m.id, true);
            q.cancel(&m.id);
        }
        Act::Remove => {
            // (it deletes gigabytes: it asks first)
            let m = m.clone();
            let parent = from.root().and_then(|r| r.downcast::<gtk::Window>().ok());
            let d = gtk::AlertDialog::builder()
                .message(trf("Remove {model}?", &[("model", m.nome.clone())]))
                .detail(trf("Its files ({size}) are deleted from this computer. You can download them again.", &[("size", data::gb(m.bytes() as f64))]))
                .buttons([tr("Cancel"), tr("Remove")])
                .cancel_button(0)
                .default_button(0)
                .build();
            d.choose(parent.as_ref(), gtk::gio::Cancellable::NONE, move |r| {
                if r == Ok(1) {
                    queue().remove(&m.id);
                }
            });
        }
        Act::FreeSpace => {
            let _ = std::env::current_exe().map(|e| std::process::Command::new(e).args(["configuracoes", "sistema"]).spawn());
        }
        Act::ChangeSource => change_source(from, |_| {}),
    }
}

fn act_label(a: Act) -> (&'static str, &'static str) {
    match a {
        Act::Download => (tr("Download"), "baixar"),
        Act::DownloadAgain => (tr("Download again"), "reiniciar"),
        Act::Remove => (tr("Remove"), "lixeira"),
        Act::Pause => (tr("Pause"), "pausa"),
        Act::Resume => (tr("Continue"), "tocar"),
        Act::Cancel => (tr("Cancel"), "fechar"),
        Act::Retry => (tr("Try now"), "reiniciar"),
        Act::FreeSpace => (tr("Free space"), "pasta"),
        Act::ChangeSource => (tr("Change source"), "configuracoes"),
    }
}

/// A label that starts at the left.
fn rotulo(t: &str, class: &str) -> gtk::Label {
    let l = crate::superficies::rotulo(t, class);
    l.set_xalign(0.0);
    l
}

fn chip(t: &str, tone: &str) -> gtk::Label {
    let l = rotulo(t, "fx-chip");
    l.add_css_class(tone);
    l.set_valign(Align::Center);
    l
}

fn clear(b: &gtk::Box) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
}

fn bar_widget() -> gtk::ProgressBar {
    let b = gtk::ProgressBar::new();
    b.add_css_class("mf-barra");
    b.set_hexpand(true);
    b.set_valign(Align::Center);
    b
}

fn set_bar(b: &gtk::ProgressBar, v: Option<(f64, &'static str)>) {
    b.set_visible(v.is_some());
    for t in ["acento", "cinza", "ruim"] {
        b.remove_css_class(t);
    }
    if let Some((f, t)) = v {
        b.set_fraction(f);
        b.add_css_class(t);
    }
}

fn banner(tone: &str, glyph: &str, title: &str, detail: &str) -> (gtk::Box, gtk::Box) {
    let b = caixa(Orientation::Horizontal, 12, "mf-aviso");
    b.add_css_class(tone);
    b.append(&ui_icon(glyph, "#8E8F99", 16));
    let t = caixa(Orientation::Vertical, 1, "");
    t.set_hexpand(true);
    let tl = rotulo(title, "mf-aviso-titulo");
    tl.set_wrap(true);
    tl.set_xalign(0.0);
    t.append(&tl);
    if !detail.is_empty() {
        let d = rotulo(detail, "mf-aviso-detalhe");
        d.set_wrap(true);
        d.set_xalign(0.0);
        t.append(&d);
    }
    b.append(&t);
    let acoes = caixa(Orientation::Horizontal, 6, "");
    acoes.set_valign(Align::Center);
    b.append(&acoes);
    (b, acoes)
}

/// What a failed test of the source says: (tone, glyph, title, detail).
pub fn probe_notice(r: &Result<Probe, NetError>, host: &str) -> (&'static str, &'static str, String, String) {
    match r {
        Ok(p) => {
            if let Some((id, _)) = p.models.iter().find(|(_, s)| *s == ModelState::SumMismatch) {
                let k = SLOTS.iter().find(|(_, i)| i == id).map(|(k, _)| kind_name(k)).unwrap_or("");
                return ("erro", "alerta", trf("The sum of {model} does not match", &[("model", k.to_string())]), tr("SHA-256 different from the catalog: the file would be refused.").to_string());
            }
            let slots = SLOTS.iter().filter(|(_, id)| p.state(id).is_some_and(|s| s.usable())).count();
            ("ok", "check", trf("Connected to {host}", &[("host", host.to_string())]), format!("{} · {}", trf("{found} of {of} models found", &[("found", slots.to_string()), ("of", SLOTS.len().to_string())]), if p.all_verified() { tr("all sums match") } else { tr("the sums are checked at download") }))
        }
        Err(NetError::Denied(c)) => ("erro", "cadeado", trf("Access denied (error {code})", &[("code", c.to_string())]), tr("The server refused the token. Ask for a new one from whoever administers it.").to_string()),
        Err(NetError::Offline) => ("aviso", "wifi-sem", tr("No internet connection").to_string(), tr("Downloads resume by themselves when the network returns.").to_string()),
        Err(NetError::Timeout) => ("erro", "alerta", trf("Could not reach {host}", &[("host", host.to_string())]), tr("Timed out after 10 s. Check the address and the network. Installed models keep working; new downloads wait for the connection.").to_string()),
        Err(NetError::Status(c)) => ("erro", "alerta", trf("{host} answered with an error (HTTP {code})", &[("host", host.to_string()), ("code", c.to_string())]), tr("The server is there but could not give the list. Installed models keep working.").to_string()),
        Err(_) => ("erro", "alerta", trf("Could not reach {host}", &[("host", host.to_string())]), tr("The server did not answer. Installed models keep working; new downloads wait for the connection.").to_string()),
    }
}

fn host_of(c: &Config) -> String {
    match c.fonte {
        SourceKind::Servidor => c.server().map(|s| s.host().to_string()).unwrap_or_default(),
        SourceKind::Pasta => c.pasta.clone().unwrap_or_default(),
        _ => String::new(),
    }
}

// ------------------------------------------------------------------------------------------------ the Central's page

struct RowW {
    root: gtk::Box,
    nome: gtk::Label,
    sub: gtk::Label,
    chip: gtk::Box,
    barra: gtk::ProgressBar,
    detalhe: gtk::Label,
    detalhe_ic: gtk::Box,
    acoes: gtk::Box,
    last: RefCell<Option<(RowView, bool)>>,
    entry: ModelEntry,
}

/// The page "Modelos de IA · downloads": the source, and the five models with their downloads.
pub struct Downloads {
    pub root: gtk::Box,
    fonte: gtk::Label,
    topo_aviso: gtk::Box,
    rows: Vec<RowW>,
    estado: Cell<u64>,
}

impl Downloads {
    pub fn new() -> Rc<Downloads> {
        let root = caixa(Orientation::Vertical, 12, "mf-pagina");
        let cab = caixa(Orientation::Horizontal, 10, "");
        cab.append(&rotulo(tr("AI models · downloads"), "mf-titulo"));
        let fonte = rotulo("", "fx-chip");
        fonte.add_css_class("cinza");
        fonte.set_valign(Align::Center);
        cab.append(&fonte);
        let esp = caixa(Orientation::Horizontal, 0, "");
        esp.set_hexpand(true);
        cab.append(&esp);
        let alterar = action_button(tr("Change source"), "configuracoes", "");
        let testar = action_button(tr("Test source"), "rede-cabo", "");
        cab.append(&alterar);
        cab.append(&testar);
        root.append(&cab);
        let topo_aviso = caixa(Orientation::Vertical, 0, "");
        root.append(&topo_aviso);
        let mut rows = Vec::new();
        for (kind, m) in slot_models() {
            let r = caixa(Orientation::Horizontal, 14, "mf-linha");
            let tile = caixa(Orientation::Horizontal, 0, "mf-tile");
            tile.set_size_request(40, 40);
            tile.set_valign(Align::Center);
            let g = ui_icon(kind_glyph(kind), "#8ED2C3", 18);
            g.set_hexpand(true);
            tile.set_hexpand(false);
            tile.set_halign(Align::Center);
            tile.append(&g);
            r.append(&tile);
            let corpo = caixa(Orientation::Vertical, 6, "");
            corpo.set_hexpand(true);
            corpo.set_valign(Align::Center);
            let l1 = caixa(Orientation::Horizontal, 8, "");
            let nome = rotulo(kind_name(kind), "mf-nome");
            l1.append(&nome);
            let sub = rotulo(&format!("{} · {}", m.nome, data::gb(m.bytes() as f64)), "mf-sub");
            l1.append(&sub);
            let chip_box = caixa(Orientation::Horizontal, 0, "");
            chip_box.set_valign(Align::Center);
            l1.append(&chip_box);
            corpo.append(&l1);
            let barra = bar_widget();
            corpo.append(&barra);
            let l3 = caixa(Orientation::Horizontal, 6, "");
            let detalhe_ic = caixa(Orientation::Horizontal, 0, "");
            l3.append(&detalhe_ic);
            let detalhe = rotulo("", "mf-detalhe");
            detalhe.set_xalign(0.0);
            detalhe.set_wrap(true);
            detalhe.set_hexpand(true);
            l3.append(&detalhe);
            corpo.append(&l3);
            r.append(&corpo);
            let acoes = caixa(Orientation::Horizontal, 8, "");
            acoes.set_valign(Align::Center);
            r.append(&acoes);
            root.append(&r);
            rows.push(RowW { root: r, nome, sub, chip: chip_box, barra, detalhe, detalhe_ic, acoes, last: RefCell::new(None), entry: m });
        }
        let d = Rc::new(Downloads { root, fonte, topo_aviso, rows, estado: Cell::new(0) });
        let w = Rc::downgrade(&d);
        alterar.connect_clicked(move |b| {
            let w = w.clone();
            change_source(b.upcast_ref(), move |_| if let Some(d) = w.upgrade() { d.tick() });
        });
        let w = Rc::downgrade(&d);
        testar.connect_clicked(move |_| {
            let Some(d) = w.upgrade() else { return };
            let Ok(cfg) = config() else { return };
            let w2 = Rc::downgrade(&d);
            test_source(cfg, None, true, move |_| if let Some(d) = w2.upgrade() { d.tick() });
        });
        let w = Rc::downgrade(&d);
        glib::timeout_add_local(Duration::from_millis(500), move || match w.upgrade() {
            Some(d) => {
                if d.root.is_mapped() {
                    d.tick();
                }
                glib::ControlFlow::Continue
            }
            None => glib::ControlFlow::Break,
        });
        d.tick();
        d
    }

    /// Reads the state again and changes what changed.
    pub fn tick(self: &Rc<Self>) {
        let cfg_r = config();
        let invalid = cfg_r.is_err();
        let cfg = cfg_r.unwrap_or_default();
        self.fonte.set_text(&if invalid { tr("settings not valid").to_string() } else { trf("source: {name}", &[("name", source_name(&cfg).to_string())]) });
        // the top: the network, or what the last test said
        let online = kalango_modelos::net::network_up();
        let nota = (online, invalid, cfg.fonte, last_test().map(|t| (t.at.elapsed().as_secs() / 30, format!("{:?}", t.result))));
        let key = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            format!("{nota:?}").hash(&mut h);
            h.finish()
        };
        if self.estado.replace(key) != key || self.topo_aviso.first_child().is_none() {
            clear(&self.topo_aviso);
            let aviso = if invalid {
                Some(banner("erro", "alerta", tr("The models' source settings are not valid"), tr("Nothing is downloaded until they are fixed. Use Change source.")).0)
            } else if !online {
                let (b, a) = banner("aviso", "wifi-sem", tr("No internet connection"), tr("Downloads resume by themselves when the network returns. Nothing is lost: the part already downloaded is kept."));
                let ver = action_button(tr("See networks"), "wifi", "");
                ver.connect_clicked(|_| {
                    let _ = std::env::current_exe().map(|e| std::process::Command::new(e).args(["configuracoes", "rede"]).spawn());
                });
                a.append(&ver);
                Some(b)
            } else if cfg.fonte == SourceKind::Depois {
                Some(banner("info", "info", tr("Nothing downloads by itself"), tr("The source is \"later\". Use Download on each model; the agents need the Text one.")).0)
            } else {
                last_test().filter(|t| t.result.is_err()).map(|t| {
                    let (tone, g, title, detail) = probe_notice(&t.result, &host_of(&cfg));
                    banner(tone, g, &title, &detail).0
                })
            };
            if let Some(a) = aviso {
                self.topo_aviso.append(&a);
            }
        }
        for r in &self.rows {
            let v = view_of(&r.entry);
            let busy = queue().snapshot().iter().any(|i| matches!(i.phase, Phase::Downloading | Phase::Verifying));
            let _ = busy;
            if r.last.borrow().as_ref().is_some_and(|(old, o)| *old == v && *o == online) {
                continue;
            }
            if let Some(c) = r.chip.first_child() {
                r.chip.remove(&c);
            }
            r.chip.append(&chip(&v.chip.0, v.chip.1));
            set_bar(&r.barra, v.bar);
            r.detalhe.set_text(&v.detail);

            clear(&r.detalhe_ic);
            let (g, c) = match v.detail_tone {
                "ok" => (Some("check"), "#8ED2C3"),
                "ruim" => (Some("alerta"), "#E06C75"),
                "aviso" => (Some("alerta"), "#E5C98C"),
                _ => (None, ""),
            };
            if let Some(g) = g {
                r.detalhe_ic.append(&ui_icon(g, c, 13));
            }
            clear(&r.acoes);
            for a in &v.actions {
                let (label, glyph) = act_label(*a);
                let b = action_button(label, glyph, "");
                let m = r.entry.clone();
                let a = *a;
                b.connect_clicked(move |b| act(a, &m, b.upcast_ref()));
                r.acoes.append(&b);
            }
            *r.last.borrow_mut() = Some((v, online));
            let _ = (&r.root, &r.nome, &r.sub);
        }
    }
}

// ------------------------------------------------------------------------------------------------ Settings: the card

/// The card's line under its name: never the public source's name when the settings are not valid.
pub fn card_subtitle(invalid: bool, cfg: &Config, host: &str) -> String {
    if invalid {
        return tr("Settings not valid").to_string();
    }
    match cfg.fonte {
        SourceKind::Servidor => format!("{} · {}", tr("Private server"), cfg.servidor.clone().unwrap_or_default()),
        SourceKind::Pasta => format!("{} · {}", tr("Folder or USB drive"), host),
        SourceKind::Padrao => tr("Default source · KalangoOS public catalog").to_string(),
        SourceKind::Depois => tr("Later · nothing downloads by itself").to_string(),
    }
}

/// What the dialog starts from: the configuration in force; nothing at all (no source chosen, empty fields) when it
/// is not valid, so the public source is not presented as the current choice.
pub fn dialog_prefill(r: &Result<Config, kalango_modelos::config::ConfigError>) -> Option<Config> {
    r.as_ref().ok().cloned()
}

/// "Save anyway": offered after a failed test, and withdrawn as soon as anything the test looked at is edited, so a
/// later change is tested again. An edit made while a test runs also counts: that test no longer offers it.
#[derive(Default)]
pub struct SaveAnyway {
    forced: Cell<bool>,
    edits: Cell<u64>,
}

impl SaveAnyway {
    /// Taken when the test starts.
    pub fn snapshot(&self) -> u64 {
        self.edits.get()
    }
    /// The test failed: offered only when nothing was edited since `snapshot`.
    pub fn failed(&self, snapshot: u64) -> bool {
        let unchanged = self.edits.get() == snapshot;
        self.forced.set(unchanged);
        unchanged
    }
    pub fn edited(&self) {
        self.edits.set(self.edits.get() + 1);
        self.forced.set(false);
    }
    pub fn forced(&self) -> bool {
        self.forced.get()
    }
}

/// "Fonte dos modelos": the source, its state, the token and the five models (Settings › Models and providers).
pub struct SourceCard {
    pub root: gtk::Box,
    corpo: gtk::Box,
}

impl SourceCard {
    pub fn new() -> Rc<SourceCard> {
        let root = caixa(Orientation::Vertical, 0, "mf-cartao");
        let corpo = caixa(Orientation::Vertical, 0, "");
        root.append(&corpo);
        let c = Rc::new(SourceCard { root, corpo });
        c.render();
        let w = Rc::downgrade(&c);
        glib::timeout_add_local(Duration::from_secs(1), move || match w.upgrade() {
            Some(c) => {
                if c.root.is_mapped() && queue().snapshot().iter().any(|i| !matches!(i.phase, Phase::Installed { .. } | Phase::Paused)) {
                    c.render();
                }
                glib::ControlFlow::Continue
            }
            None => glib::ControlFlow::Break,
        });
        c
    }

    pub fn render(self: &Rc<Self>) {
        clear(&self.corpo);
        let invalid = config().is_err();
        let cfg = config().unwrap_or_default();
        let host = host_of(&cfg);
        // the head: the source, its state, "Test" and "Change"
        let cab = caixa(Orientation::Horizontal, 12, "mf-cab");
        let tile = caixa(Orientation::Horizontal, 0, "mf-tile");
        tile.set_size_request(32, 32);
        tile.set_valign(Align::Center);
        let g = ui_icon("globo", "#A5A1E2", 15);
        tile.set_hexpand(false);
        tile.set_halign(Align::Center);
        g.set_hexpand(true);
        tile.append(&g);
        cab.append(&tile);
        let t = caixa(Orientation::Vertical, 1, "");
        t.set_hexpand(true);
        t.append(&rotulo(tr("Model source"), "mf-nome"));
        let sub = card_subtitle(invalid, &cfg, &host);
        let sl = rotulo(&sub, "mf-sub");
        sl.set_ellipsize(gtk::pango::EllipsizeMode::End);
        t.append(&sl);
        cab.append(&t);
        let tested = last_test();
        if let (Some(t), SourceKind::Servidor | SourceKind::Pasta) = (&tested, cfg.fonte) {
            match &t.result {
                Ok(_) => cab.append(&chip(&trf("Connected · checked {n} min ago", &[("n", (t.at.elapsed().as_secs() / 60).to_string())]), "teal")),
                Err(_) => cab.append(&chip(tr("Server unreachable"), "coral")),
            }
        }
        let testar = action_button(tr("Test"), "rede-cabo", "");
        testar.set_sensitive(matches!(cfg.fonte, SourceKind::Servidor | SourceKind::Pasta));
        let alterar = action_button(tr("Change"), "configuracoes", "");
        cab.append(&testar);
        cab.append(&alterar);
        self.corpo.append(&cab);
        if invalid {
            self.corpo.append(&banner("erro", "alerta", tr("The models' source settings are not valid"), tr("Nothing is downloaded until they are fixed. Use Change.")).0);
        }
        let w = Rc::downgrade(self);
        testar.connect_clicked(move |_| {
            let w = w.clone();
            if let Ok(c) = config() {
                test_source(c, None, true, move |_| if let Some(c) = w.upgrade() { c.render() });
            }
        });
        let w = Rc::downgrade(self);
        alterar.connect_clicked(move |b| {
            let w = w.clone();
            change_source(b.upcast_ref(), move |_| if let Some(c) = w.upgrade() { c.render() });
        });
        if let Some(Tested { result: r @ Err(_), .. }) = &tested {
            if matches!(cfg.fonte, SourceKind::Servidor | SourceKind::Pasta) {
                let (tone, glyph, title, detail) = probe_notice(r, &host);
                let (b, acoes) = banner(tone, glyph, &title, &detail);
                let again = action_button(tr("Try again"), "reiniciar", "");
                let w = Rc::downgrade(self);
                again.connect_clicked(move |_| {
                    let w = w.clone();
                    if let Ok(c) = config() {
                        test_source(c, None, true, move |_| if let Some(c) = w.upgrade() { c.render() });
                    }
                });
                acoes.append(&again);
                self.corpo.append(&b);
            }
        }
        // the token of a private server
        if cfg.fonte == SourceKind::Servidor {
            let linha = caixa(Orientation::Horizontal, 10, "mf-token");
            linha.append(&ui_icon("cadeado", "#8E8F99", 14));
            linha.append(&rotulo(tr("Access token"), "mf-sub"));
            let stored = cfg.server().ok().is_some_and(|u| vault::read_for(&u.origin()).is_some());
            linha.append(&rotulo(if stored { "••••••••••••••••" } else { "—" }, "mf-sub"));
            if stored {
                linha.append(&chip(tr("kept in the key vault"), "teal"));
            }
            let esp = caixa(Orientation::Horizontal, 0, "");
            esp.set_hexpand(true);
            linha.append(&esp);
            let subst = action_button(tr("Replace"), "", "");
            subst.connect_clicked(|b| change_source(b.upcast_ref(), |_| {}));
            linha.append(&subst);
            if stored {
                let rem = action_button(tr("Remove"), "lixeira", "");
                let w = Rc::downgrade(self);
                rem.connect_clicked(move |_| {
                    vault::forget();
                    if let Some(c) = w.upgrade() {
                        c.render();
                    }
                });
                linha.append(&rem);
            }
            self.corpo.append(&linha);
        }
        // the five models
        for (kind, m) in slot_models() {
            let v = view_of(&m);
            let r = caixa(Orientation::Horizontal, 12, "mf-linha-pequena");
            let ic = caixa(Orientation::Horizontal, 0, "mf-tile");
            ic.set_size_request(28, 28);
            let gi = ui_icon(kind_glyph(kind), "#8E8F99", 14);
            ic.set_hexpand(false);
            ic.set_halign(Align::Center);
            gi.set_hexpand(true);
            ic.append(&gi);
            r.append(&ic);
            let n = caixa(Orientation::Vertical, 0, "");
            n.set_size_request(190, -1);
            n.append(&rotulo(kind_name(kind), "mf-nome"));
            n.append(&rotulo(&m.nome, "mf-sub"));
            r.append(&n);
            let cb = caixa(Orientation::Horizontal, 0, "");
            cb.set_hexpand(true);
            cb.set_valign(Align::Center);
            cb.append(&chip(&v.chip.0, v.chip.1));
            r.append(&cb);
            r.append(&rotulo(&data::gb(m.bytes() as f64), "mf-sub"));
            let acoes = caixa(Orientation::Horizontal, 6, "");
            acoes.set_size_request(150, -1);
            acoes.set_halign(Align::End);
            for a in v.actions.iter().filter(|a| !matches!(a, Act::Pause | Act::Resume | Act::Cancel | Act::FreeSpace | Act::ChangeSource)) {
                let (label, glyph) = act_label(*a);
                let b = action_button(label, glyph, "");
                let (mm, a) = (m.clone(), *a);
                let w = Rc::downgrade(self);
                b.connect_clicked(move |b| {
                    act(a, &mm, b.upcast_ref());
                    if let Some(c) = w.upgrade() {
                        c.render();
                    }
                });
                acoes.append(&b);
            }
            r.append(&acoes);
            self.corpo.append(&r);
        }
    }
}

/// The card in a window of its own (session tests and captures), and optionally the dialog over it.
pub fn open_card_window(dialog: bool) {
    let w = gtk::Window::builder().title(tr("Model source")).default_width(960).default_height(520).build();
    let col = caixa(Orientation::Vertical, 0, "");
    col.set_margin_top(24);
    col.set_margin_bottom(24);
    col.set_margin_start(24);
    col.set_margin_end(24);
    let card = SourceCard::new();
    col.append(&card.root);
    w.set_child(Some(&col));
    w.present();
    if dialog {
        let from: gtk::Widget = col.upcast();
        change_source(&from, |_| {});
    }
    std::mem::forget(card);
}

// ------------------------------------------------------------------------------------------------ the dialog

/// "Alterar a fonte dos modelos": the four sources, the address and token of a private server, "Salvar e testar".
pub fn change_source(from: &gtk::Widget, saved: impl Fn(&Config) + 'static) {
    let parent = from.root().and_then(|r| r.downcast::<gtk::Window>().ok());
    let win = gtk::Window::builder().modal(true).title(tr("Change the models' source")).default_width(600).resizable(false).build();
    win.add_css_class("mf-dialogo");
    if let Some(p) = &parent {
        win.set_transient_for(Some(p));
    }
    let prefill = dialog_prefill(&config());
    let cfg = prefill.clone().unwrap_or_default();
    let col = caixa(Orientation::Vertical, 12, "mf-dialogo-corpo");
    col.append(&rotulo(tr("Change the models' source"), "mf-titulo"));
    col.append(&rotulo(tr("Models already installed stay on the computer."), "mf-sub"));
    let grade = gtk::Grid::new();
    grade.set_row_spacing(10);
    grade.set_column_spacing(10);
    grade.set_column_homogeneous(true);
    let nomes = [(tr("Default source"), tr("KalangoOS public catalog")), (tr("Private server"), tr("your HTTPS address")), (tr("Folder or USB drive"), tr("files you already have")), (tr("Download it later"), tr("download later, in the App Center"))];
    let ordem = [SourceKind::Padrao, SourceKind::Servidor, SourceKind::Pasta, SourceKind::Depois];
    let mut botoes: Vec<gtk::ToggleButton> = Vec::new();
    for (k, (t, s)) in nomes.iter().enumerate() {
        let b = gtk::ToggleButton::new();
        b.add_css_class("mf-fonte");
        let c = caixa(Orientation::Vertical, 1, "");
        c.append(&rotulo(t, "mf-nome"));
        c.append(&rotulo(s, "mf-sub"));
        b.set_child(Some(&c));
        if let Some(f) = botoes.first() {
            b.set_group(Some(f));
        }
        b.set_active(prefill.is_some() && ordem[k] == cfg.fonte);
        grade.attach(&b, (k % 2) as i32, (k / 2) as i32, 1, 1);
        botoes.push(b);
    }
    col.append(&grade);
    let srv_box = caixa(Orientation::Vertical, 6, "");
    srv_box.append(&rotulo(tr("Server address (HTTPS)"), "mf-sub"));
    let end = gtk::Entry::new();
    end.add_css_class("mf-campo");
    end.set_text(cfg.servidor.as_deref().unwrap_or(""));
    srv_box.append(&end);
    srv_box.append(&rotulo(tr("Access token (optional)"), "mf-sub"));
    let tok = gtk::Entry::new();
    tok.add_css_class("mf-campo");
    tok.set_visibility(false);
    // (a kept token is shown as kept only for the server it belongs to)
    let kept_for = vault::bound_origin();
    if cfg.server().ok().is_some_and(|u| kept_for.as_deref() == Some(u.origin().as_str())) {
        tok.set_placeholder_text(Some("••••••••••••••••"));
    }
    srv_box.append(&tok);
    srv_box.append(&rotulo(tr("Kept in the system's key vault (Settings › Keys). Here it never shows as text."), "mf-sub"));
    let lan = gtk::CheckButton::with_label(tr("This server is on my private network and has no HTTPS"));
    lan.set_active(cfg.rede_local_confirmada);
    srv_box.append(&lan);
    col.append(&srv_box);
    let pst_box = caixa(Orientation::Vertical, 6, "");
    pst_box.append(&rotulo(tr("Folder with the models"), "mf-sub"));
    let pasta = gtk::Entry::new();
    pasta.add_css_class("mf-campo");
    pasta.set_text(cfg.pasta.as_deref().unwrap_or(""));
    pst_box.append(&pasta);
    col.append(&pst_box);
    let resultado = caixa(Orientation::Vertical, 0, "");
    col.append(&resultado);
    let rodape = caixa(Orientation::Horizontal, 10, "");
    rodape.set_halign(Align::End);
    let cancelar = action_button(tr("Cancel"), "", "");
    let salvar = action_button(tr("Save and test"), "", "principal");
    rodape.append(&cancelar);
    rodape.append(&salvar);
    col.append(&rodape);
    win.set_child(Some(&col));
    let ordem2 = ordem;
    let (end2, tok2, pasta2, lan2) = (end.clone(), tok.clone(), pasta.clone(), lan.clone());
    let mostrar = {
        let botoes = botoes.clone();
        Rc::new(move || {
            let k = botoes.iter().position(|b| b.is_active()).unwrap_or(0);
            srv_box.set_visible(ordem2[k] == SourceKind::Servidor);
            pst_box.set_visible(ordem2[k] == SourceKind::Pasta);
        })
    };
    for b in &botoes {
        let m = mostrar.clone();
        b.connect_toggled(move |_| m());
    }
    mostrar();
    let w = win.downgrade();
    cancelar.connect_clicked(move |_| if let Some(w) = w.upgrade() { w.close() });
    let saved = Rc::new(saved);
    let w = win.downgrade();
    let res = resultado.clone();
    // after a failed test: the next click keeps the choice anyway
    let force = Rc::new(SaveAnyway::default());
    {
        // anything the test looked at changing withdraws "Save anyway"
        let reset = |f: &Rc<SaveAnyway>, salvar: &gtk::Button| {
            let (f, salvar) = (f.clone(), salvar.clone());
            move || {
                f.edited();
                estados_label(&salvar, tr("Save and test"));
            }
        };
        for e in [&end, &tok, &pasta] {
            let r = reset(&force, &salvar);
            e.connect_changed(move |_| r());
        }
        let r = reset(&force, &salvar);
        lan.connect_toggled(move |_| r());
        for b in &botoes {
            let r = reset(&force, &salvar);
            b.connect_toggled(move |_| r());
        }
    }
    let escolhidos = cfg.escolhidos.clone();
    salvar.connect_clicked(move |b| {
        let Some(k) = botoes.iter().position(|b| b.is_active()) else {
            clear(&res);
            res.append(&banner("erro", "alerta", tr("Choose a source."), "").0);
            return;
        };
        let mut c = Config { fonte: ordem2[k], escolhidos: escolhidos.clone(), ..Default::default() };
        match c.fonte {
            SourceKind::Servidor => {
                c.servidor = Some(end2.text().trim().to_string());
                c.rede_local_confirmada = lan2.is_active();
            }
            SourceKind::Pasta => c.pasta = Some(pasta2.text().trim().to_string()),
            SourceKind::Depois => c.escolhidos.clear(),
            SourceKind::Padrao => {}
        }
        clear(&res);
        // 1 · the choice must be valid before anything is kept
        if let Err(e) = c.validate() {
            let why = match e {
                kalango_modelos::config::ConfigError::Server(kalango_modelos::urlpolicy::UrlError::PlainHttp) => tr("Use an https:// address."),
                kalango_modelos::config::ConfigError::MissingFolder | kalango_modelos::config::ConfigError::RelativeFolder => tr("Choose a folder (its full path)."),
                _ => tr("The address is not valid."),
            };
            res.append(&banner("erro", "alerta", why, "").0);
            return;
        }
        // 2 · the token: the one typed now, else the one kept for this very server; a token kept for another server
        //     is not used (and is forgotten when this choice is saved)
        let typed = Some(tok2.text().to_string()).filter(|t| !t.is_empty()).map(Secret::new);
        let origin = c.server().ok().map(|u| u.origin());
        let host = host_of(&c);
        // 3 · what is done with a good (or forced) choice: saved, the token kept bound to its server, the queue told
        let commit = {
            let (saved, w, c, typed, origin, tok2, res) = (saved.clone(), w.clone(), c.clone(), typed.clone(), origin.clone(), tok2.clone(), res.clone());
            move || {
                if c.fonte == SourceKind::Servidor {
                    match (&typed, &origin) {
                        (Some(t), Some(o)) => {
                            if let Err(e) = vault::store(o, t) {
                                clear(&res);
                                res.append(&banner("erro", "alerta", tr("The token could not be kept in the vault."), &e).0);
                                return;
                            }
                            tok2.set_text("");
                        }
                        (None, Some(o)) if vault::bound_origin().is_some_and(|b| b != *o) => vault::forget(),
                        _ => {}
                    }
                } else {
                    // (a source that has no server keeps no token)
                    vault::forget();
                }
                if let Err(e) = c.save(&Config::user_path()) {
                    clear(&res);
                    res.append(&banner("erro", "alerta", tr("The choice could not be saved."), &e).0);
                    return;
                }
                queue().set_source(c.source_with(typed.clone()));
                saved(&c);
                if let Some(w) = w.upgrade() {
                    w.close();
                }
            }
        };
        if force.forced() || !matches!(c.fonte, SourceKind::Servidor | SourceKind::Pasta) {
            commit();
            return;
        }
        // 4 · a server or a folder is tested BEFORE it is saved; while it runs nothing can be edited, and an edit that
        //     still got in (the counter) keeps a failed test from offering "save anyway"
        b.set_sensitive(false);
        let snap = force.snapshot();
        let locked: Vec<gtk::Widget> = [end2.clone().upcast::<gtk::Widget>(), tok2.clone().upcast(), pasta2.clone().upcast(), lan2.clone().upcast()].into_iter().chain(botoes.iter().map(|b| b.clone().upcast())).collect();
        for w in &locked {
            w.set_sensitive(false);
        }
        let (b2, res2, force2) = (b.clone(), res.clone(), force.clone());
        let commit = Rc::new(commit);
        let commit2 = commit.clone();
        test_source(c, typed, false, move |r| {
            b2.set_sensitive(true);
            for w in &locked {
                w.set_sensitive(true);
            }
            match r {
                Some(r) if r.is_err() => {
                    let (tone, g, title, detail) = probe_notice(&r, &host);
                    clear(&res2);
                    res2.append(&banner(tone, g, &title, &detail).0);
                    if force2.failed(snap) {
                        res2.append(&banner("info", "info", tr("Nothing was saved. Click again to keep this choice anyway."), "").0);
                        estados_label(&b2, tr("Save anyway"));
                    }
                }
                Some(r) => {
                    record_last(&r);
                    commit2();
                }
                None => commit2(),
            }
        });
    });
    win.present();
}

#[cfg(test)]
mod tests {
    use super::*;
    use kalango_modelos::catalog::ModelFile;

    fn model() -> ModelEntry {
        ModelEntry { id: "m".into(), nome: "M".into(), descricao: "d".into(), tipo: "texto".into(), arquivos: vec![ModelFile { url: "https://x/m".into(), nome: "m.gguf".into(), bytes: 1_400_000_000, sha256: "a".repeat(64) }] }
    }

    fn item(phase: Phase, done: u64) -> Item {
        Item { id: "m".into(), phase, done, total: 1_400_000_000, bytes_per_s: 12_400_000.0 }
    }

    fn view(installed: bool, verified: bool, i: Option<Item>, online: bool, probed: Option<ModelState>) -> RowView {
        row_view(&model(), installed, verified, true, i.as_ref(), online, probed)
    }

    #[test]
    fn only_what_is_in_the_downloads_folder_can_be_removed() {
        let v = row_view(&model(), true, false, false, None, true, None);
        assert!(v.actions.is_empty(), "a model of the system has no Remove");
        let v = row_view(&model(), true, true, false, None, true, None);
        assert!(v.actions.is_empty());
        let v = row_view(&model(), true, true, true, None, true, None);
        assert_eq!(v.actions, vec![Act::Remove]);
        let v = row_view(&model(), false, false, false, Some(&item(Phase::Installed { verified: true }, 0)), true, None);
        assert!(v.actions.is_empty());
    }

    #[test]
    fn each_state_of_the_design_has_its_tone_bar_and_buttons() {
        let v = view(true, true, None, true, None);
        assert_eq!((v.chip.1, v.bar, v.actions.clone()), ("teal", None, vec![Act::Remove]));
        // an embedded model (no check recorded) shows as installed, not verified
        let v = view(true, false, None, true, None);
        assert_eq!((v.chip.1, v.actions), ("cinza", vec![Act::Remove]));
        let v = view(false, false, Some(item(Phase::Downloading, 868_000_000)), true, None);
        assert_eq!((v.chip.1, v.actions.clone()), ("acento", vec![Act::Pause]));
        let (f, t) = v.bar.unwrap();
        assert!((f - 0.62).abs() < 0.01 && t == "acento");
        let v = view(false, false, Some(item(Phase::Paused, 230_000_000)), true, None);
        assert_eq!((v.chip.1, v.actions, v.bar.unwrap().1), ("cinza", vec![Act::Resume, Act::Cancel], "cinza"));
        let v = view(false, false, Some(item(Phase::Failed(Failure::Mismatch), 0)), true, None);
        assert_eq!((v.chip.1, v.actions, v.bar, v.detail_tone), ("coral", vec![Act::DownloadAgain], Some((1.0, "ruim")), "ruim"));
        let v = view(false, false, Some(item(Phase::Failed(Failure::NoSpace { missing: 600_000_000 }), 0)), true, None);
        assert_eq!((v.chip.1, v.actions), ("ouro", vec![Act::FreeSpace]));
        let v = view(false, false, Some(item(Phase::WaitingNetwork, 870_000_000)), false, None);
        assert_eq!((v.chip.1, v.actions), ("ouro", vec![Act::Pause]));
        let v = view(false, false, Some(item(Phase::Queued, 0)), false, None);
        assert_eq!((v.chip.1, v.actions), ("cinza", vec![Act::Cancel]));
        let v = view(false, false, Some(item(Phase::Retrying { attempt: 3, max: 5, wait_s: 30 }, 0)), true, None);
        assert_eq!((v.chip.1, v.actions), ("coral", vec![Act::Retry]));
        assert!(v.detail.contains("30") && v.detail.contains('3') && v.detail.contains('5'), "{}", v.detail);
        let v = view(false, false, Some(item(Phase::Failed(Failure::Denied(401)), 0)), true, None);
        assert_eq!(v.actions, vec![Act::ChangeSource]);
        let v = view(false, false, None, true, None);
        assert_eq!((v.chip.1, v.actions), ("acento", vec![Act::Download]));
    }

    #[test]
    fn an_invalid_source_and_a_server_error_have_their_own_rows_and_notices() {
        let v = view(false, false, Some(item(Phase::Failed(Failure::SourceInvalid), 0)), true, None);
        assert_eq!((v.chip.1, v.actions), ("coral", vec![Act::ChangeSource]));
        let v = view(false, false, Some(item(Phase::Failed(Failure::Server(503)), 0)), true, None);
        assert_eq!((v.chip.1, v.actions), ("coral", vec![Act::Retry]));
        assert!(v.detail.contains("503"));
        let (tone, _, title, _) = probe_notice(&Err(NetError::Status(503)), "h");
        assert_eq!(tone, "erro");
        assert!(title.contains("503"), "an HTTP error is not \"unreachable\": {title}");
    }

    #[test]
    fn what_the_source_cannot_provide_cannot_be_downloaded() {
        for s in [ModelState::Absent, ModelState::SumMismatch, ModelState::WrongSize, ModelState::NoChecksum] {
            assert!(view(false, false, None, true, Some(s)).actions.is_empty(), "{s:?}");
        }
        assert_eq!(view(false, false, None, true, Some(ModelState::Verified)).actions, vec![Act::Download]);
    }

    #[test]
    fn the_downloading_line_has_the_progress_the_speed_and_the_time_left() {
        let v = view(false, false, Some(item(Phase::Downloading, 868_000_000)), true, None);
        assert!(v.detail.contains("12,4 MB/s"), "{}", v.detail);
        assert!(v.detail.contains("43"), "{}", v.detail);
    }

    #[test]
    fn a_failed_test_says_why() {
        let (tone, _, title, _) = probe_notice(&Err(NetError::Denied(403)), "modelos.example.org");
        assert_eq!(tone, "erro");
        assert!(title.contains("403"));
        assert_eq!(probe_notice(&Err(NetError::Timeout), "h").0, "erro");
        assert_eq!(probe_notice(&Err(NetError::Offline), "h").0, "aviso");
        let p = Probe { models: vec![("lfm3-vl-2b".into(), ModelState::SumMismatch)], sums_found: true };
        assert_eq!(probe_notice(&Ok(p), "h").0, "erro");
    }

    #[test]
    fn an_invalid_configuration_is_never_shown_as_the_public_source() {
        use kalango_modelos::config::ConfigError;
        let public = Config::default();
        assert!(card_subtitle(false, &public, "").to_lowercase().contains("catál") || card_subtitle(false, &public, "").contains("public"));
        assert_eq!(card_subtitle(true, &public, ""), tr("Settings not valid"));
        assert_ne!(card_subtitle(true, &public, ""), card_subtitle(false, &public, ""));
        // the dialog starts from nothing, not from the public source
        assert_eq!(dialog_prefill(&Err(ConfigError::MissingServer)), None);
        assert_eq!(dialog_prefill(&Ok(public.clone())), Some(public));
    }

    #[test]
    fn save_anyway_ends_with_any_edit() {
        let f = SaveAnyway::default();
        assert!(!f.forced());
        let snap = f.snapshot();
        assert!(f.failed(snap));
        assert!(f.forced());
        f.edited();
        assert!(!f.forced(), "a later edit must be tested again");
        // an edit made WHILE the test runs: its failure does not offer "save anyway" for the edited text
        let snap = f.snapshot();
        f.edited();
        assert!(!f.failed(snap));
        assert!(!f.forced());
    }

    #[test]
    fn the_five_kinds_are_the_installers() {
        assert!(is_slot("lfm3-1.7b") && is_slot("d1-2b") && !is_slot("qwen3-8b") && !is_slot("d1-omni-2b"));
    }
}
