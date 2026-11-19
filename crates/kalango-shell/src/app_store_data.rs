//! What the Central de Apps shows, read from the system (no network at open, nothing invented):
//! - apps: the Flatpak remotes' AppStream catalogs (`<root>/<remote>/<arch>/active/appstream.xml`, with the cached
//!   icons beside it), the shelf of essentials KalangoOS ships (`kalango/loja/essenciais.json`), and what is installed
//!   — by Flatpak, or the same app installed another way (a deb, a snap: the desktop ids the shelf names);
//! - agents: the agent manifests the system ships (`kalango/agentes/*.kalango-agente`), installed when a routine came
//!   from them; their cards say what they read, change, run and reach, from the permissions themselves;
//! - models: the GGUF catalog (`kalango/loja/modelos.json`) for the local llama.cpp server, installed when its files
//!   are in a models folder; whether each fits is measured on this computer (the GPU's free memory now), and the
//!   speed shown is the one measured here (the engine's token log), never an estimate.
//!
//! KALANGO_LOJA_APPSTREAM (tests): the catalogs' root (laid out as flatpak's); KALANGO_LOJA_DADOS: the folder with
//! essenciais.json and modelos.json; KALANGO_FLATPAK: a stand-in flatpak.

use crate::install_agent::Manifesto;
use gtk::glib;
use kalango_i18n::{n_, tr};
use serde::Deserialize;
use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

/// The category chips, in this order: (id, label).
pub const CATEGORIES: &[(&str, &str)] = &[
    ("", n_("All")),
    ("produtividade", n_("Productivity")),
    ("desenvolvimento", n_("Development")),
    ("criacao", n_("Creation")),
    ("sistema", n_("System")),
    ("jogos", n_("Games")),
];

pub fn flatpak() -> String {
    std::env::var("KALANGO_FLATPAK").unwrap_or_else(|_| "flatpak".into())
}

// ------------------------------------------------------------------------------------------------ the app catalog

/// An app of a Flatpak remote's catalog.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CatalogApp {
    pub id: String,
    pub name: String,
    pub summary: String,
    pub categories: Vec<String>,
    pub keywords: Vec<String>,
    pub icon: Option<PathBuf>,
    /// the remote it comes from ("flathub") and whether that remote is the person's own (`--user`) or the system's
    pub remote: String,
    pub user: bool,
}

/// The catalogs' roots: (root, the person's own installation).
fn appstream_roots() -> Vec<(PathBuf, bool)> {
    if let Some(r) = std::env::var_os("KALANGO_LOJA_APPSTREAM") {
        return vec![(PathBuf::from(r), true)];
    }
    vec![(glib::user_data_dir().join("flatpak/appstream"), true), (PathBuf::from("/var/lib/flatpak/appstream"), false)]
}

/// Every catalog file there is: (remote, user, the "active" folder).
fn catalog_files() -> Vec<(String, bool, PathBuf)> {
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        a => a,
    };
    let mut v = Vec::new();
    for (root, user) in appstream_roots() {
        let Ok(rd) = std::fs::read_dir(&root) else { continue };
        let mut remotes: Vec<_> = rd.flatten().collect();
        remotes.sort_by_key(|e| e.file_name());
        for e in remotes {
            let active = e.path().join(arch).join("active");
            if active.join("appstream.xml").is_file() {
                v.push((e.file_name().to_string_lossy().into_owned(), user, active));
            }
        }
    }
    v
}

/// The person's languages for a catalog's translated names (pt_BR, then pt).
fn lang_rank(lang: Option<&str>) -> u8 {
    let want = glib::language_names().iter().map(|s| s.to_string()).collect::<Vec<_>>();
    match lang {
        None => 1,
        Some(l) => match want.iter().position(|w| w == l || w.split(['.', '@']).next() == Some(l)) {
            Some(i) if i < 6 => 10 - i as u8,
            _ => 0,
        },
    }
}

fn entity(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        n => {
            let n = n.strip_prefix('#')?;
            let code = match n.strip_prefix('x') {
                Some(h) => u32::from_str_radix(h, 16).ok()?,
                None => n.parse().ok()?,
            };
            char::from_u32(code)
        }
    }
}

