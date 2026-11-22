//! Settings › System (designs "Configurações · Sobre", "Atualizações", "Armazenamento", "Usuários", "Data e idioma",
//! "Apps padrão", "Impressoras"): seven tabs in the section's frame, each with its panel.
//!
//! - About: the hardware and software as they are (CPU, memory, graphics, disks, board, screens; the system, the
//!   compositor, the shell, the kernel, the local model); the computer's name (changed through kalangod, approved).
//! - Updates: what PackageKit has prepared and the packages with a new version (kalangod), installed on restart or
//!   when powering off; the history from the timeline, with its undo.
//! - Storage: what fills the system disk, by kind, and the other disks; what can be freed (caches, the trash, models
//!   not used lately, copies) and the automatic clean-up of old trash.
//! - Users: the accounts (uid ≥ 1000), who is an administrator; a new account or removing one (kalangod, approved).
//! - Date and language: the time (from the network or not, the zone — kalangod), the top bar's clock; the system's
//!   language and formats (kalangod).
//! - Default apps: what opens a link, an e-mail, music, video, pictures, text, the terminal (GIO's defaults); the apps
//!   that start with the session.
//! - Printers: CUPS's printers and their queues (cancel a job, a test page, the default one).

use crate::configuracoes::{depurando, interruptor, rotulo_cartao};
use crate::configuracoes_moldura::{linha_valor, Moldura};
use crate::conversa::ui_icon;
use crate::superficies::{caixa, estrela, rotulo};
use gtk::prelude::*;
use gtk::{gio, glib, Align, Orientation};
use kalango_i18n::{n_, tr, trf};
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};

const TABS: &[(&str, &str)] = &[("sobre", n_("About")), ("atualizacoes", n_("Updates")), ("armazenamento", n_("Storage")), ("usuarios", n_("Users")),
    ("data", n_("Date and language")), ("apps", n_("Default apps")), ("impressoras", n_("Printers"))];

fn left(t: &str, c: &str) -> gtk::Label {
    let l = rotulo(t, c);
    l.set_xalign(0.0);
    l
}

fn clear(b: &gtk::Box) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
}

fn row(title: &str, sub: Option<&str>, control: &impl IsA<gtk::Widget>) -> gtk::Box {
    let r = caixa(Orientation::Horizontal, 12, "cfg-tela-opcao");
    let t = caixa(Orientation::Vertical, 1, "");
    t.set_hexpand(true);
    t.set_valign(Align::Center);
    t.append(&left(title, "cfg-texto-13"));
    if let Some(s) = sub {
        let l = left(s, "cfg-sub");
        l.set_ellipsize(gtk::pango::EllipsizeMode::End);
        l.set_max_width_chars(1);
        t.append(&l);
        r.add_css_class("alta");
    }
    r.append(&t);
    control.set_valign(Align::Center);
    r.append(control);
    r
}

/// A card's key/value line (the designs' "Processador … 8 núcleos · 16 threads").
fn kv(card: &gtk::Box, k: &str) -> gtk::Label {
    let (l, v) = linha_valor(k);
    l.add_css_class("cfg-kv");
    v.set_max_width_chars(48);
    card.append(&l);
    v
}

pub(crate) fn human(bytes: u64) -> String {
    let b = bytes as f64;
    let (v, u) = if b >= 1e12 { (b / 1e12, "TB") } else if b >= 1e9 { (b / 1e9, "GB") } else if b >= 1e6 { (b / 1e6, "MB") } else { (b / 1e3, "KB") };
    let s = if v >= 100.0 || u == "KB" { format!("{v:.0}") } else { format!("{v:.1}").replace('.', ",") };
    format!("{} {u}", s.trim_end_matches(",0"))
}

fn sh(prog: &str, args: &[&str]) -> Option<String> {
    // the C locale: the programs' labels are read here (lscpu, timedatectl)
    let o = std::process::Command::new(prog).args(args).env("LC_ALL", "C").stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).into_owned())
}

// ---------------------------------------------------------------- About: what this computer is

#[derive(Default, Clone)]
struct About {
    cpu: String,
    memory: String,
    gpu: String,
    disks: String,
    board: String,
    screens: String,
    system: String,
    compositor: String,
    shell: String,
    kernel: String,
    model: String,
    hostname: String,
}

fn read_about() -> About {
    let cpuinfo = std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    let name = cpuinfo.lines().find_map(|l| l.strip_prefix("model name")?.split(':').nth(1).map(|s| s.trim().to_string())).unwrap_or_default();
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    let lscpu = sh("lscpu", &[]).unwrap_or_default();
    let field = |k: &str| lscpu.lines().find_map(|l| l.strip_prefix(k).map(|v| v.trim().to_string()));
    let cores = field("Core(s) per socket:").and_then(|c| c.parse::<usize>().ok()).unwrap_or(threads) * field("Socket(s):").and_then(|s| s.parse::<usize>().ok()).unwrap_or(1);
    let ghz = field("CPU max MHz:").and_then(|m| m.replace(',', ".").parse::<f64>().ok()).map(|m| m / 1000.0).unwrap_or_else(|| crate::sistema::nucleos_ghz().1);
    // the name without its maker's marks and the integrated graphics ("AMD Ryzen 7 5700G")
    let short = name.replace("(R)", "").replace("(TM)", "").split(" with ").next().unwrap_or("").split(" @ ").next().unwrap_or("").replace(" Processor", "").replace(" CPU", "");
    let cpu = format!("{} · {}", short.split_whitespace().collect::<Vec<_>>().join(" "),
        trf("{cores} cores · {threads} threads · {ghz} GHz", &[("cores", cores.to_string()), ("threads", threads.to_string()), ("ghz", format!("{ghz:.1}").replace('.', ","))]));
    let mem_kb = std::fs::read_to_string("/proc/meminfo").ok().and_then(|m| m.lines().find_map(|l| l.strip_prefix("MemTotal:")?.split_whitespace().next()?.parse::<u64>().ok())).unwrap_or(0);
    let memory = format!("{} GB", ((mem_kb as f64) / 1024.0 / 1024.0).round());
    let gpu = sh("nvidia-smi", &["--query-gpu=name,memory.total", "--format=csv,noheader,nounits"])
        .and_then(|o| {
            let l = o.lines().next()?.to_string();
            let mut p = l.split(", ");
            let n = p.next()?.replace("NVIDIA GeForce ", "").replace("NVIDIA ", "");
            let mb: f64 = p.next()?.trim().parse().ok()?;
            Some(trf("{name} · {gb} GB of VRAM", &[("name", n), ("gb", format!("{:.0}", mb / 1024.0))]))
        })
        .or_else(|| sh("lspci", &[]).and_then(|o| o.lines().find(|l| l.contains("VGA") || l.contains("3D controller")).and_then(|l| l.split(": ").nth(1)).map(str::to_string)))
        .unwrap_or_default();
    // the disks: their kind and size ("NVMe 1 TB + SSD 2 TB")
    let disks = sh("lsblk", &["-d", "-b", "-J", "-o", "NAME,SIZE,ROTA,TRAN,TYPE"]).and_then(|o| serde_json::from_str::<Value>(&o).ok())
        .map(|v| v["blockdevices"].as_array().cloned().unwrap_or_default().iter().filter(|d| d["type"] == "disk" && !d["name"].as_str().unwrap_or("").starts_with("zram"))
            .map(|d| {
                let size = d["size"].as_u64().or_else(|| d["size"].as_str().and_then(|s| s.parse().ok())).unwrap_or(0);
                let kind = if d["tran"] == "nvme" || d["name"].as_str().unwrap_or("").starts_with("nvme") { "NVMe" } else if d["rota"] == true || d["rota"] == "1" { "HDD" } else if d["tran"] == "usb" { "USB" } else { "SSD" };
                format!("{kind} {}", human(size))
            }).collect::<Vec<_>>().join(" + "))
        .unwrap_or_default();
    let dmi = |f: &str| std::fs::read_to_string(format!("/sys/class/dmi/id/{f}")).map(|s| s.trim().to_string()).unwrap_or_default();
    // the maker in a word ("Micro-Star International Co., Ltd." → "MSI")
    let vendor = dmi("board_vendor");
    let vendor = match vendor.as_str() {
        v if v.starts_with("Micro-Star") => "MSI".to_string(),
        v if v.starts_with("ASUSTeK") => "ASUS".to_string(),
        v if v.starts_with("Gigabyte") => "Gigabyte".to_string(),
        v if v.starts_with("ASRock") => "ASRock".to_string(),
        v => v.split([',', ' ']).next().unwrap_or("").to_string(),
    };
    let board = [vendor, dmi("board_name")].into_iter().filter(|s| !s.is_empty() && !s.contains("To be filled")).collect::<Vec<_>>().join(" ");
    let screens = String::new();
    let os = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    let osv = |k: &str| os.lines().find_map(|l| l.strip_prefix(&format!("{k}="))).map(|v| v.trim_matches('"').to_string()).unwrap_or_default();
    let pretty = osv("PRETTY_NAME");
    let system = if osv("ID") == "kalangoos" { pretty } else { trf("KalangoOS {version} (based on {base})", &[("version", env!("CARGO_PKG_VERSION").to_string()), ("base", pretty)]) };
    let compositor = format!("niri {} · Wayland", crate::servicos::niri_versao().unwrap_or_default());
    let shell = format!("kalango-shell {} (Rust)", env!("CARGO_PKG_VERSION"));
    let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease").map(|k| format!("Linux {}", k.trim())).unwrap_or_default();
    let model = std::env::var("KALANGO_MODELO").ok().map(PathBuf::from)
        .or_else(|| std::fs::read_dir(glib::user_data_dir().join("kalango/modelos")).ok()?.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "gguf") && !p.to_string_lossy().contains("mmproj")).min())
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .map(|n| format!("{} · llama.cpp", n.replace(['-', '_'], " ").replace(" Q4 K M", "").replace(" Q8 0", "")))
        .unwrap_or_else(|| tr("none yet").to_string());
    let hostname = std::fs::read_to_string("/etc/hostname").map(|h| h.trim().to_string()).unwrap_or_else(|_| glib::host_name().to_string());
    About { cpu, memory, gpu, disks, board, screens, system, compositor, shell, kernel, model, hostname }
}

// ---------------------------------------------------------------- Storage

#[derive(Default, Clone)]
struct Storage {
    used: u64,
    free: u64,
    total: u64,
    /// (name, bytes, tone, folder to open)
    kinds: Vec<(String, u64, &'static str, Option<PathBuf>)>,
    /// (name, used, total)
    others: Vec<(String, u64, u64)>,
    cache: u64,
    trash: (u64, usize),
    old_models: (u64, usize),
    /// copies in Downloads (the same content twice): bytes they take, how many
    copies: (u64, usize),
}

fn dir_size(p: &Path, limit: &mut u32) -> u64 {
    let mut total = 0;
    let mut stack = vec![p.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            if *limit == 0 {
                return total;
            }
            *limit -= 1;
            let Ok(m) = e.metadata() else { continue };
            if m.is_dir() {
                stack.push(e.path());
            } else {
                use std::os::unix::fs::MetadataExt;
                total += m.blocks() * 512;
            }
        }
    }
    total
}

pub(crate) fn statvfs(p: &Path) -> Option<(u64, u64)> {
    let i = gio::File::for_path(p).query_filesystem_info("filesystem::size,filesystem::free", gio::Cancellable::NONE).ok()?;
    Some((i.attribute_uint64("filesystem::size"), i.attribute_uint64("filesystem::free")))
}

fn read_storage() -> Storage {
    let (total, free) = statvfs(Path::new("/")).unwrap_or((0, 0));
    let special = |d| glib::user_special_dir(d).filter(|p| p.is_dir() && *p != glib::home_dir());
    let videos = special(glib::UserDirectory::Videos);
    let models_dir = glib::user_data_dir().join("kalango/modelos");
    let projects = [glib::home_dir().join("Documentos/repositorios"), glib::home_dir().join("Projetos"), glib::home_dir().join("projects"), glib::home_dir().join("code")]
        .into_iter().find(|p| p.is_dir());
    let mut kinds = Vec::new();
    if let Some(v) = &videos {
        kinds.push((tr("Videos").to_string(), dir_size(v, &mut 300_000), "roxo", Some(v.clone())));
    }
    if models_dir.is_dir() {
        kinds.push((tr("AI models").to_string(), dir_size(&models_dir, &mut 300_000), "verde", Some(models_dir.clone())));
    }
    if let Some(p) = &projects {
        kinds.push((tr("Projects").to_string(), dir_size(p, &mut 300_000), "azul", Some(p.clone())));
    }
    // the installed packages' own sizes (dpkg keeps them), then flatpaks and snaps
    let debs: u64 = sh("dpkg-query", &["-W", "-f", "${Installed-Size}\\n"]).map(|o| o.lines().filter_map(|l| l.trim().parse::<u64>().ok()).sum::<u64>() * 1024).unwrap_or(0);
    let apps: u64 = debs + ["/var/lib/flatpak", "/var/lib/snapd/snaps"].iter().map(|p| dir_size(Path::new(p), &mut 100_000)).sum::<u64>();
    kinds.push((tr("Apps").to_string(), apps, "ouro", None));
    let used = total.saturating_sub(free);
    let known: u64 = kinds.iter().map(|k| k.1).sum();
    kinds.push((tr("System and others").to_string(), used.saturating_sub(known), "coral", None));
    // the other disks: what is mounted from them
    let mut others = Vec::new();
    for m in gio::UnixMountEntry::mounts().0 {
        let path = m.mount_path();
        let dev = m.device_path().to_string_lossy().into_owned();
        if !dev.starts_with("/dev/") || dev.contains("loop") || path == Path::new("/") || path.starts_with("/boot") || path.starts_with("/snap") {
            continue;
        }
        if let Some((t, f)) = statvfs(&path) {
            if t > 1 << 30 && !others.iter().any(|(_, _, tt): &(String, u64, u64)| *tt == t) {
                let name = path.file_name().map(|n| n.to_string_lossy().replace('_', " ")).unwrap_or_else(|| dev.clone());
                let mut c = name.chars();
                let name = c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default();
                others.push((name, t - f, t));
            }
        }
    }
    let mut l2 = 200_000u32;
    let cache = dir_size(&glib::user_cache_dir(), &mut l2);
    let trash_dir = glib::user_data_dir().join("Trash/files");
    let trash_n = std::fs::read_dir(&trash_dir).map(|d| d.count()).unwrap_or(0);
    let mut l3 = 200_000u32;
    let trash = (dir_size(&trash_dir, &mut l3), trash_n);
    // models no one has used for 60 days (by their last access)
    let now = std::time::SystemTime::now();
    let mut old = (0u64, 0usize);
    for e in std::fs::read_dir(&models_dir).into_iter().flatten().flatten() {
        let Ok(m) = e.metadata() else { continue };
        if m.is_file() && m.accessed().ok().and_then(|a| now.duration_since(a).ok()).is_some_and(|d| d.as_secs() > 60 * 86_400) {
            old.0 += m.len();
            old.1 += 1;
        }
    }
    Storage { used, free, total, kinds, others, cache, trash, old_models: old, copies: copies_in_downloads() }
}

