//! Configurações (designs "Configurações · Início" and the section pages): KalangoOS's settings — a window of the
//! system (app id os.kalango.Configuracoes), opened by the dock, by `kalango-shell configuracoes [SECTION]` and from
//! the quick controls. The rail of sections on the left; each section a page.
//!
//! Início shows the computer at a glance, each card live and acting on the system itself: Wi-Fi and Bluetooth
//! (NetworkManager, BlueZ), the sound's output and volume (PipeWire), the screen's brightness and the night light
//! (on its schedule, 20h–6h), the power profile (power-profiles-daemon), the disk and what fills it, the agents at
//! work and today's tokens, the system's updates (PackageKit), and one suggestion of the agent drawn from how the
//! computer is used.

use crate::configuracoes_aparencia::Aparencia;
use crate::configuracoes_rede::Rede;
use crate::configuracoes_bluetooth::Bluetooth;
use crate::configuracoes_tela::Tela;
use crate::configuracoes_som::Som;
use crate::configuracoes_energia::PowerPage;
use crate::configuracoes_notificacoes::NotificationsPage;
use crate::conversa::{ui_icon, ui_icon_fitted};
use crate::sistema;
use crate::superficies::{caixa, estrela, rotulo};
use crate::Shell;
use gtk::prelude::*;
use gtk::{gdk, gio, glib, Align, Orientation};
use kalango_i18n::{n_, tr, trf};
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::collections::HashMap;
use std::time::Duration;

extern "C" {
    fn gdk_wayland_toplevel_set_application_id(toplevel: *mut gdk::ffi::GdkToplevel, id: *const std::ffi::c_char);
}

/// The sections of the rail: (id, glyph, name).
pub const SECOES: &[(&str, &str, &str)] = &[
    ("inicio", "casa", n_("Home")),
    ("rede", "wifi", n_("Network")),
    ("bluetooth", "bluetooth", n_("Bluetooth")),
    ("tela", "monitor", n_("Display")),
    ("som", "volume", n_("Sound")),
    ("energia", "desligar-d", n_("Power")),
    ("notificacoes", "sino-d", n_("Notifications")),
    ("entrada", "teclado-d", n_("Keyboard and mouse")),
    ("aparencia", "paleta", n_("Appearance")),
    ("agentes", "spark", n_("Agents and AI")),
    ("privacidade", "cadeado-d", n_("Privacy and security")),
    ("acessibilidade", "olho", n_("Accessibility")),
    ("sistema", "info-d", n_("System")),
];

/// A desktop panel's name ("wifi", "display"…, as GNOME's) → the section here.
/// A page's name as shown ("tela" → "Tela"), when it is one.
pub fn nome_da_pagina(id: &str) -> Option<&'static str> {
    SECOES.iter().find(|(p, _, _)| *p == id).map(|(_, _, n)| tr(n))
}

pub fn secao_do_painel(painel: &str) -> &'static str {
    match painel {
        "wifi" | "network" | "rede" => "rede",
        "bluetooth" => "bluetooth",
        "display" | "tela" => "tela",
        "sound" | "som" => "som",
        "power" | "energia" => "energia",
        "notifications" | "notificacoes" => "notificacoes",
        "keyboard" | "mouse" | "entrada" => "entrada",
        "background" | "appearance" | "aparencia" => "aparencia",
        "agentes" => "agentes",
        "privacy" | "privacidade" => "privacidade",
        "universal-access" | "acessibilidade" => "acessibilidade",
        "system" | "info-overview" | "datetime" | "default-apps" | "printers" | "sistema" | "storage" | "armazenamento" | "users" | "usuarios" => "sistema",
        _ => "inicio",
    }
}

/// Opens the settings on a section (or a desktop panel's name).
pub fn abrir_painel(painel: &str) {
    if let Some(s) = crate::shell_atual() {
        s.abrir_configuracoes(Some(secao_do_painel(painel)));
        // a System tab by its panel's name
        let aba = match painel {
            "storage" | "armazenamento" => Some("armazenamento"),
            "datetime" => Some("data"),
            "default-apps" => Some("apps"),
            "printers" => Some("impressoras"),
            "users" | "usuarios" => Some("usuarios"),
            _ => None,
        };
        let c = s.configuracoes.borrow().clone();
        if let (Some(a), Some(c)) = (aba, c) {
            if let Some(p) = c.sistema.borrow().as_ref() {
                p.frame.aba(a);
            }
        }
    }
}

pub(crate) fn depurando() -> bool {
    std::env::var_os("KALANGO_SHELL_DEBUG").is_some()
}

// ---------------------------------------------------------------- the settings kept by the shell

fn ajustes_path() -> PathBuf {
    glib::user_config_dir().join("kalango/configuracoes.json")
}

pub(crate) fn ajustes() -> Value {
    std::fs::read(ajustes_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_else(|| json!({}))
}

pub(crate) fn salvar_ajuste(chave: &str, v: Value) {
    let mut a = ajustes();
    a[chave] = v;
    let p = ajustes_path();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(&p, serde_json::to_string_pretty(&a).unwrap_or_default());
}

/// The night light's schedule: on from 20h to 6h, when the person asked for it.
fn luz_noturna_auto() -> bool {
    ajustes()["luz_noturna_auto"].as_bool().unwrap_or(false)
}

fn na_janela_noturna(h: i32) -> bool {
    !(6..20).contains(&h)
}

/// Called every minute by the shell: the scheduled night light turns on at 20h and off at 6h.
pub fn vigiar_luz_noturna(ligada_por_agenda: &Cell<bool>) {
    if !luz_noturna_auto() || sistema::night_light_unavailable() {
        return;
    }
    let h = glib::DateTime::now_local().map(|d| d.hour()).unwrap_or(12);
    let deve = na_janela_noturna(h);
    if deve != ligada_por_agenda.get() {
        ligada_por_agenda.set(deve);
        std::thread::spawn(move || sistema::luz_noturna(deve));
    }
}

/// "Instalar à noite": from the next 2h, for three hours, when the screen is locked and no agent works, the computer
/// restarts into the prepared update (PackageKit installs it on the way). A night that passes without that moves
/// to the next one; shutting down before installs it too (the power menu's own choice).
fn proxima_madrugada(agora: i64) -> i64 {
    let Ok(d) = glib::DateTime::from_unix_local(agora) else { return agora };
    let hoje = glib::DateTime::from_local(d.year(), d.month(), d.day_of_month(), 2, 0, 0.0).map(|h| h.to_unix()).unwrap_or(agora);
    if agora < hoje + 3 * 3600 { hoje } else { hoje + 86_400 }
}

/// Checked every minute and when the screen locks.
pub fn vigiar_atualizacao_noturna(shell: &Rc<Shell>) {
    let Some(desde) = ajustes()["instalar_a_partir"].as_i64() else { return };
    let agora = glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0);
    if agora < desde {
        return;
    }
    if agora >= desde + 3 * 3600 {
        // that night went by: the next one
        salvar_ajuste("instalar_a_partir", json!(proxima_madrugada(agora)));
        return;
    }
    if !shell.bloqueio().bloqueada() || shell.estado.agent_task.borrow().is_some() {
        return;
    }
    crate::energia::prepared_update(|n| {
        salvar_ajuste("instalar_a_partir", Value::Null);
        if n > 0 {
            if depurando() {
                eprintln!("debug configuracoes: instalando a atualização à noite ({n} pacotes)");
            }
            // nobody is there: an app that waits for a person (unsaved work) puts it off to the next night
            crate::shutdown_screen::start_unattended("reiniciar", true, || {
                let amanha = glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0) + 20 * 3600;
                salvar_ajuste("instalar_a_partir", Value::from(amanha));
            });
        }
    });
}

// ---------------------------------------------------------------- what the home page reads (off the interface thread)

#[derive(Default, Clone)]
struct Wifi {
    sinal: u8,
    taxa: String,
    perto: usize,
}

#[derive(Default, Clone)]
struct Leitura {
    s: sistema::Sistema,
    host: String,
    /// the compositor and its version ("niri 26.04")
    compositor: String,
    /// the graphics card: its name and performance state (P0 full power … P8 at rest), from nvidia-smi
    gpu: Option<(String, u8)>,
    /// what fills the disk (apps, docs, media), once measured
    categorias: Option<(u64, u64, u64)>,
    wifi: Wifi,
    saida: String,
    disco: (u64, u64),
    /// approvals kalangod waits for (who asks)
    pendentes: Vec<Value>,
}

fn sh(prog: &str, args: &[&str]) -> Option<String> {
    let prog = if prog == "nmcli" { crate::configuracoes_rede::nmcli() } else { prog.to_string() };
    let o = std::process::Command::new(prog).args(args).env("LC_ALL", "C").output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).into_owned())
}

fn ler() -> Leitura {
    let s = sistema::ler();
    let mut wifi = Wifi::default();
    if s.rede.wifi_ligado {
        // "auto": NetworkManager scans again when its list is older than 30 s (then this waits for the scan)
        if let Some(l) = sh("nmcli", &["-t", "-f", "IN-USE,SSID,SIGNAL,RATE", "device", "wifi", "list", "--rescan", "auto"]) {
            let mut nomes = std::collections::BTreeSet::new();
            for linha in l.lines() {
                let f: Vec<&str> = linha.split(':').collect();
                if f.len() < 4 {
                    continue;
                }
                if f[0] == "*" {
                    wifi.sinal = f[2].parse().unwrap_or(0);
                    wifi.taxa = f[3].replace("Mbit/s", "Mb/s").trim().to_string();
                } else if !f[1].is_empty() && Some(f[1]) != s.rede.conectada.as_deref() {
                    nomes.insert(f[1].to_string());
                }
            }
            wifi.perto = nomes.len();
        }
    }
    // the output's name (PipeWire's description of the default sink)
    let saida = sh("wpctl", &["inspect", "@DEFAULT_AUDIO_SINK@"]).and_then(|t| t.lines().find_map(|l| {
        let l = l.trim().trim_start_matches('*').trim();
        l.strip_prefix("node.description = ").map(|v| v.trim_matches('"').to_string())
    })).unwrap_or_default();
    let disco = gio::File::for_path("/").query_filesystem_info("filesystem::size,filesystem::free", None::<&gio::Cancellable>).ok()
        .map(|i| (i.attribute_uint64("filesystem::size"), i.attribute_uint64("filesystem::free"))).unwrap_or((0, 0));
    // the name the computer has now (hostnamectl changes it while the session runs)
    let host = std::fs::read_to_string("/proc/sys/kernel/hostname").map(|h| h.trim().to_string()).unwrap_or_default();
    let compositor = match crate::servicos::niri_versao() {
        Some(v) => format!("niri {v}"),
        None => std::env::var("XDG_CURRENT_DESKTOP").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "niri".into()),
    };
    let gpu = sh("nvidia-smi", &["--query-gpu=pstate,name", "--format=csv,noheader"]).and_then(|o| {
        let (p, n) = o.lines().next()?.split_once(", ")?;
        Some((n.trim().replace("NVIDIA GeForce ", "").replace("NVIDIA ", ""), p.trim().trim_start_matches('P').parse().ok()?))
    });
    let pendentes = crate::servicos::kalangod_pendentes().unwrap_or_default();
    Leitura { s, host, compositor, gpu, wifi, saida, disco, categorias: None, pendentes }
}