/// Reads one AppStream catalog (the desktop apps only). `rank` chooses each text's language (higher wins).
pub fn parse_appstream(xml: &str, remote: &str, user: bool, icons: &Path, rank: &dyn Fn(Option<&str>) -> u8) -> Vec<CatalogApp> {
    use quick_xml::events::Event;
    let mut r = quick_xml::Reader::from_str(xml);
    r.config_mut().trim_text(false);
    let mut out = Vec::new();
    let mut cur: Option<CatalogApp> = None;
    // the element being read: (name, xml:lang, the icon's kind and height)
    let mut path: Vec<String> = Vec::new();
    let mut text = String::new();
    let mut lang: Option<String> = None;
    let mut icon_attr: (String, u32) = (String::new(), 0);
    let (mut name_rank, mut summary_rank, mut icon_h) = (0u8, 0u8, 0u32);
    let mut desktop = false;
    loop {
        match r.read_event() {
            Ok(Event::Start(e)) => {
                let n = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                let attr = |k: &[u8]| e.attributes().flatten().find(|a| a.key.as_ref() == k).map(|a| String::from_utf8_lossy(&a.value).into_owned());
                if n == "component" && path.len() <= 1 {
                    desktop = matches!(attr(b"type").as_deref(), Some("desktop-application") | Some("desktop"));
                    cur = Some(CatalogApp { remote: remote.to_string(), user, ..Default::default() });
                    (name_rank, summary_rank, icon_h) = (0, 0, 0);
                }
                lang = attr(b"xml:lang");
                if n == "icon" {
                    icon_attr = (attr(b"type").unwrap_or_default(), attr(b"height").and_then(|h| h.parse().ok()).unwrap_or(0));
                }
                path.push(n);
                text.clear();
            }
            Ok(Event::Text(t)) => text.push_str(&t.decode().unwrap_or_default()),
            Ok(Event::CData(t)) => text.push_str(&String::from_utf8_lossy(&t)),
            Ok(Event::GeneralRef(g)) => {
                if let Some(c) = entity(&g.decode().unwrap_or_default()) {
                    text.push(c);
                }
            }
            Ok(Event::End(_)) => {
                let n = path.pop().unwrap_or_default();
                let parent = path.last().map(String::as_str).unwrap_or("");
                if let Some(c) = cur.as_mut() {
                    let t = text.trim().to_string();
                    match (n.as_str(), parent) {
                        ("id", "component") => c.id = t.trim_end_matches(".desktop").to_string(),
                        ("name", "component") => {
                            let k = rank(lang.as_deref());
                            if c.name.is_empty() || k > name_rank {
                                c.name = t;
                                name_rank = k;
                            }
                        }
                        ("summary", "component") => {
                            let k = rank(lang.as_deref());
                            if c.summary.is_empty() || k > summary_rank {
                                c.summary = t;
                                summary_rank = k;
                            }
                        }
                        ("category", "categories") => c.categories.push(t),
                        ("keyword", "keywords") if lang.is_none() || rank(lang.as_deref()) > 1 => c.keywords.push(t.to_lowercase()),
                        ("icon", "component") if icon_attr.0 == "cached" => {
                            // the biggest cached one up to 128 px (the cards draw it at 36, twice that on sharp screens)
                            let h = if icon_attr.1 == 0 { 64 } else { icon_attr.1 };
                            let p = icons.join(format!("{h}x{h}")).join(&t);
                            if h <= 128 && h > icon_h && p.is_file() {
                                c.icon = Some(p);
                                icon_h = h;
                            }
                        }
                        ("component", _) => {
                            if let Some(c) = cur.take() {
                                if desktop && !c.id.is_empty() && !c.name.is_empty() {
                                    out.push(c);
                                }
                            }
                        }
                        _ => {}
                    }
                }
                text.clear();
                lang = None;
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out
}

/// Every app of every catalog (the first remote that has an id wins).
pub fn load_catalog() -> Vec<CatalogApp> {
    let mut seen = HashSet::new();
    let mut v = Vec::new();
    for (remote, user, dir) in catalog_files() {
        let Ok(xml) = std::fs::read_to_string(dir.join("appstream.xml")) else { continue };
        for a in parse_appstream(&xml, &remote, user, &dir.join("icons"), &lang_rank) {
            if seen.insert(a.id.clone()) {
                v.push(a);
            }
        }
    }
    v
}

/// The kind of app a card says under its name ("Comunicação", "Vídeo"…), from its catalog categories.
pub fn kind_of(categories: &[String]) -> &'static str {
    let has = |c: &str| categories.iter().any(|x| x == c);
    if has("Game") {
        n_("Game")
    } else if has("InstantMessaging") || has("Chat") || has("Email") || has("VideoConference") {
        n_("Communication")
    } else if has("Video") || has("Player") && has("AudioVideo") {
        n_("Video")
    } else if has("Audio") || has("Music") {
        n_("Music")
    } else if has("Development") || has("IDE") || has("TerminalEmulator") {
        n_("Development")
    } else if has("Graphics") || has("Photography") || has("3DGraphics") {
        n_("Graphics")
    } else if has("Office") || has("WordProcessor") || has("Spreadsheet") {
        n_("Office")
    } else if has("WebBrowser") || has("Network") {
        n_("Internet")
    } else if has("Education") || has("Science") {
        n_("Education")
    } else if has("System") || has("Utility") || has("Security") {
        n_("Utilities")
    } else {
        n_("App")
    }
}

/// The chip an app belongs to (for a catalog app outside the shelf).
pub fn category_of(categories: &[String]) -> &'static str {
    let has = |c: &str| categories.iter().any(|x| x == c);
    if has("Game") {
        "jogos"
    } else if has("Development") {
        "desenvolvimento"
    } else if has("Graphics") || has("AudioVideo") || has("Audio") || has("Video") || has("Photography") {
        "criacao"
    } else if has("System") || has("Utility") || has("Security") || has("Settings") {
        "sistema"
    } else {
        "produtividade"
    }
}

// ------------------------------------------------------------------------------------------------ the shelf

/// One app of the essentials' shelf: its Flatpak id, its chip, and the desktop ids of the same app installed by other
/// means (a deb, a snap) — "Abrir" opens that one; `tipo` is what the card says when the catalog has no word for it.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct ShelfEntry {
    pub id: String,
    #[serde(default)]
    pub categoria: String,
    #[serde(default)]
    pub local: Vec<String>,
    #[serde(default)]
    pub tipo: String,
}