/// The same content twice in Downloads (by size, then SHA-256 under 256 MB): what the copies take.
fn copies_in_downloads() -> (u64, usize) {
    let Some(d) = glib::user_special_dir(glib::UserDirectory::Downloads).filter(|p| p.is_dir() && *p != glib::home_dir()) else { return (0, 0) };
    let mut by_size: std::collections::HashMap<u64, Vec<PathBuf>> = std::collections::HashMap::new();
    for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
        if let Ok(m) = e.metadata() {
            if m.is_file() && m.len() > 0 && m.len() < 256 << 20 {
                by_size.entry(m.len()).or_default().push(e.path());
            }
        }
    }
    let (mut bytes, mut n) = (0u64, 0usize);
    for (size, ps) in by_size.into_iter().filter(|(_, v)| v.len() > 1) {
        let mut seen = std::collections::HashSet::new();
        for p in ps {
            let h = std::fs::read(&p).ok().and_then(|b| glib::compute_checksum_for_data(glib::ChecksumType::Sha256, &b)).map(|s| s.to_string());
            if let Some(h) = h {
                if !seen.insert(h) {
                    bytes += size;
                    n += 1;
                }
            }
        }
    }
    (bytes, n)
}

/// Settings › System › Storage's warning: once a day, a notice when the system disk's free part falls under the chosen
/// share (the shell's minute timer asks).
pub fn watch_space() {
    let a = crate::configuracoes::ajustes();
    let pct = a["aviso_espaco"].as_u64().unwrap_or(10);
    if pct == 0 {
        return;
    }
    let today = glib::DateTime::now_local().ok().and_then(|d| d.format("%F").ok()).map(|s| s.to_string()).unwrap_or_default();
    if a["aviso_espaco_dia"].as_str() == Some(today.as_str()) {
        return;
    }
    let Some((total, free)) = statvfs(Path::new("/")) else { return };
    if total == 0 || free * 100 / total >= pct {
        return;
    }
    crate::configuracoes::salvar_ajuste("aviso_espaco_dia", json!(today));
    if let Some(s) = crate::shell_atual() {
        s.notificar(tr("System"), tr("Little space on the disk"), &trf("{free} free of {total}. Settings › System › Storage shows what can be freed.", &[("free", human(free)), ("total", human(total))]));
    }
}

/// The trash's files deleted more than 30 days ago (their .trashinfo's date), for the automatic clean-up.
pub fn clean_old_trash() -> usize {
    let base = glib::user_data_dir().join("Trash");
    let now = glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0);
    let mut n = 0;
    for e in std::fs::read_dir(base.join("info")).into_iter().flatten().flatten() {
        let info = std::fs::read_to_string(e.path()).unwrap_or_default();
        let Some(date) = info.lines().find_map(|l| l.strip_prefix("DeletionDate=")) else { continue };
        let Ok(dt) = glib::DateTime::from_iso8601(date, Some(&glib::TimeZone::local())) else { continue };
        if now - dt.to_unix() < 30 * 86_400 {
            continue;
        }
        let name = e.path().file_stem().map(|s| s.to_os_string()).unwrap_or_default();
        let f = base.join("files").join(&name);
        let ok = if f.is_dir() { std::fs::remove_dir_all(&f) } else { std::fs::remove_file(&f) };
        if ok.is_ok() || !f.exists() {
            let _ = std::fs::remove_file(e.path());
            n += 1;
        }
    }
    n
}

// ---------------------------------------------------------------- Users

#[derive(Clone, Debug)]
struct Account {
    login: String,
    name: String,
    admin: bool,
    me: bool,
    home: PathBuf,
}

fn read_accounts() -> Vec<Account> {
    let groups = std::fs::read_to_string("/etc/group").unwrap_or_default();
    let admins: Vec<String> = groups.lines().filter(|l| l.starts_with("sudo:") || l.starts_with("wheel:") || l.starts_with("admin:"))
        .flat_map(|l| l.rsplit(':').next().unwrap_or("").split(',').map(str::to_string).collect::<Vec<_>>()).collect();
    let me = glib::user_name().to_string_lossy().into_owned();
    std::fs::read_to_string("/etc/passwd").unwrap_or_default().lines().filter_map(|l| {
        let f: Vec<&str> = l.split(':').collect();
        let uid: u32 = f.get(2)?.parse().ok()?;
        if !(1000..60000).contains(&uid) || f.get(6).is_some_and(|s| s.ends_with("nologin") || s.ends_with("false")) {
            return None;
        }
        let login = f[0].to_string();
        let name = f.get(4).and_then(|g| g.split(',').next()).filter(|n| !n.is_empty()).map(str::to_string).unwrap_or_else(|| login.clone());
        Some(Account { admin: admins.contains(&login), me: login == me, home: PathBuf::from(f.get(5).copied().unwrap_or("/")), login, name })
    }).collect()
}

// ---------------------------------------------------------------- Default apps

/// The kinds of file the "Default apps" tab chooses an app for: (name, content types; the first is the one set).
const DEFAULTS: &[(&str, &[&str])] = &[
    (n_("Browser"), &["x-scheme-handler/http", "x-scheme-handler/https", "text/html"]),
    (n_("E-mail"), &["x-scheme-handler/mailto"]),
    (n_("Music"), &["audio/mpeg", "audio/flac", "audio/ogg", "audio/x-wav"]),
    (n_("Video"), &["video/mp4", "video/x-matroska", "video/webm"]),
    (n_("Pictures"), &["image/png", "image/jpeg", "image/webp"]),
    (n_("Text and code"), &["text/plain", "text/markdown", "text/x-rust", "text/x-python"]),
    (n_("PDF documents"), &["application/pdf"]),
];

// ---------------------------------------------------------------- Printers (CUPS)

#[derive(Clone, Debug, Default)]
struct Printer {
    name: String,
    state: String,
    ready: bool,
    default: bool,
    uri: String,
    jobs: Vec<(String, String, String)>,
}

fn read_printers() -> Vec<Printer> {
    let Some(p) = sh("lpstat", &["-p"]) else { return vec![] };
    let default = sh("lpstat", &["-d"]).and_then(|d| d.split(':').nth(1).map(|s| s.trim().to_string())).unwrap_or_default();
    let uris = sh("lpstat", &["-v"]).unwrap_or_default();
    let jobs = sh("lpstat", &["-o"]).unwrap_or_default();
    p.lines().filter_map(|l| l.strip_prefix("printer ")).map(|l| {
        let name = l.split_whitespace().next().unwrap_or("").to_string();
        let ready = l.contains("idle") || l.contains("ociosa") || l.contains("inativa");
        let state = if l.contains("disabled") || l.contains("desabilitada") { tr("paused").to_string() } else if ready { tr("ready").to_string() } else { tr("printing").to_string() };
        let uri = uris.lines().find(|u| u.contains(&format!(" {name}:"))).and_then(|u| u.split(": ").nth(1)).unwrap_or("").to_string();
        let pjobs = jobs.lines().filter(|j| j.starts_with(&format!("{name}-"))).map(|j| {
            let f: Vec<&str> = j.split_whitespace().collect();
            (f.first().unwrap_or(&"").to_string(), f.get(1).unwrap_or(&"").to_string(), f.get(2).unwrap_or(&"").to_string())
        }).collect();
        Printer { default: name == default, name, state, ready, uri, jobs: pjobs }
    }).collect()
}

// ---------------------------------------------------------------- the page

pub struct SystemPage {
    pub frame: Rc<Moldura>,
    settings: Weak<crate::configuracoes::Configuracoes>,
    body: gtk::Stack,
    panel: gtk::Stack,
    tab: RefCell<String>,
    loading: Cell<bool>,
    // about
    hw: gtk::Box,
    sw: gtk::Box,
    about: RefCell<About>,
    host: gtk::Entry,
    host_note: gtk::Label,
    version: gtk::Label,
    wallpaper: gtk::DrawingArea,
    // updates
    upd_main: gtk::Box,
    upd_history: gtk::Box,
    auto_update: gtk::Switch,
    off_hours: gtk::Switch,
    restore_point: gtk::Switch,
    // storage
    st_main: gtk::Box,
    st_others: gtk::Box,
    st_clean: gtk::Box,
    auto_clean: gtk::Switch,
    storage: RefCell<Option<Storage>>,
    // users
    accounts: gtk::Box,
    account_panel: gtk::Box,
    chosen: RefCell<Option<String>>,
    // date and language
    clock: gtk::Label,
    date: gtk::Label,
    ntp: gtk::Switch,
    zone: gtk::DropDown,
    zones: RefCell<Vec<String>>,
    h24: gtk::ToggleButton,
    h12: gtk::ToggleButton,
    seconds: gtk::Switch,
    language: gtk::DropDown,
    languages: RefCell<Vec<String>>,
    formats: gtk::DropDown,
    first_day: gtk::DropDown,
    answers: gtk::DropDown,
    others: gtk::Box,
    warn_space: gtk::DropDown,
    apps_models: gtk::Box,
    // apps
    defaults: gtk::Box,
    installed: gtk::Box,
    app_search: gtk::Entry,
    autostart: gtk::Box,
    // printers
    printers: gtk::Box,
    printer_panel: gtk::Box,
    printer_chosen: RefCell<Option<String>>,
}

