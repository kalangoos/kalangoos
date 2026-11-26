//! Settings › Privacy and security (design "Configurações · Privacidade e segurança").
//!
//! "Quem pode usar o quê": the apps that run isolated (Flatpak) and the agent, one rule for all — a click on a cell
//! goes allowed → ask → denied, at once:
//! - camera and location: the portals' permission store (`flatpak permission-set`); the camera also by the app's
//!   direct access to devices;
//! - microphone, files and network: the app's sandbox (`flatpak override --user`: the PulseAudio socket, the home
//!   folder, the network) — these have no "ask";
//! - the screen: always asked by the screen-sharing portal (shown, not changed);
//! - the agent: its permissions' columns (screen, files = writing, network), as in the Agents app.
//! Apps installed as .deb packages run without isolation: the system cannot limit them, and the page says so.
//!
//! The panel: the disk's encryption (LUKS under /), the firewall (kalangod, approved), the automatic lock (the idle
//! watcher), the security updates (unattended-upgrades and what is pending); the agents' history kept for how long
//! (older conversations deleted), no telemetry at all, asking before the cloud, and erasing all the AI's data.
//! KALANGO_FLATPAK (tests): a stand-in flatpak.

use crate::configuracoes::{depurando, interruptor, rotulo_cartao};
use crate::configuracoes_moldura::Moldura;
use crate::conversa::ui_icon;
use crate::superficies::{caixa, rotulo};
use gtk::prelude::*;
use gtk::{glib, Align, Orientation};
use kalango_i18n::{n_, tr, trf};
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// The table's columns: (id, glyph, name).
const COLUNAS: &[(&str, &str, &str)] = &[
    ("camera", "camera", n_("Camera")),
    ("microfone", "microfone", n_("Microphone")),
    ("tela", "monitor", n_("Screen")),
    ("local", "globo", n_("Location")),
    ("arquivos", "pasta", n_("Files")),
    ("rede", "rede-cabo", n_("Network")),
];

/// A cell: allowed, ask, denied, or not something this app or agent can use.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Estado {
    Sim,
    Perguntar,
    Nao,
    NaoSeAplica,
}

impl Estado {
    fn classe(self) -> &'static str {
        match self {
            Estado::Sim => "sim",
            Estado::Perguntar => "perguntar",
            Estado::Nao => "nao",
            Estado::NaoSeAplica => "nada",
        }
    }
}

fn flatpak() -> String {
    std::env::var("KALANGO_FLATPAK").unwrap_or_else(|_| "flatpak".into())
}

fn run(prog: &str, args: &[&str]) -> Result<String, String> {
    let o = std::process::Command::new(prog).args(args).env("LC_ALL", "C").output().map_err(|e| format!("{prog}: {e}"))?;
    if !o.status.success() {
        return Err(String::from_utf8_lossy(&o.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&o.stdout).into_owned())
}

/// A Flatpak app and what its sandbox and the permission store give it.
#[derive(Clone, Debug)]
struct App {
    id: String,
    nome: String,
    estados: [Estado; 6],
    /// it reaches every device directly (the camera without the portal)
    todos_dispositivos: bool,
}

/// The installed Flatpak apps and their cells.
fn ler_apps() -> Vec<App> {
    let Ok(lista) = run(&flatpak(), &["list", "--app", "--columns=application,name"]) else { return vec![] };
    let store = run(&flatpak(), &["permission-list"]).unwrap_or_default();
    lista.lines().filter_map(|l| {
        let mut p = l.split('\t');
        let id = p.next()?.trim().to_string();
        let nome = p.next().map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| id.clone());
        if id.is_empty() || id == "Application ID" {
            return None;
        }
        let perms = run(&flatpak(), &["info", "--show-permissions", &id]).unwrap_or_default();
        let campo = |k: &str| perms.lines().find_map(|l| l.strip_prefix(&format!("{k}="))).unwrap_or("").split(';').filter(|x| !x.is_empty()).map(str::to_string).collect::<Vec<_>>();
        let (shared, sockets, devices, fs) = (campo("shared"), campo("sockets"), campo("devices"), campo("filesystems"));
        // the store: "devices camera APP yes" / "location location APP EXACT,…"
        let no_store = |tabela: &str, obj: &str| store.lines().find_map(|l| {
            let c: Vec<&str> = l.split_whitespace().collect();
            (c.len() >= 4 && c[0] == tabela && c[1] == obj && c[2] == id).then(|| c[3].to_string())
        });
        let todos = devices.iter().any(|d| d == "all");
        let camera = if todos { Estado::Sim } else {
            match no_store("devices", "camera").as_deref() { Some("yes") => Estado::Sim, Some("no") => Estado::Nao, _ => Estado::Perguntar }
        };
        let microfone = if sockets.iter().any(|s| s == "pulseaudio") { Estado::Sim } else { Estado::Nao };
        let local = match no_store("location", "location").as_deref() { Some("NONE") => Estado::Nao, Some(_) => Estado::Sim, None => Estado::Perguntar };
        let arquivos = if fs.iter().any(|f| f.starts_with("home") || f.starts_with("host")) { Estado::Sim } else { Estado::Perguntar };
        let rede = if shared.iter().any(|s| s == "network") { Estado::Sim } else { Estado::Nao };
        Some(App { id, nome, estados: [camera, microfone, Estado::Perguntar, local, arquivos, rede], todos_dispositivos: todos })
    }).collect()
}