/// The folders with the store's own data (the shelf, the models' catalog), the person's first.
fn data_dirs() -> Vec<PathBuf> {
    if let Some(d) = std::env::var_os("KALANGO_LOJA_DADOS") {
        return vec![PathBuf::from(d)];
    }
    std::iter::once(glib::user_data_dir()).chain(glib::system_data_dirs()).map(|d| d.join("kalango/loja")).collect()
}

fn data_file(name: &str) -> Option<String> {
    data_dirs().into_iter().find_map(|d| std::fs::read_to_string(d.join(name)).ok())
}

pub fn shelf() -> Vec<ShelfEntry> {
    data_file("essenciais.json").and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

/// What Flatpak has installed (both installations).
pub fn flatpak_installed() -> HashSet<String> {
    let Ok(o) = std::process::Command::new(flatpak()).args(["list", "--app", "--columns=application"]).stderr(std::process::Stdio::null()).output() else {
        return HashSet::new();
    };
    String::from_utf8_lossy(&o.stdout).lines().filter_map(|l| l.split('\t').next()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

/// The remotes the person has in their own installation (an install there needs no administrator).
pub fn user_remotes() -> HashSet<String> {
    let Ok(o) = std::process::Command::new(flatpak()).args(["remotes", "--user", "--columns=name"]).stderr(std::process::Stdio::null()).output() else {
        return HashSet::new();
    };
    String::from_utf8_lossy(&o.stdout).lines().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

/// A desktop entry that runs this app here, if any: its own (Flatpak exports it) or one of the shelf's others.
pub fn desktop_of(id: &str, local: &[String]) -> Option<gtk::gio::DesktopAppInfo> {
    std::iter::once(id.to_string()).chain(local.iter().cloned()).find_map(|d| gtk::gio::DesktopAppInfo::new(&format!("{d}.desktop")))
}

/// The arguments of a Flatpak install of `id` from `remote` (in the person's own installation when they have that
/// remote; else the system's, which asks for the administrator's password).
pub fn install_args(remote: &str, id: &str, user: bool) -> Vec<String> {
    [if user { "--user" } else { "--system" }, "install", "--noninteractive", "-y", remote, id].iter().map(|s| s.to_string()).collect()
}

/// The last percentage in a piece of flatpak's output ("Instalando 1/2… ████  45%").
pub fn last_percent(s: &str) -> Option<f64> {
    let b = s.as_bytes();
    let mut best = None;
    for (i, &c) in b.iter().enumerate() {
        if c == b'%' {
            let mut j = i;
            while j > 0 && b[j - 1].is_ascii_digit() {
                j -= 1;
            }
            if j < i {
                if let Ok(n) = s[j..i].parse::<f64>() {
                    if n <= 100.0 {
                        best = Some(n / 100.0);
                    }
                }
            }
        }
    }
    best
}

// ------------------------------------------------------------------------------------------------ agents

/// The agents the system's catalog has: (file, manifest).
pub fn agent_catalog() -> Vec<(PathBuf, Manifesto)> {
    let dirs: Vec<PathBuf> = std::env::var_os("KALANGO_LOJA_AGENTES").map(|d| vec![PathBuf::from(d)])
        .unwrap_or_else(|| glib::system_data_dirs().into_iter().map(|d| d.join("kalango/agentes")).collect());
    let mut seen = HashSet::new();
    let mut v = Vec::new();
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "kalango-agente")).collect();
        files.sort();
        for f in files {
            if let Ok(m) = crate::install_agent::read(&f) {
                let key = if m.id.is_empty() { m.nome.clone() } else { m.id.clone() };
                if seen.insert(key) {
                    v.push((f, m));
                }
            }
        }
    }
    v
}

/// Whether a manifest sits in the system's catalog (the store's), as opposed to a file from anywhere.
pub fn in_catalog(path: &Path) -> bool {
    let dirs: Vec<PathBuf> = std::env::var_os("KALANGO_LOJA_AGENTES").map(|d| vec![PathBuf::from(d)])
        .unwrap_or_else(|| glib::system_data_dirs().into_iter().map(|d| d.join("kalango/agentes")).collect());
    let p = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    dirs.iter().any(|d| d.canonicalize().is_ok_and(|d| p.starts_with(d)))
}

/// The routine an installed agent became (by the manifest's id).
pub fn installed_agent(m: &Manifesto) -> Option<u64> {
    let r = crate::rotinas::Rotinas::carregar();
    r.lista.iter().find(|x| x.agente.as_ref().is_some_and(|a| (!m.id.is_empty() && a.origem == m.id) || (m.id.is_empty() && x.nome == m.nome))).map(|x| x.id)
}

/// The tone of a permission chip: what it means for the person.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tone {
    Plain,
    /// reaches the network (gold)
    Network,
    /// nothing leaves the computer (teal)
    Offline,
    /// sees the screen (coral)
    Screen,
}