impl SystemPage {
    pub fn new(c: &Rc<crate::configuracoes::Configuracoes>) -> Rc<SystemPage> {
        let m = Moldura::new(tr("System"), TABS, tr("This computer"));
        let body = gtk::Stack::new();
        body.set_vhomogeneous(false);
        body.set_transition_type(gtk::StackTransitionType::Crossfade);
        body.set_transition_duration(160);
        body.set_vexpand(true);
        m.corpo.append(&body);
        let panel = gtk::Stack::new();
        panel.set_vhomogeneous(false);
        panel.set_transition_type(gtk::StackTransitionType::Crossfade);
        panel.set_transition_duration(160);
        m.painel.append(&panel);
        let scroller = |w: &gtk::Box| {
            let s = gtk::ScrolledWindow::new();
            s.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
            s.set_vexpand(true);
            s.set_child(Some(w));
            s
        };

        // ---- About
        let ab = caixa(Orientation::Vertical, 12, "");
        let hwc = caixa(Orientation::Vertical, 6, "cfg-cartao");
        hwc.append(&rotulo_cartao("cpu-d", tr("HARDWARE")));
        let hw = caixa(Orientation::Vertical, 6, "");
        hwc.append(&hw);
        ab.append(&hwc);
        let swc = caixa(Orientation::Vertical, 6, "cfg-cartao");
        swc.append(&rotulo_cartao("olho-d", tr("SOFTWARE")));
        let sw = caixa(Orientation::Vertical, 6, "");
        swc.append(&sw);
        ab.append(&swc);
        body.add_named(&scroller(&ab), Some("sobre"));
        let ap = caixa(Orientation::Vertical, 10, "");
        let wallpaper = gtk::DrawingArea::new();
        wallpaper.set_content_height(210);
        wallpaper.set_hexpand(true);
        wallpaper.add_css_class("cfg-sobre-papel");
        ap.append(&wallpaper);
        let version = left("", "cfg-sobre-versao");
        version.set_margin_top(8);
        ap.append(&version);
        let codename = left(&format!("“Lunar” · {}", glib::DateTime::now_local().ok().and_then(|d| d.format("%B %Y").ok()).map(|s| s.to_string()).unwrap_or_default()), "cfg-sub");
        ap.append(&codename);
        let hl = left(tr("This computer's name"), "cfg-sub");
        hl.set_margin_top(10);
        ap.append(&hl);
        let hb = caixa(Orientation::Horizontal, 8, "cfg-campo-caixa");
        hb.append(&ui_icon("monitor", "#8E8F99", 16));
        let host = gtk::Entry::new();
        host.add_css_class("cfg-campo-livre");
        host.set_hexpand(true);
        hb.append(&host);
        let host_ok = gtk::Button::new();
        host_ok.add_css_class("cfg-icone-botao");
        host_ok.set_child(Some(&ui_icon("check", "#8ED2C3", 16)));
        host_ok.set_tooltip_text(Some(tr("Save the name (the system asks you)")));
        hb.append(&host_ok);
        ap.append(&hb);
        let host_note = left("", "cfg-sub");
        host_note.set_wrap(true);
        host_note.set_max_width_chars(52);
        ap.append(&host_note);
        let ab2 = caixa(Orientation::Horizontal, 8, "");
        ab2.set_homogeneous(true);
        ab2.set_margin_top(4);
        let copy = gtk::Button::with_label(tr("Copy the information"));
        copy.add_css_class("cfg-botao");
        let diag = gtk::Button::with_label(tr("Diagnosis"));
        diag.add_css_class("cfg-botao");
        ab2.append(&copy);
        ab2.append(&diag);
        ap.append(&ab2);
        panel.add_named(&ap, Some("sobre"));

        // ---- Updates
        let up = caixa(Orientation::Vertical, 12, "");
        let upd_main = caixa(Orientation::Vertical, 10, "cfg-cartao");
        up.append(&upd_main);
        let uh = caixa(Orientation::Vertical, 4, "cfg-cartao");
        uh.append(&rotulo_cartao("relogio", tr("HISTORY")));
        let upd_history = caixa(Orientation::Vertical, 2, "");
        uh.append(&upd_history);
        up.append(&uh);
        body.add_named(&scroller(&up), Some("atualizacoes"));
        let upp = caixa(Orientation::Vertical, 16, "");
        let rules = caixa(Orientation::Vertical, 2, "cfg-cartao");
        rules.append(&rotulo_cartao("ajustes", tr("HOW TO UPDATE")));
        let auto_update = interruptor();
        rules.append(&row(tr("Download and install by itself"), Some(tr("prepared in the background, installed when the computer restarts")), &auto_update));
        let off_hours = interruptor();
        rules.append(&row(tr("Only outside my hours of use"), Some(tr("installed at night, with the computer locked")), &off_hours));
        let restore_point = interruptor();
        rules.append(&row(tr("Create a restore point first"), None, &restore_point));
        upp.append(&rules);
        let amc = caixa(Orientation::Vertical, 2, "cfg-cartao");
        amc.append(&rotulo_cartao("apps-n", tr("APPS AND MODELS")));
        let apps_models = caixa(Orientation::Vertical, 2, "");
        amc.append(&apps_models);
        upp.append(&amc);
        panel.add_named(&upp, Some("atualizacoes"));

        // ---- Storage
        let st = caixa(Orientation::Vertical, 12, "");
        let st_main = caixa(Orientation::Vertical, 8, "cfg-cartao");
        st.append(&st_main);
        let st_others = caixa(Orientation::Horizontal, 12, "");
        st_others.set_homogeneous(true);
        st.append(&st_others);
        body.add_named(&scroller(&st), Some("armazenamento"));
        let stp = caixa(Orientation::Vertical, 16, "");
        let st_clean = caixa(Orientation::Vertical, 4, "cfg-cartao");
        stp.append(&st_clean);
        let ac = caixa(Orientation::Vertical, 2, "cfg-cartao");
        ac.append(&rotulo_cartao("ajustes", tr("AUTOMATIC")));
        let auto_clean = interruptor();
        ac.append(&row(tr("Automatic clean-up"), Some(tr("the trash's files older than 30 days")), &auto_clean));
        let warn_space = gtk::DropDown::new(Some(gtk::StringList::new(&[tr("never"), tr("under 5%"), tr("under 10%"), tr("under 20%")])), None::<gtk::Expression>);
        warn_space.add_css_class("cfg-seletor");
        warn_space.set_size_request(150, 34);
        ac.append(&row(tr("Warn when space runs low"), None, &warn_space));
        stp.append(&ac);
        panel.add_named(&stp, Some("armazenamento"));

        // ---- Users
        let us = caixa(Orientation::Vertical, 10, "");
        let accounts = caixa(Orientation::Vertical, 10, "");
        us.append(&accounts);
        body.add_named(&scroller(&us), Some("usuarios"));
        let account_panel = caixa(Orientation::Vertical, 10, "");
        panel.add_named(&account_panel, Some("usuarios"));

        // ---- Date and language
        let dt = caixa(Orientation::Vertical, 12, "");
        let dc = caixa(Orientation::Vertical, 2, "cfg-cartao");
        dc.append(&rotulo_cartao("relogio", tr("DATE AND TIME")));
        let clock = left("", "cfg-relogio-grande");
        dc.append(&clock);
        let date = left("", "cfg-sub");
        date.set_margin_bottom(8);
        dc.append(&date);
        let ntp = interruptor();
        dc.append(&row(tr("Set automatically"), Some(tr("from the network")), &ntp));
        let zone = gtk::DropDown::new(None::<gio::ListModel>, None::<gtk::Expression>);
        zone.add_css_class("cfg-seletor");
        zone.set_size_request(220, 34);
        zone.set_enable_search(true);
        dc.append(&row(tr("Time zone"), None, &zone));
        let seg = caixa(Orientation::Horizontal, 2, "cfg-segmentado");
        seg.add_css_class("escuro");
        seg.set_homogeneous(true);
        let h24 = gtk::ToggleButton::with_label(tr("24 hours"));
        h24.add_css_class("cfg-seg");
        let h12 = gtk::ToggleButton::with_label(tr("12 hours"));
        h12.add_css_class("cfg-seg");
        h12.set_group(Some(&h24));
        seg.append(&h24);
        seg.append(&h12);
        dc.append(&row(tr("Time format"), None, &seg));
        let first_day = gtk::DropDown::new(Some(gtk::StringList::new(&[tr("Sunday"), tr("Monday")])), None::<gtk::Expression>);
        first_day.add_css_class("cfg-seletor");
        first_day.set_size_request(190, 34);
        dc.append(&row(tr("First day of the week"), None, &first_day));
        let seconds = interruptor();
        dc.append(&row(tr("Show seconds in the bar"), None, &seconds));
        dt.append(&dc);
        body.add_named(&scroller(&dt), Some("data"));
        let dp = caixa(Orientation::Vertical, 16, "");
        let lc = caixa(Orientation::Vertical, 2, "cfg-cartao");
        lc.append(&rotulo_cartao("globo", tr("LANGUAGE AND REGION")));
        let language = gtk::DropDown::new(None::<gio::ListModel>, None::<gtk::Expression>);
        language.add_css_class("cfg-seletor");
        language.set_size_request(190, 34);
        lc.append(&row(tr("System language"), None, &language));
        let formats = gtk::DropDown::new(None::<gio::ListModel>, None::<gtk::Expression>);
        formats.add_css_class("cfg-seletor");
        formats.set_size_request(190, 34);
        lc.append(&row(tr("Formats"), Some(&format!("{} · {}", glib::DateTime::now_local().ok().and_then(|d| d.format("%x").ok()).map(|s| s.to_string()).unwrap_or_default(), money_example())), &formats));
        let others = caixa(Orientation::Horizontal, 12, "cfg-tela-opcao");
        others.add_css_class("alta");
        lc.append(&others);
        let ln = left(tr("A new language comes in at the next sign-in."), "cfg-sub");
        ln.set_wrap(true);
        ln.set_max_width_chars(52);
        ln.set_margin_start(10);
        lc.append(&ln);
        dp.append(&lc);
        let at = caixa(Orientation::Vertical, 2, "cfg-cartao");
        at.append(&rotulo_cartao("spark", tr("AGENTS AND TRANSLATION")));
        let answers = gtk::DropDown::new(Some(gtk::StringList::new(&[tr("The same as the system"), "Português (Brasil)", "English", "Español"])), None::<gtk::Expression>);
        answers.add_css_class("cfg-seletor");
        answers.set_size_request(190, 34);
        at.append(&row(tr("Language of the answers"), Some(tr("what the agent writes back")), &answers));
        dp.append(&at);
        panel.add_named(&dp, Some("data"));

        // ---- Default apps
        let da = caixa(Orientation::Vertical, 12, "");
        let dfc = caixa(Orientation::Vertical, 2, "cfg-cartao");
        dfc.append(&rotulo_cartao("apps-n", tr("OPEN BY DEFAULT WITH")));
        let defaults = caixa(Orientation::Vertical, 2, "");
        dfc.append(&defaults);
        da.append(&dfc);
        body.add_named(&scroller(&da), Some("apps"));
        let dap = caixa(Orientation::Vertical, 16, "");
        let inc = caixa(Orientation::Vertical, 6, "cfg-cartao");
        inc.append(&rotulo_cartao("apps-n", tr("INSTALLED APPS")));
        let sbx = caixa(Orientation::Horizontal, 8, "cfg-campo-caixa");
        sbx.append(&ui_icon("busca", "#8E8F99", 16));
        let app_search = gtk::Entry::new();
        app_search.add_css_class("cfg-campo-livre");
        app_search.set_placeholder_text(Some(tr("Search an app…")));
        app_search.set_hexpand(true);
        sbx.append(&app_search);
        inc.append(&sbx);
        let installed = caixa(Orientation::Vertical, 2, "");
        let isc = gtk::ScrolledWindow::new();
        isc.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        isc.set_size_request(-1, 230);
        isc.set_child(Some(&installed));
        inc.append(&isc);
        dap.append(&inc);
        let asc = caixa(Orientation::Vertical, 2, "cfg-cartao");
        asc.append(&rotulo_cartao("desligar-d", tr("OPEN AT START")));
        let autostart = caixa(Orientation::Vertical, 2, "");
        asc.append(&autostart);
        dap.append(&asc);
        panel.add_named(&dap, Some("apps"));

        // ---- Printers
        let pr = caixa(Orientation::Vertical, 10, "");
        let printers = caixa(Orientation::Vertical, 10, "");
        pr.append(&printers);
        body.add_named(&scroller(&pr), Some("impressoras"));
        let printer_panel = caixa(Orientation::Vertical, 10, "");
        panel.add_named(&printer_panel, Some("impressoras"));

        let page = Rc::new(SystemPage {
            frame: m, settings: Rc::downgrade(c), body, panel, tab: RefCell::new("sobre".into()), loading: Cell::new(false),
            hw, sw, about: RefCell::default(), host, host_note, version, wallpaper,
            upd_main, upd_history, auto_update, off_hours, restore_point,
            st_main, st_others, st_clean, auto_clean, storage: RefCell::default(),
            accounts, account_panel, chosen: RefCell::default(),
            clock, date, ntp, zone, zones: RefCell::default(), h24, h12, seconds, language, languages: RefCell::default(), formats,
            first_day, answers, others, warn_space, apps_models,
            defaults, installed, app_search, autostart,
            printers, printer_panel, printer_chosen: RefCell::default(),
        });
        page.connect_signals(&host_ok, &copy, &diag);
        let w = Rc::downgrade(&page);
        page.frame.ao_trocar_aba(move |id| if let Some(p) = w.upgrade() { p.show_tab(id) });
        // the clock ticks while the tab shows
        let w = Rc::downgrade(&page);
        crate::superficies::every_while_mapped(&page.clock, std::time::Duration::from_secs(1), move || {
            let Some(p) = w.upgrade() else { return false };
            if p.tab.borrow().as_str() == "data" && p.clock.is_mapped() {
                p.tick();
            }
            true
        });
        page
    }

    fn register(&self, name: &str, w: &impl IsA<gtk::Widget>) {
        if let Some(c) = self.settings.upgrade() {
            crate::configuracoes::registrar_posicao(&c, name, w.upcast_ref());
        }
    }

    pub fn compactar(&self, c: bool) {
        self.frame.compactar(c);
    }