/// What fills the disk: apps (the system's programs, Flatpak, Snap), documents, media and the rest. Measured in the
/// background once a day (it walks big folders) and kept.
fn medir_pasta(p: &std::path::Path, limite: &mut u32) -> u64 {
    use std::os::unix::fs::MetadataExt;
    let Ok(md) = std::fs::symlink_metadata(p) else { return 0 };
    if md.file_type().is_symlink() {
        return 0;
    }
    if !md.is_dir() {
        return md.blocks() * 512;
    }
    if *limite == 0 {
        return 0;
    }
    *limite -= 1;
    let mut t = 0;
    for e in std::fs::read_dir(p).into_iter().flatten().flatten() {
        t += medir_pasta(&e.path(), limite);
    }
    t
}

fn categorias_cache() -> PathBuf {
    glib::user_cache_dir().join("kalango/armazenamento.json")
}

/// (apps, docs, media) in bytes: kept for a day.
fn categorias() -> (u64, u64, u64) {
    let agora = glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0);
    if let Some(v) = std::fs::read(categorias_cache()).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()) {
        if agora - v["t"].as_i64().unwrap_or(0) < 86_400 {
            return (v["apps"].as_u64().unwrap_or(0), v["docs"].as_u64().unwrap_or(0), v["midia"].as_u64().unwrap_or(0));
        }
    }
    let mut limite = 400_000u32;
    let pastas = |ps: &[PathBuf], limite: &mut u32| ps.iter().map(|p| medir_pasta(p, limite)).sum::<u64>();
    let especial = |d| glib::user_special_dir(d).filter(|p| p.is_dir() && *p != glib::home_dir());
    let apps = pastas(&[PathBuf::from("/usr"), PathBuf::from("/opt"), PathBuf::from("/var/lib/flatpak"), PathBuf::from("/var/lib/snapd/snaps"),
        glib::user_data_dir().join("flatpak")], &mut limite);
    let docs = pastas(&[glib::UserDirectory::Documents, glib::UserDirectory::Downloads].iter().filter_map(|d| especial(*d)).collect::<Vec<_>>(), &mut limite);
    let midia = pastas(&[glib::UserDirectory::Pictures, glib::UserDirectory::Music, glib::UserDirectory::Videos].iter().filter_map(|d| especial(*d)).collect::<Vec<_>>(), &mut limite);
    if let Some(d) = categorias_cache().parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(categorias_cache(), json!({"t": agora, "apps": apps, "docs": docs, "midia": midia}).to_string());
    (apps, docs, midia)
}

// ---------------------------------------------------------------- small pieces of the design

pub(crate) fn cartao(classe: &str) -> gtk::Box {
    let c = caixa(Orientation::Vertical, 10, "cfg-cartao");
    if !classe.is_empty() {
        c.add_css_class(classe);
    }
    c
}

/// A card's label: the glyph and the name in capitals.
pub(crate) fn rotulo_cartao(glifo: &str, t: &str) -> gtk::Box {
    let r = caixa(Orientation::Horizontal, 5, "");
    if glifo == "spark" {
        r.append(&estrela(13, "#A5A1E2"));
    } else {
        r.append(&ui_icon_fitted(glifo, "#8E8F99", 16));
    }
    r.append(&rotulo(t, "cfg-rotulo"));
    r
}

pub(crate) fn interruptor() -> gtk::Switch {
    let s = gtk::Switch::new();
    s.add_css_class("cfg-switch");
    s.set_valign(Align::Center);
    s
}

pub(crate) fn deslizador() -> gtk::Scale {
    let s = gtk::Scale::with_range(Orientation::Horizontal, 0.0, 1.0, 0.01);
    s.add_css_class("cfg-escala");
    s.set_draw_value(false);
    s.set_hexpand(true);
    s
}

pub(crate) fn ponto(cor: &str) -> gtk::Box {
    let p = caixa(Orientation::Horizontal, 0, "cfg-ponto");
    p.add_css_class(cor);
    p.set_size_request(6, 6);
    p.set_valign(Align::Center);
    p
}

/// "Excelente", "Boa", "Razoável", "Fraca" by the signal's strength.
fn qualidade(sinal: u8) -> &'static str {
    match sinal {
        75.. => tr("Excellent"),
        50..=74 => tr("Good"),
        25..=49 => tr("Fair"),
        _ => tr("Weak"),
    }
}

// ---------------------------------------------------------------- the window

pub struct Configuracoes {
    pub janela: gtk::ApplicationWindow,
    shell: Weak<Shell>,
    paginas: gtk::Stack,
    trilho: Vec<(&'static str, gtk::Button, gtk::Image, &'static str, gtk::Box)>,
    inicio: Rc<Inicio>,
    timer: RefCell<Option<glib::SourceId>>,
    trilho_box: gtk::Box,
    logo: gtk::WindowHandle,
    div: gtk::Box,
    /// the compact layout (design "Configurações · 1366×768"), for a window under 1400 × 700
    compacto: Cell<Option<bool>>,
    /// the pages' real size (maximized or tiled, the window's default size stays what it was)
    medida: Cell<(i32, i32)>,
    /// made smaller to fit a smaller screen (a scale): it grows back when the screen does
    encolhida: Cell<bool>,
    aparencia: RefCell<Option<Rc<Aparencia>>>,
    rede: RefCell<Option<Rc<Rede>>>,
    bluetooth: RefCell<Option<Rc<Bluetooth>>>,
    tela: RefCell<Option<Rc<Tela>>>,
    som: RefCell<Option<Rc<Som>>>,
    power: RefCell<Option<Rc<PowerPage>>>,
    notifications: RefCell<Option<Rc<NotificationsPage>>>,
    entrada: RefCell<Option<Rc<crate::configuracoes_entrada::InputPage>>>,
    acessibilidade: RefCell<Option<Rc<crate::configuracoes_acessibilidade::AccessibilityPage>>>,
    privacidade: RefCell<Option<Rc<crate::configuracoes_privacidade::PrivacyPage>>>,
    sistema: RefCell<Option<Rc<crate::configuracoes_sistema::SystemPage>>>,
    agentes: RefCell<Option<Rc<crate::configuracoes_agentes::AgentsAiPage>>>,
    /// the pages' controls whose place the session test reads
    posicoes_extra: RefCell<Vec<(String, glib::WeakRef<gtk::Widget>)>>,
    /// the size the window has from the design (each page its own height); a size the person chose is kept
    tamanho_design: Cell<(i32, i32)>,
}

/// For the session test: a control of a page, logged with the others.
pub(crate) fn registrar_posicao(c: &Rc<Configuracoes>, nome: &str, w: &gtk::Widget) {
    // a control made again (a list filled again) takes its name's place
    let mut v = c.posicoes_extra.borrow_mut();
    v.retain(|(n, _)| n != nome);
    v.push((nome.to_string(), w.downgrade()));
}

/// The room a window has on the main screen (its size less the bars and a margin). While the compositor changes a
/// monitor's scale GDK can report a size for a moment at an integer scale (1280×800 as 640×400): a reading smaller
/// than any desktop screen is not taken.
fn screen_room() -> Option<(i32, i32)> {
    // (niri's output as it is now: GDK's monitor keeps the old size after a scale change)
    crate::superficies::tamanho_da_saida()
        .or_else(|| crate::configuracoes_tela::monitor_principal().map(|m| (m.geometry().width(), m.geometry().height())))
        .filter(|(w, h)| *w >= 800 && *h >= 500)
        .map(|(w, h)| (w - 48, h - 116))
}

/// The design's window height for each page (the width is always 1520).
pub(crate) fn altura_da_pagina(id: &str) -> i32 {
    match id {
        "aparencia" | "rede" | "tela" | "som" | "energia" | "notificacoes" | "agentes" => 664,
        "bluetooth" => 671,
        _ => 710,
    }
}