/// The chips of an agent's card, from its permissions (never from the publisher's words): (glyph, text, tone).
pub fn chips(m: &Manifesto) -> Vec<(&'static str, String, Tone)> {
    let mut v: Vec<(&'static str, String, Tone)> = Vec::new();
    let pastas = |k: &str| -> Vec<String> { m.permissoes.iter().filter(|p| p.tipo == k).flat_map(|p| p.pastas.clone()).collect() };
    let (ler, escrever, apagar) = (pastas("ler"), pastas("escrever"), pastas("apagar"));
    // the screen first: the most personal thing an agent can ask for
    if m.permissoes.iter().any(|p| p.tipo == "tela") {
        let t = if m.quando.is_none() { tr("sees the screen when called") } else { tr("sees the screen") };
        v.push(("monitor", t.to_string(), Tone::Screen));
    }
    for f in &ler {
        if escrever.contains(f) {
            v.push(("pasta", kalango_i18n::trf("reads and changes {folder}", &[("folder", f.clone())]), Tone::Plain));
        } else {
            v.push(("olho", kalango_i18n::trf("reads {folder}", &[("folder", f.clone())]), Tone::Plain));
        }
    }
    for f in escrever.iter().filter(|f| !ler.contains(f)) {
        v.push(("pasta", kalango_i18n::trf("changes {folder}", &[("folder", f.clone())]), Tone::Plain));
    }
    for f in &apagar {
        v.push(("lixeira", kalango_i18n::trf("deletes in {folder}", &[("folder", f.clone())]), Tone::Plain));
    }
    let programas: Vec<String> = m.permissoes.iter().filter(|p| p.tipo == "programas").flat_map(|p| p.programas.clone()).collect();
    if !programas.is_empty() {
        v.push(("terminal", programas.join(", "), Tone::Plain));
    }
    let rede: Vec<&crate::install_agent::Pedido> = m.permissoes.iter().filter(|p| p.tipo == "rede").collect();
    if rede.is_empty() {
        v.push(("cadeado", tr("no network").to_string(), Tone::Offline));
    } else {
        let d: Vec<String> = rede.iter().flat_map(|p| p.dominios.clone()).collect();
        v.push(("globo", if d.is_empty() { tr("any site").to_string() } else { d.join(", ") }, Tone::Network));
    }
    v
}

/// What the agent decides alone, as its card says it.
pub fn autonomy_text(a: &str) -> &'static str {
    match a {
        "sempre" => tr("asks before every change"),
        "autonomo" => tr("acts alone within its limits"),
        _ => tr("asks before what is risky"),
    }
}