    fn connect_signals(self: &Rc<Self>, host_ok: &gtk::Button, copy: &gtk::Button, diag: &gtk::Button) {
        // About: the name (through kalangod: the system asks), the information to the clipboard, a diagnosis by the agent
        let w = Rc::downgrade(self);
        let save_host = move || {
            let Some(p) = w.upgrade() else { return };
            let name = p.host.text().trim().to_string();
            if name == p.about.borrow().hostname {
                return;
            }
            p.host_note.set_text(tr("Waiting for your approval…"));
            let w2 = Rc::downgrade(&p);
            crate::lateral::em_segundo_plano(move || crate::servicos::kalangod_chamar("sistema.nome", json!({"nome": name})), move |r| {
                let Some(p) = w2.upgrade() else { return };
                match r {
                    Ok(v) => {
                        p.host_note.set_text(v["resumo"].as_str().or(v["summary"].as_str()).unwrap_or(tr("Saved.")));
                        if depurando() {
                            eprintln!("debug sistema: nome do computador {}", v);
                        }
                        p.refresh_about();
                    }
                    Err(e) => {
                        p.host_note.set_text(&e);
                        if depurando() {
                            eprintln!("debug sistema: nome recusado: {e}");
                        }
                    }
                }
            });
        };
        let sh2 = save_host.clone();
        host_ok.connect_clicked(move |_| sh2());
        self.host.connect_activate(move |_| save_host());
        self.register("cfg sistema nome", host_ok);
        let w = Rc::downgrade(self);
        copy.connect_clicked(move |b| {
            let Some(p) = w.upgrade() else { return };
            let a = p.about.borrow().clone();
            let t = [(tr("Processor"), a.cpu), (tr("Memory"), a.memory), (tr("Graphics card"), a.gpu), (tr("Storage"), a.disks), (tr("Motherboard"), a.board),
                (tr("Screens"), a.screens), (tr("System"), a.system), (tr("Environment"), a.compositor), (tr("Shell"), a.shell), (tr("Kernel"), a.kernel),
                (tr("Local model"), a.model)].iter().filter(|(_, v)| !v.is_empty()).map(|(k, v)| format!("{k}: {v}")).collect::<Vec<_>>().join("\n");
            b.clipboard().set_text(&t);
            if depurando() {
                eprintln!("debug sistema: informações copiadas ({} linhas)", t.lines().count());
            }
        });
        self.register("cfg sistema copiar", copy);
        diag.connect_clicked(|_| {
            if let Some(s) = crate::shell_atual() {
                s.right_bar().ask_agent(tr("Make a diagnosis of this computer: disks, memory, temperature, services that failed and the last errors of the system's log. Say what needs attention."));
            }
        });
        // Updates: the rules
        for (sw, key) in [(&self.auto_update, "atualizar_sozinho"), (&self.off_hours, "atualizar_fora_do_uso"), (&self.restore_point, "ponto_antes_de_atualizar")] {
            let w = Rc::downgrade(self);
            sw.connect_state_set(move |_, on| {
                if w.upgrade().is_some_and(|p| !p.loading.get()) {
                    crate::configuracoes::salvar_ajuste(key, json!(on));
                    if depurando() {
                        eprintln!("debug sistema: {key} {on}");
                    }
                }
                glib::Propagation::Proceed
            });
        }
        // Storage: the automatic clean-up (applied now, and each time the shell starts)
        let w = Rc::downgrade(self);
        self.auto_clean.connect_state_set(move |_, on| {
            if let Some(p) = w.upgrade().filter(|p| !p.loading.get()) {
                crate::configuracoes::salvar_ajuste("limpeza_automatica", json!(on));
                if on {
                    let n = clean_old_trash();
                    if depurando() {
                        eprintln!("debug sistema: limpeza automática ({n} da lixeira)");
                    }
                    p.refresh_storage();
                }
            }
            glib::Propagation::Proceed
        });
        // Date and language
        let w = Rc::downgrade(self);
        self.ntp.connect_state_set(move |_, on| {
            if let Some(p) = w.upgrade().filter(|p| !p.loading.get()) {
                p.system_call("relogio.automatico", json!({"ligado": on}));
            }
            glib::Propagation::Proceed
        });
        let w = Rc::downgrade(self);
        self.zone.connect_selected_notify(move |d| {
            let Some(p) = w.upgrade().filter(|p| !p.loading.get()) else { return };
            let z = p.zones.borrow().get(d.selected() as usize).cloned();
            if let Some(z) = z {
                p.system_call("relogio.fuso", json!({"fuso": z}));
            }
        });
        self.register("cfg sistema fuso", &self.zone);
        let w = Rc::downgrade(self);
        self.h12.connect_toggled(move |b| {
            if w.upgrade().is_some_and(|p| !p.loading.get()) {
                crate::configuracoes::salvar_ajuste("relogio_12h", json!(b.is_active()));
                if depurando() {
                    eprintln!("debug sistema: relógio de 12 horas {}", b.is_active());
                }
            }
        });
        let w = Rc::downgrade(self);
        self.seconds.connect_state_set(move |_, on| {
            if w.upgrade().is_some_and(|p| !p.loading.get()) {
                crate::configuracoes::salvar_ajuste("relogio_segundos", json!(on));
            }
            glib::Propagation::Proceed
        });
        for d in [&self.language, &self.formats] {
            let w = Rc::downgrade(self);
            d.connect_selected_notify(move |_| {
                let Some(p) = w.upgrade().filter(|p| !p.loading.get()) else { return };
                let langs = p.languages.borrow().clone();
                let (Some(l), Some(f)) = (langs.get(p.language.selected() as usize), langs.get(p.formats.selected() as usize)) else { return };
                p.system_call("idioma.sistema", json!({"idioma": l, "formatos": f}));
            });
        }
        let w = Rc::downgrade(self);
        self.first_day.connect_selected_notify(move |d| {
            if w.upgrade().is_some_and(|p| !p.loading.get()) {
                crate::configuracoes::salvar_ajuste("primeiro_dia_da_semana", json!(d.selected()));
                if depurando() {
                    eprintln!("debug sistema: primeiro dia da semana {}", d.selected());
                }
            }
        });
        let w = Rc::downgrade(self);
        self.answers.connect_selected_notify(move |d| {
            if w.upgrade().is_some_and(|p| !p.loading.get()) {
                let v = ["", "pt_BR", "en_US", "es_ES"].get(d.selected() as usize).copied().unwrap_or("");
                crate::configuracoes::salvar_ajuste("idioma_respostas", json!(v));
                if depurando() {
                    eprintln!("debug sistema: idioma das respostas \"{v}\"");
                }
            }
        });
        let w = Rc::downgrade(self);
        self.warn_space.connect_selected_notify(move |d| {
            if w.upgrade().is_some_and(|p| !p.loading.get()) {
                let v = [0, 5, 10, 20].get(d.selected() as usize).copied().unwrap_or(10);
                crate::configuracoes::salvar_ajuste("aviso_espaco", json!(v));
                crate::configuracoes::salvar_ajuste("aviso_espaco_dia", Value::Null);
            }
        });
        // Default apps: the search of the installed ones
        let w = Rc::downgrade(self);
        self.app_search.connect_changed(move |_| if let Some(p) = w.upgrade() { p.fill_installed() });
    }

    /// A change that only the system can make: through kalangod (the person approves it); the tab is read again after.
    /// One's own avatar as a button with a camera badge: a menu to choose a photo (the file chooser, images only) or to
    /// take it away; the login screen gets it through kalangod (usuarios.foto).
    fn photo_button(self: &Rc<Self>, a: &Account) -> gtk::Widget {
        let over = gtk::Overlay::new();
        over.set_child(Some(&avatar(a, 52)));
        let badge = caixa(Orientation::Horizontal, 0, "cfg-avatar-selo");
        badge.set_halign(Align::End);
        badge.set_valign(Align::End);
        badge.set_can_target(false);
        badge.append(&ui_icon("camera", "#E2E2E6", 12));
        over.add_overlay(&badge);
        let btn = gtk::MenuButton::new();
        btn.add_css_class("cfg-avatar-botao");
        btn.set_child(Some(&over));
        btn.set_tooltip_text(Some(tr("Change the photo")));
        btn.set_valign(Align::Center);
        let pop = gtk::Popover::new();
        pop.add_css_class("cfg-menu-foto");
        pop.set_has_arrow(false);
        pop.set_position(gtk::PositionType::Bottom);
        // (left edge with the avatar's, as the design: a popover centres on its button, and the menu is wider)
        pop.connect_map(|p| {
            let w = p.width().max(p.child().map(|c| c.width()).unwrap_or(0));
            p.set_offset((w - 52) / 2, 6);
        });
        let lista = caixa(Orientation::Vertical, 2, "");
        let item = |icone: &str, texto: &str| {
            let b = gtk::Button::new();
            b.add_css_class("cfg-menu-item");
            let r = caixa(Orientation::Horizontal, 10, "");
            r.append(&ui_icon(icone, "#B9BAC4", 16));
            r.append(&left(texto, "cfg-menu-texto"));
            b.set_child(Some(&r));
            b
        };
        let escolher = item("imagem", tr("Choose photo…"));
        let remover = item("lixeira", tr("Remove photo"));
        lista.append(&escolher);
        let face = a.home.join(".face");
        if face.is_file() {
            lista.append(&remover);
        }
        pop.set_child(Some(&lista));
        btn.set_popover(Some(&pop));
        let (w, home, p2) = (Rc::downgrade(self), a.home.clone(), pop.clone());
        escolher.connect_clicked(move |b| {
            p2.popdown();
            let filtro = gtk::FileFilter::new();
            filtro.set_name(Some(tr("Images")));
            filtro.add_mime_type("image/*");
            let filtros = gtk::gio::ListStore::new::<gtk::FileFilter>();
            filtros.append(&filtro);
            let d = gtk::FileDialog::new();
            d.set_title(tr("Choose photo"));
            d.set_filters(Some(&filtros));
            d.set_initial_folder(Some(&gtk::gio::File::for_path(glib::user_special_dir(glib::UserDirectory::Pictures).unwrap_or_else(glib::home_dir))));
            let (w, home) = (w.clone(), home.clone());
            d.open(b.root().and_downcast_ref::<gtk::Window>(), gtk::gio::Cancellable::NONE, move |r| {
                let Ok(f) = r else { return };
                let Some(path) = f.path() else { return };
                match save_face(&path, &home) {
                    Ok(()) => {
                        if let Some(p) = w.upgrade() {
                            p.system_call("usuarios.foto", json!({}));
                        }
                    }
                    Err(e) => eprintln!("kalango-shell: foto da conta: {e}"),
                }
            });
        });
        let (w, home, p3) = (Rc::downgrade(self), a.home.clone(), pop.clone());
        remover.connect_clicked(move |_| {
            p3.popdown();
            let _ = std::fs::remove_file(home.join(".face"));
            if let Some(p) = w.upgrade() {
                p.system_call("usuarios.foto", json!({"limpar": true}));
            }
        });
        btn.upcast()
    }

    fn system_call(self: &Rc<Self>, tool: &'static str, args: Value) {
        if depurando() {
            eprintln!("debug sistema: pedindo {tool} {args}");
        }
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(move || crate::servicos::kalangod_chamar(tool, args), move |r| {
            if depurando() {
                match &r {
                    Ok(v) => eprintln!("debug sistema: {tool} feito: {v}"),
                    Err(e) => eprintln!("debug sistema: {tool} recusado: {e}"),
                }
            }
            if let Some(p) = w.upgrade() {
                p.mostrar();
            }
        });
    }

    fn show_tab(self: &Rc<Self>, id: &str) {
        *self.tab.borrow_mut() = id.to_string();
        self.body.set_visible_child_name(id);
        self.panel.set_visible_child_name(id);
        let title = match id {
            "atualizacoes" => tr("Rules"),
            "armazenamento" => tr("Clean-up"),
            "usuarios" => tr("Chosen account"),
            "data" => tr("Language and region"),
            "apps" => tr("Installed"),
            "impressoras" => tr("Chosen printer"),
            _ => tr("This computer"),
        };
        self.frame.painel_titulo.set_text(title);
        // (Users is a page of its own in the window's title: kalangod keeps agents operating the screen out of it)
        if let Some(w) = self.body.root().and_downcast::<gtk::Window>() {
            crate::configuracoes::set_section_title(&w, if id == "usuarios" { "usuarios" } else { "sistema" });
        }
        self.mostrar();
    }

    /// The tab showing, read again.
    pub fn mostrar(self: &Rc<Self>) {
        match self.tab.borrow().clone().as_str() {
            "atualizacoes" => self.refresh_updates(),
            "armazenamento" => self.refresh_storage(),
            "usuarios" => self.refresh_users(),
            "data" => self.refresh_date(),
            "apps" => self.refresh_apps(),
            "impressoras" => self.refresh_printers(),
            _ => self.refresh_about(),
        }
    }

    fn sub(&self, t: &str) {
        self.frame.sub.set_text(t);
    }

    // ---- About

    fn refresh_about(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(read_about, move |a| {
            let Some(p) = w.upgrade() else { return };
            clear(&p.hw);
            clear(&p.sw);
            for (k, v) in [(tr("Processor"), &a.cpu), (tr("Memory"), &a.memory), (tr("Graphics card"), &a.gpu), (tr("Storage"), &a.disks), (tr("Motherboard"), &a.board)] {
                if !v.is_empty() {
                    kv(&p.hw, k).set_text(v);
                }
            }
            for (k, v) in [(tr("System"), &a.system), (tr("Environment"), &a.compositor), (tr("Shell"), &a.shell), (tr("Kernel"), &a.kernel), (tr("Local model"), &a.model)] {
                kv(&p.sw, k).set_text(v);
            }
            let mut a = a;
            a.screens = screens_now();
            if !a.screens.is_empty() {
                kv(&p.hw, tr("Screens")).set_text(&a.screens);
            }
            p.version.set_text(&format!("KalangoOS {}", env!("CARGO_PKG_VERSION")));
            p.sub(&format!("KalangoOS {} · {}", env!("CARGO_PKG_VERSION"), tr("up to date")));
            if !p.host.has_focus() {
                p.host.set_text(&a.hostname);
            }
            if let Some(f) = crate::papel::para(1600, 900) {
                let pb = gtk::gdk_pixbuf::Pixbuf::from_file_at_scale(&f, 960, -1, true).ok();
                p.wallpaper.set_draw_func(move |_, cr, w, h| {
                    let Some(pb) = &pb else { return };
                    let (w, h) = (w as f64, h as f64);
                    let k = (w / pb.width() as f64).max(h / pb.height() as f64);
                    rounded(cr, 0.0, 0.0, w, h, 6.0);
                    cr.clip();
                    cr.scale(k, k);
                    cr.set_source_pixbuf(pb, ((w / k) - pb.width() as f64) / 2.0, ((h / k) - pb.height() as f64) / 2.0);
                    let _ = cr.paint();
                });
                p.wallpaper.queue_draw();
            }
            if depurando() {
                eprintln!("debug sistema: sobre ({}, {}, {})", a.cpu, a.memory, a.kernel);
            }
            *p.about.borrow_mut() = a;
        });
    }

    // ---- Updates

    fn refresh_updates(self: &Rc<Self>) {
        self.loading.set(true);
        let a = crate::configuracoes::ajustes();
        self.auto_update.set_active(a["atualizar_sozinho"].as_bool().unwrap_or(true));
        self.off_hours.set_active(a["atualizar_fora_do_uso"].as_bool().unwrap_or(true));
        self.restore_point.set_active(a["ponto_antes_de_atualizar"].as_bool().unwrap_or(true));
        self.loading.set(false);
        let w = Rc::downgrade(self);
        crate::energia::prepared_update(move |prepared| {
            let Some(p) = w.upgrade() else { return };
            let w2 = Rc::downgrade(&p);
            crate::lateral::em_segundo_plano(|| (crate::servicos::kalangod_chamar("atualizacoes.listar", json!({})).ok(), crate::servicos::kalangod_linha_do_tempo(200).unwrap_or_default()),
                move |(list, timeline)| {
                    if let Some(p) = w2.upgrade() {
                        p.fill_updates(prepared, list, timeline);
                    }
                });
        });
    }