/// The home page's live widgets.
struct Inicio {
    raiz: gtk::Box,
    topo: gtk::Box,
    titulo: gtk::Label,
    sub: gtk::Label,
    busca: gtk::Box,
    busca_e: gtk::Entry,
    /// "completo" (the cards with their controls) or "compacto" (one line each, a click opens the section)
    vistas: gtk::Stack,
    /// the compact cards: Wi-Fi, Bluetooth, sound, display, agents, updates — (button, value, line under it)
    resumo: Vec<(gtk::Button, gtk::Label, gtk::Label)>,
    /// updates ready to install (the compact card's click schedules them)
    prontas: Cell<usize>,
    fundo: gtk::DrawingArea,
    fundo_pb: Rc<RefCell<Option<gtk::gdk_pixbuf::Pixbuf>>>,
    host: gtk::Label,
    os: gtk::Label,
    estado: gtk::Label,
    estado_ponto: gtk::Box,
    wifi_sw: gtk::Switch,
    wifi_nome: gtk::Label,
    wifi_barras: Vec<gtk::Box>,
    wifi_info: gtk::Label,
    wifi_perto: gtk::Button,
    bt_sw: gtk::Switch,
    bt_lista: gtk::Box,
    som_nome: gtk::Label,
    som: gtk::Scale,
    brilho: gtk::Scale,
    noturna_sw: gtk::Switch,
    noturna_txt: gtk::Label,
    perfis: Vec<(&'static str, gtk::ToggleButton)>,
    energia_info: gtk::Label,
    disco_txt: gtk::Label,
    disco_barra: gtk::DrawingArea,
    /// the bar's parts: (share of the disk, colour)
    disco_partes: Rc<RefCell<Vec<(f64, &'static str)>>>,
    agentes: gtk::Box,
    tokens: gtk::Label,
    tokens_sub: gtk::Label,
    atual_titulo: gtk::Label,
    atual_sub: gtk::Label,
    atual_botao: gtk::Button,
    sugestao: gtk::Box,
    sugestao_txt: gtk::Label,
    /// changing a control from what was read: the reading must not fight it back
    lendo: Cell<bool>,
    shell: RefCell<Weak<Shell>>,
    /// a reading is under way (a Wi-Fi scan can take a few seconds): the next one waits for it
    ocupado: Rc<Cell<bool>>,
    /// when the slow readings last ran (seconds, monotonic)
    lento: Cell<Option<i64>>,
    ultimo: RefCell<Option<Leitura>>,
}

impl Configuracoes {
    pub fn new(app: &gtk::Application, shell: &Rc<Shell>) -> Rc<Configuracoes> {
        let janela = gtk::ApplicationWindow::new(app);
        janela.set_title(Some(tr("Settings")));
        // (for the session test: where the keyboard's focus goes — the arrows move it between the page's controls)
        if depurando() {
            janela.connect_focus_widget_notify(|j| {
                if let Some(w) = gtk::prelude::GtkWindowExt::focus(j) {
                    let nome = w.downcast_ref::<gtk::Button>().and_then(|b| b.label()).map(|l| l.to_string())
                        .or_else(|| Some(w.widget_name().to_string()).filter(|n| !n.is_empty() && !n.starts_with("Gtk")))
                        .unwrap_or_else(|| w.type_().name().to_string());
                    eprintln!("debug configuracoes: foco em {nome}");
                }
            });
        }
        // (niri's output as it is now: GDK's monitor keeps an old size after a scale change)
        let (mw, mh) = crate::superficies::tamanho_da_saida()
            .or_else(|| gdk::Display::default().and_then(|d| d.monitors().item(0)).and_downcast::<gdk::Monitor>().map(|m| (m.geometry().width(), m.geometry().height())))
            .unwrap_or((1920, 1080));
        // the design's 1520 × 710 where it fits (with the top bar and the dock); on a smaller screen, the compact layout
        // as large as the screen allows (1318 × 652 on 1366 × 768)
        if mw >= 1568 && mh >= 826 {
            janela.set_default_size(1520, 710);
        } else {
            janela.set_default_size(mw - 48, mh - 116);
        }
        janela.add_css_class("cfg");
        let sem_barra = gtk::Box::new(Orientation::Horizontal, 0);
        sem_barra.set_visible(false);
        janela.set_titlebar(Some(&sem_barra));
        janela.connect_close_request(|j| {
            j.set_visible(false);
            glib::Propagation::Stop
        });
        let raiz = caixa(Orientation::Horizontal, 0, "");
        // the rail
        let trilho_box = caixa(Orientation::Vertical, 4, "cfg-trilho");
        // 64 in the design, whose window border is drawn over it: here the border takes one of them
        trilho_box.set_size_request(63, -1);
        let logo = caixa(Orientation::Horizontal, 0, "cfg-logo");
        logo.set_size_request(36, 36);
        logo.set_halign(Align::Center);
        let engrenagem = ui_icon("configuracoes", "#A5A1E2", 16);
        engrenagem.set_hexpand(true);
        logo.append(&engrenagem);
        let lh = gtk::WindowHandle::new();
        lh.set_child(Some(&logo));
        trilho_box.append(&lh);
        let div = caixa(Orientation::Horizontal, 0, "cfg-div");
        div.set_size_request(28, 1);
        div.set_halign(Align::Center);
        trilho_box.append(&div);
        let paginas = gtk::Stack::new();
        paginas.set_hexpand(true);
        // each page as wide as itself needs (the widest one would hold every page wider than a small screen)
        paginas.set_hhomogeneous(false);
        paginas.set_vhomogeneous(false);
        paginas.set_transition_type(gtk::StackTransitionType::Crossfade);
        paginas.set_transition_duration(120);
        let mut trilho = Vec::new();
        for (id, glifo, nome) in SECOES {
            let b = gtk::Button::new();
            b.add_css_class("cfg-item");
            b.set_tooltip_text(Some(tr(nome)));
            b.set_halign(Align::Center);
            let sobre = gtk::Overlay::new();
            let img = if *glifo == "spark" { estrela(13, "#8E8F99") } else { ui_icon(glifo, "#8E8F99", 16) };
            sobre.set_child(Some(&img));
            let marca = caixa(Orientation::Vertical, 0, "cfg-marcador");
            marca.set_size_request(3, 16);
            marca.set_halign(Align::Start);
            marca.set_valign(Align::Center);
            marca.set_visible(false);
            sobre.add_overlay(&marca);
            b.set_child(Some(&sobre));
            trilho_box.append(&b);
            trilho.push((*id, b, img, *glifo, marca));
        }
        raiz.append(&trilho_box);
        // (a transparent layer over the pages: its size follows theirs after each layout, when the session test reads
        // where the controls went)
        let sobre_paginas = gtk::Overlay::new();
        sobre_paginas.set_child(Some(&paginas));
        let sentinela = gtk::DrawingArea::new();
        sentinela.set_can_target(false);
        sentinela.set_hexpand(true);
        sentinela.set_vexpand(true);
        sobre_paginas.add_overlay(&sentinela);
        sobre_paginas.set_measure_overlay(&sentinela, false);
        raiz.append(&sobre_paginas);
        janela.set_child(Some(&raiz));
        let inicio = Inicio::new();
        paginas.add_named(&inicio.raiz, Some("inicio"));
        let tamanho_design = (c_w(&janela), c_h(&janela));
        let c = Rc::new(Configuracoes { janela, shell: Rc::downgrade(shell), paginas, trilho, inicio, timer: RefCell::default(), trilho_box, logo: lh, div,
            compacto: Cell::new(None), medida: Cell::new((0, 0)), encolhida: Cell::new(false), aparencia: RefCell::default(), rede: RefCell::default(), bluetooth: RefCell::default(), tela: RefCell::default(), som: RefCell::default(), power: RefCell::default(), notifications: RefCell::default(), entrada: RefCell::default(), acessibilidade: RefCell::default(), privacidade: RefCell::default(), sistema: RefCell::default(), agentes: RefCell::default(), posicoes_extra: RefCell::default(), tamanho_design: Cell::new(tamanho_design) });
        if depurando() {
            let w = Rc::downgrade(&c);
            sentinela.connect_resize(move |_, largura, altura| if let Some(c) = w.upgrade() {
                // (the real size decides the compact layout: the window's default size does not follow a maximize)
                if c.medida.replace((largura, altura)) != (largura, altura) {
                    let c2 = c.clone();
                    glib::idle_add_local_once(move || c2.ajustar_layout());
                }
                c.posicoes()
            });
            // (for the session test: a page's content arrives after it shows and moves its controls; each one whose
            // place changed is logged again)
            let w = Rc::downgrade(&c);
            let last: RefCell<HashMap<String, (i32, i32)>> = RefCell::default();
            glib::timeout_add_local(Duration::from_millis(400), move || {
                let Some(c) = w.upgrade() else { return glib::ControlFlow::Break };
                if !c.janela.is_visible() {
                    last.borrow_mut().clear();
                    return glib::ControlFlow::Continue;
                }
                for (name, wd) in c.posicoes_extra.borrow().iter() {
                    let Some(wd) = wd.upgrade().filter(|x| x.is_mapped()) else { continue };
                    let Some(b) = wd.compute_bounds(&c.janela) else { continue };
                    let p = ((b.x() + b.width() / 2.0) as i32, (b.y() + b.height() / 2.0) as i32);
                    if last.borrow().get(name) != Some(&p) {
                        last.borrow_mut().insert(name.clone(), p);
                        eprintln!("debug central: {name} em {} {}", p.0, p.1);
                    }
                }
                glib::ControlFlow::Continue
            });
        }
        let aparencia = Aparencia::new(&c);
        c.paginas.add_named(&aparencia.moldura.raiz, Some("aparencia"));
        *c.aparencia.borrow_mut() = Some(aparencia);
        let rede = Rede::new(&c);
        c.paginas.add_named(&rede.moldura.raiz, Some("rede"));
        *c.rede.borrow_mut() = Some(rede);
        let bt = Bluetooth::new(&c);
        c.paginas.add_named(&bt.moldura.raiz, Some("bluetooth"));
        *c.bluetooth.borrow_mut() = Some(bt);
        let tela = Tela::new(&c);
        c.paginas.add_named(&tela.moldura.raiz, Some("tela"));
        *c.tela.borrow_mut() = Some(tela);
        let som = Som::new(&c);
        c.paginas.add_named(&som.moldura.raiz, Some("som"));
        *c.som.borrow_mut() = Some(som);
        let power = PowerPage::new(&c);
        c.paginas.add_named(&power.frame.raiz, Some("energia"));
        *c.power.borrow_mut() = Some(power);
        let notifications = NotificationsPage::new(&c);
        c.paginas.add_named(&notifications.frame.raiz, Some("notificacoes"));
        *c.notifications.borrow_mut() = Some(notifications);
        let entrada = crate::configuracoes_entrada::InputPage::new(&c);
        c.paginas.add_named(&entrada.frame.raiz, Some("entrada"));
        *c.entrada.borrow_mut() = Some(entrada);
        let acess = crate::configuracoes_acessibilidade::AccessibilityPage::new(&c);
        c.paginas.add_named(&acess.frame.raiz, Some("acessibilidade"));
        *c.acessibilidade.borrow_mut() = Some(acess);
        let priv_ = crate::configuracoes_privacidade::PrivacyPage::new(&c);
        c.paginas.add_named(&priv_.frame.raiz, Some("privacidade"));
        *c.privacidade.borrow_mut() = Some(priv_);
        let sistema = crate::configuracoes_sistema::SystemPage::new(&c);
        c.paginas.add_named(&sistema.frame.raiz, Some("sistema"));
        let agentes = crate::configuracoes_agentes::AgentsAiPage::new(&c);
        c.paginas.add_named(&agentes.frame.raiz, Some("agentes"));
        *c.agentes.borrow_mut() = Some(agentes);
        *c.sistema.borrow_mut() = Some(sistema);
        c.ajustar_layout();
        for prop in ["default-width", "default-height"] {
            let w = Rc::downgrade(&c);
            c.janela.connect_notify_local(Some(prop), move |_, _| {
                if let Some(c) = w.upgrade() {
                    c.ajustar_layout();
                    // (the controls moved: the session test reads their new places)
                    c.posicoes();
                }
            });
        }
        for (id, b, _, _, _) in &c.trilho {
            let (w, id) = (Rc::downgrade(&c), *id);
            b.connect_clicked(move |_| {
                if let Some(c) = w.upgrade() {
                    c.pagina(id);
                }
            });
        }
        let app_id = |j: &gtk::ApplicationWindow| {
            if let Some(s) = j.surface().filter(|s| s.type_().name().contains("Wayland")) {
                let id = std::ffi::CString::new("os.kalango.Configuracoes").unwrap();
                unsafe {
                    gdk_wayland_toplevel_set_application_id(s.as_ptr() as *mut gdk::ffi::GdkToplevel, id.as_ptr());
                }
            }
        };
        c.janela.connect_realize(move |j| app_id(j));
        c.janela.connect_map(move |j| app_id(j));
        c.inicio.ligar(&c);
        // the page reads the system while it shows
        let w = Rc::downgrade(&c);
        c.janela.connect_map(move |_| {
            if let Some(c) = w.upgrade() {
                c.vigiar();
                c.posicoes();
            }
        });
        let w = Rc::downgrade(&c);
        c.janela.connect_unmap(move |_| {
            if let Some(t) = w.upgrade().and_then(|c| c.timer.borrow_mut().take()) {
                t.remove();
            }
        });
        c
    }

    /// The layout follows the window's size: narrower than 1400 (a 1366 × 768 screen) the compact one; shorter than
    /// the page needs, too (the home page's cards need 700, the other pages 640).
    /// A tab with its own height in the design (Appearance › "Dock e multitarefa": 901): the window takes it when it
    /// is at the design's size, as far as the screen has room, and stays in the middle of the screen.
    pub(crate) fn pedir_altura(&self, altura: i32) {
        if self.compacto.get() != Some(false) || self.janela.is_maximized() {
            return;
        }
        let agora = (c_w(&self.janela), c_h(&self.janela));
        if agora != self.tamanho_design.get() {
            return;
        }
        let (mw, mh) = screen_room().unwrap_or((i32::MAX, i32::MAX));
        let novo = (1520.min(mw), altura.min(mh));
        if novo == agora {
            return;
        }
        self.tamanho_design.set(novo);
        self.janela.set_default_size(novo.0, novo.1);
        if depurando() {
            eprintln!("debug configuracoes: altura da aba {}", novo.1);
        }
        // (niri keeps a floating window's top where it was: it goes back to the middle once it has its new size)
        glib::timeout_add_local_once(std::time::Duration::from_millis(150), || {
            let Ok(v) = crate::servicos::niri_pedido(serde_json::json!("Windows")) else { return };
            let id = v["Windows"].as_array().and_then(|l| l.iter().find(|w| w["app_id"] == "os.kalango.Configuracoes")).and_then(|w| w["id"].as_u64());
            if let Some(id) = id {
                let _ = crate::servicos::niri_acao(serde_json::json!({"CenterWindow": {"id": id}}));
            }
        });
    }

    fn ajustar_layout(&self) {
        // the pages' measured size plus the rail, else (not laid out yet) the window's default size
        let (w, h) = match self.medida.get() {
            (mw, mh) if mw > 0 => (mw + self.trilho_box.width().max(60), mh),
            _ => (self.janela.default_width(), self.janela.default_height()),
        };
        let pagina = self.paginas.visible_child_name().map(|n| n.to_string()).unwrap_or_default();
        let minima = if pagina == "inicio" || pagina.is_empty() { 700 } else { 640 };
        let c = (h > 0 && h < minima) || (w > 0 && w < 1400);
        if self.compacto.replace(Some(c)) == Some(c) {
            return;
        }
        if c {
            self.trilho_box.add_css_class("compacta");
        } else {
            self.trilho_box.remove_css_class("compacta");
        }
        // 60 and 64 in the designs, the window's border taking one
        self.trilho_box.set_size_request(if c { 59 } else { 63 }, -1);
        self.logo.set_visible(!c);
        self.div.set_visible(!c);
        self.inicio.compactar(c);
        if let Some(a) = self.aparencia.borrow().as_ref() {
            a.compactar(c);
        }
        if let Some(r) = self.rede.borrow().as_ref() {
            r.compactar(c);
        }
        if let Some(b) = self.bluetooth.borrow().as_ref() {
            b.compactar(c);
        }
        if let Some(t) = self.tela.borrow().as_ref() {
            t.compactar(c);
        }
        if let Some(s) = self.som.borrow().as_ref() {
            s.compactar(c);
        }
        if let Some(p) = self.power.borrow().as_ref() {
            p.compactar(c);
        }
        if let Some(n) = self.notifications.borrow().as_ref() {
            n.compactar(c);
        }
        if let Some(e) = self.entrada.borrow().as_ref() {
            e.compactar(c);
        }
        if let Some(a) = self.acessibilidade.borrow().as_ref() {
            a.compactar(c);
        }
        if let Some(p) = self.privacidade.borrow().as_ref() {
            p.compactar(c);
        }
        if let Some(s) = self.sistema.borrow().as_ref() {
            s.compactar(c);
        }
        if depurando() {
            eprintln!("debug configuracoes: {} ({w}×{h})", if c { "compacto" } else { "completo" });
        }
        self.posicoes();
    }

    /// For the session test: where the controls are now.
    fn posicoes(&self) {
        if !depurando() {
            return;
        }
        let pos = crate::central::posicao_debug;
        for (id, b, _, _, _) in &self.trilho {
            pos(&format!("cfg trilho {id}"), b.upcast_ref());
        }
        let i = &self.inicio;
        for (k, (b, _, _)) in i.resumo.iter().enumerate() {
            pos(&format!("cfg resumo {k}"), b.upcast_ref());
        }
        pos("cfg luz noturna", i.noturna_sw.upcast_ref());
        pos("cfg instalar", i.atual_botao.upcast_ref());
        if let Some(a) = i.sugestao.last_child() {
            pos("cfg sugestao aplicar", &a);
        }
        // (only what is on screen: a hidden control has no place)
        for (nome, w) in self.posicoes_extra.borrow().iter() {
            if let Some(w) = w.upgrade().filter(|w| w.is_mapped()) {
                pos(nome, &w);
            }
        }
    }

    fn vigiar(self: &Rc<Self>) {
        self.inicio.atualizar();
        if self.timer.borrow().is_some() {
            return;
        }
        let w = Rc::downgrade(self);
        let t = glib::timeout_add_local(Duration::from_secs(3), move || {
            let Some(c) = w.upgrade() else { return glib::ControlFlow::Break };
            if c.paginas.visible_child_name().as_deref() == Some("inicio") {
                c.inicio.atualizar();
            }
            glib::ControlFlow::Continue
        });
        *self.timer.borrow_mut() = Some(t);
    }

    /// Shows a section (the rail marks it).
    /// The rail's button of a page (where to click to go there): for the Help's "Mostre para mim".
    pub fn alvo(&self, id: &str) -> Option<gtk::Button> {
        self.trilho.iter().find(|(p, ..)| *p == id).map(|(_, b, ..)| b.clone())
    }

    pub fn pagina(self: &Rc<Self>, id: &str) {
        if self.paginas.child_by_name(id).is_none() {
            // a section still on the desktop's own panel
            let painel = match id {
                "rede" => "wifi",
                "tela" => "display",
                "som" => "sound",
                "energia" => "power",
                "notificacoes" => "notifications",
                "entrada" => "keyboard",
                "aparencia" => "background",
                "privacidade" => "privacy",
                "acessibilidade" => "universal-access",
                "sistema" => "system",
                "agentes" => {
                    if let Some(s) = self.shell() {
                        s.abrir_central(Some("ajustes"));
                    }
                    return;
                }
                outro => outro,
            };
            let _ = std::process::Command::new("gnome-control-center").arg(painel).stdin(std::process::Stdio::null()).spawn();
            return;
        }
        self.paginas.set_visible_child_name(id);
        // the page's own height from the design, unless the person sized the window
        if self.compacto.get() == Some(false) && !self.janela.is_maximized() {
            let agora = (c_w(&self.janela), c_h(&self.janela));
            if agora == self.tamanho_design.get() {
                // the design's size, as far as the screen has room for it
                let (mw, mh) = screen_room().unwrap_or((i32::MAX, i32::MAX));
                let novo = (1520.min(mw), altura_da_pagina(id).min(mh));
                self.tamanho_design.set(novo);
                self.janela.set_default_size(novo.0, novo.1);
            }
        }
        self.ajustar_layout();
        // a page that reads the system while it shows
        if id == "agentes" {
            if let Some(a) = self.agentes.borrow().clone() {
                a.mostrar();
            }
        }
        if id == "rede" {
            if let Some(r) = self.rede.borrow().clone() {
                r.mostrar();
            }
        }
        if id == "bluetooth" {
            if let Some(b) = self.bluetooth.borrow().clone() {
                b.mostrar();
            }
        }
        if id == "tela" {
            if let Some(t) = self.tela.borrow().clone() {
                t.mostrar();
            }
        }
        if id == "som" {
            if let Some(s) = self.som.borrow().clone() {
                s.mostrar();
            }
        }
        if id == "energia" {
            if let Some(p) = self.power.borrow().clone() {
                p.mostrar();
            }
        }
        if id == "notificacoes" {
            if let Some(n) = self.notifications.borrow().clone() {
                n.mostrar();
            }
        }
        if id == "entrada" {
            if let Some(e) = self.entrada.borrow().clone() {
                e.mostrar();
            }
        }
        if id == "acessibilidade" {
            if let Some(a) = self.acessibilidade.borrow().clone() {
                a.mostrar();
            }
        }
        if id == "privacidade" {
            if let Some(p) = self.privacidade.borrow().clone() {
                p.mostrar();
            }
        }
        if id == "sistema" {
            if let Some(s) = self.sistema.borrow().clone() {
                s.mostrar();
            }
        }
        for (i, b, img, glifo, marca) in &self.trilho {
            let ativo = *i == id;
            marca.set_visible(ativo);
            if ativo {
                b.add_css_class("ativo");
            } else {
                b.remove_css_class("ativo");
            }
            if *glifo != "spark" {
                crate::conversa::set_icon(img, glifo, if ativo { "#A5A1E2" } else { "#8E8F99" });
            }
        }
        if depurando() {
            eprintln!("debug configuracoes: página {id}");
        }
        self.posicoes();
    }

    /// The window fits the screen as it is now (another monitor, a scale changed): smaller when it does not fit, and
    /// back to its size for this screen when the screen grew and the person had not sized it.
    pub fn caber_na_tela(&self, pagina_id: &str) {
        if let Some((mw, mh)) = screen_room() {
            let (w, h) = (self.janela.default_width(), self.janela.default_height());
            let desejado = if mw + 48 >= 1568 && mh + 116 >= 826 { (1520, altura_da_pagina(pagina_id).min(mh)) } else { (mw, mh) };
            if depurando() {
                eprintln!("debug configuracoes: caber {pagina_id}: tela {mw}×{mh}, janela {w}×{h}, desenho {:?}, desejado {desejado:?}", self.tamanho_design.get());
            }
            if self.encolhida.get() && (w < desejado.0 || h < desejado.1) && !self.janela.is_maximized() {
                self.encolhida.set(false);
                self.janela.set_default_size(desejado.0, desejado.1);
                self.tamanho_design.set(desejado);
                if depurando() {
                    eprintln!("debug configuracoes: janela voltou ao tamanho da tela ({}×{})", desejado.0, desejado.1);
                }
            } else if (w > mw || h > mh) && !self.janela.is_maximized() {
                let novo = (w.min(mw), h.min(mh));
                self.encolhida.set(true);
                self.janela.set_default_size(novo.0, novo.1);
                self.tamanho_design.set(novo);
                if depurando() {
                    eprintln!("debug configuracoes: janela coube na tela ({}×{})", novo.0, novo.1);
                }
            }
        }
    }

    /// Shows a page ("aparencia"), or a page's tab ("aparencia/papel").
    pub fn abrir(self: &Rc<Self>, pagina: Option<&str>) {
        let (pagina, aba) = match pagina.and_then(|p| p.split_once('/')) {
            Some((p, a)) => (Some(p), Some(a)),
            None => (pagina, None),
        };
        self.caber_na_tela(pagina.unwrap_or("inicio"));
        self.pagina(pagina.unwrap_or("inicio"));
        if let (Some("aparencia"), Some(a)) = (pagina, aba) {
            if let Some(ap) = self.aparencia.borrow().as_ref() {
                ap.aba(a);
            }
        }
        self.janela.present();
    }

    /// The shell, for the pages (to open the Agents app's pages).
    pub(crate) fn shell_publico(&self) -> Option<Rc<Shell>> {
        self.shell()
    }

    fn shell(&self) -> Option<Rc<Shell>> {
        self.shell.upgrade()
    }
}

fn c_w(j: &gtk::ApplicationWindow) -> i32 {
    j.default_width()
}

fn c_h(j: &gtk::ApplicationWindow) -> i32 {
    j.default_height()
}

/// The window's controls: minimize, maximize and close, glyphs of 16 px.
pub(crate) fn controles_janela() -> gtk::Box {
    let c = caixa(Orientation::Horizontal, 8, "");
    c.set_valign(Align::Center);
    // (its window: a right click on the header opens the title bar's menu)
    c.connect_realize(|c| {
        if let Some(w) = c.root().and_downcast::<gtk::Window>() {
            crate::context_menu::title_bar(&w);
        }
    });
    for (glifo, acao) in [("minimizar", 0), ("maximizar", 1), ("fechar", 2)] {
        let x = gtk::Button::new();
        x.add_css_class("arq-controle");
        // (the three are glyphs of 16 px — the minimize one too: a drawn dash had no height left to be clicked)
        x.set_child(Some(&ui_icon(glifo, "#E2E2E6", 16)));
        x.set_valign(Align::Center);
        // (where they are, for the session test: logged every time their window shows — a window shown again may
        // be elsewhere)
        if acao == 2 {
            crate::central::posicao_debug_sempre("janela fechar", x.upcast_ref());
        } else if acao == 0 {
            crate::central::posicao_debug_sempre("janela minimizar", x.upcast_ref());
        }
        x.connect_clicked(move |x| {
            let Some(j) = x.root().and_downcast::<gtk::Window>() else { return };
            match acao {
                0 => {
                    if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
                        eprintln!("debug janela: minimizar «{}»", j.title().unwrap_or_default());
                    }
                    j.minimize()
                }
                1 => if j.is_maximized() { j.unmaximize() } else { j.maximize() },
                _ => j.close(),
            }
        });
        c.append(&x);
    }
    c
}

/// "Buscar ou pedir" with its shortcut: a section by its name, or (three words or more) a request to the agent.
pub(crate) fn busca() -> (gtk::Box, gtk::Entry) {
    let b = caixa(Orientation::Horizontal, 7, "cfg-busca");
    b.set_size_request(460, 32);
    b.set_valign(Align::Center);
    b.append(&estrela(13, "#A5A1E2"));
    let e = gtk::Entry::new();
    e.add_css_class("arq-busca-campo");
    e.set_placeholder_text(Some(tr("Search or ask: “make the screen warmer at night”")));
    e.set_hexpand(true);
    b.append(&e);
    let atalho = rotulo("Ctrl K", "cfg-atalho");
    atalho.set_valign(Align::Center);
    b.append(&atalho);
    (b, e)
}

impl Inicio {
    fn new() -> Rc<Inicio> {
        let raiz = caixa(Orientation::Vertical, 18, "cfg-principal");
        // the top: hello, the search, the window's controls
        let topo = caixa(Orientation::Horizontal, 16, "");
        let t = caixa(Orientation::Vertical, 2, "");
        let nome = crate::energia::person_name();
        let ola = rotulo(&trf("Hello, {name}", &[("name", nome.split_whitespace().next().unwrap_or(&nome).to_string())]), "cfg-titulo");
        ola.set_xalign(0.0);
        t.append(&ola);
        let sub = rotulo("", "cfg-sub");
        sub.set_xalign(0.0);
        t.append(&sub);
        topo.append(&t);
        let esp = caixa(Orientation::Horizontal, 0, "");
        esp.set_hexpand(true);
        topo.append(&esp);
        let (bx, busca_e) = busca();
        bx.set_hexpand(false);
        topo.append(&bx);
        topo.append(&controles_janela());
        let th = gtk::WindowHandle::new();
        th.set_child(Some(&topo));
        raiz.append(&th);
        let vistas = gtk::Stack::new();
        // only the one showing counts for the window's size
        vistas.set_hhomogeneous(false);
        vistas.set_vhomogeneous(false);
        let completo = caixa(Orientation::Vertical, 18, "");
        let grade = gtk::Grid::new();
        grade.set_column_spacing(16);
        grade.set_row_spacing(16);
        grade.set_column_homogeneous(true);
        grade.set_vexpand(false);
        // this computer
        let disp = caixa(Orientation::Horizontal, 18, "cfg-cartao");
        // the wallpaper in a fixed 280 × 160 box with rounded corners (drawn at twice the size, for sharp screens)
        let fundo = gtk::DrawingArea::new();
        fundo.set_content_width(280);
        fundo.set_content_height(160);
        fundo.add_css_class("cfg-fundo");
        let fundo_pb: Rc<RefCell<Option<gtk::gdk_pixbuf::Pixbuf>>> = Rc::default();
        let pb = fundo_pb.clone();
        fundo.set_draw_func(move |_, cr, w, h| {
            use gtk::gdk::prelude::GdkCairoContextExt;
            let (w, h, r) = (f64::from(w), f64::from(h), 4.0);
            cr.new_sub_path();
            cr.arc(w - r, r, r, -std::f64::consts::FRAC_PI_2, 0.0);
            cr.arc(w - r, h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
            cr.arc(r, h - r, r, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
            cr.arc(r, r, r, std::f64::consts::PI, 1.5 * std::f64::consts::PI);
            cr.close_path();
            cr.clip();
            let c = gdk::RGBA::parse(crate::tema::cor("#16171B")).unwrap_or(gdk::RGBA::BLACK);
            cr.set_source_rgb(f64::from(c.red()), f64::from(c.green()), f64::from(c.blue()));
            let _ = cr.paint();
            if let Some(pb) = pb.borrow().as_ref() {
                cr.scale(w / f64::from(pb.width()), h / f64::from(pb.height()));
                cr.set_source_pixbuf(pb, 0.0, 0.0);
                let _ = cr.paint();
            }
        });
        fundo.set_valign(Align::Start);
        disp.append(&fundo);
        let info = caixa(Orientation::Vertical, 6, "");
        info.set_hexpand(true);
        info.append(&rotulo_cartao("monitor", tr("THIS COMPUTER")));
        let host = rotulo(&glib::host_name(), "cfg-host");
        host.set_xalign(0.0);
        info.append(&host);
        let os = rotulo("", "cfg-sub");
        os.set_xalign(0.0);
        info.append(&os);
        let er = caixa(Orientation::Horizontal, 6, "");
        let estado_ponto = ponto("verde");
        er.append(&estado_ponto);
        let estado = rotulo("", "cfg-estado");
        er.append(&estado);
        info.append(&er);
        let pers = gtk::Button::new();
        pers.add_css_class("cfg-botao");
        pers.set_halign(Align::Start);
        let pc = caixa(Orientation::Horizontal, 5, "");
        pc.append(&ui_icon_fitted("paleta", "#E2E2E6", 16));
        pc.append(&rotulo(tr("Customize"), ""));
        pers.set_child(Some(&pc));
        info.append(&pers);
        disp.append(&info);
        disp.set_size_request(-1, 196);
        grade.attach(&disp, 0, 0, 2, 1);
        // Wi-Fi
        let wf = cartao("");
        let wt = caixa(Orientation::Horizontal, 8, "");
        let wr = rotulo_cartao("wifi", tr("WI-FI"));
        wr.set_hexpand(true);
        wt.append(&wr);
        let wifi_sw = interruptor();
        wt.append(&wifi_sw);
        wf.append(&wt);
        let wifi_nome = rotulo("", "cfg-valor-grande");
        wifi_nome.set_xalign(0.0);
        wifi_nome.set_ellipsize(gtk::pango::EllipsizeMode::End);
        wf.append(&wifi_nome);
        let ws = caixa(Orientation::Horizontal, 8, "");
        let barras = caixa(Orientation::Horizontal, 2, "");
        barras.set_valign(Align::End);
        let mut wifi_barras = Vec::new();
        for h in [4, 7, 10, 13] {
            let b = caixa(Orientation::Vertical, 0, "cfg-barra-sinal");
            b.set_size_request(3, h);
            b.set_valign(Align::End);
            barras.append(&b);
            wifi_barras.push(b);
        }
        ws.append(&barras);
        let wifi_info = rotulo("", "cfg-sub");
        ws.append(&wifi_info);
        wf.append(&ws);
        let wifi_perto = gtk::Button::with_label("");
        wifi_perto.add_css_class("cfg-link");
        wifi_perto.set_halign(Align::Start);
        wf.append(&wifi_perto);
        wf.set_size_request(-1, 196);
        grade.attach(&wf, 2, 0, 1, 1);
        // Bluetooth
        let bf = cartao("");
        let bt = caixa(Orientation::Horizontal, 8, "");
        let br = rotulo_cartao("bluetooth", tr("BLUETOOTH"));
        br.set_hexpand(true);
        bt.append(&br);
        let bt_sw = interruptor();
        bt.append(&bt_sw);
        bf.append(&bt);
        let bt_lista = caixa(Orientation::Vertical, 10, "");
        bf.append(&bt_lista);
        bf.set_size_request(-1, 196);
        grade.attach(&bf, 3, 0, 1, 1);
        // sound
        let sf = cartao("");
        sf.append(&rotulo_cartao("volume", tr("SOUND")));
        let som_nome = rotulo("", "cfg-valor");
        som_nome.set_xalign(0.0);
        som_nome.set_ellipsize(gtk::pango::EllipsizeMode::End);
        sf.append(&som_nome);
        let sr = caixa(Orientation::Horizontal, 9, "");
        sr.append(&ui_icon_fitted("volume", "#8E8F99", 16));
        let som = deslizador();
        sr.append(&som);
        sf.append(&sr);
        // the design's rows: 196, 150 and 150 tall
        sf.set_size_request(-1, 150);
        grade.attach(&sf, 0, 1, 1, 1);
        // the screen
        let tf = cartao("");
        tf.append(&rotulo_cartao("monitor", tr("DISPLAY")));
        let tr_ = caixa(Orientation::Horizontal, 9, "");
        tr_.append(&ui_icon_fitted("sol", "#8E8F99", 16));
        let brilho = deslizador();
        tr_.append(&brilho);
        tf.append(&tr_);
        let nr = caixa(Orientation::Horizontal, 7, "");
        nr.append(&ui_icon_fitted("noite-d", "#8E8F99", 16));
        let noturna_txt = rotulo(tr("Night light 20h–6h"), "cfg-texto");
        noturna_txt.set_hexpand(true);
        noturna_txt.set_xalign(0.0);
        nr.append(&noturna_txt);
        let noturna_sw = interruptor();
        nr.append(&noturna_sw);
        tf.append(&nr);
        tf.set_size_request(-1, 150);
        grade.attach(&tf, 1, 1, 1, 1);
        // power
        let ef = cartao("");
        ef.append(&rotulo_cartao("desligar-d", tr("POWER")));
        let seg = caixa(Orientation::Horizontal, 2, "cfg-segmentado");
        seg.set_homogeneous(true);
        let mut perfis: Vec<(&'static str, gtk::ToggleButton)> = Vec::new();
        for (id, nome) in [("power-saver", tr("Saver")), ("balanced", tr("Balanced")), ("performance", tr("Performance"))] {
            let b = gtk::ToggleButton::with_label(nome);
            b.add_css_class("cfg-seg");
            if let Some((_, p)) = perfis.first() {
                b.set_group(Some(p));
            }
            seg.append(&b);
            perfis.push((id, b));
        }
        ef.append(&seg);
        let energia_info = rotulo("", "cfg-sub");
        energia_info.set_xalign(0.0);
        ef.append(&energia_info);
        ef.set_size_request(-1, 150);
        grade.attach(&ef, 2, 1, 1, 1);
        // storage
        let af = cartao("");
        af.append(&rotulo_cartao("pasta", tr("STORAGE")));
        let disco_txt = rotulo("", "cfg-valor-medio");
        disco_txt.set_xalign(0.0);
        af.append(&disco_txt);
        // the parts side by side, 2 px apart, each as wide as its share (at whatever width the card has)
        let disco_barra = gtk::DrawingArea::new();
        disco_barra.set_content_height(8);
        disco_barra.set_hexpand(true);
        let disco_partes: Rc<RefCell<Vec<(f64, &'static str)>>> = Rc::default();
        let partes = disco_partes.clone();
        disco_barra.set_draw_func(move |_, cr, w, h| {
            let partes = partes.borrow();
            let n = partes.len() as f64;
            if n == 0.0 {
                return;
            }
            let livre = f64::from(w) - 2.0 * (n - 1.0);
            let mut x = 0.0;
            for (k, (frac, cor)) in partes.iter().enumerate() {
                let mut pw = (frac * livre).round().max(2.0);
                if k + 1 == partes.len() {
                    pw = (f64::from(w) - x).max(0.0);
                }
                let c = gdk::RGBA::parse(crate::tema::cor(cor)).unwrap_or(gdk::RGBA::WHITE);
                cr.set_source_rgba(f64::from(c.red()), f64::from(c.green()), f64::from(c.blue()), f64::from(c.alpha()));
                let (r, hh) = (2.0f64.min(pw / 2.0), f64::from(h));
                cr.new_sub_path();
                cr.arc(x + pw - r, r, r, -std::f64::consts::FRAC_PI_2, 0.0);
                cr.arc(x + pw - r, hh - r, r, 0.0, std::f64::consts::FRAC_PI_2);
                cr.arc(x + r, hh - r, r, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
                cr.arc(x + r, r, r, std::f64::consts::PI, 1.5 * std::f64::consts::PI);
                cr.close_path();
                let _ = cr.fill();
                x += pw + 2.0;
            }
        });
        af.append(&disco_barra);
        let leg = caixa(Orientation::Horizontal, 10, "");
        for (cor, nome) in [("roxo", tr("Apps")), ("verde", tr("Docs")), ("ouro", tr("Media"))] {
            let l = caixa(Orientation::Horizontal, 4, "");
            l.append(&ponto(cor));
            l.append(&rotulo(nome, "cfg-sub"));
            leg.append(&l);
        }
        af.append(&leg);
        af.set_size_request(-1, 150);
        grade.attach(&af, 3, 1, 1, 1);
        // the agents
        let gf = caixa(Orientation::Horizontal, 18, "cfg-cartao");
        let gc = caixa(Orientation::Vertical, 8, "");
        gc.set_hexpand(true);
        gc.append(&rotulo_cartao("spark", tr("AGENTS")));
        let agentes = caixa(Orientation::Vertical, 8, "");
        gc.append(&agentes);
        let abrir_central = gtk::Button::with_label(tr("Open agents center"));
        abrir_central.add_css_class("cfg-botao");
        abrir_central.set_halign(Align::Start);
        gc.append(&abrir_central);
        gf.append(&gc);
        let tk = caixa(Orientation::Vertical, 6, "");
        let tokens = rotulo("", "cfg-tokens");
        tk.append(&tokens);
        let tokens_sub = rotulo("", "cfg-sub");
        tk.append(&tokens_sub);
        gf.append(&tk);
        gf.set_size_request(-1, 150);
        grade.attach(&gf, 0, 2, 2, 1);
        // updates
        let uf = caixa(Orientation::Horizontal, 16, "cfg-cartao");
        let ui = caixa(Orientation::Horizontal, 0, "cfg-tile-verde");
        ui.set_hexpand(false);
        ui.set_size_request(44, 44);
        ui.set_valign(Align::Start);
        let baixar = ui_icon_fitted("download", "#8ED2C3", 16);
        baixar.set_hexpand(true);
        ui.append(&baixar);
        uf.append(&ui);
        let ut = caixa(Orientation::Vertical, 4, "");
        ut.set_hexpand(true);
        let atual_titulo = rotulo("", "cfg-valor-medio");
        atual_titulo.set_xalign(0.0);
        ut.append(&atual_titulo);
        let atual_sub = rotulo("", "cfg-sub");
        atual_sub.set_xalign(0.0);
        atual_sub.set_wrap(true);
        ut.append(&atual_sub);
        uf.append(&ut);
        let atual_botao = gtk::Button::with_label(tr("Install tonight"));
        atual_botao.add_css_class("cfg-botao-principal");
        atual_botao.set_valign(Align::Start);
        uf.append(&atual_botao);
        uf.set_size_request(-1, 150);
        grade.attach(&uf, 2, 2, 2, 1);
        completo.append(&grade);
        // the agent's suggestion
        let sugestao = caixa(Orientation::Horizontal, 12, "cfg-sugestao");
        sugestao.append(&estrela(13, "#A5A1E2"));
        let sugestao_txt = rotulo("", "cfg-sugestao-texto");
        sugestao_txt.set_hexpand(true);
        sugestao_txt.set_xalign(0.0);
        sugestao_txt.set_ellipsize(gtk::pango::EllipsizeMode::End);
        sugestao.append(&sugestao_txt);
        let dispensar = gtk::Button::with_label(tr("Dismiss"));
        dispensar.add_css_class("cfg-botao-texto");
        sugestao.append(&dispensar);
        let aplicar = gtk::Button::with_label(tr("Apply"));
        aplicar.add_css_class("cfg-botao-principal");
        aplicar.add_css_class("pequeno");
        sugestao.append(&aplicar);
        sugestao.set_visible(false);
        completo.append(&sugestao);
        vistas.add_named(&completo, Some("completo"));
        // the compact layout: one card a section, in two columns
        let compacto = gtk::Grid::new();
        compacto.set_column_spacing(12);
        compacto.set_row_spacing(12);
        compacto.set_column_homogeneous(true);
        compacto.set_valign(Align::Start);
        let mut resumo = Vec::new();
        for (k, (glifo, nome)) in [("wifi", tr("WI-FI")), ("bluetooth", tr("BLUETOOTH")), ("volume", tr("SOUND")), ("monitor", tr("DISPLAY")),
            ("spark", tr("AGENTS")), ("download", tr("UPDATES"))].into_iter().enumerate() {
            let b = gtk::Button::new();
            b.add_css_class("cfg-resumo");
            b.set_size_request(-1, 89);
            let col = caixa(Orientation::Vertical, 6, "");
            col.append(&rotulo_cartao(glifo, nome));
            let v = rotulo("", "cfg-resumo-valor");
            v.set_xalign(0.0);
            v.set_ellipsize(gtk::pango::EllipsizeMode::End);
            col.append(&v);
            let s = rotulo("", "cfg-sub");
            s.set_xalign(0.0);
            s.set_ellipsize(gtk::pango::EllipsizeMode::End);
            col.append(&s);
            b.set_child(Some(&col));
            compacto.attach(&b, (k % 2) as i32, (k / 2) as i32, 1, 1);
            resumo.push((b, v, s));
        }
        vistas.add_named(&compacto, Some("compacto"));
        raiz.append(&vistas);
        let i = Rc::new(Inicio { raiz, topo, titulo: ola, sub, busca: bx, busca_e, vistas, resumo, prontas: Cell::new(0), fundo, fundo_pb, host, os, estado, estado_ponto, wifi_sw, wifi_nome, wifi_barras, wifi_info, wifi_perto, bt_sw, bt_lista, som_nome, som,
            brilho, noturna_sw, noturna_txt, perfis, energia_info, disco_txt, disco_barra, disco_partes, agentes, tokens, tokens_sub, atual_titulo, atual_sub, atual_botao, sugestao,
            sugestao_txt, lendo: Cell::new(false), shell: RefCell::default(), ocupado: Rc::default(), lento: Cell::new(None), ultimo: RefCell::default() });
        // the controls act on the system
        let w = Rc::downgrade(&i);
        i.wifi_sw.connect_state_set(move |_, on| {
            if let Some(i) = w.upgrade().filter(|i| !i.lendo.get()) {
                std::thread::spawn(move || sistema::wifi(on));
                if depurando() {
                    eprintln!("debug configuracoes: wi-fi {on}");
                }
                let _ = i;
            }
            glib::Propagation::Proceed
        });
        let w = Rc::downgrade(&i);
        i.bt_sw.connect_state_set(move |_, on| {
            if w.upgrade().is_some_and(|i| !i.lendo.get()) {
                std::thread::spawn(move || sistema::bluetooth(on));
            }
            glib::Propagation::Proceed
        });
        let w = Rc::downgrade(&i);
        i.som.connect_value_changed(move |s| {
            if w.upgrade().is_some_and(|i| !i.lendo.get()) {
                let v = s.value();
                std::thread::spawn(move || sistema::volume(v));
            }
        });
        let w = Rc::downgrade(&i);
        i.brilho.connect_value_changed(move |s| {
            if w.upgrade().is_some_and(|i| !i.lendo.get()) {
                let v = s.value();
                std::thread::spawn(move || sistema::brilho(v));
            }
        });
        let w = Rc::downgrade(&i);
        i.noturna_sw.connect_state_set(move |_, on| {
            if w.upgrade().is_some_and(|i| !i.lendo.get()) {
                // the schedule: on now if it is night, and from 20h to 6h every day
                salvar_ajuste("luz_noturna_auto", json!(on));
                let agora = na_janela_noturna(glib::DateTime::now_local().map(|d| d.hour()).unwrap_or(12));
                std::thread::spawn(move || sistema::luz_noturna(on && agora));
                if depurando() {
                    eprintln!("debug configuracoes: luz noturna automática {on}");
                }
            }
            glib::Propagation::Proceed
        });
        for (id, b) in &i.perfis {
            let (w, id) = (Rc::downgrade(&i), id.to_string());
            b.connect_toggled(move |b| {
                if b.is_active() && w.upgrade().is_some_and(|i| !i.lendo.get()) {
                    let p = id.clone();
                    std::thread::spawn(move || sistema::perfil(&p));
                    if depurando() {
                        eprintln!("debug configuracoes: perfil {id}");
                    }
                }
            });
        }
        let w = Rc::downgrade(&i);
        dispensar.connect_clicked(move |_| {
            if let Some(i) = w.upgrade() {
                salvar_ajuste("sugestao_dispensada", json!(i.sugestao_txt.text().to_string()));
                i.sugestao.set_visible(false);
            }
        });
        let w = Rc::downgrade(&i);
        aplicar.connect_clicked(move |_| {
            if let Some(i) = w.upgrade() {
                // the night light's schedule (the one suggestion there is today)
                i.noturna_sw.set_active(true);
                i.sugestao.set_visible(false);
            }
        });
        let _ = pers;
        let _ = abrir_central;
        i
    }

    fn compactar(&self, c: bool) {
        for w in [self.raiz.upcast_ref::<gtk::Widget>(), self.titulo.upcast_ref()] {
            if c {
                w.add_css_class("compacta");
            } else {
                w.remove_css_class("compacta");
            }
        }
        self.raiz.set_spacing(if c { 14 } else { 18 });
        self.topo.set_spacing(if c { 10 } else { 16 });
        self.sub.set_visible(!c);
        self.busca.set_size_request(if c { 280 } else { 460 }, 32);
        self.busca_e.set_placeholder_text(Some(if c { tr("Search or ask…") } else { tr("Search or ask: “make the screen warmer at night”") }));
        self.vistas.set_visible_child_name(if c { "compacto" } else { "completo" });
    }

    /// A compact card's two lines.
    fn resumir(&self, k: usize, valor: &str, linha: &str) {
        if let Some((_, v, s)) = self.resumo.get(k) {
            v.set_text(valor);
            s.set_text(linha);
        }
    }

    /// The buttons that go elsewhere (they need the window).
    fn ligar(self: &Rc<Self>, c: &Rc<Configuracoes>) {
        *self.shell.borrow_mut() = c.shell.clone();
        // the compact cards open their section; the updates' one schedules the install for tonight, as its line says
        for (k, (b, _, _)) in self.resumo.iter().enumerate() {
            let (cw, iw) = (Rc::downgrade(c), Rc::downgrade(self));
            b.connect_clicked(move |_| {
                let (Some(c), Some(i)) = (cw.upgrade(), iw.upgrade()) else { return };
                match k {
                    0 => c.pagina("rede"),
                    1 => c.pagina("bluetooth"),
                    2 => c.pagina("som"),
                    3 => c.pagina("tela"),
                    4 => if let Some(s) = c.shell() { s.abrir_central(None) },
                    _ if i.prontas.get() > 0 && i.atual_botao.is_sensitive() => i.atual_botao.emit_clicked(),
                    _ => c.pagina("sistema"),
                }
            });
        }
        // "Personalizar" → Aparência; "+ n redes" → Rede; the agents center
        let mut botoes: Vec<gtk::Button> = Vec::new();
        let mut w = self.raiz.first_child();
        while let Some(x) = w {
            Self::achar_botoes(&x, &mut botoes);
            w = x.next_sibling();
        }
        for b in botoes {
            let rotulo = b.label().map(|l| l.to_string()).or_else(|| {
                b.child().and_downcast::<gtk::Box>().and_then(|bx| bx.last_child()).and_downcast::<gtk::Label>().map(|l| l.text().to_string())
            }).unwrap_or_default();
            let cw = Rc::downgrade(c);
            if rotulo == tr("Customize") {
                b.connect_clicked(move |_| if let Some(c) = cw.upgrade() { c.pagina("aparencia") });
            } else if rotulo == tr("Open agents center") {
                b.connect_clicked(move |_| if let Some(s) = cw.upgrade().and_then(|c| c.shell()) { s.abrir_central(None) });
            }
        }
        let cw = Rc::downgrade(c);
        self.wifi_perto.connect_clicked(move |_| if let Some(c) = cw.upgrade() { c.pagina("rede") });
        let w = Rc::downgrade(self);
        self.atual_botao.connect_clicked(move |b| {
            if w.upgrade().is_some() {
                let agora = glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0);
                salvar_ajuste("instalar_a_partir", json!(proxima_madrugada(agora)));
                b.set_label(tr("Installs tonight"));
                b.set_sensitive(false);
                if let Some(i) = w.upgrade() {
                    i.resumir(5, &i.atual_titulo.text(), tr("Installs tonight"));
                }
                if depurando() {
                    eprintln!("debug configuracoes: instalar à noite, a partir de {}", proxima_madrugada(agora));
                }
            }
        });
    }

    fn achar_botoes(w: &gtk::Widget, v: &mut Vec<gtk::Button>) {
        if let Some(b) = w.downcast_ref::<gtk::Button>() {
            v.push(b.clone());
        }
        let mut c = w.first_child();
        while let Some(x) = c {
            Self::achar_botoes(&x, v);
            c = x.next_sibling();
        }
    }

    /// Reads the system off the interface thread and shows it.
    fn atualizar(self: &Rc<Self>) {
        if self.ocupado.replace(true) {
            return;
        }
        let w = Rc::downgrade(self);
        let ocupado = self.ocupado.clone();
        crate::lateral::em_segundo_plano(ler, move |l| {
            ocupado.set(false);
            if let Some(i) = w.upgrade() {
                i.mostrar(l);
            }
        });
        // the slow ones, once a minute: what fills the disk (measured once a day), the updates (PackageKit)
        let agora = glib::monotonic_time() / 1_000_000;
        if self.lento.get().is_some_and(|t| agora - t < 60) {
            return;
        }
        self.lento.set(Some(agora));
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(categorias, move |c| {
            if let Some(i) = w.upgrade() {
                i.mostrar_disco(c);
            }
        });
        let w = Rc::downgrade(self);
        crate::energia::prepared_update(move |n| {
            if let Some(i) = w.upgrade() {
                i.mostrar_atualizacoes(n);
            }
        });
    }

    fn mostrar(&self, l: Leitura) {
        self.lendo.set(true);
        let s = &l.s;
        let versao = env!("CARGO_PKG_VERSION").splitn(3, '.').take(2).collect::<Vec<_>>().join(".");
        self.os.set_text(&format!("KalangoOS {versao} · {} · Wayland", l.compositor));
        if !l.host.is_empty() {
            self.host.set_text(&l.host);
        }
        if self.fundo_pb.borrow().is_none() {
            // the wallpaper as the desktop shows it, cut to the box's proportion
            *self.fundo_pb.borrow_mut() = crate::superficies::papel_de_parede(560, 320);
            self.fundo.queue_draw();
        }
        // Wi-Fi
        self.wifi_sw.set_active(s.rede.wifi_ligado);
        self.wifi_sw.set_state(s.rede.wifi_ligado);
        match &s.rede.conectada {
            Some(n) => {
                self.wifi_nome.set_text(n);
                let sinal = l.wifi.sinal.max(s.rede.sinal);
                let mut t = qualidade(sinal).to_string();
                if !l.wifi.taxa.is_empty() {
                    t = format!("{t} · {}", l.wifi.taxa);
                }
                self.wifi_info.set_text(&t);
                let n_barras = match sinal { 75.. => 4, 50..=74 => 3, 25..=49 => 2, _ => 1 };
                for (k, b) in self.wifi_barras.iter().enumerate() {
                    if k < n_barras { b.add_css_class("ativa") } else { b.remove_css_class("ativa") }
                }
            }
            None => {
                self.wifi_nome.set_text(if s.rede.cabo { tr("Cable connected") } else if s.rede.wifi_ligado { tr("Not connected") } else { tr("Wi-Fi off") });
                self.wifi_info.set_text("");
                for b in &self.wifi_barras {
                    b.remove_css_class("ativa");
                }
            }
        }
        self.resumir(0, &self.wifi_nome.text(), &self.wifi_info.text());
        self.wifi_perto.set_label(&if l.wifi.perto == 1 { tr("+ 1 network nearby").to_string() } else { trf("+ {n} networks nearby", &[("n", l.wifi.perto.to_string())]) });
        self.wifi_perto.set_visible(s.rede.wifi_ligado && l.wifi.perto > 0);
        // Bluetooth
        self.bt_sw.set_sensitive(s.bt.existe);
        self.bt_sw.set_active(s.bt.ligado);
        self.bt_sw.set_state(s.bt.ligado);
        while let Some(x) = self.bt_lista.first_child() {
            self.bt_lista.remove(&x);
        }
        if !s.bt.existe {
            self.bt_lista.append(&rotulo(tr("No Bluetooth on this computer"), "cfg-sub"));
        } else if s.bt.conectados.is_empty() {
            let r = rotulo(if s.bt.ligado { tr("Nothing connected") } else { tr("Bluetooth off") }, "cfg-sub");
            r.set_xalign(0.0);
            self.bt_lista.append(&r);
        }
        for d in s.bt.conectados.iter().take(2) {
            let r = caixa(Orientation::Horizontal, 10, "");
            let ic = caixa(Orientation::Horizontal, 0, "cfg-tile");
            ic.set_hexpand(false);
            ic.set_size_request(32, 32);
            ic.set_valign(Align::Center);
            let glifo = if d.tipo.contains("audio") || d.tipo.contains("headset") || d.tipo.contains("headphone") { "fone" } else if d.tipo.contains("mouse") || d.tipo.contains("input") { "mouse-d" } else { "bluetooth" };
            let g = ui_icon_fitted(glifo, "#E2E2E6", 16);
            g.set_hexpand(true);
            ic.append(&g);
            r.append(&ic);
            let t = caixa(Orientation::Vertical, 1, "");
            t.set_valign(Align::Center);
            let n = rotulo(&d.nome, "cfg-nome");
            n.set_xalign(0.0);
            t.append(&n);
            let b = rotulo(&d.bateria.map(|b| format!("{b}%")).unwrap_or_else(|| tr("connected").to_string()), "cfg-sub");
            b.set_xalign(0.0);
            t.append(&b);
            r.append(&t);
            self.bt_lista.append(&r);
        }
        match s.bt.conectados.split_first() {
            Some((d, resto)) => {
                let v = match d.bateria { Some(b) => format!("{} · {b}%", d.nome), None => d.nome.clone() };
                let outros = resto.iter().map(|d| d.nome.as_str()).collect::<Vec<_>>().join(", ");
                self.resumir(1, &v, if outros.is_empty() { tr("connected") } else { &outros });
            }
            None if !s.bt.existe => self.resumir(1, tr("No Bluetooth on this computer"), ""),
            None => self.resumir(1, if s.bt.ligado { tr("Nothing connected") } else { tr("Bluetooth off") }, ""),
        }
        // sound
        self.som_nome.set_text(if l.saida.is_empty() { if s.som.existe { tr("Speakers") } else { tr("No sound output") } } else { &l.saida });
        self.som.set_sensitive(s.som.existe);
        if !self.som.has_css_class("arrastando") {
            self.som.set_value(if s.som.mudo { 0.0 } else { s.som.nivel.min(1.0) });
        }
        let nivel = if s.som.mudo { tr("Muted").to_string() } else { format!("{}%", (s.som.nivel * 100.0).round()) };
        self.resumir(2, &self.som_nome.text(), if s.som.existe { &nivel } else { "" });
        // the screen
        match s.brilho {
            Some(b) => {
                self.brilho.set_sensitive(true);
                self.brilho.set_value(b);
                self.brilho.set_tooltip_text(None);
            }
            None => {
                self.brilho.set_sensitive(false);
                self.brilho.set_value(1.0);
                self.brilho.set_tooltip_text(Some(tr("This screen sets its brightness on its own buttons")));
            }
        }
        let auto = luz_noturna_auto();
        let indisponivel = s.luz_noturna.is_none() || s.luz_noturna_indisponivel;
        // without gamma control it cannot turn on, but a schedule already on can always be turned off
        self.noturna_sw.set_sensitive(!indisponivel || auto);
        self.noturna_sw.set_active(auto);
        self.noturna_sw.set_state(auto);
        self.noturna_txt.set_text(if indisponivel { tr("Night light unavailable on this screen") } else { tr("Night light 20h–6h") });
        let brilho = match s.brilho {
            Some(b) => trf("Brightness {n}%", &[("n", (b * 100.0).round().to_string())]),
            None => tr("Brightness on the screen's buttons").to_string(),
        };
        self.resumir(3, &brilho, if indisponivel { tr("Night light unavailable on this screen") } else if auto { tr("Night light 20h–6h") } else { tr("Night light off") });
        // power
        for (id, b) in &self.perfis {
            b.set_sensitive(s.perfis.iter().any(|p| p == id));
            if s.perfil == *id {
                b.set_active(true);
            }
        }
        self.energia_info.set_text(&match (s.bateria, &l.gpu) {
            (Some((p, true)), _) => trf("Battery {n}% · charging", &[("n", p.to_string())]),
            (Some((p, false)), _) => trf("Battery {n}%", &[("n", p.to_string())]),
            // the card lowers its clocks on its own when idle (P8 and up), and goes up to full power (P0) when used
            (None, Some((nome, 0))) => trf("GPU {name} at full power", &[("name", nome.clone())]),
            (None, Some((nome, 1..=7))) => trf("GPU {name} in use", &[("name", nome.clone())]),
            (None, Some((nome, _))) => trf("GPU {name} at rest", &[("name", nome.clone())]),
            (None, None) => tr("Plugged in").to_string(),
        });
        // the disk
        let (t, livre) = l.disco;
        self.disco_txt.set_text(&crate::arquivos::de_total(t.saturating_sub(livre), t));
        self.mostrar_agentes(&l.pendentes);
        self.sugestao_render(s);
        if depurando() && self.ultimo.borrow().is_none() {
            eprintln!("debug configuracoes: lido host={} perfil={} disco={}/{} wifi={} bt={} sugestao={}", l.host, s.perfil, l.disco.0 - l.disco.1, l.disco.0,
                s.rede.conectada.as_deref().unwrap_or("-"), s.bt.ligado, self.sugestao.is_visible());
        }
        let categorias = self.ultimo.borrow().as_ref().and_then(|u| u.categorias);
        *self.ultimo.borrow_mut() = Some(Leitura { categorias, ..l });
        if let Some(c) = categorias {
            self.mostrar_disco(c);
        }
        self.lendo.set(false);
    }

    /// The disk's bar: apps, docs, media, the rest used, the free part.
    fn mostrar_disco(&self, (apps, docs, midia): (u64, u64, u64)) {
        // kept for the next readings (and drawn when the disk's size is known)
        let Some((t, livre)) = self.ultimo.borrow_mut().as_mut().map(|l| {
            l.categorias = Some((apps, docs, midia));
            l.disco
        }) else {
            return;
        };
        if t == 0 {
            return;
        }
        let usado = t.saturating_sub(livre);
        let resto = usado.saturating_sub(apps + docs + midia);
        *self.disco_partes.borrow_mut() = [(apps, "#A5A1E2"), (docs, "#8ED2C3"), (midia, "#E5C98C"), (resto, "#EFA2A6"), (livre, "#24252D")]
            .into_iter().filter(|(v, _)| *v > 0).map(|(v, c)| (v as f64 / t as f64, c)).collect();
        self.disco_barra.queue_draw();
    }

    fn mostrar_atualizacoes(&self, prontas: usize) {
        let versao = env!("CARGO_PKG_VERSION").splitn(3, '.').take(2).collect::<Vec<_>>().join(".");
        self.prontas.set(prontas);
        if prontas > 0 {
            self.atual_titulo.set_text(&trf("{n} updates ready", &[("n", prontas.to_string())]));
            self.atual_sub.set_text(tr("Downloaded and checked. Tonight, with the screen locked, the computer restarts to install them — or when you turn it off."));
            let marcado = ajustes()["instalar_a_partir"].as_i64().is_some();
            self.atual_botao.set_visible(true);
            self.atual_botao.set_sensitive(!marcado);
            self.atual_botao.set_label(if marcado { tr("Installs tonight") } else { tr("Install tonight") });
            self.sub.set_text(&trf("KalangoOS {v} · updates ready", &[("v", versao.clone())]));
            self.estado.set_text(tr("Update ready"));
            self.estado_ponto.remove_css_class("verde");
            self.estado_ponto.add_css_class("roxo");
            self.resumir(5, &self.atual_titulo.text(), &self.atual_botao.label().unwrap_or_default());
        } else {
            self.atual_titulo.set_text(&trf("KalangoOS {v} is up to date", &[("v", versao.clone())]));
            self.atual_sub.set_text(tr("The system looks for updates on its own and tells you when one is ready."));
            self.atual_botao.set_visible(false);
            self.sub.set_text(&trf("KalangoOS {v} · all up to date", &[("v", versao)]));
            self.estado.set_text(tr("Up to date"));
            self.estado_ponto.remove_css_class("roxo");
            self.estado_ponto.add_css_class("verde");
            self.resumir(5, &self.atual_titulo.text(), tr("Looks for updates on its own"));
        }
    }

    /// The agents at work (the side bar's, and the last tasks) and today's tokens.
    /// The agents at work (as the desktop's "Agentes ativos": the agent's task, and whoever waits for an approval in
    /// kalangod) and today's tokens.
    fn mostrar_agentes(&self, pendentes: &[Value]) {
        while let Some(x) = self.agentes.first_child() {
            self.agentes.remove(&x);
        }
        let linha = |cor: &str, quem: &str, oque: &str| {
            let r = caixa(Orientation::Horizontal, 8, "");
            r.append(&ponto(cor));
            r.append(&rotulo(quem, "cfg-nome"));
            let o = rotulo(oque, "cfg-sub");
            o.set_ellipsize(gtk::pango::EllipsizeMode::End);
            o.set_max_width_chars(1);
            o.set_hexpand(true);
            o.set_xalign(0.0);
            r.append(&o);
            r
        };
        let tarefa = self.shell.borrow().upgrade().and_then(|s| s.estado.agent_task.borrow().as_ref().map(|(t, _)| t.clone()));
        self.agentes.append(&match &tarefa {
            Some(t) => linha("verde", tr("Agent"), t),
            None => linha("cinza", tr("Agent"), tr("ready · Super+G")),
        });
        for p in pendentes.iter().take(2) {
            let quem = p["caller"].as_str().unwrap_or("").split(" (").next().unwrap_or("").to_string();
            self.agentes.append(&linha("ouro", &quem, tr("waiting for approval")));
        }
        let hoje = crate::central_uso::tokens_hoje();
        self.tokens.set_text(&crate::central_uso::mil(hoje));
        self.tokens_sub.set_text(&match crate::central_uso::limite_diario() {
            Some(lim) => trf("tokens today · {p}%", &[("p", ((hoje as f64 / lim as f64) * 100.0).round().to_string())]),
            None => tr("tokens today").to_string(),
        });
        // compact: how many are at work (the agent's task, and those waiting for an answer) and today's tokens
        let ativos = usize::from(tarefa.is_some()) + pendentes.len();
        let t = crate::central_uso::mil(hoje);
        let valor = match ativos {
            0 => trf("None active · {t} tokens", &[("t", t)]),
            1 => trf("1 active · {t} tokens", &[("t", t)]),
            n => trf("{n} active · {t} tokens", &[("n", n.to_string()), ("t", t)]),
        };
        let linha = match pendentes.len() {
            0 => tr("Nothing waiting for approval").to_string(),
            1 => tr("1 waiting for approval").to_string(),
            n => trf("{n} wait for approval", &[("n", n.to_string())]),
        };
        self.resumir(4, &valor, &linha);
    }

    /// One suggestion from how the computer is used: the night light's schedule for someone who uses it at night.
    fn sugestao_render(&self, s: &sistema::Sistema) {
        let noite = crate::configuracoes::usa_a_noite();
        let disponivel = s.luz_noturna.is_some() && !s.luz_noturna_indisponivel;
        let texto = tr("Suggestion: turn on the automatic night light at 20h — you often use the computer at night.");
        let dispensada = ajustes()["sugestao_dispensada"].as_str() == Some(texto);
        let mostrar = noite && disponivel && !luz_noturna_auto() && !dispensada;
        self.sugestao_txt.set_text(texto);
        self.sugestao.set_visible(mostrar);
    }
}

/// Whether the person uses the computer at night: apps opened after 20h on at least three of the last three weeks'
/// days (the shell's local log of launches).
pub(crate) fn usa_a_noite() -> bool {
    let agora = glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0);
    let dias: std::collections::BTreeSet<(i32, i32)> = crate::apps::uses().into_iter().filter(|(_, t)| agora - t < 21 * 86_400)
        .filter_map(|(_, t)| glib::DateTime::from_unix_local(t).ok()).filter(|d| d.hour() >= 20 || d.hour() < 6)
        .map(|d| (d.year(), d.day_of_year())).collect();
    dias.len() >= 3
}

#[cfg(test)]
mod tests {
    use super::*;

    fn em(h: i32, m: i32) -> i64 {
        glib::DateTime::from_local(2026, 11, 16, h, m, 0.0).unwrap().to_unix()
    }

    #[test]
    fn install_tonight_starts_at_the_next_2h_and_lasts_three_hours() {
        // in the afternoon: this coming night's 2h (tomorrow's date)
        assert_eq!(proxima_madrugada(em(15, 0)), em(2, 0) + 86_400);
        // after midnight, before 2h, and inside the window: this night's
        assert_eq!(proxima_madrugada(em(1, 0)), em(2, 0));
        assert_eq!(proxima_madrugada(em(4, 30)), em(2, 0));
        // after 5h: the next one
        assert_eq!(proxima_madrugada(em(5, 0)), em(2, 0) + 86_400);
    }

    #[test]
    fn desktop_panels_open_their_section() {
        assert_eq!(secao_do_painel("wifi"), "rede");
        assert_eq!(secao_do_painel("network"), "rede");
        assert_eq!(secao_do_painel("display"), "tela");
        assert_eq!(secao_do_painel("mouse"), "entrada");
        assert_eq!(secao_do_painel("universal-access"), "acessibilidade");
        assert_eq!(secao_do_painel("datetime"), "sistema");
        assert_eq!(secao_do_painel("qualquer"), "inicio");
    }

    #[test]
    fn the_night_window_is_from_20h_to_6h() {
        assert!(na_janela_noturna(20) && na_janela_noturna(23) && na_janela_noturna(0) && na_janela_noturna(5));
        assert!(!na_janela_noturna(6) && !na_janela_noturna(12) && !na_janela_noturna(19));
    }
}
