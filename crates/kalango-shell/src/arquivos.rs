//! Arquivos (designs "Arquivos" and "Arquivos · menu de contexto com ações de IA"; the agent at work in it: "Agente
//! usando a tela"): KalangoOS's file manager — a window of the system (app id os.kalango.Arquivos), opened by the
//! dock, by `kalango-shell arquivos [FOLDER]` and by any app that opens a folder (inode/directory).
//!
//! On the left the pinned folders (the person's own: home, Downloads, Documents, Pictures, then GTK's bookmarks), the
//! trash, the tags (freedesktop's `user.xdg.tags`: drop files on one to tag them, click it to filter) and the devices
//! with how full they are. In the middle the folder: kinds to filter by, the files used lately, its folders and
//! files (pictures and the system thumbnailers' previews, made in the background and kept in the freedesktop
//! thumbnail cache; duplicates found by their content); a click selects (Ctrl adds, Shift a range), a double click
//! opens, Delete sends to the trash (Ctrl+Z brings back), F2 renames, Ctrl+C/X/V copy and move, files drag to other
//! folders, tags and apps. On the right the chosen file: what it is, when it changed, where it is, its tags, and the
//! agent's summary of it (on request; kept until the file changes).
//!
//! The agent works from here too — ask about a file, summarise it, organise the folder, find it a better name —
//! through the side bar's conversation, with its permissions and the approval notice. While it works on this folder
//! everyone sees it: the banner under the top bar with a stop, the island's progress, the window outlined, its cursor
//! on the file it touches.

use crate::conversa::ui_icon;

#[path = "arquivos_dialogos.rs"]
mod dialogos;
#[path = "arquivos_seletor.rs"]
pub(crate) mod seletor;
use crate::superficies::{caixa, estrela, rotulo};
use crate::Shell;
use gtk::prelude::*;
use gtk::{gdk, gio, glib, Align, Orientation};
use kalango_i18n::{n_, tr, trf};
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ffi::{c_char, c_int, c_void, CString};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::time::Duration;

extern "C" {
    fn gdk_wayland_toplevel_set_application_id(toplevel: *mut gdk::ffi::GdkToplevel, id: *const c_char);
    fn getxattr(path: *const c_char, name: *const c_char, value: *mut c_void, size: usize) -> isize;
    fn setxattr(path: *const c_char, name: *const c_char, value: *const c_void, size: usize, flags: c_int) -> c_int;
    fn removexattr(path: *const c_char, name: *const c_char) -> c_int;
}

const FILTROS: &[&str] = &[n_("All"), n_("Documents"), n_("Images"), n_("Installers"), n_("Recent"), n_("Organised by the agent")];
const ORDENS: &[&str] = &[n_("Modified"), n_("Name"), n_("Size"), n_("Kind")];
/// The tones of the design (tiles, tags, bars): CSS classes tom-*.
const TONS: &[(&str, &str)] = &[("roxo", "#A5A1E2"), ("verde", "#8ED2C3"), ("azul", "#7CC3DF"), ("coral", "#EFA2A6"), ("ouro", "#E5C98C"), ("cinza", "#8E8F99")];

fn tom_de(cor: &str) -> &'static str {
    TONS.iter().find(|(_, c)| c.eq_ignore_ascii_case(cor)).map(|(t, _)| *t).unwrap_or("cinza")
}

fn depurando() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

// ---------------------------------------------------------------- the folder's entries

#[derive(Clone, Debug)]
struct Item {
    caminho: PathBuf,
    nome: String,
    pasta: bool,
    tamanho: u64,
    modificado: i64,
    criado: i64,
    tipo: String,
    /// a folder: how many entries, and the emblem of what it holds mostly (doc, imagem, terminal)
    filhos: Option<usize>,
    emblema: Option<&'static str>,
    etiquetas: Vec<String>,
}

fn extensao(p: &Path) -> String {
    p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

fn e_imagem(tipo: &str) -> bool {
    tipo.starts_with("image/")
}

fn e_documento_tipo(p: &Path, tipo: &str) -> bool {
    tipo.starts_with("text/") || tipo.contains("pdf") || tipo.contains("document") || tipo.contains("spreadsheet") || tipo.contains("presentation")
        || matches!(extensao(p).as_str(), "pdf" | "md" | "txt" | "odt" | "ods" | "odp" | "docx" | "xlsx" | "pptx" | "csv" | "epub" | "rtf")
}

fn e_instalador_ext(p: &Path) -> bool {
    matches!(extensao(p).as_str(), "deb" | "rpm" | "appimage" | "flatpakref" | "flatpak" | "snap" | "run" | "sh" | "exe" | "msi")
}

fn tipo_de(p: &Path) -> String {
    // the name and the first bytes (an empty slice would mean "an empty file" to GIO)
    let mut cabeca = vec![0u8; 4096];
    let n = std::fs::File::open(p).and_then(|mut f| std::io::Read::read(&mut f, &mut cabeca)).unwrap_or(0);
    cabeca.truncate(n);
    let t = if n == 0 && std::fs::metadata(p).is_ok_and(|m| m.len() > 0) {
        // unreadable: by its name alone
        gio::File::for_path(p).query_info("standard::fast-content-type", gio::FileQueryInfoFlags::NONE, None::<&gio::Cancellable>).ok()
            .and_then(|i| i.attribute_string("standard::fast-content-type")).map(|g| g.to_string()).unwrap_or_default()
    } else {
        gio::content_type_guess(Some(p), &cabeca).0.to_string()
    };
    gio::content_type_get_mime_type(&t).map(|m| m.to_string()).unwrap_or(t)
}

fn segundos(t: std::io::Result<std::time::SystemTime>) -> i64 {
    t.ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// How many entries a folder has and what it holds mostly (its emblem, as in the design: PDFs → doc, Imagens →
/// imagem, Instaladores → terminal).
fn conteudo_da_pasta(p: &Path) -> (usize, Option<&'static str>) {
    let Ok(rd) = std::fs::read_dir(p) else { return (0, None) };
    let (mut n, mut docs, mut imgs, mut progs, mut arqs, mut sons, mut videos) = (0usize, 0usize, 0usize, 0usize, 0usize, 0usize, 0usize);
    for e in rd.filter_map(|e| e.ok()).filter(|e| !e.file_name().to_string_lossy().starts_with('.')) {
        n += 1;
        if n > 2000 {
            break;
        }
        if arqs >= 300 || e.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        arqs += 1;
        let c = e.path();
        let t = tipo_de(&c);
        if e_imagem(&t) {
            imgs += 1;
        } else if t.starts_with("audio/") {
            sons += 1;
        } else if t.starts_with("video/") {
            videos += 1;
        } else if e_instalador_ext(&c) || t.contains("executable") {
            progs += 1;
        } else if e_documento_tipo(&c, &t) {
            docs += 1;
        }
    }
    let maior = docs.max(imgs).max(progs).max(sons).max(videos);
    let emblema = if arqs == 0 || maior * 2 < arqs { None } else if maior == imgs { Some("imagem") } else if maior == docs { Some("doc") }
        else if maior == sons { Some("musica") } else if maior == videos { Some("video") } else { Some("terminal") };
    (n, emblema)
}

fn ler_um(p: &Path) -> Item {
    let md = std::fs::metadata(p).ok();
    let pasta = md.as_ref().is_some_and(|m| m.is_dir());
    let (filhos, emblema) = if pasta { let (n, e) = conteudo_da_pasta(p); (Some(n), e) } else { (None, None) };
    Item {
        caminho: p.to_path_buf(),
        nome: p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string()),
        pasta,
        tamanho: md.as_ref().map(|m| m.len()).unwrap_or(0),
        modificado: md.as_ref().map(|m| segundos(m.modified())).unwrap_or(0),
        criado: md.as_ref().map(|m| segundos(m.created())).unwrap_or(0),
        tipo: if pasta { "inode/directory".into() } else { tipo_de(p) },
        filhos,
        emblema,
        etiquetas: etiquetas_de(p),
    }
}

/// A folder's entries (hidden ones out), with what the page needs.
fn ler_pasta(p: &Path, ocultos: bool) -> Vec<Item> {
    let Ok(rd) = std::fs::read_dir(p) else { return vec![] };
    let tudo = ocultos || na_lixeira(p);
    rd.filter_map(|e| e.ok()).filter(|e| tudo || !e.file_name().to_string_lossy().starts_with('.')).map(|e| ler_um(&e.path())).collect()
}

/// Files with the same content in the folder: each copy → the original it repeats (the oldest of its group).
fn duplicados(itens: &[Item]) -> BTreeMap<PathBuf, PathBuf> {
    let mut por_tamanho: BTreeMap<u64, Vec<&Item>> = BTreeMap::new();
    for i in itens.iter().filter(|i| !i.pasta && i.tamanho > 0 && i.tamanho <= 512 << 20) {
        por_tamanho.entry(i.tamanho).or_default().push(i);
    }
    let mut v = BTreeMap::new();
    for grupo in por_tamanho.values().filter(|g| g.len() > 1) {
        let mut por_conteudo: BTreeMap<String, Vec<&Item>> = BTreeMap::new();
        for i in grupo {
            if let Ok(b) = std::fs::read(&i.caminho) {
                if let Some(h) = glib::compute_checksum_for_data(glib::ChecksumType::Sha256, &b) {
                    por_conteudo.entry(h.to_string()).or_default().push(i);
                }
            }
        }
        for iguais in por_conteudo.values().filter(|g| g.len() > 1) {
            // the original: the oldest, and between equals the shortest name ("fatura.pdf" before "fatura-copia.pdf")
            let Some(orig) = iguais.iter().min_by_key(|i| (i.modificado, i.nome.len(), i.nome.clone())).map(|i| i.caminho.clone()) else { continue };
            for i in iguais.iter().filter(|i| i.caminho != orig) {
                v.insert(i.caminho.clone(), orig.clone());
            }
        }
    }
    v
}

// ---------------------------------------------------------------- sizes and dates, as the design writes them

/// Sizes in SI units as GNOME counts them: "189 KB", "2,4 MB", "1 TB".
pub(crate) fn tamanho(n: u64) -> String {
    let (num, u) = partes_tamanho(n);
    if u.is_empty() { format!("{num} B") } else { format!("{num} {u}") }
}

fn partes_tamanho(n: u64) -> (String, &'static str) {
    let x = n as f64;
    let (v, u) = if x >= 1e12 { (x / 1e12, "TB") } else if x >= 1e9 { (x / 1e9, "GB") } else if x >= 1e6 { (x / 1e6, "MB") } else if x >= 1e3 { (x / 1e3, "KB") } else { return (n.to_string(), "") };
    let s = if v >= 9.95 { format!("{v:.0}") } else { format!("{v:.1}").trim_end_matches(".0").to_string() };
    (s.replace('.', ","), u)
}

/// "412 GB de 1 TB", or "1,6 de 2 TB" when both are in the same unit.
pub(crate) fn de_total(usado: u64, total: u64) -> String {
    let ((a, ua), (b, ub)) = (partes_tamanho(usado), partes_tamanho(total));
    let usado = if ua == ub { a } else { tamanho(usado) };
    trf("{used} of {total}", &[("used", usado), ("total", format!("{b} {ub}"))])
}

fn data(t: i64, com_hora: bool) -> String {
    glib::DateTime::from_unix_local(t).ok().and_then(|d| d.format(if com_hora { "%-d %b %Y, %H:%M" } else { "%-d %b %Y" }).ok())
        .map(|s| s.replace('.', "").to_lowercase()).unwrap_or_default()
}

fn com_til(p: &Path) -> String {
    let h = glib::home_dir();
    match p.strip_prefix(&h) {
        Ok(r) if r.as_os_str().is_empty() => "~".into(),
        Ok(r) => format!("~/{}", r.display()),
        Err(_) => p.display().to_string(),
    }
}

fn expandir(s: &str) -> PathBuf {
    match s.strip_prefix("~/") {
        Some(r) => glib::home_dir().join(r),
        None if s == "~" => glib::home_dir(),
        None => PathBuf::from(s),
    }
}

// ---------------------------------------------------------------- tags (freedesktop: user.xdg.tags, comma separated)

const XATTR_TAGS: &[u8] = b"user.xdg.tags\0";

fn cstr(p: &Path) -> Option<CString> {
    CString::new(p.as_os_str().as_bytes()).ok()
}

fn etiquetas_de(p: &Path) -> Vec<String> {
    let Some(c) = cstr(p) else { return vec![] };
    let mut buf = vec![0u8; 2048];
    let r = unsafe { getxattr(c.as_ptr(), XATTR_TAGS.as_ptr().cast(), buf.as_mut_ptr().cast(), buf.len()) };
    if r <= 0 {
        return vec![];
    }
    buf.truncate(r as usize);
    String::from_utf8_lossy(&buf).split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

fn gravar_etiquetas(p: &Path, v: &[String]) -> Result<(), String> {
    let c = cstr(p).ok_or("caminho")?;
    let r = if v.is_empty() {
        let r = unsafe { removexattr(c.as_ptr(), XATTR_TAGS.as_ptr().cast()) };
        // no tag left to take: fine
        if r != 0 && std::io::Error::last_os_error().raw_os_error() == Some(61) { 0 } else { r }
    } else {
        let t = v.join(",");
        unsafe { setxattr(c.as_ptr(), XATTR_TAGS.as_ptr().cast(), t.as_ptr().cast(), t.len(), 0) }
    };
    if r == 0 { Ok(()) } else { Err(std::io::Error::last_os_error().to_string()) }
}

#[derive(Clone, Debug)]
struct Etiqueta {
    nome: String,
    cor: String,
}

/// The person's tags (`~/.config/kalango/etiquetas.json`, [{"nome", "cor"}]), else the four of the design.
fn etiquetas_conhecidas() -> Vec<Etiqueta> {
    let p = glib::user_config_dir().join("kalango/etiquetas.json");
    let lidas: Vec<Etiqueta> = std::fs::read_to_string(p).ok().and_then(|t| serde_json::from_str::<Vec<Value>>(&t).ok()).unwrap_or_default().iter()
        .filter_map(|v| Some(Etiqueta { nome: v["nome"].as_str()?.to_string(), cor: v["cor"].as_str().unwrap_or("#8E8F99").to_string() })).collect();
    if !lidas.is_empty() {
        return lidas;
    }
    [(tr("Work"), "#A5A1E2"), (tr("Personal"), "#8ED2C3"), (tr("Urgent"), "#EFA2A6"), (tr("Ideas"), "#E5C98C")].iter()
        .map(|(n, c)| Etiqueta { nome: n.to_string(), cor: c.to_string() }).collect()
}

// ---------------------------------------------------------------- the trash (freedesktop: ~/.local/share/Trash)

fn lixeira() -> PathBuf {
    glib::user_data_dir().join("Trash")
}

fn na_lixeira(p: &Path) -> bool {
    p == lixeira().join("files")
}

/// Sends to the trash (as the spec says: the file in files/, where it came from in info/). Ok(where it is now).
fn para_lixeira(p: &Path) -> Result<PathBuf, String> {
    let t = lixeira();
    std::fs::create_dir_all(t.join("files")).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(t.join("info")).map_err(|e| e.to_string())?;
    let base = p.file_name().ok_or("nome")?.to_string_lossy().into_owned();
    let (raiz, ext) = match base.rfind('.').filter(|i| *i > 0) {
        Some(i) => (base[..i].to_string(), base[i..].to_string()),
        None => (base.clone(), String::new()),
    };
    let quando = glib::DateTime::now_local().ok().and_then(|d| d.format("%Y-%m-%dT%H:%M:%S").ok()).map(|s| s.to_string()).unwrap_or_default();
    let caminho = glib::Uri::escape_string(&p.to_string_lossy(), Some("/"), true);
    for n in 1..10_000 {
        let nome = if n == 1 { base.clone() } else { format!("{raiz}.{n}{ext}") };
        let info = t.join("info").join(format!("{nome}.trashinfo"));
        let destino = t.join("files").join(&nome);
        if destino.exists() {
            continue;
        }
        // the info first, created exclusively: two at once never take the same name
        let Ok(mut f) = std::fs::OpenOptions::new().write(true).create_new(true).open(&info) else { continue };
        use std::io::Write;
        let _ = write!(f, "[Trash Info]\nPath={caminho}\nDeletionDate={quando}\n");
        return match std::fs::rename(p, &destino) {
            Ok(()) => Ok(destino),
            Err(e) => {
                let _ = std::fs::remove_file(&info);
                // another disk: GIO knows its own trash there
                gio::File::for_path(p).trash(None::<&gio::Cancellable>).map(|_| PathBuf::new()).map_err(|g| format!("{e}; {g}"))
            }
        };
    }
    Err("lixeira".into())
}

/// Where a file in the trash came from.
fn origem_na_lixeira(p: &Path) -> Option<PathBuf> {
    let info = lixeira().join("info").join(format!("{}.trashinfo", p.file_name()?.to_string_lossy()));
    let t = std::fs::read_to_string(info).ok()?;
    let c = t.lines().find_map(|l| l.strip_prefix("Path="))?;
    let c = glib::Uri::unescape_string(c, None::<&str>)?.to_string();
    Some(if c.starts_with('/') { PathBuf::from(c) } else { glib::home_dir().join(c) })
}

/// Back where it came from (another name if that place is taken now). Ok(where).
fn restaurar(p: &Path) -> Result<PathBuf, String> {
    let orig = origem_na_lixeira(p).ok_or_else(|| tr("This file has no record of where it came from.").to_string())?;
    let mut destino = orig.clone();
    let mut n = 2;
    while destino.exists() {
        let stem = orig.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let ext = orig.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
        destino = orig.with_file_name(format!("{stem} ({n}){ext}"));
        n += 1;
    }
    if let Some(d) = destino.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    std::fs::rename(p, &destino).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(lixeira().join("info").join(format!("{}.trashinfo", p.file_name().unwrap_or_default().to_string_lossy())));
    Ok(destino)
}

fn apagar_de_vez(p: &Path) -> Result<(), String> {
    let r = if p.is_dir() { std::fs::remove_dir_all(p) } else { std::fs::remove_file(p) };
    r.map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(lixeira().join("info").join(format!("{}.trashinfo", p.file_name().unwrap_or_default().to_string_lossy())));
    Ok(())
}

/// A copy of a file or folder (with its contents).
fn copiar(de: &Path, para: &Path) -> std::io::Result<()> {
    if de.is_dir() {
        std::fs::create_dir_all(para)?;
        for e in std::fs::read_dir(de)?.flatten() {
            copiar(&e.path(), &para.join(e.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(de, para).map(|_| ())
    }
}

use crate::operacao::nome_livre;

// ---------------------------------------------------------------- thumbnails (freedesktop thumbnail spec)

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Alvo {
    /// a picture filling a box of this size (cut to its proportion: its height for a width is the box's)
    Foto(i32, i32),
    /// a page of a document: this wide, cut to this high from the top
    Pagina(i32, i32),
    /// the whole picture (or page) inside a box of this size, nothing cut (the gallery's stage)
    Inteira(i32, i32),
}

type Rgba = (glib::Bytes, i32, i32, usize, bool);

fn rgba(pb: &gtk::gdk_pixbuf::Pixbuf) -> Rgba {
    (pb.read_pixel_bytes(), pb.width(), pb.height(), pb.rowstride() as usize, pb.has_alpha())
}

fn textura((b, w, h, s, a): Rgba) -> gdk::Texture {
    gdk::MemoryTexture::new(w, h, if a { gdk::MemoryFormat::R8g8b8a8 } else { gdk::MemoryFormat::R8g8b8 }, &b, s).upcast()
}

/// The system's thumbnailer for a type (`thumbnailers/*.thumbnailer` in the data dirs): its command.
fn thumbnailer_para(mime: &str) -> Option<Vec<String>> {
    let mut dirs = vec![glib::user_data_dir()];
    dirs.extend(glib::system_data_dirs());
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(d.join("thumbnailers")) else { continue };
        let mut arqs: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "thumbnailer")).collect();
        arqs.sort();
        for a in arqs {
            let k = glib::KeyFile::new();
            if k.load_from_file(&a, glib::KeyFileFlags::NONE).is_err() {
                continue;
            }
            let g = "Thumbnailer Entry";
            let tipos = k.string(g, "MimeType").map(|s| s.to_string()).unwrap_or_default();
            if !tipos.split(';').any(|t| t == mime) {
                continue;
            }
            if let Ok(t) = k.string(g, "TryExec") {
                if glib::find_program_in_path(t.as_str()).is_none() && !Path::new(t.as_str()).is_file() {
                    continue;
                }
            }
            if let Ok(e) = k.string(g, "Exec") {
                return Some(e.split_whitespace().map(str::to_string).collect());
            }
        }
    }
    None
}

/// A file's thumbnail from the shared cache (`~/.cache/thumbnails`), made by the system's thumbnailer when missing
/// or stale, and saved there as the spec says (Thumb::URI and Thumb::MTime), for every app to use.
fn miniatura_sistema(p: &Path, mime: &str, mtime: i64) -> Option<gtk::gdk_pixbuf::Pixbuf> {
    use gtk::gdk_pixbuf::Pixbuf;
    let uri = gio::File::for_path(p).uri().to_string();
    let md5 = glib::compute_checksum_for_string(glib::ChecksumType::Md5, &uri)?.to_string();
    let nome = format!("{md5}.png");
    let base = glib::user_cache_dir().join("thumbnails");
    for d in ["x-large", "large", "normal"] {
        if let Ok(pb) = Pixbuf::from_file(base.join(d).join(&nome)) {
            if pb.option("tEXt::Thumb::MTime").is_some_and(|m| m.as_str() == mtime.to_string()) {
                return Some(pb);
            }
        }
    }
    let exec = thumbnailer_para(mime)?;
    let dir = base.join("large");
    std::fs::create_dir_all(&dir).ok()?;
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o700));
    // where the thumbnailers may write: confined ones (evince's AppArmor profile) only write the GNOME thumbnail
    // factory's own temporary names
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = PathBuf::from(format!("/tmp/gnome-desktop-thumbnailer-kalango-{}-{n}-{md5}.png", std::process::id()));
    let (pi, po) = (p.to_string_lossy().into_owned(), tmp.to_string_lossy().into_owned());
    let args: Vec<String> = exec.iter().map(|a| a.replace("%s", "256").replace("%u", &uri).replace("%i", &pi).replace("%o", &po).replace("%%", "%")).collect();
    let mut filho = std::process::Command::new(&args[0]).args(&args[1..]).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null()).spawn().ok()?;
    let fim = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        match filho.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if std::time::Instant::now() < fim => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = filho.kill();
                let _ = filho.wait();
                break;
            }
        }
    }
    let pb = Pixbuf::from_file(&tmp).ok();
    let _ = std::fs::remove_file(&tmp);
    let pb = pb?;
    let final_ = dir.join(&nome);
    if pb.savev(&final_, "png", &[("tEXt::Thumb::URI", uri.as_str()), ("tEXt::Thumb::MTime", mtime.to_string().as_str()), ("tEXt::Software", "KalangoOS")]).is_ok() {
        let _ = std::fs::set_permissions(&final_, std::fs::Permissions::from_mode(0o600));
    }
    Some(pb)
}