    fn fill_updates(self: &Rc<Self>, prepared: usize, list: Option<Value>, timeline: Vec<Value>) {
        clear(&self.upd_main);
        let pkgs: Vec<String> = list.as_ref().and_then(|v| v["valor"]["pacotes"].as_array().or(v["value"]["pacotes"].as_array()).or(v["pacotes"].as_array()).cloned())
            .unwrap_or_default().iter().filter_map(|p| p.as_str().map(str::to_string).or_else(|| p["nome"].as_str().map(|n| format!("{n} {} → {}", p["atual"].as_str().unwrap_or(""), p["nova"].as_str().unwrap_or(""))))).collect();
        let n = prepared.max(pkgs.len());
        let top = caixa(Orientation::Horizontal, 14, "");
        let ic = caixa(Orientation::Horizontal, 0, "cfg-atualizacao-icone");
        ic.set_size_request(40, 40);
        ic.set_homogeneous(true);
        ic.append(&ui_icon(if n > 0 { "download" } else { "check" }, if n > 0 { "#B9BAC4" } else { "#8ED2C3" }, 18));
        ic.set_valign(Align::Center);
        top.append(&ic);
        let col = caixa(Orientation::Vertical, 2, "");
        col.set_hexpand(true);
        col.set_valign(Align::Center);
        col.append(&left(&if n == 0 { tr("The system is up to date").to_string() } else if n == 1 { tr("1 update available").to_string() } else { trf("{n} updates available", &[("n", n.to_string())]) }, "cfg-titulo-cartao"));
        col.append(&left(&if prepared > 0 { tr("downloaded · installed when the computer restarts").to_string() } else if n > 0 { tr("from the system's repositories").to_string() } else { tr("checked just now").to_string() }, "cfg-sub"));
        top.append(&col);
        if n > 0 {
            let later = gtk::Button::with_label(tr("When powering off"));
            later.add_css_class("cfg-botao");
            later.set_valign(Align::Center);
            later.set_tooltip_text(Some(tr("The power menu installs it on the way out (its box comes ticked).")));
            later.connect_clicked(|b| {
                crate::configuracoes::salvar_ajuste("instalar_ao_desligar", json!(true));
                b.set_label(tr("Marked"));
                b.set_sensitive(false);
            });
            top.append(&later);
            let now = gtk::Button::with_label(tr("Install now"));
            now.add_css_class("cfg-botao-destaque");
            now.set_valign(Align::Center);
            now.set_tooltip_text(Some(tr("Restarts into the update (a restore point comes first when the rule says so).")));
            let w = Rc::downgrade(self);
            now.connect_clicked(move |_| {
                let Some(p) = w.upgrade() else { return };
                if p.restore_point.is_active() {
                    p.system_call("pontos.criar", json!({"descricao": tr("Before the update")}));
                }
                crate::energia::Energia::fazer("reiniciar", true);
            });
            self.register("cfg sistema instalar", &now);
            top.append(&now);
        }
        self.upd_main.append(&top);
        if !pkgs.is_empty() {
            let wl = caixa(Orientation::Horizontal, 6, "");
            wl.append(&estrela(12, "#A5A1E2"));
            wl.append(&left(tr("WHAT CHANGES"), "cfg-rotulo"));
            self.upd_main.append(&wl);
            for p in pkgs.iter().take(6) {
                let r = caixa(Orientation::Horizontal, 8, "");
                r.append(&ui_icon("check", "#8ED2C3", 14));
                let l = left(p, "cfg-texto-13");
                l.set_ellipsize(gtk::pango::EllipsizeMode::End);
                l.set_max_width_chars(1);
                l.set_hexpand(true);
                r.append(&l);
                self.upd_main.append(&r);
            }
            if pkgs.len() > 6 {
                self.upd_main.append(&left(&trf("and {n} more", &[("n", (pkgs.len() - 6).to_string())]), "cfg-sub"));
            }
        }
        let note = caixa(Orientation::Horizontal, 8, "");
        note.append(&ui_icon("relogio", "#7A7B88", 14));
        note.append(&left(tr("A restore point is made before; one click goes back."), "cfg-sub"));
        self.upd_main.append(&note);
        // the history: what was installed (the timeline's update actions)
        clear(&self.upd_history);
        let installs: Vec<&Value> = timeline.iter().filter(|e| e["tool"].as_str().is_some_and(|t| t.starts_with("atualizacoes.aplicar") || t == "pacotes.versoes")).take(5).collect();
        if installs.is_empty() {
            self.upd_history.append(&left(tr("Nothing installed from here yet."), "cfg-sub"));
        }
        for e in installs {
            let r = caixa(Orientation::Horizontal, 10, "cfg-tela-opcao");
            r.add_css_class("alta");
            r.append(&ui_icon("check", "#8ED2C3", 14));
            let col = caixa(Orientation::Vertical, 1, "");
            col.set_hexpand(true);
            col.set_valign(Align::Center);
            col.append(&left(e["resumo"].as_str().or(e["summary"].as_str()).unwrap_or(tr("Update")), "cfg-texto-13"));
            col.append(&left(e["quando"].as_str().unwrap_or(""), "cfg-sub"));
            r.append(&col);
            self.upd_history.append(&r);
        }
        self.sub(&if n == 0 { tr("up to date").to_string() } else { trf("{n} updates available", &[("n", n.to_string())]) });
        // apps and models: what dpkg updated today; the local models and their sizes
        clear(&self.apps_models);
        let today = glib::DateTime::now_local().ok().and_then(|d| d.format("%F").ok()).map(|s| s.to_string()).unwrap_or_default();
        let mut upgraded: Vec<String> = std::fs::read_to_string("/var/log/dpkg.log").unwrap_or_default().lines()
            .filter(|l| l.starts_with(&today) && l.contains(" upgrade "))
            .filter_map(|l| l.split_whitespace().nth(3).map(|p| p.split(':').next().unwrap_or(p).to_string())).collect();
        upgraded.dedup();
        let ut = if upgraded.is_empty() { tr("No app updated today").to_string() } else if upgraded.len() == 1 { tr("1 app updated today").to_string() } else { trf("{n} apps updated today", &[("n", upgraded.len().to_string())]) };
        let ul = left(&upgraded.iter().take(4).cloned().collect::<Vec<_>>().join(", "), "cfg-sub");
        ul.set_ellipsize(gtk::pango::EllipsizeMode::End);
        ul.set_max_width_chars(1);
        let ur = caixa(Orientation::Vertical, 1, "cfg-tela-opcao");
        ur.add_css_class("alta");
        ur.append(&left(&ut, "cfg-texto-13"));
        if !upgraded.is_empty() {
            ur.append(&ul);
        }
        self.apps_models.append(&ur);
        for e in std::fs::read_dir(glib::user_data_dir().join("kalango/modelos")).into_iter().flatten().flatten() {
            let path = e.path();
            if path.extension().is_none_or(|x| x != "gguf") || path.to_string_lossy().contains("mmproj") {
                continue;
            }
            let size = e.metadata().map(|m| m.len()).unwrap_or(0);
            let name = path.file_stem().map(|s| s.to_string_lossy().replace(['-', '_'], " ")).unwrap_or_default();
            let open = gtk::Button::with_label(tr("Show"));
            open.add_css_class("cfg-botao");
            let pp = path.clone();
            open.connect_clicked(move |_| {
                if let Some(s) = crate::shell_atual() {
                    s.abrir_arquivos(Some(&pp));
                }
            });
            self.apps_models.append(&row(&name, Some(&format!("{} · {}", tr("local model"), human(size))), &open));
        }
        if depurando() {
            eprintln!("debug sistema: atualizações {n} ({prepared} preparadas)");
        }
    }

    // ---- Storage

    fn refresh_storage(self: &Rc<Self>) {
        self.loading.set(true);
        let a = crate::configuracoes::ajustes();
        self.auto_clean.set_active(a["limpeza_automatica"].as_bool().unwrap_or(false));
        let pct = a["aviso_espaco"].as_u64().unwrap_or(10);
        self.warn_space.set_selected([0, 5, 10, 20].iter().position(|x| *x == pct).unwrap_or(2) as u32);
        self.loading.set(false);
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(read_storage, move |s| {
            if let Some(p) = w.upgrade() {
                p.fill_storage(&s);
                *p.storage.borrow_mut() = Some(s);
            }
        });
    }

    fn fill_storage(self: &Rc<Self>, s: &Storage) {
        clear(&self.st_main);
        let lab = caixa(Orientation::Horizontal, 6, "");
        lab.append(&ui_icon("disco", "#8E8F99", 16));
        lab.append(&rotulo(&format!("{} · {}", tr("SYSTEM"), human(s.total)), "cfg-rotulo"));
        self.st_main.append(&lab);
        let big = caixa(Orientation::Horizontal, 10, "");
        big.append(&rotulo(&trf("{size} used", &[("size", human(s.used))]), "cfg-armazenamento-usado"));
        let fr = rotulo(&trf("{size} free", &[("size", human(s.free))]), "cfg-armazenamento-livre");
        fr.set_valign(Align::End);
        fr.set_margin_bottom(3);
        big.append(&fr);
        self.st_main.append(&big);
        // the bar: each kind its share of the disk
        let bar = gtk::DrawingArea::new();
        bar.set_content_height(10);
        bar.set_hexpand(true);
        let parts: Vec<(f64, &str)> = s.kinds.iter().map(|(_, b, tone, _)| (*b as f64 / s.total.max(1) as f64, *tone)).collect();
        bar.set_draw_func(move |_, cr, w, h| {
            let (w, h) = (w as f64, h as f64);
            cr.set_source_rgba(0.149, 0.153, 0.184, 1.0);
            cr.rectangle(0.0, 0.0, w, h);
            let _ = cr.fill();
            let mut x = 0.0;
            for (f, tone) in &parts {
                let (r, g, b) = match *tone { "roxo" => (0.647, 0.631, 0.886), "verde" => (0.557, 0.824, 0.765), "azul" => (0.486, 0.765, 0.875), "ouro" => (0.898, 0.788, 0.549), _ => (0.937, 0.635, 0.651) };
                let wd = (f * w).max(0.0);
                if wd < 1.0 {
                    continue;
                }
                cr.set_source_rgb(r, g, b);
                cr.rectangle(x, 0.0, (wd - 3.0).max(1.0), h);
                let _ = cr.fill();
                x += wd;
            }
        });
        bar.add_css_class("cfg-armazenamento-barra");
        self.st_main.append(&bar);
        for (name, bytes, tone, folder) in &s.kinds {
            let r = gtk::Button::new();
            r.add_css_class("cfg-armazenamento-linha");
            let c = caixa(Orientation::Horizontal, 10, "");
            c.append(&crate::configuracoes::ponto(tone));
            let n = left(name, "cfg-texto-13");
            n.set_hexpand(true);
            c.append(&n);
            c.append(&rotulo(&human(*bytes), "cfg-sub"));
            if folder.is_some() {
                c.append(&ui_icon("chevron-n", "#7A7B88", 16));
            }
            r.set_child(Some(&c));
            r.set_sensitive(folder.is_some());
            if let Some(f) = folder.clone() {
                r.set_tooltip_text(Some(tr("Open in Files")));
                r.connect_clicked(move |_| {
                    if let Some(s) = crate::shell_atual() {
                        s.abrir_arquivos(Some(&f));
                    }
                });
            }
            self.st_main.append(&r);
        }
        clear(&self.st_others);
        for (name, used, total) in &s.others {
            let c = caixa(Orientation::Vertical, 8, "cfg-cartao");
            let t = caixa(Orientation::Horizontal, 6, "");
            t.append(&ui_icon("pasta", "#8E8F99", 16));
            t.append(&rotulo(&name.to_uppercase(), "cfg-rotulo"));
            c.append(&t);
            let l = caixa(Orientation::Horizontal, 8, "");
            let a = left(&trf("{used} of {total}", &[("used", human(*used)), ("total", human(*total))]), "cfg-sub");
            a.set_hexpand(true);
            l.append(&a);
            let pct = (*used as f64 / (*total).max(1) as f64 * 100.0).round();
            l.append(&rotulo(&format!("{pct}%"), "cfg-valor-pequeno"));
            c.append(&l);
            let lb = gtk::LevelBar::for_interval(0.0, 1.0);
            lb.add_css_class("arq-disco-barra");
            if pct >= 90.0 {
                lb.add_css_class("cheio");
            }
            lb.set_value(*used as f64 / (*total).max(1) as f64);
            c.append(&lb);
            self.st_others.append(&c);
        }
        // what can be freed
        clear(&self.st_clean);
        self.st_clean.append(&rotulo_cartao("spark", tr("FREE SPACE — SUGGESTED BY THE AGENT")));
        let item = |glyph: &str, title: &str, sub: &str, action: &str| -> gtk::Button {
            let r = caixa(Orientation::Horizontal, 10, "cfg-tela-opcao");
            r.add_css_class("alta");
            r.append(&ui_icon(glyph, "#8E8F99", 16));
            let col = caixa(Orientation::Vertical, 1, "");
            col.set_hexpand(true);
            col.set_valign(Align::Center);
            col.append(&left(title, "cfg-texto-13"));
            col.append(&left(sub, "cfg-sub"));
            r.append(&col);
            let b = gtk::Button::with_label(action);
            b.add_css_class("cfg-botao");
            b.set_valign(Align::Center);
            r.append(&b);
            self.st_clean.append(&r);
            b
        };
        let cache = item("olho-d", tr("Caches"), &format!("{} · {}", human(s.cache), tr("made again when needed")), tr("Clear"));
        cache.set_sensitive(s.cache > 0);
        let w = Rc::downgrade(self);
        cache.connect_clicked(move |_| {
            let n = clear_cache();
            if depurando() {
                eprintln!("debug sistema: cache limpo ({n})");
            }
            if let Some(p) = w.upgrade() {
                p.refresh_storage();
            }
        });
        self.register("cfg sistema limpar cache", &cache);
        let models = item("cerebro", tr("Models not used for 60 days"), &format!("{} · {}", human(s.old_models.0), trf("{n} models", &[("n", s.old_models.1.to_string())])), tr("Review"));
        models.set_sensitive(s.old_models.1 > 0);
        models.connect_clicked(|_| {
            if let Some(s) = crate::shell_atual() {
                s.abrir_arquivos(Some(&glib::user_data_dir().join("kalango/modelos")));
            }
        });
        let trash = item("lixeira", tr("Trash"), &format!("{} · {}", human(s.trash.0), trf("{n} items", &[("n", s.trash.1.to_string())])), tr("Empty"));
        trash.set_sensitive(s.trash.1 > 0);
        let w = Rc::downgrade(self);
        trash.connect_clicked(move |b| {
            // a second click confirms (it cannot be undone)
            if !b.has_css_class("confirmar") {
                b.add_css_class("confirmar");
                b.set_label(tr("Empty for good?"));
                return;
            }
            let n = empty_trash();
            if n > 0 {
                crate::configuracoes_som::tocar("trash-empty");
            }
            if depurando() {
                eprintln!("debug sistema: lixeira esvaziada ({n})");
            }
            if let Some(p) = w.upgrade() {
                p.refresh_storage();
            }
        });
        self.register("cfg sistema lixeira", &trash);
        let copies = item("copiar", tr("Copies"), &format!("{} · {}", human(s.copies.0), trf("{n} files in Downloads", &[("n", s.copies.1.to_string())])), tr("Review"));
        copies.set_sensitive(s.copies.1 > 0);
        copies.connect_clicked(|_| {
            if let (Some(s), Some(d)) = (crate::shell_atual(), glib::user_special_dir(glib::UserDirectory::Downloads)) {
                s.abrir_arquivos(Some(&d));
            }
        });
        let all = gtk::Button::with_label(&trf("Free {size} safely", &[("size", human(s.cache))]));
        all.add_css_class("cfg-botao-destaque");
        all.set_margin_top(6);
        all.set_tooltip_text(Some(tr("Clears the caches (apps make them again); the trash and the models stay for you to review.")));
        all.set_sensitive(s.cache > 0);
        let w = Rc::downgrade(self);
        all.connect_clicked(move |_| {
            clear_cache();
            if let Some(p) = w.upgrade() {
                p.refresh_storage();
            }
        });
        self.st_clean.append(&all);
        self.sub(&trf("{free} free of {total}", &[("free", human(s.free)), ("total", human(s.total))]));
        if depurando() {
            eprintln!("debug sistema: armazenamento {} usados, {} livres, cache {}", human(s.used), human(s.free), human(s.cache));
        }
    }