/// The next state of an app's cell, as the system can apply it.
fn mudar_app(app: &App, col: usize) -> Result<(), String> {
    let f = flatpak();
    let id = app.id.as_str();
    let atual = app.estados[col];
    match COLUNAS[col].0 {
        "camera" => {
            // allowed → ask → denied → allowed
            if app.todos_dispositivos {
                // its direct access goes (the GPU stays): from now on the portal decides
                run(&f, &["override", "--user", "--nodevice=all", "--device=dri", id])?;
            }
            let v = match atual { Estado::Sim => "ask", Estado::Perguntar => "no", _ => "yes" };
            if v == "ask" {
                run(&f, &["permission-remove", "devices", "camera", id]).map(|_| ())
            } else {
                run(&f, &["permission-set", "devices", "camera", id, v]).map(|_| ())
            }
        }
        "local" => match atual {
            Estado::Sim => run(&f, &["permission-remove", "location", "location", id]).map(|_| ()),
            Estado::Perguntar => run(&f, &["permission-set", "location", "location", id, "NONE"]).map(|_| ()),
            _ => run(&f, &["permission-set", "location", "location", id, "EXACT", "0"]).map(|_| ()),
        },
        "microfone" => run(&f, &["override", "--user", if atual == Estado::Sim { "--nosocket=pulseaudio" } else { "--socket=pulseaudio" }, id]).map(|_| ()),
        "arquivos" => if atual == Estado::Sim {
            run(&f, &["override", "--user", "--nofilesystem=home", "--nofilesystem=host", id]).map(|_| ())
        } else {
            run(&f, &["override", "--user", "--filesystem=home", id]).map(|_| ())
        },
        "rede" => run(&f, &["override", "--user", if atual == Estado::Sim { "--unshare=network" } else { "--share=network" }, id]).map(|_| ()),
        _ => Ok(()),
    }
}

/// The agent's cells from its permissions' columns (kalango-agente perfil).
fn ler_agente() -> Option<[Estado; 6]> {
    let bin = crate::voz::agent_binary();
    let o = std::process::Command::new(&bin).arg("perfil").stdin(std::process::Stdio::null()).output().ok().filter(|o| o.status.success())?;
    let v: Value = serde_json::from_slice(&o.stdout).ok()?;
    let col = |id: &str| -> Estado {
        let c = v["colunas"].as_array().and_then(|a| a.iter().find(|c| c["id"] == id).cloned()).unwrap_or_default();
        match c["valor"].as_str() { Some("livre") => Estado::Sim, Some("nunca") => Estado::Nao, _ => Estado::Perguntar }
    };
    Some([Estado::NaoSeAplica, Estado::NaoSeAplica, col("tela"), Estado::NaoSeAplica, col("escrever"), col("rede")])
}

fn agente_coluna(col: usize) -> Option<&'static str> {
    match COLUNAS[col].0 {
        "tela" => Some("tela"),
        "arquivos" => Some("escrever"),
        "rede" => Some("rede"),
        _ => None,
    }
}

/// The security panel's reading.
#[derive(Default, Clone)]
struct Seguranca {
    criptografado: bool,
    firewall: Option<bool>,
    automaticas: bool,
    pendentes: usize,
}

fn ler_seguranca() -> Seguranca {
    // LUKS somewhere under the root's device
    let raiz = run("findmnt", &["-no", "SOURCE", "/"]).unwrap_or_default();
    let criptografado = !raiz.trim().is_empty() && run("lsblk", &["-s", "-no", "TYPE", raiz.trim()]).unwrap_or_default().lines().any(|l| l.trim() == "crypt");
    let firewall = crate::servicos::kalangod_chamar("firewall.estado", json!({})).ok().and_then(|v| v["ligado"].as_bool());
    let auto = std::fs::read_to_string("/etc/apt/apt.conf.d/20auto-upgrades").unwrap_or_default();
    let automaticas = auto.lines().any(|l| l.contains("Unattended-Upgrade") && l.contains("\"1\""));
    let pendentes = run("apt", &["list", "--upgradable"]).unwrap_or_default().lines().filter(|l| l.contains("-security")).count();
    Seguranca { criptografado, firewall, automaticas, pendentes }
}