/// A preview, made off the interface thread.
fn miniatura(p: &Path, mime: &str, mtime: i64, alvo: Alvo) -> Option<Rgba> {
    use gtk::gdk_pixbuf::{InterpType, Pixbuf};
    let pb = if e_imagem(mime) {
        let lado = match alvo { Alvo::Foto(w, h) | Alvo::Inteira(w, h) => w.max(h) * 2, Alvo::Pagina(w, _) => w * 2 };
        let pb = Pixbuf::from_file_at_scale(p, lado, lado, true).ok()?;
        pb.apply_embedded_orientation().unwrap_or(pb)
    } else {
        miniatura_sistema(p, mime, mtime)?
    };
    match alvo {
        Alvo::Foto(w, h) => {
            // cover: scaled to fill w×h (twice, for sharpness), the middle kept
            let (w, h) = (w * 2, h * 2);
            let k = (w as f64 / pb.width().max(1) as f64).max(h as f64 / pb.height().max(1) as f64);
            let (sw, sh) = (((pb.width() as f64) * k).ceil().max(w as f64) as i32, ((pb.height() as f64) * k).ceil().max(h as f64) as i32);
            let s = pb.scale_simple(sw, sh, InterpType::Bilinear)?;
            Some(rgba(&s.new_subpixbuf((sw - w) / 2, (sh - h) / 2, w, h).copy()?))
        }
        Alvo::Pagina(w, h) => {
            let h2 = ((pb.height() as f64) * (w as f64) / (pb.width().max(1) as f64)).round().max(1.0) as i32;
            let s = pb.scale_simple(w, h2, InterpType::Bilinear)?;
            let s = if h2 > h { s.new_subpixbuf(0, 0, w, h).copy()? } else { s };
            Some(rgba(&s))
        }
        Alvo::Inteira(w, h) => {
            // contain: never larger than it is (a small picture stays sharp; the stage centres it)
            let k = (w as f64 / pb.width().max(1) as f64).min(h as f64 / pb.height().max(1) as f64).min(1.0);
            let s = pb.scale_simple(((pb.width() as f64) * k).round().max(1.0) as i32, ((pb.height() as f64) * k).round().max(1.0) as i32, InterpType::Bilinear)?;
            Some(rgba(&s))
        }
    }
}

/// KalangoOS's own art in its own colors (the folder of the design, the agent's cursor): an image of a fixed size
/// (a Picture would ask a height in proportion to its box's width, and every row of the grid would grow).
fn arte(nome: &str, px: i32) -> gtk::Image {
    let svg = match nome {
        "pasta-grande-doc" => include_str!("../assets/ui/pasta-grande-doc.svg"),
        "pasta-grande-imagem" => include_str!("../assets/ui/pasta-grande-imagem.svg"),
        "pasta-grande-terminal" => include_str!("../assets/ui/pasta-grande-terminal.svg"),
        "pasta-grande-musica" => include_str!("../assets/ui/pasta-grande-musica.svg"),
        "pasta-grande-video" => include_str!("../assets/ui/pasta-grande-video.svg"),
        "pasta-grande-download" => include_str!("../assets/ui/pasta-grande-download.svg"),
        "pasta-grande-desktop" => include_str!("../assets/ui/pasta-grande-desktop.svg"),
        "cursor-agente" => include_str!("../assets/ui/cursor-agente.svg"),
        _ => include_str!("../assets/ui/pasta-grande.svg"),
    };
    let img = gtk::Image::new();
    let alto = if nome == "cursor-agente" { px * 22 / 16 } else { px };
    // drawn at the size it shows (twice, for sharp screens), not at the SVG's own size
    // the folders are the design's in this theme's colours (the accent's, the light theme's)
    let fluxo = gio::MemoryInputStream::from_bytes(&glib::Bytes::from_owned(crate::tema::recolor_svg(svg).into_bytes()));
    if let Ok(pb) = gtk::gdk_pixbuf::Pixbuf::from_stream_at_scale(&fluxo, px * 2, alto * 2, true, None::<&gio::Cancellable>) {
        img.set_paintable(Some(&textura(rgba(&pb))));
    }
    img.set_pixel_size(px.max(alto));
    img.set_size_request(px, alto);
    img.set_halign(Align::Center);
    img.set_valign(Align::Center);
    img
}

// ---------------------------------------------------------------- the window

/// Puts a finished preview where it was asked for.
type Mostrar = Box<dyn Fn(&gdk::Texture)>;

/// The agent at work on the open folder.
struct Trabalho {
    total: usize,
    tocados: BTreeSet<PathBuf>,
    atual: Option<PathBuf>,
}

/// A tab: its folder and its own way back and forward.
#[derive(Clone, Default)]
struct Aba {
    pasta: PathBuf,
    atras: Vec<PathBuf>,
    frente: Vec<PathBuf>,
}

/// The views (design "Arquivos · nova"): icons, list (the first one shown), columns, gallery.
const VISTAS: &[(&str, &str, &str)] = &[("grade", n_("Icons"), "grade"), ("lista", n_("List"), "lista"), ("colunas", n_("Columns"), "colunas"), ("galeria", n_("Gallery"), "galeria")];
const MODO_ICONES: u8 = 0;
const MODO_LISTA: u8 = 1;
const MODO_COLUNAS: u8 = 2;
const MODO_GALERIA: u8 = 3;
/// The list's columns: the order each one sorts by (ORDENS), its name and width (0: the rest).
const COLUNAS: &[(usize, &str, i32)] = &[(1, n_("Name"), 0), (0, n_("Modified"), 170), (3, n_("Kind"), 150), (2, n_("Size"), 90)];
const AGRUPAR: &[&str] = &[n_("Date"), n_("Kind"), n_("None")];

pub struct Arquivos {
    pub janela: gtk::ApplicationWindow,
    shell: Weak<Shell>,
    pasta: RefCell<PathBuf>,
    atras: RefCell<Vec<PathBuf>>,
    frente: RefCell<Vec<PathBuf>>,
    abas: RefCell<Vec<Aba>>,
    aba_atual: Cell<usize>,
    /// folders opened in this window, the latest first (the history button)
    visitadas: RefCell<Vec<PathBuf>>,
    itens: RefCell<Vec<Item>>,
    dups: RefCell<BTreeMap<PathBuf, PathBuf>>,
    pelo_agente: RefCell<BTreeSet<PathBuf>>,
    selecionados: RefCell<BTreeSet<PathBuf>>,
    ancora: RefCell<Option<PathBuf>>,
    filtro: Cell<usize>,
    etiqueta_filtro: RefCell<Option<String>>,
    ordem: Cell<usize>,
    crescente: Cell<bool>,
    /// 0 by date, 1 by kind, 2 none
    agrupar: Cell<usize>,
    modo: Cell<u8>,
    painel_aberto: Cell<bool>,
    texturas: RefCell<HashMap<(PathBuf, i64, Alvo), gdk::Texture>>,
    sem_miniatura: RefCell<BTreeSet<(PathBuf, i64)>>,
    /// previews being made: who waits for each (one job per preview, however many places ask)
    esperando: RefCell<HashMap<(PathBuf, i64, Alvo), Vec<Mostrar>>>,
    /// the files on screen: (path, its row or tile, its check)
    cartoes: RefCell<Vec<(PathBuf, gtk::Widget, gtk::Widget)>>,
    /// the last files sent to the trash from here: (where they were, where they are) — Ctrl+Z
    ultimo_lixo: RefCell<Vec<(PathBuf, PathBuf)>>,
    /// Ctrl+C / Ctrl+X: the files and whether they move
    area: RefCell<(Vec<PathBuf>, bool)>,
    trabalho: RefCell<Option<Trabalho>>,
    resumindo: RefCell<BTreeSet<PathBuf>>,
    monitor: RefCell<Option<gio::FileMonitor>>,
    pendente: Cell<bool>,
    geracao: Cell<u64>,
    /// the compact layout (design "Arquivos · 1366×768"): a short or narrow window
    compacto: Cell<Option<bool>>,
    /// hidden files shown (Ctrl+H; kept in arquivos.json)
    ocultos: Cell<bool>,
    /// the window's layer for dialogs (Properties, Open with, a conflict), over the dimmed window
    sobre: gtk::Overlay,
    /// closes the dialog open now (Esc)
    fechar_dialogo: RefCell<Option<Rc<dyn Fn()>>>,
    // ---- the window's parts
    lateral: gtk::Box,
    trilho: gtk::Box,
    lugares: gtk::Box,
    locais: gtk::Box,
    etiquetas_box: gtk::Box,
    lixeira_box: gtk::Box,
    trilho_lugares: gtk::Box,
    trilho_lixeira: gtk::Box,
    abas_box: gtk::Box,
    voltar: gtk::Button,
    avancar: gtk::Button,
    acima: gtk::Button,
    caminho: gtk::Stack,
    migalha: gtk::Box,
    caminho_campo: gtk::Entry,
    historico: gtk::Box,
    busca_campo: gtk::Box,
    busca: gtk::Entry,
    comandos: Vec<(&'static str, gtk::Button)>,
    ordem_menu: gtk::Box,
    /// what the list shows when a filter (Sort › Show) or a tag hides part of it: "Mostrando: Imagens ✕"
    filtro_chip: gtk::Button,
    filtro_chip_rotulo: gtk::Label,
    agrupar_menu: gtk::Box,
    organizar_rotulo: gtk::Label,
    vistas: Vec<gtk::ToggleButton>,
    painel_b: gtk::ToggleButton,
    info_b: gtk::MenuButton,
    gaveta: gtk::Box,
    cabecalho: gtk::Box,
    cab_colunas: Vec<(usize, gtk::Button, gtk::Label, gtk::Image)>,
    conteudo: gtk::Stack,
    lista: gtk::Box,
    grade: gtk::Box,
    colunas_box: gtk::Box,
    colunas_rol: gtk::ScrolledWindow,
    palco: gtk::Box,
    faixa: gtk::Box,
    rodape: gtk::Label,
    livre: gtk::Label,
    agente_chip: gtk::Box,
    agente_chip_texto: gtk::Label,
    painel: gtk::Box,
}

fn botao_icone(glifo: &str, cor: &str, px: i32, classe: &str, dica: &str) -> gtk::Button {
    let b = gtk::Button::new();
    b.add_css_class(classe);
    b.set_child(Some(&ui_icon(glifo, cor, px)));
    if !dica.is_empty() {
        b.set_tooltip_text(Some(dica));
    }
    b.set_valign(Align::Center);
    b
}

fn titulo_secao(t: &str) -> gtk::Label {
    let l = rotulo(t, "arq-secao");
    l.set_xalign(0.0);
    l
}

fn limpar(w: &impl IsA<gtk::Widget>) {
    let w = w.as_ref();
    while let Some(x) = w.first_child() {
        if let Some(bx) = w.downcast_ref::<gtk::Box>() {
            bx.remove(&x);
        } else if let Some(f) = w.downcast_ref::<gtk::FlowBox>() {
            f.remove(&x);
        } else {
            x.unparent();
        }
    }
}

/// A place of a fixed size with its content centred in it (nothing in it grows the row it is in).
fn centro(w: i32, h: i32, filho: &impl IsA<gtk::Widget>) -> gtk::Box {
    let c = caixa(Orientation::Horizontal, 0, "");
    c.set_size_request(w, h);
    c.set_homogeneous(true);
    c.set_valign(Align::Center);
    c.set_hexpand(false);
    let f = filho.as_ref();
    f.set_halign(Align::Center);
    f.set_valign(Align::Center);
    c.append(f);
    c
}

fn espaco() -> gtk::Box {
    let e = caixa(Orientation::Horizontal, 0, "");
    e.set_hexpand(true);
    e
}

fn divisor_v() -> gtk::Box {
    let d = caixa(Orientation::Horizontal, 0, "arq-divisor");
    d.set_size_request(1, 20);
    d.set_valign(Align::Center);
    d
}

/// A folder's emblem: a special folder's (Downloads, Documents…), a project's, else what it holds mostly.
fn emblema_de(p: &Path, conteudo: Option<&'static str>) -> Option<&'static str> {
    let home = glib::home_dir();
    for (d, e) in [(glib::UserDirectory::Downloads, "download"), (glib::UserDirectory::Documents, "doc"), (glib::UserDirectory::Pictures, "imagem"),
        (glib::UserDirectory::Music, "musica"), (glib::UserDirectory::Videos, "video"), (glib::UserDirectory::Desktop, "desktop")] {
        if glib::user_special_dir(d).is_some_and(|s| s == p && s != home) {
            return Some(e);
        }
    }
    if p.join(".git").exists() {
        return Some("terminal");
    }
    conteudo
}

/// A page drawn for a file with no preview: lines of text and its extension's badge (design "Arquivos · nova
/// (ícones)", the documents' cards).
fn pagina_falsa(i: &Item, w: i32, h: i32, selo: bool) -> gtk::Box {
    let p = caixa(Orientation::Vertical, (h / 24).max(3), "arq-pagina-falsa");
    p.set_size_request(w, h);
    let topo = caixa(Orientation::Vertical, 0, "");
    topo.set_size_request(-1, h / 8);
    p.append(&topo);
    for (k, f) in [0.62, 0.54, 0.66, 0.4, 0.6, 0.5].iter().enumerate() {
        let l = caixa(Orientation::Horizontal, 0, "arq-pagina-linha");
        l.set_size_request((w as f64 * f) as i32, (h / 52).max(3));
        l.set_halign(Align::Start);
        l.set_margin_start(w / 7);
        if k == 3 {
            l.set_margin_top(h / 20);
        }
        p.append(&l);
    }
    let esp = caixa(Orientation::Vertical, 0, "");
    esp.set_vexpand(true);
    p.append(&esp);
    let ext = extensao(&i.caminho);
    if selo && !ext.is_empty() && ext.len() <= 5 {
        let b = rotulo(&ext.to_uppercase(), "arq-ext");
        let (_, tom, _) = glifo_do_tipo(i);
        b.add_css_class(&format!("tom-{tom}"));
        b.set_halign(Align::Start);
        b.set_margin_start(w / 14);
        b.set_margin_bottom(h / 18);
        p.append(&b);
    }
    p
}

/// A folder's art at a size (its emblem on it).
fn arte_pasta(p: &Path, conteudo: Option<&'static str>, px: i32) -> gtk::Image {
    arte(&emblema_de(p, conteudo).map(|e| format!("pasta-grande-{e}")).unwrap_or_else(|| "pasta-grande".into()), px)
}

/// A file's kind as the list shows it: a glyph, its tone and the tone's colour.
fn glifo_do_tipo(i: &Item) -> (&'static str, &'static str, &'static str) {
    let t = i.tipo.as_str();
    let ext = extensao(&i.caminho);
    if t.contains("pdf") {
        ("doc", "coral", "#EFA2A6")
    } else if e_instalador_ext(&i.caminho) {
        ("download", "ouro", "#E5C98C")
    } else if t.starts_with("audio/") {
        ("musica", "azul", "#7CC3DF")
    } else if t.starts_with("video/") {
        ("video-d", "roxo", "#A5A1E2")
    } else if matches!(ext.as_str(), "zip" | "gz" | "xz" | "bz2" | "7z" | "rar" | "tar" | "zst") {
        ("pasta", "ouro", "#E5C98C")
    } else if t.contains("script") || matches!(ext.as_str(), "rs" | "py" | "js" | "ts" | "c" | "h" | "cpp" | "go" | "java" | "sh" | "toml" | "json" | "yaml" | "yml") {
        ("codigo", "verde", "#8ED2C3")
    } else {
        ("doc", "cinza", "#B9BAC4")
    }
}

/// What a file is, in words ("Documento PDF"; a folder: "Pasta").
fn descricao_tipo(i: &Item) -> String {
    if i.pasta {
        return tr("Folder").to_string();
    }
    let ct = gio::content_type_from_mime_type(&i.tipo).unwrap_or_else(|| i.tipo.clone().into());
    gio::content_type_get_description(&ct).to_string()
}

/// Its size, or a folder's items.
fn tamanho_ou_itens(i: &Item) -> String {
    if i.pasta {
        let n = i.filhos.unwrap_or(0);
        if n == 1 { tr("1 item").to_string() } else { trf("{n} items", &[("n", n.to_string())]) }
    } else {
        tamanho(i.tamanho)
    }
}

/// When it changed, as the design's list writes it: "Hoje, 09:12", "Ontem, 18:42", "Sex, 14:00", "12 nov, 16:20".
fn quando_hora(t: i64) -> String {
    let (Ok(agora), Ok(d)) = (glib::DateTime::now_local(), glib::DateTime::from_unix_local(t)) else { return String::new() };
    let dia = |x: &glib::DateTime| glib::DateTime::from_local(x.year(), x.month(), x.day_of_month(), 0, 0, 0.0).map(|d| d.to_unix()).unwrap_or(0);
    let hora = d.format("%H:%M").map(|s| s.to_string()).unwrap_or_default();
    let maiuscula = |s: String| {
        let mut c = s.chars();
        c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
    };
    match (dia(&agora) - dia(&d)) / 86_400 {
        ..=0 => trf("Today, {time}", &[("time", hora)]),
        1 => trf("Yesterday, {time}", &[("time", hora)]),
        2..=6 => format!("{}, {hora}", maiuscula(d.format("%a").map(|s| s.trim_end_matches('.').to_string()).unwrap_or_default())),
        _ if d.year() == agora.year() => format!("{}, {hora}", d.format("%-d %b").map(|s| s.trim_end_matches('.').to_lowercase()).unwrap_or_default()),
        _ => d.format("%-d %b %Y").map(|s| s.replace('.', "").to_lowercase()).unwrap_or_default(),
    }
}

/// A folder's name as the window shows it (the person's folder, the trash, the system's root).
fn nome_da_pasta(p: &Path) -> String {
    if *p == glib::home_dir() {
        tr("Home folder").to_string()
    } else if na_lixeira(p) {
        tr("Trash").to_string()
    } else if p == Path::new("/") {
        tr("System").to_string()
    } else {
        p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string())
    }
}

/// A folder's entries for the columns view (names and whether they are folders; no reading of the files' contents).
fn ler_leve(p: &Path) -> Vec<(PathBuf, String, bool)> {
    let Ok(rd) = std::fs::read_dir(p) else { return vec![] };
    let mut v: Vec<(PathBuf, String, bool)> = rd.filter_map(|e| e.ok()).filter(|e| !e.file_name().to_string_lossy().starts_with('.')).take(2000)
        .map(|e| (e.path(), e.file_name().to_string_lossy().into_owned(), e.file_type().is_ok_and(|t| t.is_dir()) || e.path().is_dir())).collect();
    v.sort_by_key(|(_, n, d)| (!*d, n.to_lowercase()));
    v
}

impl Arquivos {
    pub fn new(app: &gtk::Application, shell: &Rc<Shell>) -> Rc<Arquivos> {
        let janela = gtk::ApplicationWindow::new(app);
        janela.set_title(Some(tr("Files")));
        // the design's 1400 × 868; on a small screen almost all of it
        let (w, h) = crate::superficies::tamanho_de_janela(1400, 868);
        janela.set_default_size(w, h);
        janela.add_css_class("arquivos");
        let sem_barra = gtk::Box::new(Orientation::Horizontal, 0);
        sem_barra.set_visible(false);
        janela.set_titlebar(Some(&sem_barra));
        janela.connect_close_request(|j| {
            j.set_visible(false);
            glib::Propagation::Stop
        });
        let raiz = caixa(Orientation::Horizontal, 0, "");

        // ---- the side bar: the app, the places, the disks, the tags, the trash (compact: a rail of glyphs)
        let lateral = caixa(Orientation::Vertical, 1, "arq-lateral");
        lateral.set_size_request(232, -1);
        lateral.set_hexpand(false);
        let lc = caixa(Orientation::Horizontal, 10, "arq-app");
        lc.append(&crate::apps::imagem_kalango("arquivos", 22));
        lc.append(&rotulo(tr("Files"), "arq-app-nome"));
        let lch = gtk::WindowHandle::new();
        lch.set_child(Some(&lc));
        lateral.append(&lch);
        let lista_lateral = caixa(Orientation::Vertical, 1, "");
        let t1 = titulo_secao(tr("FAVOURITES"));
        t1.add_css_class("primeira");
        lista_lateral.append(&t1);
        let lugares = caixa(Orientation::Vertical, 1, "");
        lista_lateral.append(&lugares);
        lista_lateral.append(&titulo_secao(tr("PLACES")));
        let locais = caixa(Orientation::Vertical, 1, "");
        lista_lateral.append(&locais);
        lista_lateral.append(&titulo_secao(tr("TAGS")));
        let etiquetas_box = caixa(Orientation::Vertical, 1, "");
        lista_lateral.append(&etiquetas_box);
        let rol_lateral = gtk::ScrolledWindow::new();
        rol_lateral.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        rol_lateral.set_vexpand(true);
        rol_lateral.set_child(Some(&lista_lateral));
        lateral.append(&rol_lateral);
        let lixeira_box = caixa(Orientation::Vertical, 0, "");
        lateral.append(&lixeira_box);
        raiz.append(&lateral);
        let trilho = caixa(Orientation::Vertical, 4, "arq-trilho");
        trilho.set_size_request(56, -1);
        trilho.set_hexpand(false);
        let ti = crate::apps::imagem_kalango("arquivos", 24);
        ti.set_size_request(40, 40);
        ti.set_halign(Align::Center);
        trilho.append(&ti);
        let td = caixa(Orientation::Horizontal, 0, "arq-trilho-div");
        td.set_size_request(24, 1);
        td.set_halign(Align::Center);
        trilho.append(&td);
        let trilho_lugares = caixa(Orientation::Vertical, 4, "");
        trilho_lugares.set_vexpand(true);
        trilho.append(&trilho_lugares);
        let trilho_lixeira = caixa(Orientation::Vertical, 0, "");
        trilho.append(&trilho_lixeira);
        trilho.set_visible(false);
        raiz.append(&trilho);

        // ---- the right of the window: the tabs (with the window's controls), the toolbar, the commands, the body
        let direita = caixa(Orientation::Vertical, 0, "");
        direita.set_hexpand(true);
        let tira = caixa(Orientation::Horizontal, 2, "arq-abas");
        let abas_box = caixa(Orientation::Horizontal, 2, "");
        abas_box.set_valign(Align::End);
        tira.append(&abas_box);
        tira.append(&espaco());
        let controles = crate::configuracoes::controles_janela();
        controles.set_margin_start(8);
        controles.set_valign(Align::Center);
        tira.append(&controles);
        let tira_h = gtk::WindowHandle::new();
        tira_h.set_child(Some(&tira));
        direita.append(&tira_h);

        // the toolbar: back, forward, up; the path (click it to type one); search or ask
        let barra = caixa(Orientation::Horizontal, 6, "arq-barra-ferr");
        let voltar = botao_icone("seta-esquerda", "#E2E2E6", 16, "arq-icone", tr("Back (Alt+←)"));
        let avancar = botao_icone("seta-direita", "#E2E2E6", 16, "arq-icone", tr("Forward (Alt+→)"));
        let acima = botao_icone("acima", "#E2E2E6", 16, "arq-icone", tr("Up a folder (Alt+↑)"));
        barra.append(&voltar);
        barra.append(&avancar);
        barra.append(&acima);
        let caminho_box = caixa(Orientation::Horizontal, 6, "arq-caminho");
        caminho_box.set_hexpand(true);
        caminho_box.set_valign(Align::Center);
        let caminho = gtk::Stack::new();
        caminho.set_hexpand(true);
        caminho.set_hhomogeneous(false);
        let migalha = caixa(Orientation::Horizontal, 4, "");
        migalha.set_valign(Align::Center);
        caminho.add_named(&migalha, Some("migalhas"));
        let caminho_campo = gtk::Entry::new();
        caminho_campo.add_css_class("arq-caminho-campo");
        caminho_campo.set_hexpand(true);
        caminho.add_named(&caminho_campo, Some("editar"));
        caminho_box.append(&caminho);
        let historico_b = gtk::MenuButton::new();
        historico_b.add_css_class("arq-icone-menu");
        historico_b.set_child(Some(&ui_icon("relogio", "#7A7B88", 16)));
        historico_b.set_tooltip_text(Some(tr("Recent folders")));
        let hp = gtk::Popover::new();
        hp.add_css_class("arq-menu");
        hp.set_has_arrow(false);
        let historico = caixa(Orientation::Vertical, 2, "");
        historico.set_size_request(300, -1);
        hp.set_child(Some(&historico));
        historico_b.set_popover(Some(&hp));
        caminho_box.append(&historico_b);
        barra.append(&caminho_box);
        let busca_campo = caixa(Orientation::Horizontal, 8, "arq-busca");
        busca_campo.set_size_request(320, -1);
        busca_campo.set_hexpand(false);
        busca_campo.set_valign(Align::Center);
        busca_campo.append(&ui_icon("busca", "#8E8F99", 16));
        let busca = gtk::Entry::new();
        busca.add_css_class("arq-busca-campo");
                busca.set_hexpand(true);
        busca_campo.append(&busca);
        let pedir = gtk::Button::new();
        pedir.add_css_class("arq-icone-mini");
        pedir.set_child(Some(&estrela(14, "#A5A1E2")));
        pedir.set_tooltip_text(Some(tr("Ask the agent (Enter, with three words or more)")));
        busca_campo.append(&pedir);
        barra.append(&busca_campo);
        direita.append(&barra);

        // the command bar: New, the selection's commands, Sort, the agent, the views, the details
        let cmd = caixa(Orientation::Horizontal, 4, "arq-comandos");
        let novo_b = gtk::MenuButton::new();
        novo_b.add_css_class("arq-novo");
        let nc = caixa(Orientation::Horizontal, 6, "");
        nc.append(&ui_icon("mais", "#1C1D24", 14));
        nc.append(&rotulo(tr("New"), ""));
        nc.append(&ui_icon("chevron-baixo", "#1C1D24", 12));
        novo_b.set_child(Some(&nc));
        let np = gtk::Popover::new();
        np.add_css_class("arq-menu");
        np.set_has_arrow(false);
        let nl = caixa(Orientation::Vertical, 2, "");
        nl.set_size_request(240, -1);
        np.set_child(Some(&nl));
        novo_b.set_popover(Some(&np));
        novo_b.set_valign(Align::Center);
        cmd.append(&novo_b);
        let folga = caixa(Orientation::Horizontal, 0, "");
        folga.set_size_request(6, -1);
        cmd.append(&folga);
        let mut comandos: Vec<(&'static str, gtk::Button)> = Vec::new();
        for (id, glifo, dica) in [("cortar", "tesoura", tr("Cut (Ctrl+X)")), ("copiar", "copiar", tr("Copy (Ctrl+C)")), ("colar", "colar", tr("Paste (Ctrl+V)")),
            ("renomear", "lapis", tr("Rename (F2)")), ("compartilhar", "compartilhar", tr("Share")), ("lixeira", "lixeira", tr("Move to the trash (Del)"))] {
            let b = botao_icone(glifo, "#B9BAC4", 16, "arq-icone", dica);
            cmd.append(&b);
            comandos.push((id, b));
        }
        cmd.append(&divisor_v());
        let ordenar_b = gtk::MenuButton::new();
        ordenar_b.add_css_class("arq-texto");
        let oc = caixa(Orientation::Horizontal, 6, "");
        oc.append(&ui_icon("ordenar", "#B9BAC4", 16));
        oc.append(&rotulo(tr("Sort"), ""));
        oc.append(&ui_icon("chevron-baixo", "#7A7B88", 12));
        ordenar_b.set_child(Some(&oc));
        let op = gtk::Popover::new();
        op.add_css_class("arq-menu");
        op.set_has_arrow(false);
        let ordem_menu = caixa(Orientation::Vertical, 2, "");
        ordem_menu.set_size_request(240, -1);
        op.set_child(Some(&ordem_menu));
        ordenar_b.set_popover(Some(&op));
        ordenar_b.set_valign(Align::Center);
        cmd.append(&ordenar_b);
        let agrupar_b = gtk::MenuButton::new();
        agrupar_b.add_css_class("arq-texto");
        let ac = caixa(Orientation::Horizontal, 6, "");
        ac.append(&ui_icon("agrupar", "#B9BAC4", 16));
        ac.append(&rotulo(tr("Group"), ""));
        ac.append(&ui_icon("chevron-baixo", "#7A7B88", 12));
        agrupar_b.set_child(Some(&ac));
        let gp = gtk::Popover::new();
        gp.add_css_class("arq-menu");
        gp.set_has_arrow(false);
        let agrupar_menu = caixa(Orientation::Vertical, 2, "");
        agrupar_menu.set_size_request(200, -1);
        gp.set_child(Some(&agrupar_menu));
        agrupar_b.set_popover(Some(&gp));
        agrupar_b.set_valign(Align::Center);
        cmd.append(&agrupar_b);
        // (an active filter or tag is said here: otherwise a folder looks empty with no reason)
        let filtro_chip = gtk::Button::new();
        filtro_chip.add_css_class("arq-filtro-chip");
        let fcc = caixa(Orientation::Horizontal, 6, "");
        let filtro_chip_rotulo = rotulo("", "");
        fcc.append(&filtro_chip_rotulo);
        fcc.append(&ui_icon("fechar", "#C9CAD1", 10));
        filtro_chip.set_child(Some(&fcc));
        filtro_chip.set_tooltip_text(Some(tr("Show everything")));
        filtro_chip.set_valign(Align::Center);
        filtro_chip.set_visible(false);
        cmd.append(&filtro_chip);
        cmd.append(&espaco());
        let organizar_b = gtk::Button::new();
        organizar_b.add_css_class("arq-agente");
        let og = caixa(Orientation::Horizontal, 6, "");
        og.append(&estrela(16, "#A5A1E2"));
        let organizar_rotulo = rotulo(tr("Organise with the agent"), "");
        og.append(&organizar_rotulo);
        organizar_b.set_child(Some(&og));
        organizar_b.set_valign(Align::Center);
        cmd.append(&organizar_b);
        cmd.append(&divisor_v());
        let vbox = caixa(Orientation::Horizontal, 2, "arq-vistas");
        vbox.set_valign(Align::Center);
        let mut vistas: Vec<gtk::ToggleButton> = Vec::new();
        for (k, (_, nome, glifo)) in VISTAS.iter().enumerate() {
            let b = gtk::ToggleButton::new();
            b.add_css_class("arq-vista");
            b.set_child(Some(&ui_icon(glifo, "#8E8F99", 16)));
            b.set_tooltip_text(Some(&format!("{} (Ctrl+{})", tr(nome), k + 1)));
            if let Some(p) = vistas.first() {
                b.set_group(Some(p));
            }
            vbox.append(&b);
            vistas.push(b);
        }
        cmd.append(&vbox);
        let painel_b = gtk::ToggleButton::new();
        painel_b.add_css_class("arq-icone");
        painel_b.set_child(Some(&ui_icon("painel-direita", "#A5A1E2", 16)));
        painel_b.set_tooltip_text(Some(tr("Details")));
        painel_b.set_valign(Align::Center);
        cmd.append(&painel_b);
        // compact: the details in a drawer under this button
        let info_b = gtk::MenuButton::new();
        info_b.add_css_class("arq-info-botao");
        info_b.set_child(Some(&ui_icon("painel-direita", "#A5A1E2", 16)));
        info_b.set_tooltip_text(Some(tr("Details")));
        info_b.set_valign(Align::Center);
        let gaveta_pop = gtk::Popover::new();
        gaveta_pop.add_css_class("arq-gaveta");
        gaveta_pop.set_has_arrow(false);
        gaveta_pop.set_position(gtk::PositionType::Bottom);
        gaveta_pop.set_halign(Align::End);
        gaveta_pop.set_autohide(false);
        let gaveta = caixa(Orientation::Vertical, 10, "");
        gaveta.set_size_request(268, -1);
        gaveta_pop.set_child(Some(&gaveta));
        info_b.set_popover(Some(&gaveta_pop));
        info_b.set_visible(false);
        cmd.append(&info_b);
        direita.append(&cmd);

        // ---- the body: the files (and the list's columns), the status; the details on the right
        let corpo = caixa(Orientation::Horizontal, 0, "");
        corpo.set_vexpand(true);
        let coluna = caixa(Orientation::Vertical, 0, "");
        coluna.set_hexpand(true);
        let cabecalho = caixa(Orientation::Horizontal, 0, "arq-cabecalho");
        let mut cab_colunas = Vec::new();
        for (k, (ordem, nome, largura)) in COLUNAS.iter().enumerate() {
            let b = gtk::Button::new();
            b.add_css_class("arq-coluna");
            let c = caixa(Orientation::Horizontal, 4, "");
            let l = rotulo(tr(nome), "");
            let seta = ui_icon("chevron-baixo", "#A5A1E2", 12);
            if k == 3 {
                // the size: on the right of its column, as its values
                c.set_halign(Align::End);
                c.append(&seta);
                c.append(&l);
            } else {
                c.append(&l);
                c.append(&seta);
            }
            b.set_child(Some(&c));
            if *largura == 0 {
                b.set_hexpand(true);
                // (the names start after the check and the icon)
                c.set_margin_start(26);
            } else {
                b.set_size_request(*largura, -1);
            }
            cabecalho.append(&b);
            cab_colunas.push((*ordem, b, l, seta));
        }
        let tags_cab = rotulo(tr("Tags"), "arq-coluna-texto");
        tags_cab.set_size_request(130, -1);
        tags_cab.set_margin_start(16);
        tags_cab.set_xalign(0.0);
        cabecalho.append(&tags_cab);
        coluna.append(&cabecalho);
        let conteudo = gtk::Stack::new();
        conteudo.set_vexpand(true);
        conteudo.set_vhomogeneous(false);
        conteudo.set_hhomogeneous(false);
        conteudo.set_focusable(true);
        let lista = caixa(Orientation::Vertical, 0, "arq-lista");
        lista.set_valign(Align::Start);
        let lista_rol = gtk::ScrolledWindow::new();
        lista_rol.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        lista_rol.set_child(Some(&lista));
        conteudo.add_named(&lista_rol, Some("lista"));
        let grade = caixa(Orientation::Vertical, 4, "arq-grade");
        grade.set_valign(Align::Start);
        let grade_rol = gtk::ScrolledWindow::new();
        grade_rol.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        grade_rol.set_child(Some(&grade));
        conteudo.add_named(&grade_rol, Some("grade"));
        let colunas_box = caixa(Orientation::Horizontal, 0, "");
        let colunas_rol = gtk::ScrolledWindow::new();
        colunas_rol.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Never);
        colunas_rol.set_child(Some(&colunas_box));
        conteudo.add_named(&colunas_rol, Some("colunas"));
        let galeria = caixa(Orientation::Vertical, 12, "arq-galeria");
        let palco = caixa(Orientation::Vertical, 10, "arq-palco");
        palco.set_vexpand(true);
        galeria.append(&palco);
        let faixa = caixa(Orientation::Horizontal, 8, "");
        let faixa_rol = gtk::ScrolledWindow::new();
        faixa_rol.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Never);
        faixa_rol.set_child(Some(&faixa));
        faixa_rol.set_size_request(-1, 84);
        galeria.append(&faixa_rol);
        conteudo.add_named(&galeria, Some("galeria"));
        coluna.append(&conteudo);
        let rp = caixa(Orientation::Horizontal, 12, "arq-status");
        let rodape = rotulo("", "arq-status-texto");
        rodape.set_hexpand(true);
        rodape.set_xalign(0.0);
        rodape.set_ellipsize(gtk::pango::EllipsizeMode::End);
        rp.append(&rodape);
        let agente_chip = caixa(Orientation::Horizontal, 6, "arq-agente-chip");
        agente_chip.append(&estrela(12, "#A5A1E2"));
        let agente_chip_texto = rotulo("", "");
        agente_chip.append(&agente_chip_texto);
        agente_chip.set_visible(false);
        agente_chip.set_valign(Align::Center);
        rp.append(&agente_chip);
        let livre = rotulo("", "arq-status-texto");
        rp.append(&livre);
        coluna.append(&rp);
        corpo.append(&coluna);
        let painel = caixa(Orientation::Vertical, 14, "arq-painel");
        painel.set_size_request(304, -1);
        painel.set_hexpand(false);
        corpo.append(&painel);
        direita.append(&corpo);
        raiz.append(&direita);
        let sobre = gtk::Overlay::new();
        sobre.set_child(Some(&raiz));
        janela.set_child(Some(&sobre));