    // ---- Users

    fn refresh_users(self: &Rc<Self>) {
        let accounts = read_accounts();
        clear(&self.accounts);
        if self.chosen.borrow().is_none() {
            *self.chosen.borrow_mut() = accounts.iter().find(|a| a.me).map(|a| a.login.clone());
        }
        let chosen = self.chosen.borrow().clone();
        for a in &accounts {
            let b = gtk::Button::new();
            b.add_css_class("cfg-conta");
            if chosen.as_deref() == Some(&a.login) {
                b.add_css_class("escolhida");
            }
            let c = caixa(Orientation::Horizontal, 12, "");
            c.append(&avatar(a, 36));
            let col = caixa(Orientation::Vertical, 2, "");
            col.set_hexpand(true);
            col.set_valign(Align::Center);
            col.append(&left(&a.name, "cfg-texto-13"));
            let role = if a.admin { tr("administrator") } else { tr("standard") };
            col.append(&left(&if a.me { format!("{role} · {}", tr("you")) } else { role.to_string() }, "cfg-sub"));
            c.append(&col);
            c.append(&ui_icon("chevron-n", "#7A7B88", 16));
            b.set_child(Some(&c));
            let (w, login) = (Rc::downgrade(self), a.login.clone());
            b.connect_clicked(move |_| {
                if let Some(p) = w.upgrade() {
                    *p.chosen.borrow_mut() = Some(login.clone());
                    p.refresh_users();
                }
            });
            self.register(&format!("cfg conta {}", a.login), &b);
            self.accounts.append(&b);
        }
        // a new account: its name and whether it administers (kalangod: the system asks)
        let add = gtk::MenuButton::new();
        add.add_css_class("cfg-adicionar-tracejado");
        let ac = caixa(Orientation::Horizontal, 6, "");
        ac.set_halign(Align::Center);
        ac.append(&ui_icon("mais", "#8E8F99", 14));
        ac.append(&rotulo(tr("Add a person"), ""));
        add.set_child(Some(&ac));
        let pop = gtk::Popover::new();
        pop.add_css_class("arq-menu");
        pop.set_has_arrow(false);
        let f = caixa(Orientation::Vertical, 8, "");
        f.set_size_request(300, -1);
        f.set_margin_top(8);
        f.set_margin_bottom(8);
        f.set_margin_start(8);
        f.set_margin_end(8);
        let full = gtk::Entry::new();
        full.add_css_class("cfg-campo");
        full.set_placeholder_text(Some(tr("Full name")));
        let login = gtk::Entry::new();
        login.add_css_class("cfg-campo");
        login.set_placeholder_text(Some(tr("User name (lower case, no spaces)")));
        let admin = gtk::CheckButton::with_label(tr("Administrator"));
        let create = gtk::Button::with_label(tr("Create account"));
        create.add_css_class("cfg-botao-destaque");
        let note = left("", "cfg-sub");
        note.set_wrap(true);
        note.set_max_width_chars(40);
        f.append(&full);
        f.append(&login);
        f.append(&admin);
        f.append(&create);
        f.append(&note);
        pop.set_child(Some(&f));
        add.set_popover(Some(&pop));
        let lw = login.downgrade();
        full.connect_changed(move |e| {
            if let Some(l) = lw.upgrade() {
                let s: String = e.text().split_whitespace().next().unwrap_or("").to_lowercase().chars().filter(|c| c.is_ascii_alphanumeric()).collect();
                l.set_text(&s);
            }
        });
        let (w, nw) = (Rc::downgrade(self), note.downgrade());
        create.connect_clicked(move |_| {
            let Some(p) = w.upgrade() else { return };
            let l = login.text().trim().to_string();
            if l.is_empty() || !l.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_') {
                if let Some(n) = nw.upgrade() {
                    n.set_text(tr("A user name has lower-case letters and numbers."));
                }
                return;
            }
            p.system_call("usuarios.criar", json!({"nome": l, "nome_completo": full.text().trim(), "administrador": admin.is_active()}));
            if let Some(n) = nw.upgrade() {
                n.set_text(tr("Waiting for your approval…"));
            }
        });
        self.accounts.append(&add);
        // the guest (the login screen's "convidado" account, when the system has it)
        let passwd = std::fs::read_to_string("/etc/passwd").unwrap_or_default();
        if let Some(gl) = passwd.lines().find(|l| l.starts_with("convidado:")) {
            let on = !gl.ends_with("nologin") && !gl.ends_with("false");
            let g = caixa(Orientation::Horizontal, 12, "cfg-conta-fixa");
            let gi = caixa(Orientation::Horizontal, 0, "cfg-avatar");
            gi.set_size_request(36, 36);
            gi.set_homogeneous(true);
            gi.set_valign(Align::Center);
            gi.append(&ui_icon("usuario", "#8E8F99", 16));
            g.append(&gi);
            let col = caixa(Orientation::Vertical, 2, "");
            col.set_hexpand(true);
            col.set_valign(Align::Center);
            col.append(&left(tr("Guest"), "cfg-texto-13"));
            col.append(&left(tr("a session that is wiped when it ends · no agents"), "cfg-sub"));
            g.append(&col);
            let sw = interruptor();
            sw.set_active(on);
            let w = Rc::downgrade(self);
            sw.connect_state_set(move |_, v| {
                if let Some(p) = w.upgrade() {
                    p.system_call("usuarios.convidado", json!({"ligado": v}));
                }
                glib::Propagation::Proceed
            });
            g.append(&sw);
            self.accounts.insert_child_after(&g, self.accounts.last_child().and_then(|l| l.prev_sibling()).as_ref());
        }
        // the chosen account's panel
        clear(&self.account_panel);
        let Some(a) = accounts.iter().find(|a| Some(&a.login) == chosen.as_ref()).cloned() else { return };
        self.account_panel.append(&rotulo_cartao("usuario", tr("CHOSEN ACCOUNT")));
        let head = caixa(Orientation::Horizontal, 12, "");
        // (one's own photo: the avatar opens "Choose photo…" / "Remove photo", design 129857)
        head.append(&if a.me { self.photo_button(&a) } else { avatar(&a, 52) });
        let hc = caixa(Orientation::Vertical, 2, "");
        hc.set_valign(Align::Center);
        hc.append(&left(&a.name, "cfg-titulo-cartao"));
        hc.append(&left(&format!("{} · {}", a.login, if a.admin { tr("administrator") } else { tr("standard") }), "cfg-sub"));
        head.append(&hc);
        self.account_panel.append(&head);
        let card = caixa(Orientation::Vertical, 2, "cfg-cartao");
        let chev = || ui_icon("chevron-n", "#7A7B88", 16);
        if a.me {
            let pw = gtk::Button::new();
            pw.add_css_class("cfg-linha-botao");
            pw.set_child(Some(&row(tr("Password"), Some(tr("change it in the terminal (passwd)")), &chev())));
            // (in KalangoOS's own Terminal: the other systems' terminals are not in KalangoOS, and nothing happened)
            pw.connect_clicked(|_| in_terminal("passwd"));
            card.append(&pw);
        }
        let lock = interruptor();
        lock.set_active(crate::configuracoes::ajustes()["bloquear_ao_suspender"].as_bool().unwrap_or(true));
        lock.connect_state_set(|_, on| {
            crate::configuracoes::salvar_ajuste("bloquear_ao_suspender", json!(on));
            glib::Propagation::Proceed
        });
        if a.me {
            card.append(&row(tr("Lock when suspending"), None, &lock));
            // the fingerprints fprintd has for this person (a click enrolls one in the terminal)
            if let Some(out) = sh("fprintd-list", &[&a.login]) {
                let n = out.lines().filter(|l| l.trim_start().starts_with("- #")).count();
                let fp = gtk::Button::new();
                fp.add_css_class("cfg-linha-botao");
                fp.set_child(Some(&row(tr("Fingerprint"), Some(&if n == 0 { tr("none enrolled").to_string() } else { trf("{n} fingers enrolled", &[("n", n.to_string())]) }), &chev())));
                fp.connect_clicked(|_| in_terminal("fprintd-enroll"));
                card.append(&fp);
            }
            // signing in without a password at start (greetd's initial session; kalangod, approved)
            let greetd = std::fs::read_to_string("/etc/greetd/config.toml").unwrap_or_default();
            let auto_now = greetd.split("[initial_session]").nth(1).and_then(|t| t.lines().find_map(|l| l.trim().strip_prefix("user").map(|r| r.trim_start_matches([' ', '=']).trim().trim_matches('"').to_string()))).unwrap_or_default();
            let auto = interruptor();
            auto.set_active(auto_now == a.login);
            let (w, login) = (Rc::downgrade(self), a.login.clone());
            auto.connect_state_set(move |_, v| {
                if let Some(p) = w.upgrade() {
                    p.system_call("sessao.entrar_sem_senha", json!({"usuario": if v { login.clone() } else { String::new() }}));
                }
                glib::Propagation::Proceed
            });
            card.append(&row(tr("Sign in without a password at start"), None, &auto));
            // the time of use: the shell's samples of the last 7 days (one each 5 minutes the session is on)
            let state = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| glib::home_dir().join(".local/state"));
            let samples = std::fs::read_to_string(state.join("kalango/energia.jsonl")).unwrap_or_default();
            let now = glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0);
            let n = samples.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()).filter(|v| v["t"].as_i64().is_some_and(|t| now - t < 7 * 86_400)).count();
            if n > 0 {
                let hours = n as f64 * 5.0 / 60.0 / 7.0;
                card.append(&row(tr("Time of use"), Some(&trf("about {h} h a day", &[("h", format!("{hours:.1}").replace('.', ","))])), &rotulo("", "")));
            }
        }
        let agents = gtk::Button::new();
        agents.add_css_class("cfg-linha-botao");
        agents.set_child(Some(&row(tr("Agents and memory of this account"), Some(tr("each person has their own memory")), &chev())));
        agents.connect_clicked(|_| {
            if let Some(s) = crate::shell_atual() {
                s.abrir_central(Some("memoria"));
            }
        });
        card.append(&agents);
        self.account_panel.append(&card);
        if !a.me {
            let rm = gtk::Button::new();
            rm.add_css_class("cfg-botao-perigo");
            let rc = caixa(Orientation::Horizontal, 6, "");
            rc.append(&ui_icon("lixeira", "#EFA2A6", 14));
            rc.append(&rotulo(tr("Remove this account"), ""));
            rm.set_child(Some(&rc));
            rm.set_halign(Align::Start);
            let (w, login) = (Rc::downgrade(self), a.login.clone());
            rm.connect_clicked(move |b| {
                if !b.has_css_class("confirmar") {
                    b.add_css_class("confirmar");
                    return;
                }
                if let Some(p) = w.upgrade() {
                    p.system_call("usuarios.remover", json!({"nome": login, "apagar_casa": false}));
                }
            });
            self.account_panel.append(&rm);
        }
        self.sub(&if accounts.len() == 1 { tr("1 person uses this computer").to_string() } else { trf("{n} people use this computer", &[("n", accounts.len().to_string())]) });
    }

    // ---- Date and language

    fn tick(&self) {
        let now = glib::DateTime::now_local().ok();
        let h12 = self.h12.is_active();
        let fmt = match (h12, self.seconds.is_active()) {
            (false, false) => "%H:%M",
            (false, true) => "%H:%M:%S",
            (true, false) => "%I:%M %p",
            (true, true) => "%I:%M:%S %p",
        };
        self.clock.set_text(&now.as_ref().and_then(|d| d.format(fmt).ok()).map(|s| s.to_string()).unwrap_or_default());
        self.date.set_text(&now.as_ref().and_then(|d| d.format(tr("%A, %B %-d, %Y")).ok()).map(|s| s.to_string()).unwrap_or_default());
    }

    fn refresh_date(self: &Rc<Self>) {
        self.loading.set(true);
        let a = crate::configuracoes::ajustes();
        if a["relogio_12h"].as_bool().unwrap_or(false) { self.h12.set_active(true) } else { self.h24.set_active(true) }
        self.seconds.set_active(a["relogio_segundos"].as_bool().unwrap_or(false));
        self.first_day.set_selected(a["primeiro_dia_da_semana"].as_u64().unwrap_or(0).min(1) as u32);
        let ans = a["idioma_respostas"].as_str().unwrap_or("");
        self.answers.set_selected(["", "pt_BR", "en_US", "es_ES"].iter().position(|x| *x == ans).unwrap_or(0) as u32);
        self.tick();
        self.loading.set(false);
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(|| {
            let show = sh("timedatectl", &["show"]).unwrap_or_default();
            let ntp = show.lines().any(|l| l == "NTP=yes");
            let zone = show.lines().find_map(|l| l.strip_prefix("Timezone=")).map(str::to_string).unwrap_or_else(|| "UTC".into());
            let zones: Vec<String> = sh("timedatectl", &["list-timezones"]).map(|z| z.lines().map(str::to_string).collect()).unwrap_or_default();
            let locales: Vec<String> = sh("locale", &["-a"]).map(|l| l.lines().filter(|x| x.contains(".utf8") || x.contains(".UTF-8")).map(|x| x.replace(".utf8", ".UTF-8")).collect()).unwrap_or_default();
            let lang = std::env::var("LANG").unwrap_or_default().replace(".utf8", ".UTF-8");
            let fmt = std::env::var("LC_TIME").ok().filter(|l| !l.is_empty()).unwrap_or_else(|| lang.clone()).replace(".utf8", ".UTF-8");
            (ntp, zone, zones, locales, lang, fmt)
        }, move |(ntp, zone, zones, locales, lang, fmt)| {
            let Some(p) = w.upgrade() else { return };
            p.loading.set(true);
            p.ntp.set_active(ntp);
            if *p.zones.borrow() != zones {
                let names: Vec<String> = zones.iter().map(|z| z.replace('_', " ")).collect();
                p.zone.set_model(Some(&gtk::StringList::new(&names.iter().map(String::as_str).collect::<Vec<_>>())));
                *p.zones.borrow_mut() = zones.clone();
            }
            if let Some(i) = zones.iter().position(|z| *z == zone) {
                if p.zone.selected() != i as u32 {
                    p.zone.set_selected(i as u32);
                }
            }
            let mut locs = locales;
            locs.sort();
            locs.dedup();
            if *p.languages.borrow() != locs {
                let names: Vec<String> = locs.iter().map(|l| locale_name(l)).collect();
                let model = gtk::StringList::new(&names.iter().map(String::as_str).collect::<Vec<_>>());
                p.language.set_model(Some(&model));
                p.formats.set_model(Some(&model));
                *p.languages.borrow_mut() = locs.clone();
            }
            if let Some(i) = locs.iter().position(|l| *l == lang) {
                p.language.set_selected(i as u32);
            }
            if let Some(i) = locs.iter().position(|l| *l == fmt) {
                p.formats.set_selected(i as u32);
            }
            p.loading.set(false);
            p.fill_other_languages(&lang, &fmt, &locs);
            p.sub(&format!("{} · {}", locale_name(&lang), zone.rsplit('/').next().unwrap_or(&zone).replace('_', " ")));
            if depurando() {
                eprintln!("debug sistema: data (ntp {ntp}, fuso {zone}, {} fusos, {} idiomas)", zones.len(), locs.len());
            }
        });
    }

    /// "Outros idiomas": the languages after the system's (LANGUAGE), and one more from the installed ones.
    fn fill_other_languages(self: &Rc<Self>, lang: &str, fmt: &str, locs: &[String]) {
        clear(&self.others);
        let others: Vec<String> = std::env::var("LANGUAGE").unwrap_or_default().split(':').skip(1).filter(|l| !l.is_empty()).map(|l| l.to_string()).collect();
        let t = caixa(Orientation::Vertical, 1, "");
        t.set_hexpand(true);
        t.set_valign(Align::Center);
        t.append(&left(tr("Other languages"), "cfg-texto-13"));
        t.append(&left(&if others.is_empty() { tr("none").to_string() } else { others.iter().map(|o| locale_name(o)).collect::<Vec<_>>().join(", ") }, "cfg-sub"));
        self.others.append(&t);
        let add = gtk::MenuButton::new();
        add.add_css_class("cfg-botao-menu");
        add.set_label(tr("Add"));
        add.set_valign(Align::Center);
        let pop = gtk::Popover::new();
        pop.add_css_class("arq-menu");
        pop.set_has_arrow(false);
        let c = caixa(Orientation::Vertical, 2, "");
        c.set_size_request(220, -1);
        for l in locs.iter().filter(|l| l.as_str() != lang && !others.iter().any(|o| l.starts_with(o.as_str()))) {
            let b = crate::arquivos::item_menu_publico(&c, "nenhum", "#E2E2E6", &locale_name(l));
            let (w, l2, lang, fmt, outros, pw) = (Rc::downgrade(self), l.clone(), lang.to_string(), fmt.to_string(), others.clone(), pop.downgrade());
            b.connect_clicked(move |_| {
                if let Some(p) = pw.upgrade() {
                    p.popdown();
                }
                let mut outros = outros.clone();
                outros.push(l2.clone());
                if let Some(p) = w.upgrade() {
                    p.system_call("idioma.sistema", json!({"idioma": lang, "formatos": fmt, "outros": outros}));
                }
            });
        }
        pop.set_child(Some(&c));
        add.set_popover(Some(&pop));
        self.others.append(&add);
    }

    // ---- Default apps

    fn refresh_apps(self: &Rc<Self>) {
        clear(&self.defaults);
        for (name, types) in DEFAULTS {
            let ct = types[0];
            let mut apps: Vec<gio::AppInfo> = Vec::new();
            for t in types.iter() {
                for a in gio::AppInfo::all_for_type(t) {
                    if a.should_show() && !apps.iter().any(|x| x.id() == a.id()) {
                        apps.push(a);
                    }
                }
            }
            let now = gio::AppInfo::default_for_type(ct, false);
            let names: Vec<String> = apps.iter().map(|a| a.display_name().to_string()).collect();
            let d = gtk::DropDown::new(Some(gtk::StringList::new(&names.iter().map(String::as_str).collect::<Vec<_>>())), None::<gtk::Expression>);
            d.add_css_class("cfg-seletor");
            d.set_size_request(220, 34);
            d.set_sensitive(!apps.is_empty());
            if let Some(i) = now.as_ref().and_then(|n| apps.iter().position(|a| a.id() == n.id())) {
                d.set_selected(i as u32);
            }
            let types: Vec<&'static str> = types.to_vec();
            d.connect_selected_notify(move |d| {
                let Some(a) = apps.get(d.selected() as usize) else { return };
                for t in &types {
                    if let Err(e) = a.set_as_default_for_type(t) {
                        eprintln!("kalango-shell: app padrão {t}: {e}");
                    }
                }
                if depurando() {
                    eprintln!("debug sistema: padrão {} → {}", types[0], a.display_name());
                }
            });
            self.register(&format!("cfg padrao {}", ct), &d);
            self.defaults.append(&row(tr(name), None, &d));
        }
        // the default agent: the local model it runs on (the installed ones; the agent reads the choice)
        let models: Vec<PathBuf> = std::fs::read_dir(glib::user_data_dir().join("kalango/modelos")).into_iter().flatten().flatten().map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "gguf") && !p.to_string_lossy().contains("mmproj") && !p.to_string_lossy().contains("Audio")).collect();
        let names: Vec<String> = models.iter().map(|m| m.file_stem().map(|s| s.to_string_lossy().replace(['-', '_'], " ")).unwrap_or_default()).collect();
        let ad = gtk::DropDown::new(Some(gtk::StringList::new(&names.iter().map(String::as_str).collect::<Vec<_>>())), None::<gtk::Expression>);
        ad.add_css_class("cfg-seletor");
        ad.set_size_request(220, 34);
        ad.set_sensitive(models.len() > 1);
        let now = crate::configuracoes::ajustes()["modelo_padrao"].as_str().map(PathBuf::from);
        if let Some(i) = now.and_then(|n| models.iter().position(|m| *m == n)) {
            ad.set_selected(i as u32);
        }
        ad.connect_selected_notify(move |d| {
            if let Some(m) = models.get(d.selected() as usize) {
                crate::configuracoes::salvar_ajuste("modelo_padrao", json!(m.display().to_string()));
                if depurando() {
                    eprintln!("debug sistema: agente padrão {}", m.display());
                }
            }
        });
        self.defaults.append(&row(tr("Default agent"), Some(tr("the local model the agent runs on")), &ad));
        // the terminal: the one the system's shortcuts open (KALANGO_TERMINAL), shown as it is
        let term = std::env::var("KALANGO_TERMINAL").ok().filter(|t| !t.is_empty()).unwrap_or_else(|| "kgx".into());
        self.defaults.append(&row(tr("Terminal"), Some(tr("the one Super+T opens")), &rotulo(&term, "cfg-valor-pequeno")));
        self.fill_installed();
        // the apps that start with the session (~/.config/autostart)
        clear(&self.autostart);
        let dir = glib::user_config_dir().join("autostart");
        let mut any = false;
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let p = e.path();
            if p.extension().is_none_or(|x| x != "desktop") {
                continue;
            }
            let text = std::fs::read_to_string(&p).unwrap_or_default();
            let name = text.lines().find_map(|l| l.strip_prefix("Name=")).unwrap_or("").to_string();
            let off = text.lines().any(|l| l.trim() == "Hidden=true" || l.trim() == "X-GNOME-Autostart-enabled=false");
            let sw = interruptor();
            sw.set_active(!off);
            let pp = p.clone();
            sw.connect_state_set(move |_, on| {
                let t = std::fs::read_to_string(&pp).unwrap_or_default();
                let mut lines: Vec<String> = t.lines().filter(|l| !l.starts_with("Hidden=") && !l.starts_with("X-GNOME-Autostart-enabled=")).map(str::to_string).collect();
                if !on {
                    lines.push("Hidden=true".into());
                }
                let _ = std::fs::write(&pp, lines.join("\n") + "\n");
                glib::Propagation::Proceed
            });
            self.autostart.append(&row(&name, None, &sw));
            any = true;
        }
        if !any {
            let l = left(tr("No app opens by itself when you sign in."), "cfg-sub");
            l.set_margin_start(10);
            self.autostart.append(&l);
        }
        let n = gio::AppInfo::all().iter().filter(|a| a.should_show()).count();
        self.sub(&trf("{n} apps installed", &[("n", n.to_string())]));
    }

    fn fill_installed(self: &Rc<Self>) {
        clear(&self.installed);
        let q = self.app_search.text().trim().to_lowercase();
        let mut apps: Vec<gio::AppInfo> = gio::AppInfo::all().into_iter().filter(|a| a.should_show()).collect();
        apps.sort_by_key(|a| a.display_name().to_lowercase());
        for a in apps.into_iter().filter(|a| q.is_empty() || a.display_name().to_lowercase().contains(&q)).take(60) {
            let r = caixa(Orientation::Horizontal, 10, "cfg-tela-opcao");
            let ic = a.icon().map(|i| gtk::Image::from_gicon(&i)).unwrap_or_else(|| gtk::Image::from_icon_name("application-x-executable"));
            ic.set_pixel_size(24);
            r.append(&ic);
            let col = caixa(Orientation::Vertical, 1, "");
            col.set_hexpand(true);
            col.set_valign(Align::Center);
            col.append(&left(&a.display_name(), "cfg-texto-13"));
            let id = a.id().map(|s| s.to_string()).unwrap_or_default();
            let sub = left(&id.trim_end_matches(".desktop").to_string(), "cfg-sub");
            sub.set_ellipsize(gtk::pango::EllipsizeMode::End);
            sub.set_max_width_chars(1);
            col.append(&sub);
            r.append(&col);
            self.installed.append(&r);
        }
    }

    // ---- Printers

    fn refresh_printers(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(read_printers, move |ps| {
            if let Some(p) = w.upgrade() {
                p.fill_printers(ps);
            }
        });
    }

    fn fill_printers(self: &Rc<Self>, ps: Vec<Printer>) {
        clear(&self.printers);
        if self.printer_chosen.borrow().is_none() {
            *self.printer_chosen.borrow_mut() = ps.iter().find(|p| p.default).or(ps.first()).map(|p| p.name.clone());
        }
        let chosen = self.printer_chosen.borrow().clone();
        for p in &ps {
            let b = gtk::Button::new();
            b.add_css_class("cfg-conta");
            if chosen.as_deref() == Some(&p.name) {
                b.add_css_class("escolhida");
            }
            let c = caixa(Orientation::Horizontal, 12, "");
            let ic = caixa(Orientation::Horizontal, 0, "cfg-impressora-icone");
            ic.set_size_request(36, 36);
            ic.set_valign(Align::Center);
            ic.set_homogeneous(true);
            ic.append(&ui_icon("doc", if p.ready { "#8ED2C3" } else { "#E5C98C" }, 16));
            c.append(&ic);
            let col = caixa(Orientation::Vertical, 2, "");
            col.set_hexpand(true);
            col.set_valign(Align::Center);
            col.append(&left(&p.name.replace('_', " "), "cfg-texto-13"));
            let mut sub = p.state.clone();
            if p.default {
                sub = format!("{sub} · {}", tr("default"));
            }
            col.append(&left(&sub, if p.ready { "cfg-sub" } else { "cfg-sub-alerta" }));
            c.append(&col);
            c.append(&ui_icon("chevron-n", "#7A7B88", 16));
            b.set_child(Some(&c));
            let (w, n) = (Rc::downgrade(self), p.name.clone());
            b.connect_clicked(move |_| {
                if let Some(pg) = w.upgrade() {
                    *pg.printer_chosen.borrow_mut() = Some(n.clone());
                    pg.refresh_printers();
                }
            });
            self.printers.append(&b);
        }
        let add = gtk::Button::new();
        add.add_css_class("cfg-adicionar-tracejado");
        let ac = caixa(Orientation::Horizontal, 6, "");
        ac.set_halign(Align::Center);
        ac.append(&ui_icon("mais", "#8E8F99", 14));
        ac.append(&rotulo(tr("Add a printer or scanner"), ""));
        add.set_child(Some(&ac));
        let tool = ["system-config-printer", "gnome-control-center"].iter().find(|t| glib::find_program_in_path(t).is_some()).map(|t| t.to_string());
        add.set_sensitive(tool.is_some());
        add.connect_clicked(move |_| {
            if let Some(t) = &tool {
                let mut c = std::process::Command::new(t);
                if t == "gnome-control-center" {
                    c.arg("printers");
                }
                let _ = c.spawn();
            }
        });
        self.printers.append(&add);
        clear(&self.printer_panel);
        let Some(p) = ps.iter().find(|p| Some(&p.name) == chosen.as_ref()).cloned() else {
            if ps.is_empty() {
                let l = left(tr("No printer yet. A printer on the network or plugged in shows here."), "cfg-sub");
                l.set_wrap(true);
                l.set_max_width_chars(52);
                self.printer_panel.append(&l);
            }
            self.sub(&tr("no printer"));
            return;
        };
        self.printer_panel.append(&rotulo_cartao("doc", &p.name.replace('_', " ").to_uppercase()));
        let card = caixa(Orientation::Vertical, 6, "");
        kv(&card, tr("State")).set_text(&p.state);
        kv(&card, tr("Address")).set_text(p.uri.split("://").nth(1).unwrap_or(&p.uri));
        // the toner or ink, when the printer says (IPP's marker-levels)
        let name_ipp = p.name.clone();
        let tc = card.clone();
        crate::lateral::em_segundo_plano(move || sh("ipptool", &["-tv", &format!("ipp://localhost/printers/{name_ipp}"), "get-printer-attributes.test"])
            .and_then(|o| o.lines().find(|l| l.contains("marker-levels")).and_then(|l| l.rsplit('=').next()).and_then(|v| v.trim().split(',').next()?.trim().parse::<i32>().ok())),
            move |lvl| {
                if let Some(l) = lvl.filter(|l| *l >= 0) {
                    kv(&tc, tr("Toner")).set_text(&format!("{l}%"));
                    let lb = gtk::LevelBar::for_interval(0.0, 100.0);
                    lb.add_css_class("arq-disco-barra");
                    lb.set_value(l as f64);
                    tc.append(&lb);
                }
            });
        self.printer_panel.append(&card);
        self.printer_panel.append(&rotulo_cartao("relogio", tr("PRINT QUEUE")));
        if p.jobs.is_empty() {
            self.printer_panel.append(&left(tr("Nothing waiting."), "cfg-sub"));
        }
        for (id, user, size) in &p.jobs {
            let cancel = gtk::Button::with_label(tr("Cancel"));
            cancel.add_css_class("cfg-botao");
            let (w, id2) = (Rc::downgrade(self), id.clone());
            cancel.connect_clicked(move |_| {
                let _ = std::process::Command::new("cancel").arg(&id2).status();
                if let Some(p) = w.upgrade() {
                    p.refresh_printers();
                }
            });
            self.printer_panel.append(&row(id, Some(&format!("{user} · {size} bytes")), &cancel));
        }
        let bs = caixa(Orientation::Horizontal, 8, "");
        bs.set_homogeneous(true);
        let test = gtk::Button::with_label(tr("Test page"));
        test.add_css_class("cfg-botao");
        let name = p.name.clone();
        test.connect_clicked(move |_| {
            let page = ["/usr/share/cups/data/testprint", "/usr/share/cups/data/default-testpage.pdf"].iter().find(|f| Path::new(f).is_file()).map(|s| s.to_string());
            if let Some(f) = page {
                let _ = std::process::Command::new("lp").args(["-d", &name, &f]).status();
            }
        });
        bs.append(&test);
        // pausing its queue (CUPS: cupsdisable / cupsenable; the lpadmin group can)
        let paused = p.state == tr("paused");
        let pause = gtk::Button::with_label(if paused { tr("Resume queue") } else { tr("Pause queue") });
        pause.add_css_class("cfg-botao");
        let (w, name) = (Rc::downgrade(self), p.name.clone());
        pause.connect_clicked(move |_| {
            let _ = std::process::Command::new(if paused { "cupsenable" } else { "cupsdisable" }).arg(&name).status();
            if let Some(p) = w.upgrade() {
                p.refresh_printers();
            }
        });
        bs.append(&pause);
        self.printer_panel.append(&bs);
        // both sides by default (the person's own options, lpoptions)
        let opts = sh("lpoptions", &["-p", &p.name]).unwrap_or_default();
        let duplex = interruptor();
        duplex.set_active(opts.contains("sides=two-sided"));
        let name = p.name.clone();
        duplex.connect_state_set(move |_, v| {
            let _ = std::process::Command::new("lpoptions").args(["-p", &name, "-o", if v { "sides=two-sided-long-edge" } else { "sides=one-sided" }]).status();
            glib::Propagation::Proceed
        });
        self.printer_panel.append(&row(tr("Both sides by default"), None, &duplex));
        let def = interruptor();
        def.set_active(p.default);
        def.set_sensitive(!p.default);
        let (w, name) = (Rc::downgrade(self), p.name.clone());
        def.connect_state_set(move |_, on| {
            if on {
                // the person's own default (lpoptions), no administrator needed
                let _ = std::process::Command::new("lpoptions").args(["-d", &name]).status();
                if let Some(p) = w.upgrade() {
                    p.refresh_printers();
                }
            }
            glib::Propagation::Proceed
        });
        self.printer_panel.append(&row(tr("Use as default"), None, &def));
        let ready = ps.iter().filter(|p| p.ready).count();
        self.sub(&if ready == 1 { tr("1 printer ready").to_string() } else { trf("{n} printers ready", &[("n", ready.to_string())]) });
    }
}