// ------------------------------------------------------------------------------------------------ models

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct ModelFile {
    pub url: String,
    pub nome: String,
    pub bytes: u64,
}

/// A model of the catalog: its files (the weights and, for voice and vision, the projector).
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct ModelEntry {
    pub id: String,
    pub nome: String,
    pub descricao: String,
    /// "texto", "voz" or "visao"
    #[serde(default)]
    pub tipo: String,
    pub arquivos: Vec<ModelFile>,
}

impl ModelEntry {
    pub fn bytes(&self) -> u64 {
        self.arquivos.iter().map(|f| f.bytes).sum()
    }
    /// The weights' file (not the projector).
    pub fn main_file(&self) -> Option<&ModelFile> {
        self.arquivos.iter().find(|f| !f.nome.contains("mmproj")).or(self.arquivos.first())
    }
}

pub fn model_catalog() -> Vec<ModelEntry> {
    data_file("modelos.json").and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

/// Where the models are (the person's, the system's) and where a download goes (the person's).
pub fn models_dirs() -> Vec<PathBuf> {
    std::env::var_os("KALANGO_MODELOS").map(PathBuf::from).into_iter()
        .chain([glib::user_data_dir().join("kalango/modelos"), PathBuf::from("/usr/share/kalango/modelos")]).collect()
}

pub fn download_dir() -> PathBuf {
    glib::user_data_dir().join("kalango/modelos")
}

pub fn model_installed(m: &ModelEntry) -> bool {
    !m.arquivos.is_empty() && m.arquivos.iter().all(|f| models_dirs().iter().any(|d| d.join(&f.nome).is_file()))
}

/// The speed measured here for a model (the median of the engine's last 50 answers with it), in tokens per second.
pub fn measured_speed(file: &str) -> Option<f64> {
    let stem = file.trim_end_matches(".gguf");
    let log = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).filter(|p| p.is_absolute())
        .unwrap_or_else(|| glib::home_dir().join(".local/state")).join("kalango/uso-tokens.jsonl");
    let t = std::fs::read_to_string(log).ok()?;
    let mut v: Vec<f64> = t.lines().rev().filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|j| j["modelo"].as_str() == Some(stem)).filter_map(|j| j["tok_s"].as_f64()).filter(|x| *x > 0.0).take(50).collect();
    if v.is_empty() {
        return None;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    Some(v[v.len() / 2])
}

/// Whether a model fits this computer now.
#[derive(Clone, Debug, PartialEq)]
pub enum Fit {
    /// in the graphics card's free memory with room to spare
    Roomy,
    Fits,
    /// only if what uses the graphics card now is closed: (free, total) bytes
    NotNow(f64, f64),
    /// bigger than the whole card: it runs partly on the processor, slowly
    TooBig(f64),
    /// no graphics card: on the processor (fits the memory or not)
    Cpu(bool),
}

/// `bytes` of weights need about a fifth more for the context.
pub fn fit(bytes: u64, gpu: Option<(f64, f64)>, ram_free: f64) -> Fit {
    let need = bytes as f64 * 1.2;
    match gpu {
        Some((used, total)) => {
            let free = (total - used).max(0.0);
            if need <= free * 0.6 {
                Fit::Roomy
            } else if need <= free {
                Fit::Fits
            } else if need <= total {
                Fit::NotNow(free, total)
            } else {
                Fit::TooBig(total)
            }
        }
        None => Fit::Cpu(need <= ram_free),
    }
}