        let home = glib::home_dir();
        let a = Rc::new(Arquivos {
            janela, shell: Rc::downgrade(shell), pasta: RefCell::new(home.clone()), atras: RefCell::default(), frente: RefCell::default(),
            abas: RefCell::new(vec![Aba { pasta: home, ..Default::default() }]), aba_atual: Cell::new(0), visitadas: RefCell::default(),
            itens: RefCell::default(), dups: RefCell::default(), pelo_agente: RefCell::default(), selecionados: RefCell::default(), ancora: RefCell::default(),
            filtro: Cell::new(0), etiqueta_filtro: RefCell::default(), ordem: Cell::new(0), crescente: Cell::new(false), agrupar: Cell::new(0),
            modo: Cell::new(MODO_LISTA), painel_aberto: Cell::new(true), texturas: RefCell::default(), sem_miniatura: RefCell::default(),
            esperando: RefCell::default(), cartoes: RefCell::default(), ultimo_lixo: RefCell::default(), area: RefCell::default(), trabalho: RefCell::default(),
            resumindo: RefCell::default(), monitor: RefCell::default(), pendente: Cell::new(false), geracao: Cell::new(0), compacto: Cell::new(None),
            ocultos: Cell::new(preferencias()["ocultos"].as_bool().unwrap_or(false)), sobre, fechar_dialogo: RefCell::default(),
            lateral, trilho, lugares, locais, etiquetas_box, lixeira_box, trilho_lugares, trilho_lixeira, abas_box, voltar, avancar, acima, caminho,
            migalha, caminho_campo, historico, busca_campo, busca, comandos, ordem_menu, filtro_chip: filtro_chip.clone(), filtro_chip_rotulo, agrupar_menu, organizar_rotulo, vistas, painel_b, info_b, gaveta,
            cabecalho, cab_colunas, conteudo, lista, grade, colunas_box, colunas_rol, palco, faixa, rodape, livre, agente_chip, agente_chip_texto, painel,
        });
        let w = Rc::downgrade(&a);
        filtro_chip.connect_clicked(move |_| {
            if let Some(a) = w.upgrade() {
                a.mostrar_tudo();
            }
        });
        crate::central::posicao_debug_sempre("arquivos filtro chip", filtro_chip.upcast_ref());
        // the app id: the dock and Alt+Tab know it as Arquivos
        let app_id = |j: &gtk::ApplicationWindow| {
            if let Some(s) = j.surface().filter(|s| s.type_().name().contains("Wayland")) {
                let id = CString::new("os.kalango.Arquivos").unwrap();
                unsafe {
                    gdk_wayland_toplevel_set_application_id(s.as_ptr() as *mut gdk::ffi::GdkToplevel, id.as_ptr());
                }
            }
        };
        a.janela.connect_realize(move |j| app_id(j));
        a.janela.connect_map(move |j| app_id(j));
        // the layout follows the window's size (a short or narrow window is the compact one)
        for prop in ["default-width", "default-height"] {
            let w = Rc::downgrade(&a);
            a.janela.connect_notify_local(Some(prop), move |_, _| {
                if let Some(a) = w.upgrade() {
                    a.ajustar_layout();
                }
            });
        }
        if let Some(pop) = a.info_b.popover() {
            let w = Rc::downgrade(&a);
            pop.connect_show(move |_| {
                if let Some(a) = w.upgrade() {
                    a.gaveta_render();
                }
            });
        }
        a.ligar(&nl, &organizar_b, &pedir, &lista_rol, &grade_rol);
        let w = Rc::downgrade(&a);
        shell.right_bar().conversa.ao_evento(move |v| {
            if let Some(a) = w.upgrade() {
                a.agente_fez(v);
            }
        });
        a
    }