/// The screens' sizes as GDK sees them ("3440 × 1440 e 1080 × 1920").
fn screens_now() -> String {
    let Some(d) = gtk::gdk::Display::default() else { return String::new() };
    let ms = d.monitors();
    let v: Vec<String> = (0..ms.n_items()).filter_map(|i| ms.item(i).and_downcast::<gtk::gdk::Monitor>()).map(|m| {
        let g = m.geometry();
        let k = m.scale_factor();
        format!("{} × {}", g.width() * k, g.height() * k)
    }).collect();
    match v.len() {
        0 => String::new(),
        1 => v[0].clone(),
        _ => {
            let (last, rest) = v.split_last().unwrap();
            format!("{} {} {last}", rest.join(", "), tr("and"))
        }
    }
}

fn rounded(cr: &gtk::cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    use std::f64::consts::PI;
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -PI / 2.0, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, PI / 2.0);
    cr.arc(x + r, y + h - r, r, PI / 2.0, PI);
    cr.arc(x + r, y + r, r, PI, 3.0 * PI / 2.0);
    cr.close_path();
}

/// The photo as the account's: cropped to the middle square, 256 px, written as ~/.face (beside, then renamed).
fn save_face(from: &Path, home: &Path) -> Result<(), String> {
    let pb = gtk::gdk_pixbuf::Pixbuf::from_file(from).map_err(|e| e.to_string())?;
    let (w, h) = (pb.width(), pb.height());
    let side = w.min(h);
    let square = pb.new_subpixbuf((w - side) / 2, (h - side) / 2, side, side);
    let out = square.scale_simple(256, 256, gtk::gdk_pixbuf::InterpType::Bilinear).ok_or("scale")?;
    let tmp = home.join(".face.novo");
    out.savev(&tmp, "png", &[]).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, home.join(".face")).map_err(|e| e.to_string())
}