/// How many days the agents' history stays (0: always).
pub fn dias_de_historico() -> u64 {
    crate::configuracoes::ajustes()["historico_agentes_dias"].as_u64().unwrap_or(30)
}

/// Keeps the agents' history for `dias` (0: always) and deletes what is older now.
pub fn definir_historico(dias: u64) {
    crate::configuracoes::salvar_ajuste("historico_agentes_dias", json!(dias));
    let n = podar_historico();
    log(&format!("histórico de {dias} dias ({n} conversas antigas apagadas)"));
}

/// Whether what the agents do is recorded for Kalango Train (it stays on this computer): `rastros.gravar`, on by default.
pub fn rastros_gravar() -> bool {
    crate::configuracoes::ajustes()["rastros"]["gravar"].as_bool().unwrap_or(true)
}

/// Whether the recording has the text too (`"completo"`, the default) or only names, times and counts (`"so_metadados"`).
pub fn rastros_completo() -> bool {
    crate::configuracoes::ajustes()["rastros"]["conteudo"].as_str().is_none_or(|c| c == "completo")
}

/// Saves the person's choice; the agent reads it at the start of every turn (traces.rs of kalango-agente).
pub fn definir_rastros(gravar: bool, completo: bool) {
    crate::configuracoes::salvar_ajuste("rastros", json!({"gravar": gravar, "conteudo": if completo { "completo" } else { "so_metadados" }}));
    log(&format!("rastros: gravar {gravar}, conteúdo {}", if completo { "completo" } else { "só metadados" }));
}

/// Whether the agents ask before using the cloud (the hybrid or local mode; the cloud mode does not).
pub fn pergunta_antes_da_nuvem() -> bool {
    crate::configuracoes::ajustes()["modo_agentes"].as_str().unwrap_or("hibrido") != "nuvem"
}

/// Asking before the cloud is the agents' hybrid mode (first use's choice); not asking, the cloud mode.
pub fn definir_pergunta_antes_da_nuvem(on: bool) {
    let atual = crate::configuracoes::ajustes()["modo_agentes"].as_str().unwrap_or("hibrido").to_string();
    let novo = if on { if atual == "local" { "local" } else { "hibrido" } } else { "nuvem" };
    crate::configuracoes::salvar_ajuste("modo_agentes", json!(novo));
    log(&format!("perguntar antes da nuvem {on} (modo {novo})"));
}

/// The agents' conversations older than the history's days are deleted (none when "always").
pub fn podar_historico() -> usize {
    let dias = crate::configuracoes::ajustes()["historico_agentes_dias"].as_u64().unwrap_or(30);
    if dias == 0 {
        return 0;
    }
    let limite = std::time::SystemTime::now() - std::time::Duration::from_secs(dias * 86_400);
    let dir = glib::user_data_dir().join("kalango/conversas");
    let mut n = 0;
    for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        let antiga = e.path().join("eventos.jsonl").metadata().or_else(|_| e.metadata()).and_then(|m| m.modified()).is_ok_and(|t| t < limite);
        if antiga && std::fs::remove_dir_all(e.path()).is_ok() {
            n += 1;
        }
    }
    n
}

/// "Apagar todos os dados de IA": the conversations, what the agent remembers and its usage records.
fn apagar_dados_ia() -> Result<(), String> {
    let _ = std::fs::remove_dir_all(glib::user_data_dir().join("kalango/conversas"));
    let bin = crate::voz::agent_binary();
    let o = std::process::Command::new(&bin).args(["memoria", "esquecer-tudo"]).stdin(std::process::Stdio::null()).output().map_err(|e| e.to_string())?;
    if !o.status.success() {
        return Err(String::from_utf8_lossy(&o.stderr).trim().to_string());
    }
    // the token counts (Agents › Usage)
    let _ = std::fs::remove_file(state_dir().join("kalango/uso-tokens.jsonl"));
    Ok(())
}

/// XDG_STATE_HOME or ~/.local/state.
fn state_dir() -> std::path::PathBuf {
    std::env::var_os("XDG_STATE_HOME").map(std::path::PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| glib::home_dir().join(".local/state"))
}

pub struct PrivacyPage {
    pub frame: Rc<Moldura>,
    loading: Cell<bool>,
    tabela: gtk::Box,
    apps: RefCell<Vec<App>>,
    agente: RefCell<Option<[Estado; 6]>>,
    disco: (gtk::Label, gtk::Label),
    firewall: (gtk::Label, gtk::Button),
    bloqueio: gtk::DropDown,
    atualizacoes: (gtk::Label, gtk::Label),
    historico: gtk::DropDown,
    nuvem: gtk::Switch,
    rastros: gtk::Switch,
    rastros_conteudo: (gtk::Label, gtk::DropDown),
    settings: std::rc::Weak<crate::configuracoes::Configuracoes>,
}