/// "1,1 GB" (decimal, as disks and downloads count).
pub fn gb(bytes: f64) -> String {
    if bytes < 1e9 {
        return format!("{:.0} MB", (bytes / 1e6).max(1.0));
    }
    let g = bytes / 1e9;
    let s = if g >= 100.0 { format!("{g:.0}") } else { format!("{g:.1}") };
    format!("{} GB", s.replace('.', ","))
}

/// A graphics card's memory as it is sold ("24 GB": binary gigabytes, whole).
pub fn vram_gb(bytes: f64) -> String {
    format!("{:.0} GB", bytes / 1073741824.0)
}

/// The line under a model: (text, good news).
pub fn fit_text(f: &Fit, speed: Option<f64>) -> (String, bool) {
    let (t, ok) = match f {
        Fit::Roomy => (tr("fits with room to spare").to_string(), true),
        Fit::Fits => (tr("fits your GPU").to_string(), true),
        Fit::NotNow(free, total) => (kalango_i18n::trf("does not fit now: {free} free of {total} on the GPU", &[("free", vram_gb(*free)), ("total", vram_gb(*total))]), false),
        Fit::TooBig(total) => (kalango_i18n::trf("bigger than the {total} GPU: runs slowly, partly on the CPU", &[("total", vram_gb(*total))]), false),
        Fit::Cpu(true) => (tr("runs on the CPU (no GPU)").to_string(), true),
        Fit::Cpu(false) => (tr("does not fit the free memory").to_string(), false),
    };
    match speed {
        Some(s) if ok => (format!("{t} · {} tok/s", s.round() as i64), ok),
        _ => (t, ok),
    }
}

/// A download running: how much came, cancel.
#[derive(Clone, Default)]
pub struct Transfer {
    pub done: Arc<AtomicU64>,
    pub total: u64,
    pub cancel: Arc<AtomicBool>,
}