/// Runs `command` in a new tab of KalangoOS's Terminal (passwd, fprintd-enroll: they ask in the terminal).
fn in_terminal(command: &str) {
    if let Some(shell) = crate::shell_atual() {
        crate::terminal::run(&shell, None, command);
    }
}

/// An account's picture (~/.face) or its initial.
fn avatar(a: &Account, px: i32) -> gtk::Widget {
    let face = a.home.join(".face");
    if face.is_file() {
        let pic = gtk::Picture::for_filename(&face);
        pic.set_content_fit(gtk::ContentFit::Cover);
        pic.set_size_request(px, px);
        pic.add_css_class("cfg-avatar");
        pic.set_overflow(gtk::Overflow::Hidden);
        pic.set_valign(Align::Center);
        return pic.upcast();
    }
    let b = caixa(Orientation::Horizontal, 0, "cfg-avatar");
    b.set_size_request(px, px);
    b.set_homogeneous(true);
    b.set_valign(Align::Center);
    let l = rotulo(&a.name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default(), "cfg-avatar-letra");
    b.append(&l);
    b.upcast()
}

/// A locale's name in words ("pt_BR.UTF-8" → "Português (Brasil)").
fn locale_name(l: &str) -> String {
    let code = l.split('.').next().unwrap_or(l);
    let (lang, region) = code.split_once('_').unwrap_or((code, ""));
    // Display names for locales from `locale -a` (not the UI catalog list — see kalango_i18n::has_ui_catalog).
    let langs = [
        ("pt", "Português"), ("en", "English"), ("es", "Español"), ("fr", "Français"), ("de", "Deutsch"),
        ("it", "Italiano"), ("ja", "日本語"), ("zh", "中文"), ("ru", "Русский"), ("ar", "العربية"),
        ("hi", "हिन्दी"), ("ko", "한국어"), ("nl", "Nederlands"), ("pl", "Polski"), ("tr", "Türkçe"),
        ("uk", "Українська"), ("vi", "Tiếng Việt"), ("th", "ไทย"), ("id", "Bahasa Indonesia"),
        ("sv", "Svenska"), ("nb", "Norsk"), ("nn", "Norsk"), ("no", "Norsk"), ("fi", "Suomi"),
        ("da", "Dansk"), ("cs", "Čeština"), ("el", "Ελληνικά"), ("he", "עברית"), ("fa", "فارسی"),
        ("bn", "বাংলা"), ("ro", "Română"), ("hu", "Magyar"), ("sk", "Slovenčina"), ("hr", "Hrvatski"),
        ("ca", "Català"), ("gl", "Galego"), ("eu", "Euskara"), ("C", "C"),
    ];
    let regions = [
        ("BR", "Brasil"), ("PT", "Portugal"), ("US", "US"), ("GB", "UK"), ("ES", "España"),
        ("MX", "México"), ("AR", "Argentina"), ("FR", "France"), ("DE", "Deutschland"), ("IT", "Italia"),
        ("JP", "日本"), ("CN", "中国"), ("TW", "台灣"), ("KR", "한국"), ("RU", "Россия"),
        ("IN", "India"), ("SA", "السعودية"), ("AE", "UAE"), ("NL", "Nederland"), ("PL", "Polska"),
        ("TR", "Türkiye"), ("UA", "Україна"), ("VN", "Việt Nam"), ("TH", "ไทย"), ("ID", "Indonesia"),
        ("SE", "Sverige"), ("NO", "Norge"), ("FI", "Suomi"), ("DK", "Danmark"), ("CZ", "Česko"),
        ("GR", "Ελλάδα"), ("IL", "ישראל"), ("IR", "ایران"), ("BD", "বাংলাদেশ"), ("RO", "România"),
        ("HU", "Magyarország"), ("SK", "Slovensko"), ("HR", "Hrvatska"), ("CA", "Canada"),
        ("AU", "Australia"), ("NZ", "New Zealand"), ("ZA", "South Africa"), ("CH", "Schweiz"),
        ("AT", "Österreich"), ("BE", "België"), ("CL", "Chile"), ("CO", "Colombia"), ("PE", "Perú"),
    ];
    let ln = langs.iter().find(|(c, _)| *c == lang).map(|(_, n)| n.to_string()).unwrap_or_else(|| lang.to_string());
    match regions.iter().find(|(c, _)| *c == region).map(|(_, n)| n.to_string()).or_else(|| (!region.is_empty()).then(|| region.to_string())) {
        Some(r) => format!("{ln} ({r})"),
        None => ln,
    }
}

/// "R$ 1.234,56" in the person's formats.
fn money_example() -> String {
    let lc = std::env::var("LC_MONETARY").or_else(|_| std::env::var("LANG")).unwrap_or_default();
    if lc.starts_with("pt_BR") { "R$ 1.234,56".into() } else if lc.starts_with("en_US") { "$1,234.56".into() } else { "1 234,56".into() }
}

/// The person's caches (what apps make again); the shell's own previews stay.
pub(crate) fn clear_cache() -> usize {
    let base = glib::user_cache_dir();
    let mut n = 0;
    for e in std::fs::read_dir(&base).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name == "kalango" || name.starts_with('.') {
            continue;
        }
        let p = e.path();
        let ok = if p.is_dir() { std::fs::remove_dir_all(&p) } else { std::fs::remove_file(&p) };
        if ok.is_ok() {
            n += 1;
        }
    }
    n
}

pub(crate) fn empty_trash() -> usize {
    let base = glib::user_data_dir().join("Trash");
    let mut n = 0;
    for sub in ["files", "info"] {
        for e in std::fs::read_dir(base.join(sub)).into_iter().flatten().flatten() {
            let p = e.path();
            let ok = if p.is_dir() { std::fs::remove_dir_all(&p) } else { std::fs::remove_file(&p) };
            if ok.is_ok() && sub == "files" {
                n += 1;
            }
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_and_locales_read_as_people_say_them() {
        assert_eq!(human(1_000_000_000_000), "1 TB");
        assert_eq!(human(412_300_000_000), "412 GB");
        assert_eq!(human(2_400_000_000), "2,4 GB");
        assert_eq!(locale_name("pt_BR.UTF-8"), "Português (Brasil)");
        assert_eq!(locale_name("en_US.UTF-8"), "English (US)");
        assert_eq!(locale_name("ja_JP.UTF-8"), "日本語 (日本)");
        assert_eq!(locale_name("ar_SA.UTF-8"), "العربية (السعودية)");
    }

    #[test]
    fn about_reads_this_computer() {
        let a = read_about();
        assert!(a.cpu.contains("threads"), "{}", a.cpu);
        assert!(a.memory.ends_with("GB"));
        assert!(a.kernel.starts_with("Linux"));
    }
}

#[cfg(test)]
mod photo_tests {
    use super::*;

    #[test]
    fn a_photo_becomes_a_256_square_face() {
        let d = std::env::temp_dir().join(format!("kalango-face-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        // a wide picture: the middle square is kept
        let pb = gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, false, 8, 600, 300).unwrap();
        pb.fill(0x3366ccff);
        let src = d.join("foto.png");
        pb.savev(&src, "png", &[]).unwrap();
        save_face(&src, &d).unwrap();
        let face = gtk::gdk_pixbuf::Pixbuf::from_file(d.join(".face")).unwrap();
        assert_eq!((face.width(), face.height()), (256, 256));
        assert!(!d.join(".face.novo").exists());
        // not a picture: nothing is written
        std::fs::write(d.join("nota.txt"), "oi").unwrap();
        std::fs::remove_file(d.join(".face")).unwrap();
        assert!(save_face(&d.join("nota.txt"), &d).is_err());
        assert!(!d.join(".face").exists());
        std::fs::remove_dir_all(&d).unwrap();
    }
}