const BLOQUEIOS: &[u64] = &[0, 1, 5, 10, 15, 30];
const HISTORICOS: &[u64] = &[7, 30, 90, 0];

fn selo(t: &str, classe: &str) -> gtk::Label {
    let l = rotulo(t, "cfg-selo");
    l.add_css_class(classe);
    l.set_valign(Align::Center);
    l
}

fn linha(glifo: &str, titulo: &str, sub: &gtk::Label, controle: &impl IsA<gtk::Widget>) -> gtk::Box {
    let r = caixa(Orientation::Horizontal, 12, "cfg-tela-opcao");
    // (the data rows have no glyph in the design)
    if !glifo.is_empty() {
        let i = ui_icon(glifo, "#8E8F99", 16);
        i.set_valign(Align::Center);
        r.append(&i);
    }
    let t = caixa(Orientation::Vertical, 1, "");
    t.set_hexpand(true);
    t.set_valign(Align::Center);
    let tl = rotulo(titulo, "cfg-texto-13");
    tl.set_xalign(0.0);
    t.append(&tl);
    if !sub.text().is_empty() || sub.is_visible() {
        sub.add_css_class("cfg-sub");
        sub.set_xalign(0.0);
        sub.set_ellipsize(gtk::pango::EllipsizeMode::End);
        sub.set_max_width_chars(1);
        sub.set_hexpand(true);
        t.append(sub);
        r.add_css_class("alta");
    }
    r.append(&t);
    controle.set_valign(Align::Center);
    r.append(controle);
    r
}

fn seletor(itens: &[String], w: i32) -> gtk::DropDown {
    let d = gtk::DropDown::new(Some(gtk::StringList::new(&itens.iter().map(String::as_str).collect::<Vec<_>>())), None::<gtk::Expression>);
    d.add_css_class("cfg-seletor");
    d.set_size_request(w, 34);
    d
}