    /// What every control does.
    fn ligar(self: &Rc<Self>, novo_lista: &gtk::Box, organizar_b: &gtk::Button, pedir: &gtk::Button, lista_rol: &gtk::ScrolledWindow, grade_rol: &gtk::ScrolledWindow) {
        // the views: Ctrl+1…4, the buttons; the list is the first one shown
        self.vistas[MODO_LISTA as usize].set_active(true);
        for (k, b) in self.vistas.iter().enumerate() {
            let w = Rc::downgrade(self);
            let glifo = VISTAS[k].2;
            b.connect_toggled(move |b| {
                if let Some(img) = b.child().and_downcast::<gtk::Image>() {
                    crate::conversa::set_icon(&img, glifo, if b.is_active() { "#E2E2E6" } else { "#8E8F99" });
                }
                if let Some(a) = w.upgrade().filter(|_| b.is_active()) {
                    a.modo.set(k as u8);
                    a.render();
                }
            });
        }
        crate::conversa::set_icon(&self.vistas[MODO_LISTA as usize].child().and_downcast::<gtk::Image>().unwrap(), "lista", "#E2E2E6");
        self.painel_b.set_active(true);
        let w = Rc::downgrade(self);
        self.painel_b.connect_toggled(move |b| {
            if let Some(a) = w.upgrade() {
                a.painel_aberto.set(b.is_active());
                a.painel.set_visible(b.is_active() && !a.e_compacto());
                if let Some(img) = b.child().and_downcast::<gtk::Image>() {
                    crate::conversa::set_icon(&img, "painel-direita", if b.is_active() { "#A5A1E2" } else { "#8E8F99" });
                }
            }
        });
        // back, forward, up
        let w = Rc::downgrade(self);
        self.voltar.connect_clicked(move |_| if let Some(a) = w.upgrade() { a.voltar_pasta() });
        let w = Rc::downgrade(self);
        self.avancar.connect_clicked(move |_| if let Some(a) = w.upgrade() { a.avancar_pasta() });
        let w = Rc::downgrade(self);
        self.acima.connect_clicked(move |_| if let Some(a) = w.upgrade() { a.subir() });
        // the path: a click where there is no folder name makes it a field to type one
        let editar = gtk::GestureClick::new();
        let w = Rc::downgrade(self);
        editar.connect_released(move |g, _, x, y| {
            let Some(a) = w.upgrade() else { return };
            let num_botao = g.widget().and_then(|m| m.pick(x, y, gtk::PickFlags::DEFAULT)).is_some_and(|mut p| loop {
                if p.is::<gtk::Button>() {
                    break true;
                }
                match p.parent() {
                    Some(q) => p = q,
                    None => break false,
                }
            });
            if !num_botao {
                a.editar_caminho();
            }
        });
        self.migalha.add_controller(editar);
        let w = Rc::downgrade(self);
        self.caminho_campo.connect_activate(move |e| {
            let Some(a) = w.upgrade() else { return };
            let p = expandir(e.text().trim());
            if p.is_dir() {
                a.navegar(&p);
                a.caminho.set_visible_child_name("migalhas");
            } else if p.is_file() {
                a.abrir_em(Some(&p));
                a.caminho.set_visible_child_name("migalhas");
            } else {
                e.add_css_class("erro");
                let e = e.clone();
                glib::timeout_add_local_once(Duration::from_millis(900), move || e.remove_css_class("erro"));
            }
        });
        let saida = gtk::EventControllerFocus::new();
        let w = Rc::downgrade(self);
        saida.connect_leave(move |_| if let Some(a) = w.upgrade() { a.caminho.set_visible_child_name("migalhas") });
        self.caminho_campo.add_controller(saida);
        // search: typing filters by name; Enter with three words or more (or the spark) asks the agent here
        let w = Rc::downgrade(self);
        self.busca.connect_changed(move |_| if let Some(a) = w.upgrade() { a.render() });
        let w = Rc::downgrade(self);
        self.busca.connect_activate(move |e| {
            let Some(a) = w.upgrade() else { return };
            if e.text().split_whitespace().count() >= 3 {
                a.pedir_da_busca();
            }
        });
        let w = Rc::downgrade(self);
        pedir.connect_clicked(move |_| if let Some(a) = w.upgrade() { a.pedir_da_busca() });
        // New: a folder, a text document
        let w = Rc::downgrade(self);
        let pop = novo_lista.ancestor(gtk::Popover::static_type()).and_downcast::<gtk::Popover>();
        let fechar = move || if let Some(p) = &pop { p.popdown() };
        let nova = Self::item_menu(novo_lista, "pasta", "#E2E2E6", tr("Folder"), "Ctrl+Shift+N", "");
        let f = fechar.clone();
        let w2 = w.clone();
        nova.connect_clicked(move |_| {
            f();
            if let Some(a) = w2.upgrade() {
                a.nova_pasta();
            }
        });
        let doc = Self::item_menu(novo_lista, "doc", "#E2E2E6", tr("Text document"), "", "");
        doc.connect_clicked(move |_| {
            fechar();
            if let Some(a) = w.upgrade() {
                a.novo_documento();
            }
        });
        // the selection's commands
        for (id, b) in &self.comandos {
            let (w, id) = (Rc::downgrade(self), *id);
            b.connect_clicked(move |b| {
                let Some(a) = w.upgrade() else { return };
                let sel: Vec<PathBuf> = a.selecionados.borrow().iter().cloned().collect();
                match id {
                    "cortar" | "copiar" => a.copiar_selecao(id == "cortar"),
                    "colar" => a.colar(),
                    "renomear" => a.renomear(),
                    "compartilhar" => a.menu_compartilhar(b.upcast_ref(), &sel),
                    _ => if na_lixeira(&a.pasta.borrow()) { a.apagar_de_vez(&sel) } else { a.lixeira(&sel) },
                }
            });
            crate::central::posicao_debug(&format!("arquivos comando {id}"), b.upcast_ref());
        }
        let w = Rc::downgrade(self);
        organizar_b.connect_clicked(move |_| if let Some(a) = w.upgrade() { a.organizar() });
        // the list's columns sort it (again: the other way)
        for (ordem, b, _, _) in &self.cab_colunas {
            let (w, ordem) = (Rc::downgrade(self), *ordem);
            b.connect_clicked(move |_| if let Some(a) = w.upgrade() { a.ordenar_por(ordem) });
        }
        // the window's width changes: the icons' lines hold as many as fit
        let w = Rc::downgrade(self);
        grade_rol.hadjustment().connect_page_size_notify(move |_| if let Some(a) = w.upgrade() { a.ajustar_grade() });
        // the empty part of the files: the folder's own menu; a click there chooses nothing
        for rol in [lista_rol.upcast_ref::<gtk::Widget>(), grade_rol.upcast_ref(), self.colunas_rol.upcast_ref()] {
            let fundo = gtk::GestureClick::new();
            fundo.set_button(3);
            let w = Rc::downgrade(self);
            fundo.connect_pressed(move |g, _, x, y| {
                if let (Some(a), Some(wd)) = (w.upgrade(), g.widget()) {
                    a.selecionados.borrow_mut().clear();
                    a.selecao_mudou();
                    a.menu_da_pasta(&wd, x, y);
                }
            });
            rol.add_controller(fundo);
            let fundo_esq = gtk::GestureClick::new();
            let w = Rc::downgrade(self);
            fundo_esq.connect_released(move |g, _, x, y| {
                let Some(a) = w.upgrade() else { return };
                a.conteudo.grab_focus();
                // a click on a file is the file's (it chose it); only an empty part clears the choice
                let em_arquivo = g.widget().and_then(|c| c.pick(x, y, gtk::PickFlags::DEFAULT)).is_some_and(|mut w| loop {
                    if ["arq-linha", "arq-tile", "arq-col-item", "arq-faixa-item"].iter().any(|c| w.has_css_class(c)) {
                        break true;
                    }
                    match w.parent() {
                        Some(p) => w = p,
                        None => break false,
                    }
                });
                if !em_arquivo && !a.selecionados.borrow().is_empty() {
                    a.selecionados.borrow_mut().clear();
                    a.selecao_mudou();
                }
            });
            rol.add_controller(fundo_esq);
            // files dropped here from another app are copied in
            let alvo = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY | gdk::DragAction::MOVE);
            let w = Rc::downgrade(self);
            alvo.connect_drop(move |t, v, _, _| {
                let (Some(a), Ok(fl)) = (w.upgrade(), v.get::<gdk::FileList>()) else { return false };
                let destino = a.pasta.borrow().clone();
                let ps: Vec<PathBuf> = fl.files().iter().filter_map(|f| f.path()).filter(|p| p.parent() != Some(destino.as_path())).collect();
                a.colar_em(&ps, &destino, mover_ao_soltar(t, &ps, &destino));
                true
            });
            rol.add_controller(alvo);
        }
        // the keys: before the buttons (Capture), so the arrows move among the files when the files have the keyboard
        let k = gtk::EventControllerKey::new();
        k.set_propagation_phase(gtk::PropagationPhase::Capture);
        let w = Rc::downgrade(self);
        k.connect_key_pressed(move |_, tecla, _, mods| {
            let Some(a) = w.upgrade() else { return glib::Propagation::Proceed };
            a.tecla(tecla, mods)
        });
        self.janela.add_controller(k);
    }

    /// Compact when the window is short or narrow (the design's 1366×768 version); it changes as it is resized.
    fn ajustar_layout(self: &Rc<Self>) {
        let (w, h) = (self.janela.default_width(), self.janela.default_height());
        let compacto = (h > 0 && h < 700) || (w > 0 && w < 1100);
        if self.compacto.get() == Some(compacto) {
            return;
        }
        self.compacto.set(Some(compacto));
        let c = compacto;
        self.lateral.set_visible(!c);
        self.trilho.set_visible(c);
        self.painel.set_visible(!c && self.painel_aberto.get());
        self.painel_b.set_visible(!c);
        self.info_b.set_visible(c);
        self.busca_campo.set_size_request(if c { 240 } else { 320 }, -1);
        self.organizar_rotulo.set_text(if c { tr("Organise") } else { tr("Organise with the agent") });
        if depurando() {
            eprintln!("debug arquivos: {} ({w}×{h})", if c { "compacto" } else { "completo" });
            if c {
                crate::central::posicao_debug("arquivos info", self.info_b.upcast_ref());
            }
        }
        self.lateral_render();
        self.render();
    }

    fn e_compacto(&self) -> bool {
        self.compacto.get().unwrap_or(false)
    }

    pub fn abrir_em(self: &Rc<Self>, p: Option<&Path>) {
        self.ajustar_layout();
        match p {
            Some(p) if p.is_dir() => self.navegar(p),
            Some(p) if p.is_file() => {
                // a file: its folder, with it chosen
                if let Some(d) = p.parent() {
                    self.navegar(d);
                    *self.selecionados.borrow_mut() = BTreeSet::from([p.to_path_buf()]);
                    *self.ancora.borrow_mut() = Some(p.to_path_buf());
                }
            }
            _ => {
                let atual = self.pasta.borrow().clone();
                if self.itens.borrow().is_empty() {
                    self.ir(&atual);
                }
            }
        }
        self.janela.present();
    }

    fn navegar(self: &Rc<Self>, p: &Path) {
        if *self.pasta.borrow() != p {
            self.atras.borrow_mut().push(self.pasta.borrow().clone());
            self.frente.borrow_mut().clear();
        }
        self.ir(p);
    }

    fn voltar_pasta(self: &Rc<Self>) {
        let p = self.atras.borrow_mut().pop();
        if let Some(p) = p {
            self.frente.borrow_mut().push(self.pasta.borrow().clone());
            self.ir(&p);
        }
    }

    fn avancar_pasta(self: &Rc<Self>) {
        let p = self.frente.borrow_mut().pop();
        if let Some(p) = p {
            self.atras.borrow_mut().push(self.pasta.borrow().clone());
            self.ir(&p);
        }
    }

    /// Up a folder: the one it was in comes chosen.
    fn subir(self: &Rc<Self>) {
        let atual = self.pasta.borrow().clone();
        if let Some(pai) = atual.parent().filter(|_| !na_lixeira(&atual) || atual != lixeira().join("files")) {
            let pai = pai.to_path_buf();
            self.navegar(&pai);
            *self.selecionados.borrow_mut() = BTreeSet::from([atual.clone()]);
            *self.ancora.borrow_mut() = Some(atual);
        }
    }

    /// Shows a folder (reads it in the background; follows its changes).
    fn ir(self: &Rc<Self>, p: &Path) {
        *self.pasta.borrow_mut() = p.to_path_buf();
        self.selecionados.borrow_mut().clear();
        self.itens.borrow_mut().clear();
        self.dups.borrow_mut().clear();
        self.busca.set_text("");
        self.busca.set_placeholder_text(Some(&trf("Search in {folder} or ask the agent", &[("folder", nome_da_pasta(p))])));
        self.voltar.set_sensitive(!self.atras.borrow().is_empty());
        self.avancar.set_sensitive(!self.frente.borrow().is_empty());
        self.acima.set_sensitive(p.parent().is_some());
        {
            let mut v = self.visitadas.borrow_mut();
            v.retain(|q| q != p);
            v.insert(0, p.to_path_buf());
            v.truncate(16);
        }
        if let Some(aba) = self.abas.borrow_mut().get_mut(self.aba_atual.get()) {
            aba.pasta = p.to_path_buf();
        }
        let mon = gio::File::for_path(p).monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, None::<&gio::Cancellable>).ok();
        if let Some(m) = &mon {
            let w = Rc::downgrade(self);
            m.connect_changed(move |_, _, _, _| {
                if let Some(a) = w.upgrade() {
                    a.recarregar_depois();
                }
            });
        }
        *self.monitor.borrow_mut() = mon;
        self.lateral_render();
        self.recarregar();
    }

    // ---- tabs

    fn guardar_aba(&self) {
        if let Some(aba) = self.abas.borrow_mut().get_mut(self.aba_atual.get()) {
            aba.pasta = self.pasta.borrow().clone();
            aba.atras = self.atras.borrow().clone();
            aba.frente = self.frente.borrow().clone();
        }
    }

    fn trocar_aba(self: &Rc<Self>, k: usize) {
        if k == self.aba_atual.get() || k >= self.abas.borrow().len() {
            return;
        }
        self.guardar_aba();
        self.aba_atual.set(k);
        let aba = self.abas.borrow()[k].clone();
        *self.atras.borrow_mut() = aba.atras;
        *self.frente.borrow_mut() = aba.frente;
        self.ir(&aba.pasta);
        if depurando() {
            eprintln!("debug arquivos: aba {k} ({})", aba.pasta.display());
        }
    }

    fn nova_aba(self: &Rc<Self>, p: &Path) {
        self.guardar_aba();
        self.abas.borrow_mut().push(Aba { pasta: p.to_path_buf(), ..Default::default() });
        self.aba_atual.set(self.abas.borrow().len() - 1);
        self.atras.borrow_mut().clear();
        self.frente.borrow_mut().clear();
        self.ir(p);
        if depurando() {
            eprintln!("debug arquivos: nova aba ({} abas)", self.abas.borrow().len());
        }
    }

    fn fechar_aba(self: &Rc<Self>, k: usize) {
        if self.abas.borrow().len() <= 1 {
            self.janela.close();
            return;
        }
        self.guardar_aba();
        self.abas.borrow_mut().remove(k);
        let n = self.abas.borrow().len();
        let atual = self.aba_atual.get();
        let novo = if k < atual { atual - 1 } else if k == atual { k.min(n - 1) } else { atual };
        self.aba_atual.set(usize::MAX);
        self.aba_atual.set(novo);
        let aba = self.abas.borrow()[novo].clone();
        *self.atras.borrow_mut() = aba.atras;
        *self.frente.borrow_mut() = aba.frente;
        self.ir(&aba.pasta);
    }

    fn abas_render(self: &Rc<Self>) {
        limpar(&self.abas_box);
        let abas = self.abas.borrow().clone();
        for (k, aba) in abas.iter().enumerate() {
            let b = gtk::Button::new();
            b.add_css_class("arq-aba");
            if k == self.aba_atual.get() {
                b.add_css_class("ativa");
            }
            let c = caixa(Orientation::Horizontal, 8, "");
            c.append(&arte_pasta(&aba.pasta, None, 16));
            let n = rotulo(&nome_da_pasta(&aba.pasta), "arq-aba-nome");
            n.set_ellipsize(gtk::pango::EllipsizeMode::End);
            n.set_max_width_chars(18);
            c.append(&n);
            if abas.len() > 1 {
                let x = botao_icone("fechar", "#7A7B88", 12, "arq-aba-fechar", tr("Close the tab (Ctrl+W)"));
                let w = Rc::downgrade(self);
                x.connect_clicked(move |_| if let Some(a) = w.upgrade() { a.fechar_aba(k) });
                c.append(&x);
            }
            b.set_child(Some(&c));
            b.set_tooltip_text(Some(&com_til(&aba.pasta)));
            let w = Rc::downgrade(self);
            b.connect_clicked(move |_| if let Some(a) = w.upgrade() { a.trocar_aba(k) });
            // the middle button closes it, as in a browser
            let meio = gtk::GestureClick::new();
            meio.set_button(2);
            let w = Rc::downgrade(self);
            meio.connect_released(move |_, _, _, _| if let Some(a) = w.upgrade() { a.fechar_aba(k) });
            b.add_controller(meio);
            // a file dropped on a tab goes to its folder
            self.alvo_de_arquivos(&b, aba.pasta.clone());
            self.abas_box.append(&b);
        }
        let nova = botao_icone("mais", "#8E8F99", 16, "arq-aba-nova", tr("New tab (Ctrl+T)"));
        let w = Rc::downgrade(self);
        nova.connect_clicked(move |_| if let Some(a) = w.upgrade() { a.nova_aba(&glib::home_dir()) });
        crate::central::posicao_debug("arquivos nova aba", nova.upcast_ref());
        self.abas_box.append(&nova);
    }

    // ---- the path

    /// The path, as buttons: "⌂ Pasta pessoal › Downloads" (deep ones start with "…").
    fn migalha_render(self: &Rc<Self>) {
        limpar(&self.migalha);
        let p = self.pasta.borrow().clone();
        let home = glib::home_dir();
        let mut partes: Vec<(String, PathBuf)> = Vec::new();
        let glifo;
        if na_lixeira(&p) {
            glifo = "lixeira";
            partes.push((tr("Trash").to_string(), p.clone()));
        } else {
            glifo = if p.starts_with(&home) { "casa" } else { "disco" };
            let mut c = p.clone();
            loop {
                partes.push((nome_da_pasta(&c), c.clone()));
                match c.parent() {
                    Some(pai) if c != home => c = pai.to_path_buf(),
                    _ => break,
                }
            }
            partes.reverse();
        }
        self.migalha.append(&ui_icon(glifo, "#8E8F99", 16));
        let n = partes.len();
        // at most four names: the ones before go under "…" (compact: three)
        let primeira = n.saturating_sub(if self.e_compacto() { 3 } else { 4 });
        for (i, (nome, alvo)) in partes.into_iter().enumerate().skip(primeira) {
            let nome = if primeira > 0 && i == primeira && i + 1 != n { "…".to_string() } else { nome };
            if i > primeira {
                self.migalha.append(&ui_icon("seta-direita", "#7A7B88", 12));
            }
            let b = gtk::Button::with_label(&nome);
            b.add_css_class("arq-migalha");
            if i + 1 == n {
                b.add_css_class("atual");
            } else if let Some(l) = b.child().and_downcast::<gtk::Label>() {
                l.set_ellipsize(gtk::pango::EllipsizeMode::End);
            }
            let (w, alvo2) = (Rc::downgrade(self), alvo.clone());
            b.connect_clicked(move |_| if let Some(a) = w.upgrade() { a.navegar(&alvo2) });
            // a name takes files dropped on it (moved there)
            if i + 1 != n {
                self.alvo_de_arquivos(&b, alvo);
            }
            self.migalha.append(&b);
        }
        let resto = espaco();
        resto.set_size_request(24, -1);
        self.migalha.append(&resto);
        self.historico_render();
    }

    /// Ctrl+L or a click on the path's empty part: the path as text, to type another one.
    fn editar_caminho(self: &Rc<Self>) {
        self.caminho_campo.set_text(&self.pasta.borrow().display().to_string());
        self.caminho.set_visible_child_name("editar");
        self.caminho_campo.grab_focus();
        self.caminho_campo.select_region(0, -1);
    }

    /// The folders opened lately (this window's), then those of the files used lately.
    fn historico_render(self: &Rc<Self>) {
        limpar(&self.historico);
        let atual = self.pasta.borrow().clone();
        let mut pastas: Vec<PathBuf> = self.visitadas.borrow().iter().filter(|p| **p != atual && p.is_dir()).cloned().collect();
        for r in gtk::RecentManager::default().items().into_iter().take(40) {
            if let Some(d) = gio::File::for_uri(&r.uri()).path().and_then(|p| p.parent().map(Path::to_path_buf)) {
                if d != atual && d.is_dir() && !pastas.contains(&d) {
                    pastas.push(d);
                }
            }
        }
        if pastas.is_empty() {
            let v = rotulo(tr("No other folder opened yet."), "arq-sub");
            v.set_margin_top(8);
            v.set_margin_bottom(8);
            self.historico.append(&v);
            return;
        }
        let pop = self.historico.ancestor(gtk::Popover::static_type()).and_downcast::<gtk::Popover>();
        for p in pastas.into_iter().take(12) {
            let b = gtk::Button::new();
            b.add_css_class("arq-menu-item");
            let c = caixa(Orientation::Horizontal, 10, "");
            c.set_margin_start(8);
            c.append(&arte_pasta(&p, None, 16));
            let n = rotulo(&nome_da_pasta(&p), "");
            c.append(&n);
            let caminho = rotulo(&com_til(p.parent().unwrap_or(Path::new("/"))), "arq-menu-atalho");
            caminho.set_hexpand(true);
            caminho.set_xalign(1.0);
            caminho.set_ellipsize(gtk::pango::EllipsizeMode::Start);
            c.append(&caminho);
            b.set_child(Some(&c));
            let (w, pop) = (Rc::downgrade(self), pop.clone());
            b.connect_clicked(move |_| {
                if let Some(p) = &pop {
                    p.popdown();
                }
                if let Some(a) = w.upgrade() {
                    a.navegar(&p);
                }
            });
            self.historico.append(&b);
        }
    }

    // ---- sorting, grouping, showing

    fn ordenar_por(self: &Rc<Self>, ordem: usize) {
        if self.ordem.get() == ordem {
            self.crescente.set(!self.crescente.get());
        } else {
            self.ordem.set(ordem);
            // names and kinds A→Z; dates and sizes the newest and largest first
            self.crescente.set(matches!(ordem, 1 | 3));
        }
        self.render();
    }

    /// The Sort menu: by what, grouped how, and which files show.
    fn ordem_menu_render(self: &Rc<Self>) {
        limpar(&self.ordem_menu);
        let pop = self.ordem_menu.ancestor(gtk::Popover::static_type()).and_downcast::<gtk::Popover>();
        let secao = |t: &str| {
            let l = rotulo(t, "arq-menu-titulo");
            l.set_xalign(0.0);
            l
        };
        self.ordem_menu.append(&secao(tr("SORT BY")));
        for (k, o) in ORDENS.iter().enumerate() {
            let marca = if self.ordem.get() == k { if self.crescente.get() { "seta-cima" } else { "chevron-baixo" } } else { "" };
            let b = Self::item_menu(&self.ordem_menu, if marca.is_empty() { "nenhum" } else { marca }, "#A5A1E2", tr(o), "", "");
            let (w, pop) = (Rc::downgrade(self), pop.clone());
            b.connect_clicked(move |_| {
                if let Some(p) = &pop {
                    p.popdown();
                }
                if let Some(a) = w.upgrade() {
                    a.ordenar_por(k);
                }
            });
        }
        limpar(&self.agrupar_menu);
        let gpop = self.agrupar_menu.ancestor(gtk::Popover::static_type()).and_downcast::<gtk::Popover>();
        self.agrupar_menu.append(&secao(tr("GROUP BY")));
        for (k, g) in AGRUPAR.iter().enumerate() {
            let b = Self::item_menu(&self.agrupar_menu, if self.agrupar.get() == k { "check" } else { "nenhum" }, "#A5A1E2", tr(g), "", "");
            let (w, pop) = (Rc::downgrade(self), gpop.clone());
            b.connect_clicked(move |_| {
                if let Some(p) = &pop {
                    p.popdown();
                }
                if let Some(a) = w.upgrade() {
                    a.agrupar.set(k);
                    a.render();
                }
            });
        }
        self.ordem_menu.append(&secao(tr("SHOW")));
        for (k, f) in FILTROS.iter().enumerate() {
            let b = Self::item_menu(&self.ordem_menu, if self.filtro.get() == k { "check" } else { "nenhum" }, "#A5A1E2", tr(f), "", "");
            let (w, pop) = (Rc::downgrade(self), pop.clone());
            b.connect_clicked(move |_| {
                if let Some(p) = &pop {
                    p.popdown();
                }
                if let Some(a) = w.upgrade() {
                    a.filtro.set(k);
                    a.render();
                }
            });
        }
    }

    fn recarregar_depois(self: &Rc<Self>) {
        if self.pendente.replace(true) {
            return;
        }
        let w = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_millis(300), move || {
            if let Some(a) = w.upgrade() {
                a.pendente.set(false);
                a.recarregar();
            }
        });
    }

    fn recarregar(self: &Rc<Self>) {
        let p = self.pasta.borrow().clone();
        let ocultos = self.ocultos.get();
        let g = self.geracao.get() + 1;
        self.geracao.set(g);
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(move || {
            let itens = ler_pasta(&p, ocultos);
            let dups = duplicados(&itens);
            (itens, dups)
        }, move |(itens, dups)| {
            let Some(a) = w.upgrade() else { return };
            if a.geracao.get() != g {
                return;
            }
            // what was chosen and is gone (moved, deleted) is not chosen anymore
            let existe: BTreeSet<PathBuf> = itens.iter().map(|i| i.caminho.clone()).collect();
            a.selecionados.borrow_mut().retain(|p| existe.contains(p));
            *a.itens.borrow_mut() = itens;
            *a.dups.borrow_mut() = dups;
            a.lateral_render();
            a.render();
            a.agente_organizou();
        });
    }

    /// The files an agent moved into this folder (kalangod's timeline, not undone): "Organizado pelo agente".
    fn agente_organizou(self: &Rc<Self>) {
        let p = self.pasta.borrow().clone();
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(move || crate::servicos::kalangod_registros(0, 5000).unwrap_or_default().into_iter()
            .filter(|e| e["tool"] == "arquivos.mover" && e["status"] == "ok" && e["desfeito"] != true)
            .filter_map(|e| e["args"]["para"].as_str().map(expandir)).filter(|d| d.parent() == Some(p.as_path())).collect::<BTreeSet<PathBuf>>(),
            move |s| {
                if let Some(a) = w.upgrade() {
                    let mudou = *a.pelo_agente.borrow() != s;
                    *a.pelo_agente.borrow_mut() = s;
                    if mudou && a.filtro.get() == FILTROS.len() - 1 {
                        a.render();
                    }
                }
            });
    }

    /// The filters hiding part of the folder now: the kind (Sort › Show), the tag, the typed name.
    fn filtros_ativos(&self) -> Vec<String> {
        let mut f = Vec::new();
        if self.filtro.get() != 0 {
            f.push(tr(FILTROS[self.filtro.get().min(FILTROS.len() - 1)]).to_string());
        }
        if let Some(e) = self.etiqueta_filtro.borrow().as_ref() {
            f.push(trf("tag {name}", &[("name", e.clone())]));
        }
        let busca = self.busca.text().trim().to_string();
        if !busca.is_empty() && busca.split_whitespace().count() < 3 {
            f.push(format!("“{busca}”"));
        }
        f
    }

    /// "Mostrar tudo": no kind, no tag, no name.
    fn mostrar_tudo(self: &Rc<Self>) {
        self.filtro.set(0);
        *self.etiqueta_filtro.borrow_mut() = None;
        if self.busca.text().split_whitespace().count() < 3 {
            self.busca.set_text("");
        }
        self.lateral_render();
        self.render();
    }

    fn visiveis(&self) -> Vec<Item> {
        let busca = self.busca.text().trim().to_lowercase();
        // a request typed for the agent filters nothing until Enter: only one or two words filter by name
        let busca = if busca.split_whitespace().count() >= 3 { String::new() } else { busca };
        let semana = glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0) - 7 * 86_400;
        let pelo_agente = self.pelo_agente.borrow();
        let etiqueta = self.etiqueta_filtro.borrow().clone();
        let mut v: Vec<Item> = self.itens.borrow().iter().filter(|i| match self.filtro.get() {
            1 => !i.pasta && e_documento_tipo(&i.caminho, &i.tipo),
            2 => !i.pasta && e_imagem(&i.tipo),
            3 => !i.pasta && e_instalador_ext(&i.caminho),
            4 => !i.pasta && i.modificado >= semana,
            5 => pelo_agente.contains(&i.caminho),
            _ => true,
        }).filter(|i| etiqueta.as_ref().is_none_or(|e| i.etiquetas.contains(e)))
            .filter(|i| busca.is_empty() || i.nome.to_lowercase().contains(&busca)).cloned().collect();
        match self.ordem.get() {
            1 => v.sort_by_key(|i| i.nome.to_lowercase()),
            2 => v.sort_by_key(|i| i.tamanho),
            3 => v.sort_by_key(|i| (descricao_tipo(i), i.nome.to_lowercase())),
            _ => v.sort_by_key(|i| i.modificado),
        }
        if !self.crescente.get() {
            v.reverse();
        }
        v.sort_by_key(|i| !i.pasta);
        v
    }

    /// The groups of the list and the icons: by date (Pastas, Hoje, Ontem, Esta semana…), by kind, or none.
    fn agrupados(&self, v: &[Item]) -> Vec<(String, Vec<Item>)> {
        let agora = glib::DateTime::now_local().ok();
        let dia = |x: &glib::DateTime| glib::DateTime::from_local(x.year(), x.month(), x.day_of_month(), 0, 0, 0.0).map(|d| d.to_unix()).unwrap_or(0);
        let hoje = agora.as_ref().map(dia).unwrap_or(0);
        let grupo = |i: &Item| -> (u8, String) {
            if i.pasta {
                return (0, tr("Folders").to_string());
            }
            match self.agrupar.get() {
                0 => {
                    let d = glib::DateTime::from_unix_local(i.modificado).ok();
                    let dias = (hoje - d.as_ref().map(dia).unwrap_or(0)) / 86_400;
                    let mesmo = |f: fn(&glib::DateTime) -> i32| d.as_ref().zip(agora.as_ref()).is_some_and(|(d, a)| f(d) == f(a));
                    match dias {
                        ..=0 => (1, tr("Today").to_string()),
                        1 => (2, tr("Yesterday").to_string()),
                        2..=6 => (3, tr("This week").to_string()),
                        _ if mesmo(|x| x.year()) && mesmo(|x| x.month()) => (4, tr("This month").to_string()),
                        _ if mesmo(|x| x.year()) => (5, tr("Earlier this year").to_string()),
                        _ => (6, tr("Older").to_string()),
                    }
                }
                1 => {
                    let t = i.tipo.as_str();
                    if e_imagem(t) { (2, tr("Images").to_string()) }
                    else if t.starts_with("audio/") { (3, tr("Music and audio").to_string()) }
                    else if t.starts_with("video/") { (4, tr("Videos").to_string()) }
                    else if e_instalador_ext(&i.caminho) { (5, tr("Installers").to_string()) }
                    else if e_documento_tipo(&i.caminho, t) { (1, tr("Documents").to_string()) }
                    else { (6, tr("Other files").to_string()) }
                }
                _ => (1, String::new()),
            }
        };
        let mut gs: Vec<(u8, String, Vec<Item>)> = Vec::new();
        for i in v {
            let (k, nome) = grupo(i);
            match gs.iter_mut().find(|(kk, n, _)| *kk == k && *n == nome) {
                Some((_, _, l)) => l.push(i.clone()),
                None => gs.push((k, nome, vec![i.clone()])),
            }
        }
        gs.sort_by_key(|(k, _, _)| *k);
        // grouping by nothing: no headers (the folders still first)
        if self.agrupar.get() == 2 {
            return vec![(String::new(), v.to_vec())];
        }
        gs.into_iter().map(|(_, n, l)| (n, l)).collect()
    }

    fn pedir_da_busca(self: &Rc<Self>) {
        let t = self.busca.text().trim().to_string();
        if t.is_empty() {
            return;
        }
        self.busca.set_text("");
        let p = self.pasta.borrow().display().to_string();
        let total = self.itens.borrow().iter().filter(|i| !i.pasta).count();
        self.pedir_ao_agente(&trf("In the folder {folder}: {request}", &[("folder", p), ("request", t)]), total);
    }

    fn copiar_selecao(self: &Rc<Self>, mover: bool) {
        let sel: Vec<PathBuf> = self.selecionados.borrow().iter().cloned().collect();
        if sel.is_empty() {
            return;
        }
        *self.area.borrow_mut() = (sel.clone(), mover);
        let files: Vec<gio::File> = sel.iter().map(gio::File::for_path).collect();
        self.janela.clipboard().set_content(Some(&gdk::ContentProvider::for_value(&gdk::FileList::from_array(&files).to_value()))).ok();
        if depurando() {
            eprintln!("debug arquivos: {} {}", if mover { "recortados" } else { "copiados para colar" }, sel.len());
        }
    }

    fn novo_documento(self: &Rc<Self>) {
        let p = nome_livre(&self.pasta.borrow(), &format!("{}.txt", tr("New document")));
        if std::fs::write(&p, b"").is_ok() {
            *self.selecionados.borrow_mut() = BTreeSet::from([p.clone()]);
            *self.ancora.borrow_mut() = Some(p);
            self.recarregar();
            let w = Rc::downgrade(self);
            glib::timeout_add_local_once(Duration::from_millis(500), move || if let Some(a) = w.upgrade() { a.renomear() });
        }
    }

    /// "Compartilhar": what can be done with the chosen files outside this window.
    fn menu_compartilhar(self: &Rc<Self>, w: &gtk::Widget, sel: &[PathBuf]) {
        if sel.is_empty() {
            return;
        }
        let pop = Self::popover_em(w, f64::from(w.width()) / 2.0, f64::from(w.height()));
        let c = caixa(Orientation::Vertical, 2, "");
        c.set_size_request(280, -1);
        let copiar_b = Self::item_menu(&c, "copiar", "#E2E2E6", tr("Copy (to paste in another app)"), "Ctrl+C", "");
        let caminho_b = Self::item_menu(&c, "codigo", "#E2E2E6", tr("Copy the path"), "", "");
        let email_b = Self::item_menu(&c, "enviar", "#E2E2E6", tr("Send by email"), "", "");
        email_b.set_visible(glib::find_program_in_path("xdg-email").is_some());
        pop.set_child(Some(&c));
        let fechar = {
            let pw = pop.downgrade();
            move || if let Some(p) = pw.upgrade() { p.popdown() }
        };
        let (s, f) = (sel.to_vec(), fechar.clone());
        copiar_b.connect_clicked(move |b| {
            f();
            let files: Vec<gio::File> = s.iter().map(gio::File::for_path).collect();
            b.clipboard().set_content(Some(&gdk::ContentProvider::for_value(&gdk::FileList::from_array(&files).to_value()))).ok();
        });
        let (s, f) = (sel.to_vec(), fechar.clone());
        caminho_b.connect_clicked(move |b| {
            f();
            b.clipboard().set_text(&s.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n"));
        });
        let (s, f) = (sel.to_vec(), fechar);
        email_b.connect_clicked(move |_| {
            f();
            let mut c = std::process::Command::new("xdg-email");
            for p in &s {
                c.arg("--attach").arg(p);
            }
            let _ = c.spawn();
        });
        pop.popup();
    }

    // ---- the views

    fn render(self: &Rc<Self>) {
        self.abas_render();
        self.migalha_render();
        self.ordem_menu_render();
        let v = self.visiveis();
        for c in [&self.lista, &self.grade, &self.colunas_box, &self.palco, &self.faixa] {
            limpar(c);
        }
        self.cartoes.borrow_mut().clear();
        let modo = self.modo.get();
        self.cabecalho.set_visible(modo == MODO_LISTA);
        self.conteudo.set_visible_child_name(VISTAS[modo as usize].0);
        self.cabecalho_render();
        let atual = self.trabalho.borrow().as_ref().and_then(|t| t.atual.clone());
        match modo {
            MODO_LISTA | MODO_ICONES => {
                for (nome, itens) in self.agrupados(&v) {
                    if !nome.is_empty() {
                        let g = caixa(Orientation::Horizontal, 6, "arq-grupo-linha");
                        g.append(&rotulo(&nome, "arq-grupo"));
                        g.append(&rotulo(&itens.len().to_string(), "arq-grupo-n"));
                        if modo == MODO_LISTA { self.lista.append(&g) } else { self.grade.append(&g) }
                    }
                    if modo == MODO_LISTA {
                        for i in &itens {
                            self.lista.append(&self.linha(i, atual.as_ref() == Some(&i.caminho)));
                        }
                    } else {
                        let fb = gtk::FlowBox::new();
                        fb.set_selection_mode(gtk::SelectionMode::None);
                        fb.set_column_spacing(8);
                        fb.set_row_spacing(8);
                        fb.set_min_children_per_line(1);
                        fb.set_max_children_per_line(40);
                        fb.set_valign(Align::Start);
                        fb.set_halign(Align::Start);
                        fb.set_homogeneous(true);
                        fb.add_css_class("arq-grade-fluxo");
                        for i in &itens {
                            let t = self.tile(i, atual.as_ref() == Some(&i.caminho));
                            fb.insert(&t, -1);
                            if let Some(ch) = t.parent() {
                                ch.set_focusable(false);
                            }
                        }
                        self.grade.append(&fb);
                    }
                }
            }
            MODO_COLUNAS => self.colunas_render(&v),
            _ => self.galeria_render(&v),
        }
        // the filter that hides files, said (and undone in one click)
        let filtros = self.filtros_ativos();
        self.filtro_chip.set_visible(!filtros.is_empty());
        self.filtro_chip_rotulo.set_text(&trf("Showing: {filters}", &[("filters", filtros.join(" · "))]));
        if v.is_empty() && modo != MODO_COLUNAS {
            let vazia = self.itens.borrow().is_empty();
            let e = caixa(Orientation::Vertical, 8, "");
            let t = rotulo(&if vazia {
                tr("This folder is empty.").to_string()
            } else if filtros.is_empty() {
                tr("Nothing here matches.").to_string()
            } else {
                trf("Nothing here matches “{filters}” ({n} items hidden).", &[("filters", filtros.join(" · ")), ("n", self.itens.borrow().len().to_string())])
            }, "arq-vazio");
            t.set_xalign(0.0);
            t.set_wrap(true);
            e.append(&t);
            if !vazia && !filtros.is_empty() {
                let b = gtk::Button::with_label(tr("Show everything"));
                b.add_css_class("midia-acao");
                b.set_halign(Align::Start);
                b.set_margin_start(10);
                let w = Rc::downgrade(self);
                b.connect_clicked(move |_| {
                    if let Some(a) = w.upgrade() {
                        a.mostrar_tudo();
                    }
                });
                crate::central::posicao_debug("arquivos mostrar tudo", b.upcast_ref());
                e.append(&b);
            }
            match modo {
                MODO_LISTA => self.lista.append(&e),
                MODO_ICONES => self.grade.append(&e),
                _ => self.palco.append(&e),
            }
            if depurando() {
                eprintln!("debug arquivos: nada à vista ({} itens; filtros: {})", self.itens.borrow().len(), filtros.join(", "));
            }
        }
        self.ajustar_grade();
        // the documents' pages are made in the background (the icons and the gallery show them at once)
        for i in v.iter().filter(|i| !i.pasta && !e_imagem(&i.tipo)).take(60) {
            let _ = self.previa(i, 96, 72, (52, 66, 3), 64);
        }
        self.selecao_mudou();
        if depurando() {
            eprintln!("debug arquivos: {} ({} itens, {} visíveis, {} duplicados, {} selecionados)", self.pasta.borrow().display(), self.itens.borrow().len(), v.len(),
                self.dups.borrow().len(), self.selecionados.borrow().len());
        }
    }

    /// How many 112 px tiles (8 apart) fit in the icons' width now.
    fn por_linha(&self) -> i32 {
        let w = self.grade.width().max(self.conteudo.width());
        ((w - 32 + 8) / (112 + 8)).max(1)
    }

    /// The icons' lines hold exactly as many tiles as fit: the tiles keep their 112 px, packed from the left.
    fn ajustar_grade(&self) {
        let n = self.por_linha() as u32;
        let mut c = self.grade.first_child();
        while let Some(x) = c {
            if let Some(fb) = x.downcast_ref::<gtk::FlowBox>() {
                if fb.max_children_per_line() != n {
                    fb.set_min_children_per_line(n);
                    fb.set_max_children_per_line(n);
                }
            }
            c = x.next_sibling();
        }
    }

    /// The list's column names: the sorting one lit, with its direction.
    fn cabecalho_render(&self) {
        for (ordem, b, _, seta) in &self.cab_colunas {
            let ativa = *ordem == self.ordem.get();
            if ativa { b.add_css_class("ativa") } else { b.remove_css_class("ativa") }
            seta.set_visible(ativa);
            crate::conversa::set_icon(seta, if self.crescente.get() { "seta-cima" } else { "chevron-baixo" }, "#A5A1E2");
        }
    }

    /// The check of a chosen file (it keeps its place when not chosen: the names stay aligned).
    fn marca(px: i32) -> (gtk::Box, gtk::Box) {
        let lugar = caixa(Orientation::Horizontal, 0, "");
        lugar.set_size_request(px, px);
        lugar.set_valign(Align::Center);
        let check = caixa(Orientation::Horizontal, 0, "arq-check");
        check.set_size_request(px, px);
        check.append(&ui_icon("check", "#1C1D24", px * 3 / 4));
        check.set_visible(false);
        lugar.append(&check);
        (lugar, check)
    }

    /// A file's small icon (the list, the columns): a folder's art, a picture's thumbnail, else its kind's glyph.
    fn icone_pequeno(self: &Rc<Self>, i: &Item, px: i32) -> gtk::Widget {
        if i.pasta {
            return arte_pasta(&i.caminho, i.emblema, px).upcast();
        }
        if e_imagem(&i.tipo) && i.tamanho < 96 << 20 {
            // an Image of exactly px (a Picture would ask the texture's own size and grow the row)
            let img = gtk::Image::new();
            img.set_pixel_size(px);
            img.set_size_request(px, px);
            img.add_css_class("arq-mini");
            img.set_valign(Align::Center);
            img.set_overflow(gtk::Overflow::Hidden);
            let iw = img.downgrade();
            self.com_textura(i, Alvo::Foto(px, px), Box::new(move |t| if let Some(img) = iw.upgrade() { img.set_paintable(Some(t)) }));
            return img.upcast();
        }
        let (g, tom, cor) = glifo_do_tipo(i);
        let b = caixa(Orientation::Horizontal, 0, "arq-tipo");
        b.add_css_class(&format!("tom-{tom}"));
        b.set_size_request(px, px);
        b.set_valign(Align::Center);
        let ic = ui_icon(g, cor, px * 7 / 10);
        b.set_homogeneous(true);
        ic.set_halign(Align::Center);
        b.append(&ic);
        b.upcast()
    }

    /// A preview for `mostrar`, at once when it is made already, else made in the background (one job per preview,
    /// however many places wait for it). False when the file has none.
    fn com_textura(self: &Rc<Self>, i: &Item, alvo: Alvo, mostrar: Mostrar) -> bool {
        let chave = (i.caminho.clone(), i.modificado, alvo);
        let pronta = self.texturas.borrow().get(&chave).cloned();
        if let Some(t) = pronta {
            mostrar(&t);
            return true;
        }
        if self.sem_miniatura.borrow().contains(&(i.caminho.clone(), i.modificado)) {
            return false;
        }
        let ja = self.esperando.borrow().contains_key(&chave);
        self.esperando.borrow_mut().entry(chave.clone()).or_default().push(mostrar);
        if !ja {
            let (wk, p, tipo, mt) = (Rc::downgrade(self), i.caminho.clone(), i.tipo.clone(), i.modificado);
            crate::lateral::em_segundo_plano(move || miniatura(&p, &tipo, mt, alvo), move |r| {
                let Some(a) = wk.upgrade() else { return };
                let quem = a.esperando.borrow_mut().remove(&chave).unwrap_or_default();
                match r {
                    Some(r) => {
                        let t = textura(r);
                        a.texturas.borrow_mut().insert(chave, t.clone());
                        for f in quem {
                            f(&t);
                        }
                    }
                    None => {
                        a.sem_miniatura.borrow_mut().insert((chave.0, chave.1));
                    }
                }
            });
        }
        true
    }

    /// A row of the list (design "Arquivos · nova (lista)"): check, icon, name (and "duplicado"), modified, kind, size, tags.
    fn linha(self: &Rc<Self>, i: &Item, agente_aqui: bool) -> gtk::Widget {
        let r = caixa(Orientation::Horizontal, 0, "arq-linha");
        let nome = caixa(Orientation::Horizontal, 10, "");
        nome.set_hexpand(true);
        let (lugar, check) = Self::marca(16);
        nome.append(&lugar);
        nome.append(&self.icone_pequeno(i, 20));
        let n = rotulo(&i.nome, "arq-nome");
        n.set_xalign(0.0);
        n.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        nome.append(&n);
        if self.dups.borrow().contains_key(&i.caminho) {
            let d = rotulo(tr("duplicate"), "arq-dup");
            d.set_valign(Align::Center);
            nome.append(&d);
        }
        r.append(&nome);
        for (txt, largura, x) in [(quando_hora(i.modificado), 170, 0.0), (descricao_tipo(i), 150, 0.0), (tamanho_ou_itens(i), 90, 1.0)] {
            let l = rotulo(&txt, "arq-col");
            l.set_size_request(largura, -1);
            l.set_max_width_chars(1);
            l.set_xalign(x);
            l.set_ellipsize(gtk::pango::EllipsizeMode::End);
            r.append(&l);
        }
        let tags = caixa(Orientation::Horizontal, 6, "");
        tags.set_size_request(130, -1);
        tags.set_margin_start(16);
        if let Some(e) = i.etiquetas.first() {
            let cor = etiquetas_conhecidas().into_iter().find(|x| x.nome == *e).map(|x| x.cor).unwrap_or_else(|| "#8E8F99".into());
            let p = caixa(Orientation::Horizontal, 0, "arq-ponto8");
            p.add_css_class(&format!("tom-{}", tom_de(&cor)));
            p.set_size_request(8, 8);
            p.set_valign(Align::Center);
            tags.append(&p);
            let t = rotulo(&if i.etiquetas.len() > 1 { format!("{e} +{}", i.etiquetas.len() - 1) } else { e.clone() }, "arq-tag-texto");
            t.set_ellipsize(gtk::pango::EllipsizeMode::End);
            tags.append(&t);
        }
        r.append(&tags);
        let raiz: gtk::Widget = if agente_aqui {
            let o = gtk::Overlay::new();
            o.set_child(Some(&r));
            o.add_overlay(&cursor_do_agente());
            o.upcast()
        } else {
            r.clone().upcast()
        };
        self.cliques(&raiz, i);
        self.arrastavel(&raiz, i);
        if i.pasta {
            self.alvo_de_arquivos(&raiz, i.caminho.clone());
        }
        crate::central::posicao_debug(&format!("arquivo {}", i.nome), raiz.upcast_ref());
        self.cartoes.borrow_mut().push((i.caminho.clone(), r.upcast(), check.upcast()));
        raiz
    }

    /// A tile of the icons view (design "Arquivos · nova (ícones)"): the art, the name on two lines, the size.
    fn tile(self: &Rc<Self>, i: &Item, agente_aqui: bool) -> gtk::Widget {
        let t = caixa(Orientation::Vertical, 6, "arq-tile");
        t.set_size_request(112, 132);
        let art = caixa(Orientation::Horizontal, 0, "");
        art.set_size_request(96, 72);
        art.set_halign(Align::Center);
        let a: gtk::Widget = if i.pasta {
            arte_pasta(&i.caminho, i.emblema, 64).upcast()
        } else {
            let p = self.previa(i, 96, 72, (52, 66, 3), 64);
            p.add_css_class("arq-tile-arte");
            p
        };
        art.set_homogeneous(true);
        a.set_halign(Align::Center);
        a.set_valign(Align::Center);
        art.append(&a);
        t.append(&art);
        let n = rotulo(&i.nome, "arq-tile-nome");
        n.set_wrap(true);
        n.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        n.set_lines(2);
        n.set_ellipsize(gtk::pango::EllipsizeMode::End);
        n.set_justify(gtk::Justification::Center);
        n.set_max_width_chars(14);
        n.set_size_request(100, -1);
        t.append(&n);
        t.append(&rotulo(&tamanho_ou_itens(i), "arq-tile-meta"));
        let o = gtk::Overlay::new();
        o.set_child(Some(&t));
        let (lugar, check) = Self::marca(18);
        lugar.set_halign(Align::Start);
        lugar.set_valign(Align::Start);
        lugar.set_margin_start(8);
        lugar.set_margin_top(8);
        lugar.set_can_target(false);
        o.add_overlay(&lugar);
        if agente_aqui {
            o.add_overlay(&cursor_do_agente());
        }
        self.cliques(&o, i);
        self.arrastavel(&o, i);
        if i.pasta {
            self.alvo_de_arquivos(&o, i.caminho.clone());
        }
        crate::central::posicao_debug(&format!("arquivo {}", i.nome), o.upcast_ref());
        self.cartoes.borrow_mut().push((i.caminho.clone(), t.upcast(), check.upcast()));
        o.upcast()
    }

    /// The columns view (design "Arquivos · nova (colunas)"): a column for each folder from the person's folder (or the
    /// system's root) to this one; a folder clicked opens the next column.
    fn colunas_render(self: &Rc<Self>, v: &[Item]) {
        let pasta = self.pasta.borrow().clone();
        let home = glib::home_dir();
        let raiz = if pasta.starts_with(&home) { home } else { PathBuf::from("/") };
        let mut cadeia = vec![];
        let mut c = pasta.clone();
        loop {
            cadeia.push(c.clone());
            if c == raiz {
                break;
            }
            match c.parent() {
                Some(p) => c = p.to_path_buf(),
                None => break,
            }
        }
        cadeia.reverse();
        let inicio = cadeia.len().saturating_sub(4);
        for k in inicio..cadeia.len() {
            let col = caixa(Orientation::Vertical, 1, "arq-coluna-pasta");
            col.set_valign(Align::Start);
            if k + 1 == cadeia.len() {
                for i in v {
                    col.append(&self.item_coluna(i));
                }
                if v.is_empty() {
                    let e = rotulo(tr("This folder is empty."), "arq-vazio");
                    e.set_margin_start(8);
                    col.append(&e);
                }
            } else {
                let proximo = &cadeia[k + 1];
                for (p, nome, e_pasta) in ler_leve(&cadeia[k]) {
                    col.append(&self.entrada_coluna(&p, &nome, e_pasta, p == *proximo));
                }
            }
            let rol = gtk::ScrolledWindow::new();
            rol.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
            rol.set_size_request(232, -1);
            rol.set_hexpand(false);
            rol.set_child(Some(&col));
            rol.add_css_class("arq-coluna-rol");
            self.colunas_box.append(&rol);
        }
        // the open folder's column in view
        let rol = self.colunas_rol.clone();
        glib::idle_add_local_once(move || {
            let h = rol.hadjustment();
            h.set_value(h.upper() - h.page_size());
        });
    }

    /// A file of the open folder in its column (chosen as in the list).
    fn item_coluna(self: &Rc<Self>, i: &Item) -> gtk::Widget {
        let r = caixa(Orientation::Horizontal, 8, "arq-col-item");
        r.append(&self.icone_pequeno(i, 16));
        let n = rotulo(&i.nome, "arq-nome");
        n.set_xalign(0.0);
        n.set_hexpand(true);
        n.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        r.append(&n);
        if i.pasta {
            r.append(&ui_icon("seta-direita", "#7A7B88", 12));
        }
        self.cliques(&r, i);
        self.arrastavel(&r, i);
        if i.pasta {
            self.alvo_de_arquivos(&r, i.caminho.clone());
        }
        crate::central::posicao_debug(&format!("arquivo {}", i.nome), r.upcast_ref());
        let vazio = caixa(Orientation::Horizontal, 0, "");
        self.cartoes.borrow_mut().push((i.caminho.clone(), r.clone().upcast(), vazio.upcast()));
        r.upcast()
    }

    /// An entry of a column before the open folder: the folder on the way is marked; a click opens it.
    fn entrada_coluna(self: &Rc<Self>, p: &Path, nome: &str, e_pasta: bool, no_caminho: bool) -> gtk::Widget {
        let r = caixa(Orientation::Horizontal, 8, "arq-col-item");
        if no_caminho {
            r.add_css_class("caminho");
        }
        if e_pasta {
            r.append(&arte_pasta(p, None, 16));
        } else {
            let tipo = gio::content_type_guess(Some(p), &[]).0;
            let mime = gio::content_type_get_mime_type(&tipo).map(|m| m.to_string()).unwrap_or_default();
            let falso = Item { caminho: p.to_path_buf(), nome: nome.to_string(), pasta: false, tamanho: 0, modificado: 0, criado: 0, tipo: mime, filhos: None, emblema: None, etiquetas: vec![] };
            let (g, tom, cor) = glifo_do_tipo(&falso);
            let b = caixa(Orientation::Horizontal, 0, "arq-tipo");
            b.add_css_class(&format!("tom-{tom}"));
            b.set_size_request(16, 16);
            b.set_valign(Align::Center);
            let ic = ui_icon(g, cor, 11);
            b.set_homogeneous(true);
            ic.set_halign(Align::Center);
            b.append(&ic);
            r.append(&b);
        }
        let n = rotulo(nome, "arq-nome");
        n.set_xalign(0.0);
        n.set_hexpand(true);
        n.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        r.append(&n);
        if e_pasta {
            r.append(&ui_icon("seta-direita", "#7A7B88", 12));
        }
        let c = gtk::GestureClick::new();
        let (w, p2) = (Rc::downgrade(self), p.to_path_buf());
        c.connect_released(move |_, n, _, _| {
            let Some(a) = w.upgrade() else { return };
            if p2.is_dir() {
                a.navegar(&p2);
            } else if let Some(d) = p2.parent() {
                a.navegar(d);
                *a.selecionados.borrow_mut() = BTreeSet::from([p2.clone()]);
                *a.ancora.borrow_mut() = Some(p2.clone());
                if n >= 2 {
                    a.abrir(&p2);
                }
            }
        });
        r.add_controller(c);
        if e_pasta {
            self.alvo_de_arquivos(&r, p.to_path_buf());
        }
        r.upcast()
    }

    /// The gallery (design "Arquivos · nova (galeria)"): the chosen file as large as it fits, its name and kind under it,
    /// and a strip of the folder's files.
    fn galeria_render(self: &Rc<Self>, v: &[Item]) {
        let foco = self.ancora.borrow().clone().filter(|a| v.iter().any(|i| i.caminho == *a))
            .or_else(|| v.iter().find(|i| !i.pasta).or(v.first()).map(|i| i.caminho.clone()));
        if self.ancora.borrow().is_none() {
            *self.ancora.borrow_mut() = foco.clone();
        }
        self.palco_render(v);
        for i in v {
            let o = gtk::Overlay::new();
            let t = caixa(Orientation::Horizontal, 0, "arq-faixa-item");
            t.set_size_request(92, 68);
            let a: gtk::Widget = if i.pasta { arte_pasta(&i.caminho, i.emblema, 40).upcast() } else { self.previa(i, 92, 68, (40, 52, 8), 40) };
            t.set_homogeneous(true);
            t.append(&a);
            t.set_tooltip_text(Some(&i.nome));
            o.set_child(Some(&t));
            self.cliques(&o, i);
            self.arrastavel(&o, i);
            crate::central::posicao_debug(&format!("arquivo {}", i.nome), o.upcast_ref());
            let vazio = caixa(Orientation::Horizontal, 0, "");
            self.cartoes.borrow_mut().push((i.caminho.clone(), t.upcast(), vazio.upcast()));
            self.faixa.append(&o);
        }
    }

    /// The gallery's stage: the anchor file, large.
    fn palco_render(self: &Rc<Self>, v: &[Item]) {
        limpar(&self.palco);
        let a = self.ancora.borrow().clone();
        let Some(i) = a.and_then(|p| v.iter().find(|i| i.caminho == p).cloned()) else { return };
        let area = caixa(Orientation::Vertical, 0, "");
        area.set_vexpand(true);
        if i.pasta {
            let art = arte_pasta(&i.caminho, i.emblema, 160);
            art.set_vexpand(true);
            area.append(&art);
        } else {
            let pic = gtk::Picture::new();
            pic.set_can_shrink(true);
            pic.set_content_fit(gtk::ContentFit::Contain);
            pic.set_vexpand(true);
            pic.set_hexpand(true);
            pic.set_margin_top(24);
            pic.set_margin_start(24);
            pic.set_margin_end(24);
            let pw = pic.downgrade();
            let falsa = pagina_falsa(&i, 180, 232, true);
            falsa.set_halign(Align::Center);
            falsa.set_valign(Align::Center);
            falsa.set_vexpand(true);
            let (pw2, fw) = (pic.downgrade(), falsa.downgrade());
            let tem = self.com_textura(&i, Alvo::Inteira(900, 640), Box::new(move |t| {
                if let Some(p) = pw2.upgrade() {
                    p.set_paintable(Some(t));
                    p.set_visible(true);
                }
                if let Some(f) = fw.upgrade() {
                    f.set_visible(false);
                }
            }));
            let _ = pw;
            // until (or unless) the preview comes, the page stands in
            if pic.paintable().is_none() {
                pic.set_visible(false);
                area.append(&falsa);
            }
            let _ = tem;
            area.prepend(&pic);
        }
        self.palco.append(&area);
        let leg = caixa(Orientation::Horizontal, 8, "");
        leg.set_halign(Align::Center);
        leg.set_margin_bottom(16);
        leg.append(&rotulo(&i.nome, "arq-legenda-nome"));
        leg.append(&rotulo(&format!("{} · {}", descricao_tipo(&i), tamanho_ou_itens(&i)), "arq-sub"));
        self.palco.append(&leg);
    }

    /// A preview box: a folder's art, the picture itself, or the document's first page (system thumbnailers), else
    /// its kind's glyph; with the extension's badge ("PDF" in coral, as in the design).
    fn previa(self: &Rc<Self>, i: &Item, w: i32, h: i32, pagina: (i32, i32, i32), pasta_px: i32) -> gtk::Widget {
        let sobre = gtk::Overlay::new();
        sobre.add_css_class("arq-previa");
        // in a card or the panel it takes their width (`w` is what the picture is made for)
        sobre.set_size_request(if w >= 150 { -1 } else { w }, h);
        // its height is its own (the glyph inside expands within it, not the panel), and a small one keeps its
        // width (in a recent file's card the text takes the rest)
        sobre.set_vexpand(false);
        if w < 150 {
            sobre.set_hexpand(false);
        }
        sobre.set_overflow(gtk::Overflow::Hidden);
        let fundo = caixa(Orientation::Vertical, 0, "");
        fundo.set_hexpand(true);
        sobre.set_child(Some(&fundo));
        if i.pasta {
            let art = arte(&i.emblema.map(|e| format!("pasta-grande-{e}")).unwrap_or_else(|| "pasta-grande".into()), pasta_px);
            art.set_vexpand(true);
            fundo.append(&art);
            return sobre.upcast();
        }
        let imagem = e_imagem(&i.tipo) && i.tamanho < 96 << 20;
        let alvo = if imagem { Alvo::Foto(w, h) } else { Alvo::Pagina(pagina.0, pagina.1) };
        let chave = (i.caminho.clone(), i.modificado, alvo);
        // a picture fills the box; a page is an image of its own size (a Picture would ask the box's whole width
        // in its proportion, and the panel's preview would grow)
        let pic = gtk::Picture::new();
        pic.set_can_shrink(true);
        pic.set_content_fit(gtk::ContentFit::Cover);
        pic.set_vexpand(true);
        pic.set_hexpand(true);
        let pagina_img = gtk::Image::new();
        pagina_img.set_halign(Align::Center);
        pagina_img.set_valign(Align::Start);
        pagina_img.set_margin_top(pagina.2);
        pagina_img.add_css_class("arq-pagina");
        pagina_img.set_visible(false);
        if imagem {
            fundo.append(&pic);
        } else {
            fundo.append(&pagina_img);
        }
        // until (or unless) the page comes: a drawn page with the kind's badge (dark in the icons' tiles, light
        // elsewhere, as the design's cards), or the kind's glyph in a small box
        let glifo: gtk::Widget = if h >= 60 && pagina.0 >= 40 {
            let f = pagina_falsa(i, pagina.0, pagina.1.min(h - pagina.2), false);
            if w == 96 && h == 72 {
                f.add_css_class("escura");
            }
            f.set_halign(Align::Center);
            f.set_valign(Align::Start);
            f.set_margin_top(pagina.2);
            f.upcast()
        } else {
            let g = ui_icon(if e_instalador_ext(&i.caminho) { "download" } else { "doc" }, "#8E8F99", 14);
            g.set_vexpand(true);
            g.upcast()
        };
        if !imagem {
            fundo.append(&glifo);
        }
        let mostrar = {
            let (pic, pg, glifo) = (pic.downgrade(), pagina_img.downgrade(), glifo.downgrade());
            move |t: &gdk::Texture| {
                if imagem {
                    if let Some(pic) = pic.upgrade() {
                        pic.set_paintable(Some(t));
                    }
                } else if let Some(pg) = pg.upgrade() {
                    pg.set_paintable(Some(t));
                    pg.set_pixel_size(t.width().max(t.height()));
                    pg.set_size_request(t.width(), t.height());
                    pg.set_visible(true);
                    if let Some(g) = glifo.upgrade() {
                        g.set_visible(false);
                    }
                }
            }
        };
        let pronta = self.texturas.borrow().get(&chave).cloned();
        if let Some(t) = pronta {
            mostrar(&t);
        } else if !self.sem_miniatura.borrow().contains(&(i.caminho.clone(), i.modificado)) && (imagem || i.tamanho < 512 << 20) {
            let ja_pedida = self.esperando.borrow().contains_key(&chave);
            self.esperando.borrow_mut().entry(chave.clone()).or_default().push(Box::new(mostrar));
            if !ja_pedida {
                let (wk, p, tipo, mt, nome) = (Rc::downgrade(self), i.caminho.clone(), i.tipo.clone(), i.modificado, i.nome.clone());
                crate::lateral::em_segundo_plano(move || miniatura(&p, &tipo, mt, alvo), move |r| {
                    let Some(a) = wk.upgrade() else { return };
                    let quem = a.esperando.borrow_mut().remove(&chave).unwrap_or_default();
                    match r {
                        Some(r) => {
                            let t = textura(r);
                            a.texturas.borrow_mut().insert(chave, t.clone());
                            for f in quem {
                                f(&t);
                            }
                            if depurando() && !imagem {
                                eprintln!("debug arquivos: miniatura {nome}");
                            }
                        }
                        None => {
                            a.sem_miniatura.borrow_mut().insert((chave.0, chave.1));
                        }
                    }
                });
            }
        }
        let ext = extensao(&i.caminho);
        if !imagem && !ext.is_empty() && ext.len() <= 5 && h >= 40 {
            let b = rotulo(&ext.to_uppercase(), "arq-ext");
            if ext == "pdf" {
                b.add_css_class("pdf");
            }
            b.set_halign(Align::Start);
            b.set_valign(Align::End);
            b.set_margin_start(8);
            b.set_margin_bottom(3);
            sobre.add_overlay(&b);
        }
        sobre.upcast()
    }


    /// The files in the order they show (the list's and the icons' groups, one after the other).
    fn ordem_na_tela(&self) -> Vec<PathBuf> {
        let v = self.visiveis();
        match self.modo.get() {
            MODO_LISTA | MODO_ICONES => self.agrupados(&v).into_iter().flat_map(|(_, l)| l.into_iter().map(|i| i.caminho)).collect(),
            _ => v.into_iter().map(|i| i.caminho).collect(),
        }
    }

    /// The selection changed: the marks, the commands, the status line and the details (nothing else is rebuilt).
    fn selecao_mudou(self: &Rc<Self>) {
        let sel = self.selecionados.borrow().clone();
        for (p, card, check) in self.cartoes.borrow().iter() {
            if sel.contains(p) {
                card.add_css_class("escolhido");
            } else {
                card.remove_css_class("escolhido");
            }
            check.set_visible(sel.contains(p));
        }
        if self.modo.get() == MODO_GALERIA {
            let v = self.visiveis();
            self.palco_render(&v);
        }
        for (id, b) in &self.comandos {
            b.set_sensitive(*id == "colar" || !sel.is_empty());
        }
        self.status_render();
        self.painel_render();
        if self.info_b.popover().is_some_and(|p| p.is_visible()) {
            self.gaveta_render();
        }
        if depurando() {
            eprintln!("debug arquivos: {} selecionados", sel.len());
        }
    }

    /// The chosen file in view (the list scrolls to it).
    fn mostrar_cartao(&self, p: &Path) {
        let Some(w) = self.cartoes.borrow().iter().find(|(q, _, _)| q == p).map(|(_, c, _)| c.clone()) else { return };
        let Some(rol) = w.ancestor(gtk::ScrolledWindow::static_type()).and_downcast::<gtk::ScrolledWindow>() else { return };
        let Some(vista) = rol.child() else { return };
        let Some(b) = w.compute_bounds(&vista) else { return };
        let (adj, h) = (rol.vadjustment(), rol.hadjustment());
        let (y0, y1) = (f64::from(b.y()), f64::from(b.y() + b.height()));
        if y0 < 0.0 {
            adj.set_value(adj.value() + y0 - 8.0);
        } else if y1 > adj.page_size() {
            adj.set_value(adj.value() + y1 - adj.page_size() + 8.0);
        }
        let (x0, x1) = (f64::from(b.x()), f64::from(b.x() + b.width()));
        if x0 < 0.0 {
            h.set_value(h.value() + x0 - 8.0);
        } else if x1 > h.page_size() {
            h.set_value(h.value() + x1 - h.page_size() + 8.0);
        }
    }

    fn status_render(&self) {
        let p = self.pasta.borrow().clone();
        let livre = gio::File::for_path(&p).query_filesystem_info("filesystem::free", None::<&gio::Cancellable>).ok().map(|f| f.attribute_uint64("filesystem::free")).unwrap_or(0);
        let sel = self.selecionados.borrow();
        let n = self.itens.borrow().len();
        let tam: u64 = self.itens.borrow().iter().filter(|i| sel.contains(&i.caminho)).map(|i| i.tamanho).sum();
        let mut t = if n == 1 { tr("1 item").to_string() } else { trf("{n} items", &[("n", n.to_string())]) };
        if !sel.is_empty() {
            t += " · ";
            t.push_str(&if sel.len() == 1 { trf("1 selected ({size})", &[("size", tamanho(tam))]) } else { trf("{n} selected ({size})", &[("n", sel.len().to_string()), ("size", tamanho(tam))]) });
        }
        self.rodape.set_text(&t);
        self.livre.set_text(&trf("{size} free", &[("size", tamanho(livre))]));
    }

    /// The details (design: the right pane): the chosen file (or the folder, or what several chosen add up to).
    fn painel_render(self: &Rc<Self>) {
        limpar(&self.painel);
        let sel: Vec<PathBuf> = self.selecionados.borrow().iter().cloned().collect();
        // several chosen: the one clicked last shows (the status line says how many)
        let anc = self.ancora.borrow().clone().filter(|a| sel.contains(a));
        let item_de = |p: &Path| self.itens.borrow().iter().find(|i| i.caminho == p).cloned().unwrap_or_else(|| ler_um(p));
        let i = match (sel.as_slice(), anc) {
            ([], _) => ler_um(&self.pasta.borrow()),
            ([um], _) => item_de(um),
            (_, Some(a)) => item_de(&a),
            (varios, None) => {
                let tam: u64 = self.itens.borrow().iter().filter(|i| varios.contains(&i.caminho)).map(|i| i.tamanho).sum();
                let pv = caixa(Orientation::Vertical, 0, "arq-painel-previa");
                pv.set_size_request(-1, 220);
                pv.set_homogeneous(true);
                pv.append(&arte("pasta-grande", 112));
                self.painel.append(&pv);
                let n = rotulo(&trf("{n} items", &[("n", varios.len().to_string())]), "arq-painel-nome");
                n.set_xalign(0.0);
                self.painel.append(&n);
                let s = rotulo(&tamanho(tam), "arq-sub");
                s.set_xalign(0.0);
                self.painel.append(&s);
                let nomes = rotulo(&varios.iter().take(8).filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).collect::<Vec<_>>().join("\n"), "arq-valor");
                nomes.set_xalign(0.0);
                self.painel.append(&nomes);
                self.painel.append(&{
                    let e = caixa(Orientation::Vertical, 0, "");
                    e.set_vexpand(true);
                    e
                });
                let perguntar = gtk::Button::with_label(tr("Ask about these files"));
                perguntar.add_css_class("arq-acao");
                let (w, ps) = (Rc::downgrade(self), varios.to_vec());
                perguntar.connect_clicked(move |_| if let Some(a) = w.upgrade() { a.perguntar(&ps) });
                self.painel.append(&perguntar);
                return;
            }
        };
        // the preview: the folder's art, the picture, the document's first page (with its kind's badge)
        let pv: gtk::Widget = if i.pasta {
            let c = caixa(Orientation::Vertical, 0, "arq-painel-previa");
            c.set_size_request(-1, 220);
            c.set_homogeneous(true);
            c.append(&arte_pasta(&i.caminho, i.emblema, 112));
            c.upcast()
        } else {
            let p = self.previa(&i, 288, 220, (128, 172, 24), 112);
            p.add_css_class("arq-painel-previa");
            p
        };
        self.painel.append(&pv);
        let titulo = caixa(Orientation::Vertical, 4, "");
        let n = rotulo(&i.nome, "arq-painel-nome");
        n.set_xalign(0.0);
        n.set_wrap(true);
        n.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        n.set_max_width_chars(1);
        n.set_hexpand(true);
        titulo.append(&n);
        let desc = if i.pasta { format!("{} · {}", tr("Folder"), tamanho_ou_itens(&i)) } else { format!("{} · {}", descricao_tipo(&i), tamanho(i.tamanho)) };
        let s = rotulo(&desc, "arq-sub");
        s.set_xalign(0.0);
        s.set_wrap(true);
        s.set_max_width_chars(1);
        s.set_hexpand(true);
        titulo.append(&s);
        self.painel.append(&titulo);
        let info = caixa(Orientation::Vertical, 8, "");
        let linha = |k: &str, v: &str| {
            let l = caixa(Orientation::Horizontal, 8, "");
            let a = rotulo(k, "arq-sub");
            a.set_hexpand(true);
            a.set_xalign(0.0);
            l.append(&a);
            let b = rotulo(v, "arq-valor");
            b.set_ellipsize(gtk::pango::EllipsizeMode::Start);
            b.set_max_width_chars(24);
            l.append(&b);
            (l, b)
        };
        for (k, v) in [(tr("Modified"), quando_hora(i.modificado)), (tr("Created"), data(i.criado, false)), (tr("Where"), com_til(i.caminho.parent().unwrap_or(Path::new("/"))))] {
            if !v.is_empty() {
                info.append(&linha(k, &v).0);
            }
        }
        if e_imagem(&i.tipo) {
            if let Some((_, w, h)) = gtk::gdk_pixbuf::Pixbuf::file_info(&i.caminho) {
                info.append(&linha(tr("Dimensions"), &format!("{w} × {h}")).0);
            }
        } else if i.tipo.contains("pdf") && i.tamanho < 40 << 20 {
            // the pages: counted in the background (a large PDF takes a moment)
            let (l, v) = linha(tr("Pages"), "…");
            info.append(&l);
            let (vw, p) = (v.downgrade(), i.caminho.clone());
            crate::lateral::em_segundo_plano(move || contar_paginas(&p), move |n| {
                if let Some(v) = vw.upgrade() {
                    match n {
                        Some(n) => v.set_text(&n.to_string()),
                        None => {
                            if let Some(l) = v.parent() {
                                l.set_visible(false);
                            }
                        }
                    }
                }
            });
        }
        self.painel.append(&info);
        let orig = self.dups.borrow().get(&i.caminho).cloned();
        if !i.etiquetas.is_empty() || orig.is_some() {
            let tags = caixa(Orientation::Horizontal, 6, "");
            for e in &i.etiquetas {
                tags.append(&self.chip_etiqueta(e, false));
            }
            if orig.is_some() {
                tags.append(&chip(tr("Duplicate"), "coral"));
            }
            self.painel.append(&tags);
        }
        if !i.pasta {
            self.painel.append(&self.caixa_resumo(&i, orig.as_deref()));
        }
        let esp = caixa(Orientation::Vertical, 0, "");
        esp.set_vexpand(true);
        self.painel.append(&esp);
        let bs = caixa(Orientation::Horizontal, 8, "");
        let abrir = gtk::Button::with_label(tr("Open"));
        abrir.add_css_class("arq-acao-principal");
        abrir.set_hexpand(true);
        let compartilhar = gtk::Button::with_label(tr("Share"));
        compartilhar.add_css_class("arq-acao");
        compartilhar.set_hexpand(true);
        let mais = botao_icone("pontos-v", "#B9BAC4", 16, "arq-acao-mais", tr("More"));
        bs.append(&abrir);
        bs.append(&compartilhar);
        bs.append(&mais);
        self.painel.append(&bs);
        let (w, p) = (Rc::downgrade(self), i.caminho.clone());
        abrir.connect_clicked(move |_| if let Some(a) = w.upgrade() { a.abrir(&p) });
        let (w, p) = (Rc::downgrade(self), i.caminho.clone());
        compartilhar.connect_clicked(move |b| if let Some(a) = w.upgrade() { a.menu_compartilhar(b.upcast_ref(), std::slice::from_ref(&p)) });
        let (w, p, pasta) = (Rc::downgrade(self), i.caminho.clone(), i.pasta && i.caminho == *self.pasta.borrow());
        mais.connect_clicked(move |b| {
            let Some(a) = w.upgrade() else { return };
            if pasta {
                a.menu_da_pasta(b.upcast_ref(), f64::from(b.width()) / 2.0, f64::from(b.height()));
            } else {
                if !a.selecionados.borrow().contains(&p) {
                    *a.selecionados.borrow_mut() = BTreeSet::from([p.clone()]);
                }
                a.menu(b.upcast_ref(), f64::from(b.width()) / 2.0, f64::from(b.height()));
            }
        });
    }

    /// The compact layout's drawer: the chosen file (else the folder), its size, the agent's summary.
    fn gaveta_render(self: &Rc<Self>) {
        limpar(&self.gaveta);
        let sel: Vec<PathBuf> = self.selecionados.borrow().iter().cloned().collect();
        let anc = self.ancora.borrow().clone().filter(|a| sel.contains(a)).or_else(|| sel.first().cloned());
        let i = match anc {
            Some(p) => self.itens.borrow().iter().find(|i| i.caminho == p).cloned().unwrap_or_else(|| ler_um(&p)),
            None => ler_um(&self.pasta.borrow()),
        };
        let n = rotulo(&i.nome, "arq-gaveta-nome");
        n.set_xalign(0.0);
        n.set_max_width_chars(1);
        n.set_hexpand(true);
        n.set_wrap(true);
        n.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        self.gaveta.append(&n);
        let dup = self.dups.borrow().contains_key(&i.caminho);
        let sub = if i.pasta { tamanho_ou_itens(&i) } else if dup { format!("{} · {} · {}", descricao_tipo(&i), tamanho(i.tamanho), tr("duplicate")) } else { format!("{} · {}", descricao_tipo(&i), tamanho(i.tamanho)) };
        let s = rotulo(&sub, if dup { "arq-sub-dup" } else { "arq-sub" });
        s.set_xalign(0.0);
        s.set_wrap(true);
        self.gaveta.append(&s);
        if !i.pasta {
            let orig = self.dups.borrow().get(&i.caminho).cloned();
            self.gaveta.append(&self.caixa_resumo(&i, orig.as_deref()));
            let bs = caixa(Orientation::Horizontal, 8, "");
            bs.set_homogeneous(true);
            let abrir = gtk::Button::with_label(tr("Open"));
            abrir.add_css_class("arq-acao-principal");
            let compartilhar = gtk::Button::with_label(tr("Share"));
            compartilhar.add_css_class("arq-acao");
            bs.append(&abrir);
            bs.append(&compartilhar);
            let (w, p) = (Rc::downgrade(self), i.caminho.clone());
            abrir.connect_clicked(move |_| if let Some(a) = w.upgrade() { a.abrir(&p) });
            let (w, p) = (Rc::downgrade(self), i.caminho.clone());
            compartilhar.connect_clicked(move |b| if let Some(a) = w.upgrade() { a.menu_compartilhar(b.upcast_ref(), std::slice::from_ref(&p)) });
            self.gaveta.append(&bs);
        }
        if depurando() {
            eprintln!("debug arquivos: gaveta {}", i.nome);
        }
    }

    /// "Resumo do agente": kept until the file changes; made on request. A copy's: the folder's copies go to the
    /// trash in one click (Ctrl+Z brings them back).
    fn caixa_resumo(self: &Rc<Self>, i: &Item, orig: Option<&Path>) -> gtk::Box {
        let r = caixa(Orientation::Vertical, 8, "arq-resumo");
        let rc = caixa(Orientation::Horizontal, 6, "");
        rc.append(&estrela(14, "#A5A1E2"));
        rc.append(&rotulo(tr("Agent's summary"), "arq-resumo-titulo"));
        r.append(&rc);
        let texto = |t: &str| {
            let l = rotulo(t, "arq-resumo-texto");
            l.set_xalign(0.0);
            l.set_wrap(true);
            l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            l.set_max_width_chars(1);
            l.set_hexpand(true);
            l
        };
        if let Ok(t) = std::fs::read_to_string(resumo_path(i)) {
            r.append(&texto(t.trim()));
        } else if self.resumindo.borrow().contains(&i.caminho) {
            let l = texto(tr("Reading the file…"));
            l.add_css_class("pulsando");
            r.append(&l);
        } else {
            r.append(&texto(tr("The agent reads it here, on this computer, and tells you what it is.")));
            let b = gtk::Button::with_label(tr("Summarize"));
            b.add_css_class("arq-resumir");
            b.set_halign(Align::Start);
            let (w, it, o) = (Rc::downgrade(self), i.clone(), orig.map(Path::to_path_buf));
            b.connect_clicked(move |_| if let Some(a) = w.upgrade() { a.resumir(&it, o.as_deref()) });
            crate::central::posicao_debug("resumir", b.upcast_ref());
            r.append(&b);
        }
        let copias: Vec<PathBuf> = self.dups.borrow().keys().cloned().collect();
        if orig.is_some() && !copias.is_empty() {
            let b = gtk::Button::new();
            b.add_css_class("arq-resumo-acao");
            b.set_halign(Align::Start);
            let c = caixa(Orientation::Horizontal, 4, "");
            c.append(&rotulo(&if copias.len() == 1 { tr("Move the copy to the trash").to_string() } else { trf("Move the {n} copies to the trash", &[("n", copias.len().to_string())]) }, ""));
            c.append(&ui_icon("seta-direita", "#A5A1E2", 12));
            b.set_child(Some(&c));
            let w = Rc::downgrade(self);
            b.connect_clicked(move |_| if let Some(a) = w.upgrade() { a.lixeira(&copias) });
            r.append(&b);
        }
        r
    }

    /// The agent's summary of a file (the vision model for pictures), on this computer; kept until the file changes.
    fn resumir(self: &Rc<Self>, i: &Item, orig: Option<&Path>) {
        if !self.resumindo.borrow_mut().insert(i.caminho.clone()) {
            return;
        }
        self.painel_render();
        let it = i.clone();
        let bin = crate::voz::agent_binary();
        let copia_de = orig.map(|o| trf(" It is an identical copy of {original} (same content).", &[("original", o.display().to_string())])).unwrap_or_default();
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(move || {
            let o = if e_imagem(&it.tipo) {
                std::process::Command::new(&bin).args(["--json", "ver"]).arg(&it.caminho).arg(tr("Describe this picture in one short sentence.")).stdin(std::process::Stdio::null()).output()
            } else {
                std::process::Command::new(&bin).arg("--json")
                    .arg(trf("Read the file {path} and summarize it in one or two short sentences: what it is and what matters in it (dates, amounts).{copy} Answer only the summary.",
                        &[("path", it.caminho.display().to_string()), ("copy", copia_de)]))
                    .stdin(std::process::Stdio::null()).output()
            };
            let texto = o.ok().map(|o| String::from_utf8_lossy(&o.stdout).lines().filter_map(|l| serde_json::from_str::<Value>(l).ok())
                .filter(|v| v["tipo"] == "resposta" || v["tipo"] == "visao").filter_map(|v| v["texto"].as_str().map(str::to_string)).next_back().unwrap_or_default()).unwrap_or_default();
            if !texto.trim().is_empty() {
                let p = resumo_path(&it);
                if let Some(d) = p.parent() {
                    let _ = std::fs::create_dir_all(d);
                }
                let _ = std::fs::write(&p, texto.trim());
            }
            (it.caminho, texto)
        }, move |(p, t)| {
            if let Some(a) = w.upgrade() {
                a.resumindo.borrow_mut().remove(&p);
                if depurando() {
                    eprintln!("debug arquivos: resumo ({} caracteres)", t.trim().chars().count());
                }
                a.painel_render();
            }
        });
    }

    /// Click chooses (Ctrl adds, Shift a range), double click opens, right click: the file's menu.
    fn cliques(self: &Rc<Self>, w: &impl IsA<gtk::Widget>, i: &Item) {
        let c = gtk::GestureClick::new();
        c.set_button(0);
        let (a, it) = (Rc::downgrade(self), i.clone());
        c.connect_pressed(move |g, n, x, y| {
            let Some(a) = a.upgrade() else { return };
            let mods = g.current_event_state();
            if g.current_button() == 3 {
                // (the folder's own menu stays out; a left press is not claimed: it may become a drag)
                g.set_state(gtk::EventSequenceState::Claimed);
                if !a.selecionados.borrow().contains(&it.caminho) {
                    *a.selecionados.borrow_mut() = BTreeSet::from([it.caminho.clone()]);
                    a.selecao_mudou();
                }
                if let Some(wd) = g.widget() {
                    a.menu(&wd, x, y);
                }
                return;
            }
            // a click on a file gives the files the keyboard (Delete, F2, the arrows act on them next)
            a.conteudo.grab_focus();
            if n >= 2 {
                let p = it.caminho.clone();
                let w = Rc::downgrade(&a);
                glib::idle_add_local_once(move || if let Some(a) = w.upgrade() { a.abrir(&p) });
                return;
            }
            if mods.contains(gdk::ModifierType::CONTROL_MASK) {
                let mut s = a.selecionados.borrow_mut();
                if !s.remove(&it.caminho) {
                    s.insert(it.caminho.clone());
                }
            } else if mods.contains(gdk::ModifierType::SHIFT_MASK) {
                let v: Vec<PathBuf> = a.ordem_na_tela();
                let anc = a.ancora.borrow().clone();
                let (x0, x1) = (anc.and_then(|p| v.iter().position(|q| *q == p)).unwrap_or(0), v.iter().position(|q| *q == it.caminho).unwrap_or(0));
                *a.selecionados.borrow_mut() = v[x0.min(x1)..=x0.max(x1)].iter().cloned().collect();
            } else {
                *a.selecionados.borrow_mut() = BTreeSet::from([it.caminho.clone()]);
                *a.ancora.borrow_mut() = Some(it.caminho.clone());
            }
            a.selecao_mudou();
        });
        w.add_controller(c);
    }

    /// Dragging a file (or the chosen ones) out: to a folder, a tag, the trash, another app.
    fn arrastavel(self: &Rc<Self>, w: &impl IsA<gtk::Widget>, i: &Item) {
        let ds = gtk::DragSource::new();
        ds.set_actions(gdk::DragAction::COPY | gdk::DragAction::MOVE);
        let (a, p) = (Rc::downgrade(self), i.caminho.clone());
        ds.connect_prepare(move |_, _, _| {
            let a = a.upgrade()?;
            let sel = a.selecionados.borrow().clone();
            let ps: Vec<PathBuf> = if sel.contains(&p) { sel.into_iter().collect() } else { vec![p.clone()] };
            let files: Vec<gio::File> = ps.iter().map(gio::File::for_path).collect();
            Some(gdk::ContentProvider::for_value(&gdk::FileList::from_array(&files).to_value()))
        });
        w.add_controller(ds);
    }

    /// A folder that takes dropped files: moved there (copied when the drag only offers a copy).
    fn alvo_de_arquivos(self: &Rc<Self>, w: &impl IsA<gtk::Widget>, destino: PathBuf) {
        let alvo = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::MOVE | gdk::DragAction::COPY);
        let a = Rc::downgrade(self);
        alvo.connect_drop(move |t, v, _, _| {
            let (Some(a), Ok(fl)) = (a.upgrade(), v.get::<gdk::FileList>()) else { return false };
            let ps: Vec<PathBuf> = fl.files().iter().filter_map(|f| f.path()).filter(|p| *p != destino && p.parent() != Some(destino.as_path())).collect();
            a.colar_em(&ps, &destino, mover_ao_soltar(t, &ps, &destino));
            true
        });
        w.as_ref().add_controller(alvo);
    }

    /// Copies (or moves) files into a folder. A long one (much data, many items, or another disk) goes to the island
    /// (design "Operação de arquivos"): it goes on with this window closed. A short one is done right here.
    fn colar_em(self: &Rc<Self>, ps: &[PathBuf], destino: &Path, mover: bool) {
        if ps.is_empty() {
            return;
        }
        let (ps, destino) = (ps.to_vec(), destino.to_path_buf());
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(move || {
            use std::os::unix::fs::MetadataExt;
            let pr = crate::operacao::Progresso::default();
            for p in &ps {
                crate::operacao::contar(p, &pr);
            }
            let disco = |p: &Path| std::fs::metadata(p).ok().map(|m| m.dev());
            let renomear = mover && ps.iter().all(|p| disco(p) == disco(&destino));
            let longo = !renomear && (pr.total_bytes.load(std::sync::atomic::Ordering::Relaxed) > 64 << 20 || pr.total_itens.load(std::sync::atomic::Ordering::Relaxed) > 300);
            (ps, destino, longo)
        }, move |(ps, destino, longo)| {
            let Some(a) = w.upgrade() else { return };
            match a.shell.upgrade().and_then(|s| s.operacoes()).filter(|_| longo) {
                Some(ops) => ops.iniciar(ps, destino, mover),
                None => a.com_conflitos(ps, destino, mover),
            }
        });
    }

    /// A short copy or move, done at once (off the interface thread). A name already there: as the person decided
    /// (skip it, keep both, or replace it — the old one to the trash); not asked, both are kept.
    fn colar_ja(self: &Rc<Self>, ps: Vec<PathBuf>, destino: PathBuf, mover: bool, decisoes: HashMap<PathBuf, dialogos::Decisao>) {
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(move || {
            let mut erros = Vec::new();
            let mut feitos = 0;
            for p in &ps {
                let Some(nome) = p.file_name() else { continue };
                if destino.starts_with(p) {
                    continue;
                }
                let alvo = match decisoes.get(p) {
                    Some(dialogos::Decisao::Pular) => continue,
                    Some(dialogos::Decisao::Substituir) => {
                        let ja = destino.join(nome);
                        if ja.exists() {
                            if let Err(e) = para_lixeira(&ja) {
                                erros.push(format!("{}: {e}", ja.display()));
                                continue;
                            }
                        }
                        ja
                    }
                    _ => nome_livre(&destino, &nome.to_string_lossy()),
                };
                let r = if mover {
                    std::fs::rename(p, &alvo).or_else(|_| copiar(p, &alvo).and_then(|_| if p.is_dir() { std::fs::remove_dir_all(p) } else { std::fs::remove_file(p) }))
                } else {
                    copiar(p, &alvo)
                };
                match r {
                    Ok(()) => feitos += 1,
                    Err(e) => erros.push(format!("{}: {e}", p.display())),
                }
            }
            (feitos, erros)
        }, move |(feitos, erros)| {
            if depurando() {
                eprintln!("debug arquivos: {} {feitos} arquivos", if mover { "movidos" } else { "copiados" });
            }
            for e in erros {
                eprintln!("kalango-shell: arquivos: {e}");
            }
            if let Some(a) = w.upgrade() {
                a.recarregar();
            }
        });
    }

    /// The keys. Anywhere in the window: Ctrl+F search, Ctrl+L type a path, Ctrl+T / Ctrl+W tabs, Ctrl+Tab the next
    /// tab, Ctrl+1…4 the views, Alt+← → ↑ back, forward and up, F5 read again. With the files' keyboard (a click among
    /// them): the arrows choose (Shift extends), Enter opens, Backspace goes up, Delete trashes, F2 renames, Ctrl+A/C/X/V/Z.
    /// On a button or a field the keys are theirs (the arrows move between the controls).
    fn tecla(self: &Rc<Self>, tecla: gdk::Key, mods: gdk::ModifierType) -> glib::Propagation {
        use gdk::Key;
        let ctrl = mods.contains(gdk::ModifierType::CONTROL_MASK);
        let alt = mods.contains(gdk::ModifierType::ALT_MASK);
        let shift = mods.contains(gdk::ModifierType::SHIFT_MASK);
        let foco = gtk::prelude::GtkWindowExt::focus(&self.janela);
        let num_campo = foco.as_ref().is_some_and(|f| f.is::<gtk::Text>() || f.ancestor(gtk::Entry::static_type()).is_some());
        let conteudo: gtk::Widget = self.conteudo.clone().upcast();
        let nos_arquivos = foco.as_ref().is_none_or(|f| *f == conteudo || f.is_ancestor(&conteudo));
        let feito = glib::Propagation::Stop;
        let dialogo = self.fechar_dialogo.borrow().clone();
        if let Some(f) = dialogo {
            if tecla == Key::Escape {
                f();
                return feito;
            }
            return glib::Propagation::Proceed;
        }
        if tecla == Key::Escape {
            if let Some(p) = self.info_b.popover().filter(|p| p.is_visible()) {
                p.popdown();
                return feito;
            }
            if self.trabalho.borrow().is_some() {
                if let Some(u) = self.shell.upgrade().and_then(|s| s.usando()) {
                    u.parar_agora();
                } else {
                    self.parar_agente();
                }
                return feito;
            }
            if self.caminho.visible_child_name().is_some_and(|n| n == "editar") {
                self.caminho.set_visible_child_name("migalhas");
                self.conteudo.grab_focus();
                return feito;
            }
            if self.busca.has_focus() || foco.as_ref().is_some_and(|f| f.is_ancestor(&self.busca)) {
                self.busca.set_text("");
                self.conteudo.grab_focus();
                return feito;
            }
            if nos_arquivos && !self.selecionados.borrow().is_empty() {
                self.selecionados.borrow_mut().clear();
                self.selecao_mudou();
                return feito;
            }
            return glib::Propagation::Proceed;
        }
        // anywhere in the window
        match tecla {
            Key::f if ctrl => {
                self.busca.grab_focus();
                return feito;
            }
            Key::l if ctrl => {
                self.editar_caminho();
                return feito;
            }
            Key::t if ctrl => {
                let p = self.pasta.borrow().clone();
                self.nova_aba(&p);
                return feito;
            }
            Key::w if ctrl => {
                self.fechar_aba(self.aba_atual.get());
                return feito;
            }
            Key::Tab | Key::ISO_Left_Tab if ctrl => {
                let n = self.abas.borrow().len();
                if n > 1 {
                    let k = self.aba_atual.get();
                    self.trocar_aba(if shift || tecla == Key::ISO_Left_Tab { (k + n - 1) % n } else { (k + 1) % n });
                }
                return feito;
            }
            Key::_1 | Key::_2 | Key::_3 | Key::_4 if ctrl => {
                let k = match tecla { Key::_1 => 0, Key::_2 => 1, Key::_3 => 2, _ => 3 };
                self.vistas[k].set_active(true);
                return feito;
            }
            Key::Left if alt => {
                self.voltar_pasta();
                return feito;
            }
            Key::Right if alt => {
                self.avancar_pasta();
                return feito;
            }
            Key::Up if alt => {
                self.subir();
                return feito;
            }
            Key::F5 => {
                self.recarregar();
                return feito;
            }
            Key::N if ctrl => {
                self.nova_pasta();
                return feito;
            }
            Key::h if ctrl => {
                self.alternar_ocultos();
                return feito;
            }
            Key::d if ctrl => {
                let p = self.pasta.borrow().clone();
                self.alternar_favorito(&p);
                return feito;
            }
            _ => {}
        }
        if num_campo || !nos_arquivos {
            return glib::Propagation::Proceed;
        }
        let sel: Vec<PathBuf> = self.selecionados.borrow().iter().cloned().collect();
        match tecla {
            Key::Delete | Key::KP_Delete => {
                if na_lixeira(&self.pasta.borrow()) {
                    self.apagar_de_vez(&sel);
                } else {
                    self.lixeira(&sel);
                }
            }
            Key::F2 => self.renomear(),
            Key::Return | Key::KP_Enter | Key::i if alt || (ctrl && tecla == Key::i) => {
                // Alt+Enter, Ctrl+I: the properties (of the chosen file, else the folder)
                let p = sel.first().cloned().unwrap_or_else(|| self.pasta.borrow().clone());
                self.propriedades(&p);
            }
            Key::Return | Key::KP_Enter if ctrl => {
                for p in sel.iter().filter(|p| p.is_dir()) {
                    self.nova_aba(p);
                }
            }
            Key::Return | Key::KP_Enter => {
                if let [p] = sel.as_slice() {
                    self.abrir(p);
                }
            }
            Key::BackSpace => self.subir(),
            Key::Left | Key::Right | Key::Up | Key::Down | Key::Home | Key::End => self.mover_selecao(tecla, shift),
            Key::a if ctrl => {
                *self.selecionados.borrow_mut() = self.ordem_na_tela().into_iter().collect();
                self.selecao_mudou();
            }
            Key::z if ctrl => self.desfazer_lixeira(),
            Key::c | Key::x if ctrl => self.copiar_selecao(tecla == Key::x),
            Key::v if ctrl => self.colar(),
            _ => return glib::Propagation::Proceed,
        }
        feito
    }

    /// The arrows choose: one step in the list and the columns, a row in the icons (as many as fit), one in the gallery;
    /// in the columns ← goes up and → into the chosen folder; Home and End the first and the last.
    fn mover_selecao(self: &Rc<Self>, tecla: gdk::Key, estender: bool) {
        use gdk::Key;
        let v = self.ordem_na_tela();
        let modo = self.modo.get();
        let atual = self.ancora.borrow().clone().and_then(|p| v.iter().position(|q| *q == p));
        if modo == MODO_COLUNAS {
            match tecla {
                Key::Left => return self.subir(),
                Key::Right => {
                    if let Some(p) = atual.map(|k| v[k].clone()).filter(|p| p.is_dir()) {
                        self.navegar(&p);
                    }
                    return;
                }
                _ => {}
            }
        }
        if v.is_empty() {
            return;
        }
        let por_linha: isize = if modo == MODO_ICONES { self.por_linha() as isize } else { 1 };
        let passo: isize = match (modo, tecla) {
            (_, Key::Home) => -(v.len() as isize),
            (_, Key::End) => v.len() as isize,
            (MODO_LISTA | MODO_COLUNAS, Key::Left | Key::Right) => return,
            (MODO_ICONES, Key::Up) => -por_linha,
            (MODO_ICONES, Key::Down) => por_linha,
            (_, Key::Left | Key::Up) => -1,
            _ => 1,
        };
        let novo = match atual {
            Some(i) => (i as isize + passo).clamp(0, v.len() as isize - 1) as usize,
            None => 0,
        };
        if estender {
            self.selecionados.borrow_mut().insert(v[novo].clone());
        } else {
            *self.selecionados.borrow_mut() = BTreeSet::from([v[novo].clone()]);
        }
        *self.ancora.borrow_mut() = Some(v[novo].clone());
        self.selecao_mudou();
        self.mostrar_cartao(&v[novo]);
    }

    fn colar(self: &Rc<Self>) {
        let destino = self.pasta.borrow().clone();
        // what was copied here (Ctrl+X moves); else what another app put on the clipboard
        let (ps, mover) = self.area.borrow().clone();
        if !ps.is_empty() {
            if mover {
                self.area.borrow_mut().0.clear();
            }
            self.colar_em(&ps, &destino, mover);
            return;
        }
        let w = Rc::downgrade(self);
        self.janela.clipboard().read_value_async(gdk::FileList::static_type(), glib::Priority::DEFAULT, None::<&gio::Cancellable>, move |r| {
            let (Some(a), Ok(v)) = (w.upgrade(), r) else { return };
            if let Ok(fl) = v.get::<gdk::FileList>() {
                let ps: Vec<PathBuf> = fl.files().iter().filter_map(|f| f.path()).collect();
                a.colar_em(&ps, &destino, false);
            }
        });
    }

    fn nova_pasta(self: &Rc<Self>) {
        let p = nome_livre(&self.pasta.borrow(), tr("New folder"));
        if std::fs::create_dir(&p).is_ok() {
            *self.selecionados.borrow_mut() = BTreeSet::from([p]);
            self.recarregar();
            let w = Rc::downgrade(self);
            glib::timeout_add_local_once(Duration::from_millis(500), move || if let Some(a) = w.upgrade() { a.renomear() });
        }
    }

    fn abrir(self: &Rc<Self>, p: &Path) {
        if p.is_dir() {
            self.navegar(p);
            return;
        }
        // a KalangoOS agent's manifest: its install page (permissions first), not a text editor
        if p.extension().is_some_and(|e| e == "kalango-agente") {
            if let Some(s) = self.shell.upgrade() {
                s.instalar_agente(p);
            }
            return;
        }
        let l = gtk::FileLauncher::new(Some(&gio::File::for_path(p)));
        l.launch(Some(&self.janela), None::<&gio::Cancellable>, |_| {});
        gtk::RecentManager::default().add_item(&gio::File::for_path(p).uri());
    }

    /// "Perguntar": a new conversation in the agents center, the file(s) already in what is being written.
    fn perguntar(self: &Rc<Self>, ps: &[PathBuf]) {
        let Some(s) = self.shell.upgrade() else { return };
        s.abrir_central(Some("conversas"));
        if let Some(c) = s.central() {
            let lista = ps.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ");
            c.conversas().preencher(&if ps.len() == 1 { trf("About the file {path}: ", &[("path", lista)]) } else { trf("About the files {paths}: ", &[("paths", lista)]) });
        }
    }

    /// A request to the agent in the side bar's conversation, which works off screen: this window shows what it
    /// does here; an approval it needs comes as the island's notice.
    fn pedir_ao_agente(self: &Rc<Self>, pedido: &str, total: usize) {
        let Some(s) = self.shell.upgrade() else { return };
        *self.trabalho.borrow_mut() = Some(Trabalho { total: total.max(1), tocados: BTreeSet::new(), atual: None });
        self.trabalho_render();
        s.right_bar().conversa.ask(pedido);
    }

    fn parar_agente(self: &Rc<Self>) {
        if let Some(s) = self.shell.upgrade() {
            s.right_bar().conversa.stop(true);
        }
        self.terminar_trabalho();
    }

    fn terminar_trabalho(self: &Rc<Self>) {
        if self.trabalho.borrow_mut().take().is_none() {
            return;
        }
        self.trabalho_render();
        self.render();
    }

    /// What the agent at work shows: the banner and the island (the shell's), the outline and the chip (here).
    fn trabalho_render(self: &Rc<Self>) {
        let t = self.trabalho.borrow().as_ref().map(|t| (t.tocados.len(), t.total));
        let Some(s) = self.shell.upgrade() else { return };
        let Some(u) = s.usando() else { return };
        match t {
            Some((n, total)) => {
                self.janela.add_css_class("agente-usando");
                self.agente_chip_texto.set_text(&format!("{} · {n}/{total}", tr("Agent")));
                self.agente_chip.set_visible(true);
                let w = Rc::downgrade(self);
                u.mostrar(tr("Agent"), tr("Files"), n, total, tr("files"), move || {
                    if let Some(a) = w.upgrade() {
                        a.parar_agente();
                    }
                });
                if depurando() {
                    eprintln!("debug arquivos: agente usando ({n}/{total})");
                }
            }
            None => {
                self.janela.remove_css_class("agente-usando");
                self.agente_chip.set_visible(false);
                u.esconder();
            }
        }
    }

    /// The agent did something in the side bar's conversation: on files of this folder, the window shows it.
    fn agente_fez(self: &Rc<Self>, v: &Value) {
        match v["tipo"].as_str().unwrap_or("") {
            "fim" | "erro" | "parado" | "saiu" => {
                if self.trabalho.borrow().is_some() {
                    let w = Rc::downgrade(self);
                    glib::timeout_add_local_once(Duration::from_millis(1200), move || if let Some(a) = w.upgrade() { a.terminar_trabalho() });
                }
            }
            "ferramenta" => {
                let nome = v["nome"].as_str().unwrap_or("");
                if !nome.starts_with("arquivos.") || nome == "arquivos.listar" {
                    return;
                }
                let pasta = self.pasta.borrow().clone();
                let a = &v["argumentos"];
                let alvo = ["caminho", "de"].iter().filter_map(|k| a[*k].as_str()).map(expandir).find(|p| p.parent() == Some(pasta.as_path()));
                let Some(alvo) = alvo else { return };
                if self.trabalho.borrow().is_none() {
                    // the agent came here by itself (a request in the side bar): its size is the folder's
                    let total = self.itens.borrow().iter().filter(|i| !i.pasta).count();
                    *self.trabalho.borrow_mut() = Some(Trabalho { total: total.max(1), tocados: BTreeSet::new(), atual: None });
                }
                if let Some(t) = self.trabalho.borrow_mut().as_mut() {
                    t.tocados.insert(alvo.clone());
                    t.total = t.total.max(t.tocados.len());
                    t.atual = Some(alvo);
                }
                self.trabalho_render();
                self.render();
            }
            _ => {}
        }
    }

    fn lixeira(self: &Rc<Self>, sel: &[PathBuf]) {
        if sel.is_empty() {
            return;
        }
        let mut feitos = Vec::new();
        for p in sel {
            match para_lixeira(p) {
                Ok(t) => feitos.push((p.clone(), t)),
                Err(e) => eprintln!("kalango-shell: lixeira {}: {e}", p.display()),
            }
        }
        if depurando() {
            eprintln!("debug arquivos: lixeira {}", feitos.len());
        }
        *self.ultimo_lixo.borrow_mut() = feitos;
        self.selecionados.borrow_mut().clear();
        self.recarregar();
    }

    /// Ctrl+Z: what went to the trash from here last comes back.
    fn desfazer_lixeira(self: &Rc<Self>) {
        let lixo = std::mem::take(&mut *self.ultimo_lixo.borrow_mut());
        let mut n = 0;
        for (_, t) in lixo.iter().filter(|(_, t)| !t.as_os_str().is_empty()) {
            if restaurar(t).is_ok() {
                n += 1;
            }
        }
        if depurando() {
            eprintln!("debug arquivos: restaurados {n}");
        }
        self.recarregar();
    }

    fn apagar_de_vez(self: &Rc<Self>, sel: &[PathBuf]) {
        for p in sel {
            if let Err(e) = apagar_de_vez(p) {
                eprintln!("kalango-shell: apagar {}: {e}", p.display());
            }
        }
        self.selecionados.borrow_mut().clear();
        self.recarregar();
    }

    /// F2: the name in a popover over the file's card, the part before the extension selected.
    fn renomear(self: &Rc<Self>) {
        let sel: Vec<PathBuf> = self.selecionados.borrow().iter().cloned().collect();
        let [p] = sel.as_slice() else { return };
        let ancora: gtk::Widget = self.cartoes.borrow().iter().find(|(q, _, _)| q == p).map(|(_, c, _)| c.clone()).unwrap_or_else(|| self.migalha.clone().upcast());
        let pop = gtk::Popover::new();
        pop.add_css_class("arq-menu");
        pop.set_parent(&ancora);
        let e = gtk::Entry::new();
        e.add_css_class("arq-renomear");
        e.set_text(&p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
        e.set_width_chars(28);
        pop.set_child(Some(&e));
        let (w, p2, pw) = (Rc::downgrade(self), p.clone(), pop.downgrade());
        e.connect_activate(move |e| {
            let novo = e.text().trim().to_string();
            if let (Some(a), false) = (w.upgrade(), novo.is_empty() || novo.contains('/')) {
                let destino = p2.with_file_name(&novo);
                if destino != p2 && !destino.exists() && std::fs::rename(&p2, &destino).is_ok() {
                    *a.selecionados.borrow_mut() = BTreeSet::from([destino]);
                    if depurando() {
                        eprintln!("debug arquivos: renomeado {novo}");
                    }
                    a.recarregar();
                }
            }
            if let Some(p) = pw.upgrade() {
                p.popdown();
            }
        });
        pop.connect_closed(|p| {
            let p = p.clone();
            glib::idle_add_local_once(move || p.unparent());
        });
        pop.popup();
        let texto = e.text();
        let fim = texto.rfind('.').filter(|i| *i > 0).map(|i| texto[..i].chars().count() as i32).unwrap_or(-1);
        e.grab_focus();
        e.select_region(0, fim);
    }

    /// A menu item as in the design: the glyph's left edge 12 px in, the text 10 px after the glyph's own width (a
    /// narrow glyph brings its text closer), the shortcut at the right.
    fn item_menu(c: &gtk::Box, glifo: &str, cor: &str, t: &str, atalho: &str, classe: &str) -> gtk::Button {
        let b = gtk::Button::new();
        b.add_css_class("arq-menu-item");
        if !classe.is_empty() {
            b.add_css_class(classe);
        }
        let l = caixa(Orientation::Horizontal, 0, "");
        let (img, sobra) = glifo_alinhado(glifo, cor);
        img.set_margin_start(12 - sobra);
        l.append(&img);
        let a = rotulo(t, "");
        a.set_hexpand(true);
        a.set_xalign(0.0);
        a.set_margin_start(10 - sobra);
        l.append(&a);
        if !atalho.is_empty() {
            l.append(&rotulo(atalho, "arq-menu-atalho"));
        }
        b.set_child(Some(&l));
        c.append(&b);
        b
    }

    fn popover_em(w: &gtk::Widget, x: f64, y: f64) -> gtk::Popover {
        let pop = gtk::Popover::new();
        pop.set_parent(w);
        pop.set_has_arrow(false);
        pop.add_css_class("arq-menu");
        pop.set_position(gtk::PositionType::Bottom);
        // its corner at the pointer, as a context menu opens
        pop.set_halign(Align::Start);
        pop.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        pop.connect_closed(|p| {
            let p = p.clone();
            glib::idle_add_local_once(move || p.unparent());
        });
        pop
    }

    fn organizar(self: &Rc<Self>) {
        let p = self.pasta.borrow().display().to_string();
        let total = self.itens.borrow().iter().filter(|i| !i.pasta).count();
        self.pedir_ao_agente(&trf("Organise the folder {folder}: list it, then move each file into a subfolder by kind (for example PDFs, Images, Installers, Documents), keeping the names.", &[("folder", p)]), total);
    }

    /// A tag: in the side bar a row (a dot and its name) that filters and takes dropped files; elsewhere a chip.
    fn chip_etiqueta(self: &Rc<Self>, nome: &str, lateral: bool) -> gtk::Widget {
        let cor = etiquetas_conhecidas().into_iter().find(|e| e.nome == nome).map(|e| e.cor).unwrap_or_else(|| "#8E8F99".into());
        if !lateral {
            return chip(nome, tom_de(&cor)).upcast();
        }
        let b = gtk::ToggleButton::new();
        b.add_css_class("arq-etiqueta-linha");
        let c = caixa(Orientation::Horizontal, 10, "");
        let ponto = caixa(Orientation::Horizontal, 0, "arq-ponto8");
        ponto.add_css_class(&format!("tom-{}", tom_de(&cor)));
        ponto.set_size_request(8, 8);
        ponto.set_valign(Align::Center);
        let lugar = centro(16, 16, &ponto);
        c.append(&lugar);
        let n = rotulo(nome, "arq-lugar-nome");
        n.set_xalign(0.0);
        c.append(&n);
        b.set_child(Some(&c));
        b.set_active(self.etiqueta_filtro.borrow().as_deref() == Some(nome));
        b.set_tooltip_text(Some(tr("Click to see the files with this tag; drop files here to tag them.")));
        let (w, n) = (Rc::downgrade(self), nome.to_string());
        b.connect_toggled(move |b| {
            if let Some(a) = w.upgrade() {
                *a.etiqueta_filtro.borrow_mut() = b.is_active().then(|| n.clone());
                let w2 = Rc::downgrade(&a);
                glib::idle_add_local_once(move || if let Some(a) = w2.upgrade() { a.lateral_render(); a.render(); });
            }
        });
        // files dropped here get the tag (all of them already have it: it comes off)
        let alvo = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY | gdk::DragAction::MOVE);
        let (w, n) = (Rc::downgrade(self), nome.to_string());
        alvo.connect_drop(move |_, v, _, _| {
            let (Some(a), Ok(fl)) = (w.upgrade(), v.get::<gdk::FileList>()) else { return false };
            let ps: Vec<PathBuf> = fl.files().iter().filter_map(|f| f.path()).collect();
            a.etiquetar(&ps, &n);
            true
        });
        b.add_controller(alvo);
        crate::central::posicao_debug(&format!("etiqueta {nome}"), b.upcast_ref());
        b.upcast()
    }

    /// Puts the tag on the files (or takes it off, when all of them already have it).
    fn etiquetar(self: &Rc<Self>, ps: &[PathBuf], tag: &str) {
        let todas = ps.iter().all(|p| etiquetas_de(p).iter().any(|t| t == tag));
        let mut n = 0;
        for p in ps {
            let mut v = etiquetas_de(p);
            if todas {
                v.retain(|t| t != tag);
            } else if !v.iter().any(|t| t == tag) {
                v.push(tag.to_string());
            }
            match gravar_etiquetas(p, &v) {
                Ok(()) => n += 1,
                Err(e) => eprintln!("kalango-shell: etiqueta em {}: {e}", p.display()),
            }
        }
        if depurando() {
            eprintln!("debug arquivos: etiqueta {tag} {} {n}", if todas { "tirada de" } else { "posta em" });
        }
        self.recarregar();
    }

    /// A place of the side bar: its glyph, its name, its count (the open one's, and the trash's).
    fn lugar(self: &Rc<Self>, nome: &str, p: &Path, glifo: &str, conta: Option<usize>) -> gtk::Button {
        let ativo = *p == *self.pasta.borrow();
        let b = gtk::Button::new();
        b.add_css_class("arq-lugar");
        if ativo {
            b.add_css_class("ativo");
        }
        let c = caixa(Orientation::Horizontal, 10, "");
        c.append(&ui_icon(glifo, if ativo { "#A5A1E2" } else { "#8E8F99" }, 16));
        let n = rotulo(nome, "arq-lugar-nome");
        n.set_hexpand(true);
        n.set_xalign(0.0);
        n.set_ellipsize(gtk::pango::EllipsizeMode::End);
        n.set_max_width_chars(1);
        c.append(&n);
        if let Some(k) = conta.filter(|k| *k > 0) {
            c.append(&rotulo(&k.to_string(), "arq-lugar-conta"));
        }
        b.set_child(Some(&c));
        self.ligar_lugar(&b, p);
        crate::central::posicao_debug(&format!("lugar {nome}"), b.upcast_ref());
        b
    }

    /// The compact layout's place: the glyph alone (its name in the tooltip).
    fn lugar_trilho(self: &Rc<Self>, nome: &str, p: &Path, glifo: &str) -> gtk::Button {
        let ativo = *p == *self.pasta.borrow();
        let b = botao_icone(glifo, if ativo { "#A5A1E2" } else { "#8E8F99" }, 16, "arq-trilho-lugar", nome);
        b.set_halign(Align::Center);
        if ativo {
            b.add_css_class("ativo");
        }
        self.ligar_lugar(&b, p);
        b
    }

    /// A place opens its folder and takes files dropped on it (the trash: to the trash).
    fn ligar_lugar(self: &Rc<Self>, b: &gtk::Button, p: &Path) {
        let (w, p2) = (Rc::downgrade(self), p.to_path_buf());
        b.connect_clicked(move |_| {
            if let Some(a) = w.upgrade() {
                if na_lixeira(&p2) {
                    let _ = std::fs::create_dir_all(&p2);
                    let _ = std::fs::create_dir_all(lixeira().join("info"));
                }
                a.navegar(&p2);
            }
        });
        if na_lixeira(p) {
            let alvo = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::MOVE | gdk::DragAction::COPY);
            let w = Rc::downgrade(self);
            alvo.connect_drop(move |_, v, _, _| {
                let (Some(a), Ok(fl)) = (w.upgrade(), v.get::<gdk::FileList>()) else { return false };
                let ps: Vec<PathBuf> = fl.files().iter().filter_map(|f| f.path()).collect();
                a.lixeira(&ps);
                true
            });
            b.add_controller(alvo);
        } else {
            self.alvo_de_arquivos(b, p.to_path_buf());
        }
    }

    /// A disk: its name, how full it is (the bar coral from 90 %), a click opens it.
    fn disco(self: &Rc<Self>, nome: &str, f: &gio::File) -> Option<gtk::Widget> {
        let i = f.query_filesystem_info("filesystem::size,filesystem::free", None::<&gio::Cancellable>).ok()?;
        let (t, l) = (i.attribute_uint64("filesystem::size"), i.attribute_uint64("filesystem::free"));
        if t == 0 {
            return None;
        }
        let b = gtk::Button::new();
        b.add_css_class("arq-disco");
        let c = caixa(Orientation::Horizontal, 10, "");
        c.append(&ui_icon("disco", "#8E8F99", 16));
        let col = caixa(Orientation::Vertical, 4, "");
        col.set_hexpand(true);
        col.set_valign(Align::Center);
        let topo = caixa(Orientation::Horizontal, 6, "");
        let n = rotulo(nome, "arq-disco-nome");
        n.set_hexpand(true);
        n.set_xalign(0.0);
        n.set_ellipsize(gtk::pango::EllipsizeMode::End);
        n.set_max_width_chars(1);
        topo.append(&n);
        topo.append(&rotulo(&de_total(t - l, t), "arq-disco-uso"));
        col.append(&topo);
        let fracao = (t - l) as f64 / t as f64;
        let lb = gtk::LevelBar::for_interval(0.0, 1.0);
        lb.add_css_class("arq-disco-barra");
        if fracao >= 0.9 {
            lb.add_css_class("cheio");
        }
        lb.set_value(fracao);
        col.append(&lb);
        c.append(&col);
        b.set_child(Some(&c));
        if let Some(p) = f.path() {
            let w = Rc::downgrade(self);
            b.connect_clicked(move |_| if let Some(a) = w.upgrade() { a.navegar(&p) });
        }
        Some(b.upcast())
    }

    /// The side bar (design "Arquivos · nova"): FAVORITOS (the person's folders and GTK's bookmarks), LOCAIS (the disks,
    /// the network's shares when there are some), TAGS, and the trash at the bottom; compact, the rail of glyphs.
    fn lateral_render(self: &Rc<Self>) {
        for b in [&self.lugares, &self.locais, &self.etiquetas_box, &self.lixeira_box, &self.trilho_lugares, &self.trilho_lixeira] {
            limpar(b);
        }
        let home = glib::home_dir();
        let atual = self.pasta.borrow().clone();
        let mut fixos: Vec<(String, PathBuf, &str)> = vec![(tr("Home folder").to_string(), home.clone(), "casa")];
        for (d, glifo) in [(glib::UserDirectory::Desktop, "monitor"), (glib::UserDirectory::Downloads, "download"), (glib::UserDirectory::Documents, "doc"),
            (glib::UserDirectory::Pictures, "imagem"), (glib::UserDirectory::Music, "musica"), (glib::UserDirectory::Videos, "video-d")] {
            if let Some(p) = glib::user_special_dir(d).filter(|p| p.is_dir() && *p != home) {
                fixos.push((p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), p, glifo));
            }
        }
        // the person's bookmarks (GTK's), the ones not already there
        let marcadores = std::fs::read_to_string(glib::user_config_dir().join("gtk-3.0/bookmarks")).unwrap_or_default();
        for l in marcadores.lines() {
            let (uri, nome) = l.split_once(' ').map(|(u, n)| (u, Some(n))).unwrap_or((l, None));
            if let Some(p) = gio::File::for_uri(uri).path().filter(|p| p.is_dir()) {
                if fixos.iter().any(|(_, q, _)| *q == p) {
                    continue;
                }
                let n = nome.map(str::to_string).unwrap_or_else(|| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                let codigo = p.join(".git").exists() || std::fs::read_dir(&p).into_iter().flatten().flatten().take(40).any(|e| e.path().join(".git").exists());
                fixos.push((n, p, if codigo { "codigo" } else { "pasta" }));
            }
        }
        for (nome, p, glifo) in fixos.into_iter().take(12) {
            let conta = (p == atual).then(|| self.itens.borrow().len());
            self.lugares.append(&self.lugar(&nome, &p, glifo, conta));
            self.trilho_lugares.append(&self.lugar_trilho(&nome, &p, glifo));
        }
        // the disks: the system's and what is mounted
        let mut discos: Vec<(String, gio::File)> = vec![(tr("System").to_string(), gio::File::for_path("/"))];
        for m in gio::VolumeMonitor::get().mounts() {
            discos.push((m.name().to_string(), m.root()));
        }
        for (nome, f) in discos {
            if let Some(d) = self.disco(&nome, &f) {
                self.locais.append(&d);
            }
        }
        // the network's shares (GVfs) when there are some
        let rede = glib::user_runtime_dir().join("gvfs");
        if std::fs::read_dir(&rede).is_ok_and(|mut d| d.next().is_some()) {
            self.locais.append(&self.lugar(tr("Network"), &rede, "globo", None));
        }
        // the tags: the person's, and any other found on the files here
        let mut nomes: Vec<String> = etiquetas_conhecidas().into_iter().map(|e| e.nome).collect();
        for i in self.itens.borrow().iter() {
            for e in &i.etiquetas {
                if !nomes.contains(e) {
                    nomes.push(e.clone());
                }
            }
        }
        for n in nomes {
            self.etiquetas_box.append(&self.chip_etiqueta(&n, true));
        }
        // the trash, at the bottom, with how many files are in it
        let lx = lixeira().join("files");
        let n = std::fs::read_dir(&lx).map(|d| d.count()).unwrap_or(0);
        self.lixeira_box.append(&self.lugar(tr("Trash"), &lx, "lixeira", Some(n)));
        self.trilho_lixeira.append(&self.lugar_trilho(tr("Trash"), &lx, "lixeira"));
    }
}