/// Downloads a model's files into `dir` (each through a ".part" file, renamed when whole). `file://` sources are
/// copied (a local mirror). Off the interface's thread.
pub fn download(files: &[ModelFile], dir: &Path, t: &Transfer) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut base = 0u64;
    for f in files {
        let dest = dir.join(&f.nome);
        if dest.is_file() {
            base += f.bytes;
            t.done.store(base, Ordering::Relaxed);
            continue;
        }
        let part = dir.join(format!("{}.part", f.nome));
        let mut reader: Box<dyn Read> = match f.url.strip_prefix("file://") {
            Some(p) => Box::new(std::fs::File::open(p).map_err(|e| format!("{p}: {e}"))?),
            None => Box::new(ureq::get(&f.url).call().map_err(|e| e.to_string())?.into_reader()),
        };
        let mut out = std::fs::File::create(&part).map_err(|e| e.to_string())?;
        let mut buf = vec![0u8; 1 << 16];
        let mut got = 0u64;
        loop {
            if t.cancel.load(Ordering::Relaxed) {
                drop(out);
                let _ = std::fs::remove_file(&part);
                return Err("cancelado".into());
            }
            let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            got += n as u64;
            t.done.store(base + got, Ordering::Relaxed);
        }
        out.flush().map_err(|e| e.to_string())?;
        if f.bytes > 0 && got != f.bytes {
            let _ = std::fs::remove_file(&part);
            return Err(format!("{}: {got} de {} bytes", f.nome, f.bytes));
        }
        std::fs::rename(&part, &dest).map_err(|e| e.to_string())?;
        base += got;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<components version="0.8" origin="flathub">
  <component type="desktop-application">
    <id>com.discordapp.Discord</id>
    <name>Discord</name>
    <name xml:lang="pt_BR">Discord BR</name>
    <summary>Talk, play &amp; hang out</summary>
    <categories><category>Network</category><category>InstantMessaging</category></categories>
    <keywords><keyword>chat</keyword></keywords>
    <icon type="cached" height="64" width="64">com.discordapp.Discord.png</icon>
    <releases><release version="1"><description><p>x</p></description></release></releases>
  </component>
  <component type="runtime"><id>org.freedesktop.Platform</id><name>Platform</name></component>
  <component type="desktop">
    <id>org.videolan.VLC.desktop</id>
    <name>VLC</name>
    <summary>Video player</summary>
    <categories><category>AudioVideo</category><category>Video</category></categories>
  </component>
</components>"#;

    #[test]
    fn reads_desktop_apps_with_the_person_language() {
        let rank = |l: Option<&str>| match l {
            None => 1,
            Some("pt_BR") => 9,
            _ => 0,
        };
        let v = parse_appstream(XML, "flathub", false, Path::new("/nao/existe"), &rank);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].id, "com.discordapp.Discord");
        assert_eq!(v[0].name, "Discord BR");
        assert_eq!(v[0].summary, "Talk, play & hang out");
        assert_eq!(v[0].categories, vec!["Network", "InstantMessaging"]);
        assert_eq!(v[0].keywords, vec!["chat"]);
        assert_eq!(v[0].icon, None);
        assert_eq!(v[1].id, "org.videolan.VLC");
        assert_eq!(kind_of(&v[0].categories), "Communication");
        assert_eq!(kind_of(&v[1].categories), "Video");
        assert_eq!(category_of(&v[1].categories), "criacao");
    }

    #[test]
    fn fit_and_sizes() {
        let g = 1e9;
        assert_eq!(fit((1.1 * g) as u64, Some((7.0 * g, 24.0 * g)), 0.0), Fit::Roomy);
        assert_eq!(fit((12.0 * g) as u64, Some((7.0 * g, 24.0 * g)), 0.0), Fit::Fits);
        assert_eq!(fit((19.8 * g) as u64, Some((7.0 * g, 24.0 * g)), 0.0), Fit::NotNow(17.0 * g, 24.0 * g));
        assert_eq!(fit((30.0 * g) as u64, Some((7.0 * g, 24.0 * g)), 0.0), Fit::TooBig(24.0 * g));
        assert_eq!(fit((4.0 * g) as u64, None, 16.0 * g), Fit::Cpu(true));
        assert_eq!(gb(1_099_627_808.0), "1,1 GB");
        assert_eq!(gb(19_762_149_024.0), "19,8 GB");
        assert_eq!(gb(3_145_728.0), "3 MB");
        assert_eq!(last_percent("Instalando… 12%\rInstalando… 45%"), Some(0.45));
        assert_eq!(last_percent("nada"), None);
        assert_eq!(install_args("flathub", "org.videolan.VLC", true), vec!["--user", "install", "--noninteractive", "-y", "flathub", "org.videolan.VLC"]);
    }

    #[test]
    fn chips_come_from_the_permissions() {
        let m: Manifesto = toml::from_str(r#"
nome = "Organizador"
instrucoes = "x"
[[permissoes]]
tipo = "ler"
pastas = ["~/Downloads"]
[[permissoes]]
tipo = "escrever"
pastas = ["~/Downloads"]
"#).unwrap();
        let c = chips(&m);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].0, "pasta");
        assert_eq!(c[1].2, Tone::Offline);
        let m: Manifesto = toml::from_str(r#"
nome = "Dev"
instrucoes = "x"
[[permissoes]]
tipo = "ler"
pastas = ["~/Projetos"]
[[permissoes]]
tipo = "programas"
programas = ["git"]
[[permissoes]]
tipo = "rede"
dominios = ["github.com"]
"#).unwrap();
        let c = chips(&m);
        assert_eq!(c.iter().map(|x| x.0).collect::<Vec<_>>(), vec!["olho", "terminal", "globo"]);
        assert_eq!(c[2].1, "github.com");
        assert_eq!(c[2].2, Tone::Network);
    }

    #[test]
    fn downloads_a_local_mirror_through_a_part_file() {
        let d = std::env::temp_dir().join(format!("kalango-loja-teste-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let src = d.join("fonte.gguf");
        std::fs::write(&src, vec![7u8; 200_000]).unwrap();
        let files = vec![ModelFile { url: format!("file://{}", src.display()), nome: "m.gguf".into(), bytes: 200_000 }];
        let t = Transfer::default();
        download(&files, &d.join("modelos"), &t).unwrap();
        assert_eq!(std::fs::metadata(d.join("modelos/m.gguf")).unwrap().len(), 200_000);
        assert!(!d.join("modelos/m.gguf.part").exists());
        assert_eq!(t.done.load(Ordering::Relaxed), 200_000);
        let _ = std::fs::remove_dir_all(&d);
    }
}