impl PrivacyPage {
    pub fn new(c: &Rc<crate::configuracoes::Configuracoes>) -> Rc<PrivacyPage> {
        let m = Moldura::new(tr("Privacy and security"), &[], tr("Security and data"));
        let cartao = caixa(Orientation::Vertical, 0, "cfg-cartao");
        cartao.append(&rotulo_cartao("escudo", tr("WHO CAN USE WHAT")));
        let tabela = caixa(Orientation::Vertical, 0, "cfg-perm-tabela");
        cartao.append(&tabela);
        let legenda = caixa(Orientation::Horizontal, 16, "cfg-perm-legenda");
        for (cls, t) in [("sim", tr("allowed")), ("perguntar", tr("always ask")), ("nao", tr("denied"))] {
            let b = caixa(Orientation::Horizontal, 6, "");
            let p = caixa(Orientation::Horizontal, 0, "cfg-perm-ponto");
            p.add_css_class(cls);
            p.set_size_request(6, 6);
            p.set_valign(Align::Center);
            b.append(&p);
            b.append(&rotulo(t, "cfg-sub"));
            legenda.append(&b);
        }
        cartao.append(&legenda);
        let nota = rotulo(tr("Apps installed as .deb packages (outside the App Center) run without isolation: the system cannot limit what they use."), "cfg-sub");
        nota.set_xalign(0.0);
        nota.set_wrap(true);
        nota.set_max_width_chars(110);
        nota.set_margin_top(6);
        cartao.append(&nota);
        m.corpo.append(&cartao);
        // the panel: security
        let seg = caixa(Orientation::Vertical, 2, "cfg-cartao");
        seg.append(&rotulo_cartao("cadeado-d", tr("SECURITY")));
        let disco_sub = gtk::Label::new(Some(" "));
        let disco_selo = selo(tr("inactive"), "nao");
        seg.append(&linha("cadeado", tr("Encrypted disk"), &disco_sub, &disco_selo));
        let fw_sub = gtk::Label::new(Some(tr("blocks incoming connections")));
        let fw = gtk::Button::new();
        fw.add_css_class("cfg-selo-botao");
        fw.set_child(Some(&selo(tr("active"), "sim")));
        seg.append(&linha("escudo", tr("Firewall"), &fw_sub, &fw));
        let nomes_b: Vec<String> = BLOQUEIOS.iter().map(|m| if *m == 0 { tr("never").to_string() } else { trf("after {n} min", &[("n", m.to_string())]) }).collect();
        let bloqueio = seletor(&nomes_b, 140);
        let bs = gtk::Label::new(None);
        bs.set_visible(false);
        seg.append(&linha("relogio", tr("Automatic lock"), &bs, &bloqueio));
        let at_sub = gtk::Label::new(Some(tr("installed by themselves")));
        let at_selo = selo(tr("current"), "sim");
        seg.append(&linha("download", tr("Security fixes"), &at_sub, &at_selo));
        m.painel.append(&seg);
        // the panel: data and AI
        let dados = caixa(Orientation::Vertical, 2, "cfg-cartao");
        dados.append(&rotulo_cartao("spark", tr("DATA AND AI")));
        let nomes_h: Vec<String> = HISTORICOS.iter().map(|d| if *d == 0 { tr("always").to_string() } else { trf("{n} days", &[("n", d.to_string())]) }).collect();
        let historico = seletor(&nomes_h, 140);
        let hs = gtk::Label::new(None);
        hs.set_visible(false);
        dados.append(&linha("", tr("Agents' history"), &hs, &historico));
        // KalangoOS has no telemetry at all: the switch says so and cannot be turned on
        let telemetria = interruptor();
        telemetria.set_sensitive(false);
        telemetria.set_tooltip_text(Some(tr("KalangoOS has no telemetry: nothing about how you use it is sent anywhere.")));
        let ts = gtk::Label::new(Some(tr("no usage data is sent")));
        dados.append(&linha("", tr("Telemetry"), &ts, &telemetria));
        let nuvem = interruptor();
        let ns = gtk::Label::new(None);
        ns.set_visible(false);
        dados.append(&linha("", tr("Ask before using the cloud"), &ns, &nuvem));
        let apagar = gtk::Button::new();
        apagar.add_css_class("cfg-botao-perigo");
        let ac = caixa(Orientation::Horizontal, 8, "");
        ac.set_halign(Align::Center);
        ac.append(&ui_icon("lixeira", "#EFA2A6", 14));
        ac.append(&rotulo(tr("Erase all the AI's data"), ""));
        apagar.set_child(Some(&ac));
        apagar.set_margin_top(8);
        dados.append(&apagar);
        m.painel.append(&dados);
        // the panel: privacy and traces (what the agents do is recorded for Kalango Train; it never leaves this computer)
        let rastros_card = caixa(Orientation::Vertical, 2, "cfg-cartao");
        rastros_card.append(&rotulo_cartao("escudo", tr("PRIVACY AND TRACES")));
        let rastros = interruptor();
        let rs = gtk::Label::new(Some(tr("Stays only on this computer")));
        rastros_card.append(&linha("", tr("Record what the agents do"), &rs, &rastros));
        let nomes_r = vec![tr("Everything").to_string(), tr("Only metadata").to_string()];
        let rastros_sel = seletor(&nomes_r, 140);
        let rcs = gtk::Label::new(Some(" "));
        rastros_card.append(&linha("", tr("What is recorded"), &rcs, &rastros_sel));
        m.painel.append(&rastros_card);
        let page = Rc::new(PrivacyPage {
            frame: m, loading: Cell::new(false), tabela, apps: RefCell::default(), agente: RefCell::default(),
            disco: (disco_sub, disco_selo), firewall: (fw_sub, fw.clone()), bloqueio, atualizacoes: (at_sub, at_selo), historico, nuvem, rastros, rastros_conteudo: (rcs, rastros_sel), settings: Rc::downgrade(c),
        });
        crate::configuracoes::registrar_posicao(c, "cfg priv firewall", fw.upcast_ref());
        crate::configuracoes::registrar_posicao(c, "cfg priv bloqueio", page.bloqueio.upcast_ref());
        crate::configuracoes::registrar_posicao(c, "cfg priv historico", page.historico.upcast_ref());
        crate::configuracoes::registrar_posicao(c, "cfg priv nuvem", page.nuvem.upcast_ref());
        crate::configuracoes::registrar_posicao(c, "cfg priv rastros", page.rastros.upcast_ref());
        crate::configuracoes::registrar_posicao(c, "cfg priv rastros conteudo", page.rastros_conteudo.1.upcast_ref());
        crate::configuracoes::registrar_posicao(c, "cfg priv apagar", apagar.upcast_ref());
        page.ligar(&fw, &apagar);
        page
    }