/// A menu's row in the system's style (Files' menus), for other windows' menus.
pub(crate) fn item_menu_publico(c: &gtk::Box, glifo: &str, cor: &str, t: &str) -> gtk::Button {
    Arquivos::item_menu(c, glifo, cor, t, "", "")
}

/// The same row with its shortcut at the right and a class ("perigo", "agente").
pub(crate) fn item_menu_completo(c: &gtk::Box, glifo: &str, cor: &str, t: &str, atalho: &str, classe: &str) -> gtk::Button {
    Arquivos::item_menu(c, glifo, cor, t, atalho, classe)
}

/// The window's own preferences (hidden files shown…), in ~/.config/kalango/arquivos.json.
fn preferencias() -> Value {
    std::fs::read(glib::user_config_dir().join("kalango/arquivos.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_else(|| serde_json::json!({}))
}

fn salvar_preferencia(chave: &str, v: Value) {
    let mut p = preferencias();
    p[chave] = v;
    let d = glib::user_config_dir().join("kalango");
    let _ = std::fs::create_dir_all(&d);
    let _ = std::fs::write(d.join("arquivos.json"), serde_json::to_string_pretty(&p).unwrap_or_default());
}

/// A PDF's pages (its page objects; not the page tree's nodes).
fn contar_paginas(p: &Path) -> Option<usize> {
    let b = std::fs::read(p).ok()?;
    let s = String::from_utf8_lossy(&b);
    let mut n = 0;
    let mut resto = s.as_ref();
    while let Some(i) = resto.find("/Type") {
        resto = &resto[i + 5..];
        let t = resto.trim_start();
        if t.starts_with("/Page") && !t.starts_with("/Pages") {
            n += 1;
        }
    }
    (n > 0).then_some(n)
}

/// A glyph and how much empty room its box has on each side (the design places glyphs by their drawn edge).
fn glifo_alinhado(glifo: &str, cor: &str) -> (gtk::Image, i32) {
    if glifo == "nenhum" {
        let i = gtk::Image::new();
        i.set_pixel_size(16);
        i.set_size_request(16, 16);
        return (i, 2);
    }
    if glifo == "spark" {
        return (estrela(13, cor), 0);
    }
    // the drawn width of each glyph in its 16 px box (they are centred in it)
    let w: f32 = match glifo {
        "pasta" | "imagem" => 13.0,
        "compartilhar" | "codigo" | "casa" | "download" | "grade" | "lista" | "enviar" => 12.0,
        "lixeira" | "mais" => 11.0,
        "doc" | "desfazer" => 9.0,
        "seta-esquerda" | "seta-direita" => 4.5,
        _ => 12.0,
    };
    (ui_icon(glifo, cor, 16), ((16.0 - w) / 2.0).round() as i32)
}

/// Dropped files move when they stay on the same disk and copy to another one, as file managers do; Ctrl copies.
fn mover_ao_soltar(t: &gtk::DropTarget, ps: &[PathBuf], destino: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    if t.current_event_state().contains(gdk::ModifierType::CONTROL_MASK) {
        return false;
    }
    let disco = |p: &Path| std::fs::metadata(p).ok().map(|m| m.dev());
    let d = disco(destino);
    d.is_some() && ps.iter().all(|p| disco(p) == d)
}

/// A chip with a colored dot (tags, "Duplicado").
fn chip(t: &str, tom: &str) -> gtk::Box {
    let c = caixa(Orientation::Horizontal, 5, "arq-etiqueta");
    let ponto = caixa(Orientation::Horizontal, 0, "arq-ponto");
    ponto.add_css_class(&format!("tom-{tom}"));
    ponto.set_size_request(6, 6);
    ponto.set_valign(Align::Center);
    c.append(&ponto);
    c.append(&rotulo(t, ""));
    c
}

/// The agent's cursor with its name, over what it touches.
fn cursor_do_agente() -> gtk::Widget {
    let f = gtk::Fixed::new();
    f.put(&arte("cursor-agente", 16), 0.0, 0.0);
    let r = rotulo(tr("Agent"), "arq-cursor-rotulo");
    f.put(&r, 18.0, 25.0);
    f.set_halign(Align::Center);
    f.set_valign(Align::Start);
    f.set_margin_top(18);
    f.set_can_target(false);
    f.upcast()
}

/// Where the agent's summaries are kept (by path and modification time: a changed file is summarised again).
fn resumo_path(i: &Item) -> PathBuf {
    let k = glib::compute_checksum_for_string(glib::ChecksumType::Sha256, format!("{}:{}", i.caminho.display(), i.modificado)).map(|s| s.to_string()).unwrap_or_default();
    glib::user_cache_dir().join("kalango/resumos").join(format!("{}.txt", &k[..k.len().min(32)]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicates_by_content_point_at_the_oldest() {
        let d = std::env::temp_dir().join(format!("kalango-arq-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        std::fs::write(d.join("fatura.pdf"), b"conteudo da fatura").unwrap();
        std::thread::sleep(Duration::from_millis(1100));
        std::fs::write(d.join("fatura-copia.pdf"), b"conteudo da fatura").unwrap();
        std::fs::write(d.join("outra.pdf"), b"conteudo de outra!").unwrap();
        let itens = ler_pasta(&d, false);
        let dups = duplicados(&itens);
        assert_eq!(dups, BTreeMap::from([(d.join("fatura-copia.pdf"), d.join("fatura.pdf"))]), "same size, other content is not a duplicate");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn sizes_as_the_design_writes_them() {
        assert_eq!(tamanho(189_000), "189 KB");
        assert_eq!(tamanho(2_400_000), "2,4 MB");
        assert_eq!(tamanho(88_000_000), "88 MB");
        assert_eq!(tamanho(4_000), "4 KB");
        assert_eq!(tamanho(1_000_000_000_000), "1 TB");
        assert_eq!(tamanho(999), "999 B");
        assert_eq!(de_total(1_600_000_000_000, 2_000_000_000_000), trf("{used} of {total}", &[("used", "1,6".into()), ("total", "2 TB".into())]));
        assert_eq!(de_total(412_000_000_000, 1_000_000_000_000), trf("{used} of {total}", &[("used", "412 GB".into()), ("total", "1 TB".into())]));
    }

    /// A one-page PDF with a line of text (for the thumbnailer).
    fn pdf_simples(p: &Path) {
        let conteudo = "BT /F1 24 Tf 60 770 Td (Fatura de outubro) Tj ET";
        let objs = ["<< /Type /Catalog /Pages 2 0 R >>".to_string(), "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".into(),
            format!("<< /Length {} >>\nstream\n{conteudo}\nendstream", conteudo.len()), "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into()];
        let mut out = String::from("%PDF-1.4\n");
        let mut offs = vec![];
        for (i, o) in objs.iter().enumerate() {
            offs.push(out.len());
            out += &format!("{} 0 obj\n{o}\nendobj\n", i + 1);
        }
        let xref = out.len();
        out += &format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1);
        for o in offs {
            out += &format!("{o:010} 00000 n \n");
        }
        out += &format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1);
        std::fs::write(p, out).unwrap();
    }

    #[test]
    fn previews_of_pictures_and_pages() {
        let d = std::env::temp_dir().join(format!("kalango-arq-m-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        let png = d.join("foto.png");
        let pb = gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, true, 8, 400, 300).unwrap();
        pb.fill(0x336699ff);
        pb.savev(&png, "png", &[]).unwrap();
        assert_eq!(tipo_de(&png), "image/png");
        let (_, w, h, _, _) = miniatura(&png, "image/png", 0, Alvo::Foto(161, 84)).expect("a picture's preview");
        assert_eq!((w, h), (322, 168), "cut to the box's proportion, twice its size");
        let pdf = d.join("fatura.pdf");
        pdf_simples(&pdf);
        assert_eq!(tipo_de(&pdf), "application/pdf");
        if thumbnailer_para("application/pdf").is_some() {
            // the shared cache of this test (never the person's)
            std::env::set_var("XDG_CACHE_HOME", d.join("cache"));
            let mt = std::fs::metadata(&pdf).unwrap().modified().map(|t| segundos(Ok(t))).unwrap();
            let (_, w, h, _, _) = miniatura(&pdf, "application/pdf", mt, Alvo::Pagina(75, 74)).expect("a page's preview");
            assert_eq!((w, h), (75, 74), "the page, this wide, cut from the top");
        }
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn tags_live_in_the_file() {
        let d = std::env::temp_dir().join(format!("kalango-arq-t-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        let f = d.join("nota.md");
        std::fs::write(&f, b"x").unwrap();
        if gravar_etiquetas(&f, &["Trabalho".into(), "Ideias".into()]).is_ok() {
            assert_eq!(etiquetas_de(&f), vec!["Trabalho".to_string(), "Ideias".to_string()]);
            gravar_etiquetas(&f, &[]).unwrap();
            assert!(etiquetas_de(&f).is_empty());
        }
        let _ = std::fs::remove_dir_all(d);
    }
}