    fn ligar(self: &Rc<Self>, fw: &gtk::Button, apagar: &gtk::Button) {
        let w = Rc::downgrade(self);
        fw.connect_clicked(move |_| {
            let Some(p) = w.upgrade() else { return };
            let ligar = !p.firewall.1.has_css_class("ligado");
            let w2 = Rc::downgrade(&p);
            crate::lateral::em_segundo_plano(move || crate::servicos::kalangod_chamar("firewall.ligar", json!({"ligado": ligar})), move |r| {
                log(&format!("firewall {ligar} ({})", if r.is_ok() { "pedido aprovado" } else { "recusado" }));
                if let Some(p) = w2.upgrade() {
                    p.mostrar();
                }
            });
        });
        let w = Rc::downgrade(self);
        self.bloqueio.connect_selected_notify(move |d| {
            if w.upgrade().is_some_and(|p| !p.loading.get()) {
                let m = BLOQUEIOS.get(d.selected() as usize).copied().unwrap_or(5);
                crate::configuracoes::salvar_ajuste("bloqueio_apos_min", json!(m));
                crate::configuracoes_energia::watch_idle();
                log(&format!("bloqueio após {m} min"));
            }
        });
        let w = Rc::downgrade(self);
        self.historico.connect_selected_notify(move |d| {
            if w.upgrade().is_some_and(|p| !p.loading.get()) {
                definir_historico(HISTORICOS.get(d.selected() as usize).copied().unwrap_or(30));
            }
        });
        let w = Rc::downgrade(self);
        self.nuvem.connect_state_set(move |_, on| {
            if w.upgrade().is_some_and(|p| !p.loading.get()) {
                definir_pergunta_antes_da_nuvem(on);
            }
            glib::Propagation::Proceed
        });
        let w = Rc::downgrade(self);
        self.rastros.connect_state_set(move |_, on| {
            if let Some(p) = w.upgrade().filter(|p| !p.loading.get()) {
                definir_rastros(on, rastros_completo());
                p.mostrar_rastros();
            }
            glib::Propagation::Proceed
        });
        let w = Rc::downgrade(self);
        self.rastros_conteudo.1.connect_selected_notify(move |d| {
            if let Some(p) = w.upgrade().filter(|p| !p.loading.get()) {
                definir_rastros(rastros_gravar(), d.selected() == 0);
                p.mostrar_rastros();
            }
        });
        let w = Rc::downgrade(self);
        apagar.connect_clicked(move |b| {
            let Some(p) = w.upgrade() else { return };
            // a second click confirms (the button says what it will do)
            if !b.has_css_class("confirmar") {
                b.add_css_class("confirmar");
                if let Some(l) = b.child().and_then(|c| c.last_child()).and_downcast::<gtk::Label>() {
                    l.set_text(tr("Click again: conversations, memories and usage go"));
                }
                let bw = b.downgrade();
                glib::timeout_add_seconds_local_once(5, move || {
                    if let Some(b) = bw.upgrade() {
                        b.remove_css_class("confirmar");
                        if let Some(l) = b.child().and_then(|c| c.last_child()).and_downcast::<gtk::Label>() {
                            l.set_text(tr("Erase all the AI's data"));
                        }
                    }
                });
                return;
            }
            b.remove_css_class("confirmar");
            let r = apagar_dados_ia();
            log(&format!("dados de IA apagados ({})", r.as_ref().map(|_| "ok".to_string()).unwrap_or_else(|e| e.clone())));
            if let Some(l) = b.child().and_then(|c| c.last_child()).and_downcast::<gtk::Label>() {
                l.set_text(if r.is_ok() { tr("Erased") } else { tr("Could not erase everything") });
            }
            let _ = p;
        });
    }

    /// Reads everything again (off the main thread) and fills the page.
    pub fn mostrar(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(|| (ler_apps(), ler_agente(), ler_seguranca()), move |(apps, agente, seg)| {
            let Some(p) = w.upgrade() else { return };
            *p.apps.borrow_mut() = apps;
            *p.agente.borrow_mut() = agente;
            p.preencher_tabela();
            p.preencher_painel(&seg);
        });
    }

    /// The traces card from the saved choice: the switch, the content, and the line that says what is being recorded
    /// (paused: the content choice is dimmed).
    fn mostrar_rastros(&self) {
        let antes = self.loading.replace(true);
        let (gravar, completo) = (rastros_gravar(), rastros_completo());
        self.rastros.set_active(gravar);
        self.rastros.set_state(gravar);
        self.rastros_conteudo.1.set_selected(if completo { 0 } else { 1 });
        self.rastros_conteudo.1.set_sensitive(gravar);
        self.rastros_conteudo.0.set_text(if !gravar { tr("Nothing is recorded") } else if completo { tr("Prompts, answers and actions are recorded") } else { tr("Only names, times and counts are recorded") });
        self.loading.set(antes);
    }

    fn preencher_painel(&self, s: &Seguranca) {
        self.loading.set(true);
        // the line under the title: protected or what to look at; this build sends nothing to any cloud
        let revisar = [!s.criptografado, s.firewall != Some(true), !s.automaticas || s.pendentes > 0].iter().filter(|x| **x).count();
        self.frame.sub.set_text(&if revisar == 0 {
            format!("{} · {}", tr("All protected"), tr("nothing sent to the cloud today"))
        } else {
            format!("{} · {}", trf("{n} things to look at", &[("n", revisar.to_string())]), tr("nothing sent to the cloud today"))
        });
        self.disco.0.set_text(if s.criptografado { tr("LUKS2 · asks for the password at start") } else { tr("chosen at the installation") });
        self.disco.1.set_text(if s.criptografado { tr("active") } else { tr("inactive") });
        for c in ["sim", "nao"] {
            self.disco.1.remove_css_class(c);
        }
        self.disco.1.add_css_class(if s.criptografado { "sim" } else { "nao" });
        if let Some(l) = self.firewall.1.child().and_downcast::<gtk::Label>() {
            for c in ["sim", "nao"] {
                l.remove_css_class(c);
            }
            match s.firewall {
                Some(on) => {
                    l.set_text(if on { tr("active") } else { tr("inactive") });
                    l.add_css_class(if on { "sim" } else { "nao" });
                    if on { self.firewall.1.add_css_class("ligado") } else { self.firewall.1.remove_css_class("ligado") }
                    self.firewall.0.set_text(if on { tr("blocks incoming connections") } else { tr("incoming connections are not blocked") });
                }
                None => {
                    l.set_text(tr("unknown"));
                    l.add_css_class("nao");
                    self.firewall.0.set_text(tr("could not read ufw"));
                }
            }
        }
        self.atualizacoes.0.set_text(if s.automaticas { tr("installed by themselves") } else { tr("not installed by themselves") });
        for c in ["sim", "perguntar", "nao"] {
            self.atualizacoes.1.remove_css_class(c);
        }
        let (t, c) = match (s.automaticas, s.pendentes) {
            (_, 0) if s.automaticas => (tr("current").to_string(), "sim"),
            (_, 0) => (tr("inactive").to_string(), "nao"),
            (_, n) => (trf("{n} pending", &[("n", n.to_string())]), "perguntar"),
        };
        self.atualizacoes.1.set_text(&t);
        self.atualizacoes.1.add_css_class(c);
        let a = crate::configuracoes::ajustes();
        let dev = std::env::var_os("KALANGO_SHELL_DEV").is_some();
        let m = a["bloqueio_apos_min"].as_u64().unwrap_or(if dev { 0 } else { 5 });
        self.bloqueio.set_selected(BLOQUEIOS.iter().position(|x| *x == m).unwrap_or(2) as u32);
        let d = dias_de_historico();
        self.historico.set_selected(HISTORICOS.iter().position(|x| *x == d).unwrap_or(1) as u32);
        let pergunta = pergunta_antes_da_nuvem();
        self.nuvem.set_active(pergunta);
        self.nuvem.set_state(pergunta);
        self.mostrar_rastros();
        self.loading.set(false);
        if depurando() {
            eprintln!("debug privacidade: segurança (disco {}, firewall {:?}, automáticas {}, {} pendentes)", s.criptografado, s.firewall, s.automaticas, s.pendentes);
        }
    }

    fn preencher_tabela(self: &Rc<Self>) {
        while let Some(x) = self.tabela.first_child() {
            self.tabela.remove(&x);
        }
        // the header: the columns' glyphs and names
        let cab = caixa(Orientation::Horizontal, 0, "cfg-perm-cabecalho");
        let ql = rotulo(tr("App or agent"), "cfg-sub");
        ql.set_xalign(0.0);
        ql.set_hexpand(true);
        ql.set_ellipsize(gtk::pango::EllipsizeMode::End);
        ql.set_max_width_chars(1);
        cab.append(&ql);
        for (_, glifo, nome) in COLUNAS {
            let col = caixa(Orientation::Vertical, 4, "");
            col.set_size_request(92, -1);
            let i = ui_icon(glifo, "#8E8F99", 14);
            i.set_halign(Align::Center);
            col.append(&i);
            // (a column's name wraps inside its 92 px instead of widening the table in German or Russian)
            let nl = rotulo(tr(nome), "cfg-sub");
            nl.set_wrap(true);
            nl.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            nl.set_max_width_chars(10);
            nl.set_justify(gtk::Justification::Center);
            col.append(&nl);
            cab.append(&col);
        }
        self.tabela.append(&cab);
        let apps = self.apps.borrow().clone();
        let agente = *self.agente.borrow();
        let mut n = 0;
        for app in &apps {
            self.linha(&app.nome, tr("isolated app (Flatpak)"), "pasta", app.estados, Some(app.clone()));
            n += 1;
        }
        if let Some(e) = agente {
            self.linha(tr("Agent"), tr("agent · local model"), "spark", e, None);
            n += 1;
        }
        if n == 0 {
            let l = rotulo(tr("No isolated app is installed yet."), "cfg-sub");
            l.set_margin_top(10);
            self.tabela.append(&l);
        }
        if depurando() {
            eprintln!("debug privacidade: {} apps isolados{}", apps.len(), if agente.is_some() { " e o agente" } else { "" });
        }
    }

    /// A row of the table: its name and kind, and a cell per column.
    fn linha(self: &Rc<Self>, nome: &str, tipo: &str, glifo: &str, estados: [Estado; 6], app: Option<App>) {
        let r = caixa(Orientation::Horizontal, 0, "cfg-perm-linha");
        let quem = caixa(Orientation::Horizontal, 10, "");
        quem.set_hexpand(true);
        let ic = caixa(Orientation::Horizontal, 0, "cfg-perm-icone");
        ic.set_size_request(26, 26);
        ic.set_homogeneous(true);
        ic.set_valign(Align::Center);
        // the app's own icon (as the dock and the grid show it), else the row's glyph
        let todos = crate::apps::todos();
        match app.as_ref().and_then(|a| crate::apps::da_janela(&a.id, &todos)) {
            Some(a) => ic.append(&crate::apps::imagem(a, 22)),
            None => ic.append(&ui_icon(glifo, "#A5A1E2", 14)),
        }
        quem.append(&ic);
        let t = caixa(Orientation::Vertical, 1, "");
        t.set_valign(Align::Center);
        let nl = rotulo(nome, "cfg-texto-13");
        nl.set_xalign(0.0);
        nl.set_ellipsize(gtk::pango::EllipsizeMode::End);
        nl.set_max_width_chars(1);
        nl.set_hexpand(true);
        t.append(&nl);
        let tl = rotulo(tipo, "cfg-sub");
        tl.set_xalign(0.0);
        tl.set_ellipsize(gtk::pango::EllipsizeMode::End);
        tl.set_max_width_chars(1);
        t.append(&tl);
        t.set_hexpand(true);
        quem.append(&t);
        r.append(&quem);
        for (k, e) in estados.iter().enumerate() {
            let cel = caixa(Orientation::Horizontal, 0, "");
            cel.set_size_request(92, -1);
            let b = gtk::Button::new();
            b.add_css_class("cfg-perm-celula");
            b.add_css_class(e.classe());
            b.set_halign(Align::Center);
            b.set_valign(Align::Center);
            let simbolo = match e { Estado::Sim => "✓", Estado::Perguntar => "?", _ => "–" };
            b.set_label(simbolo);
            // what cannot change here: the screen (always asked by its portal) and what an agent does not use
            let fixo = *e == Estado::NaoSeAplica || (app.is_some() && COLUNAS[k].0 == "tela") || (app.is_none() && agente_coluna(k).is_none());
            b.set_sensitive(!fixo);
            if !fixo {
                let (w, app, nome) = (Rc::downgrade(self), app.clone(), nome.to_string());
                b.connect_clicked(move |_| {
                    let Some(p) = w.upgrade() else { return };
                    let app = app.clone();
                    let nome = nome.clone();
                    let w2 = Rc::downgrade(&p);
                    crate::lateral::em_segundo_plano(move || match &app {
                        Some(a) => mudar_app(a, k),
                        None => {
                            let col = agente_coluna(k).unwrap_or("");
                            let proximo = match estados[k] { Estado::Sim => "perguntar", Estado::Perguntar => "nunca", _ => "livre" };
                            let bin = crate::voz::agent_binary();
                            std::process::Command::new(&bin).args(["coluna", col, proximo]).output().map_err(|e| e.to_string())
                                .and_then(|o| if o.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&o.stderr).trim().to_string()) })
                        }
                    }, move |r| {
                        log(&format!("{nome} · {} ({})", COLUNAS[k].0, r.as_ref().map(|_| "mudou".to_string()).unwrap_or_else(|e| e.clone())));
                        if let Some(p) = w2.upgrade() {
                            p.mostrar();
                        }
                    });
                });
            }
            if let Some(c) = self.settings.upgrade() {
                crate::configuracoes::registrar_posicao(&c, &format!("cfg priv {} {}", nome, COLUNAS[k].0), b.upcast_ref());
            }
            cel.append(&b);
            r.append(&cel);
        }
        self.tabela.append(&r);
    }

    pub fn compactar(&self, c: bool) {
        self.frame.compactar(c);
    }
}

fn log(s: &str) {
    if depurando() {
        eprintln!("debug privacidade: {s}");
    }
}
