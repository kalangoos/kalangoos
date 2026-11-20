//! Configurações › Rede (design "Configurações · Rede"): the networks around and the one in use, through
//! NetworkManager (nmcli, as the person: the same permissions as the desktop's settings). Wi-Fi: each network a card —
//! its signal, whether it is protected, connected, saved or open —, and the chosen one in the panel: its state, address,
//! speed and security, connecting by itself, metered, and whether the agents may use it (kalangod keeps that and its
//! web tool follows it), and forgetting it. Ethernet, VPN, proxy and firewall in their tabs (the firewall through
//! kalangod, with the person's approval).

use crate::configuracoes::{cartao, depurando, interruptor, rotulo_cartao};
use crate::configuracoes_moldura::{linha_interruptor, linha_valor, Moldura};
use crate::conversa::ui_icon_fitted;
use crate::superficies::{caixa, rotulo};
use gtk::prelude::*;
use gtk::{gio, glib, Align, Orientation};
use kalango_i18n::{n_, tr, trf};
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

const ABAS: &[(&str, &str)] = &[("wifi", n_("Wi-Fi")), ("ethernet", n_("Ethernet")), ("vpn", n_("VPN")), ("proxy", n_("Proxy")), ("firewall", n_("Firewall"))];

/// NetworkManager's command (a stand-in in the session test).
pub fn nmcli() -> String {
    std::env::var("KALANGO_NMCLI").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "nmcli".into())
}

fn nm(args: &[&str]) -> Result<String, String> {
    let o = crate::sistema::output_timeout(std::process::Command::new(nmcli()).args(args).env("LC_ALL", "C"), crate::sistema::STATUS_LIMIT)
        .map_err(|e| e.to_string())?;
    if o.status.success() {
        Ok(String::from_utf8_lossy(&o.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&o.stderr).trim().to_string())
    }
}

/// nmcli's terse lines: fields split on ":" (a "\:" is a colon inside a value).
pub fn campos(linha: &str) -> Vec<String> {
    let mut v = vec![String::new()];
    let mut esc = false;
    for c in linha.chars() {
        match (esc, c) {
            (true, _) => {
                v.last_mut().unwrap().push(c);
                esc = false;
            }
            (false, '\\') => esc = true,
            (false, ':') => v.push(String::new()),
            (false, _) => v.last_mut().unwrap().push(c),
        }
    }
    v
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Wifi {
    pub ssid: String,
    pub sinal: u8,
    pub seguranca: String,
    pub taxa: String,
    pub freq: u32,
    pub em_uso: bool,
    /// its saved connection's name
    pub salva: Option<String>,
}

#[derive(Clone, Debug, Default)]
struct Conexao {
    nome: String,
    tipo: String,
    autoconnect: bool,
    ativa: bool,
}

#[derive(Clone, Debug, Default)]
struct Cabo {
    dispositivo: String,
    estado: String,
    conexao: String,
    /// a container's, a virtual machine's or a bridge's interface, not a port of this computer
    virtual_: bool,
}

/// An interface of a container, a virtual machine or a bridge (veth*, docker*, br-*, virbr*…): where the system knows
/// it, one with no device behind it; else by its name.
fn interface_virtual(dev: &str) -> bool {
    let sys = std::path::Path::new("/sys/class/net").join(dev);
    if sys.exists() {
        return !sys.join("device").exists();
    }
    ["veth", "docker", "br-", "virbr", "vnet", "tun", "tap", "wg", "lo", "vboxnet", "vmnet", "cni", "flannel", "podman"].iter().any(|p| dev.starts_with(p))
}

#[derive(Clone, Debug, Default)]
struct Leitura {
    wifi_ligado: bool,
    redes: Vec<Wifi>,
    conexoes: Vec<Conexao>,
    cabos: Vec<Cabo>,
    wifi_dispositivo: String,
    ip: HashMap<String, String>,
    limitada: HashMap<String, bool>,
    sem_agentes: Vec<String>,
}

/// The visible networks (one card each, the strongest of a name), with the saved ones' names.
pub fn ler_redes(lista: &str, salvas: &HashMap<String, String>) -> Vec<Wifi> {
    let mut v: Vec<Wifi> = Vec::new();
    for l in lista.lines() {
        let f = campos(l);
        if f.len() < 6 || f[1].is_empty() {
            continue;
        }
        let w = Wifi {
            ssid: f[1].clone(),
            sinal: f[2].parse().unwrap_or(0),
            seguranca: f[3].trim().to_string(),
            taxa: f[4].replace("Mbit/s", "Mb/s").trim().to_string(),
            freq: f[5].trim_end_matches(" MHz").trim().parse().unwrap_or(0),
            em_uso: f[0] == "*",
            salva: salvas.get(&f[1]).cloned(),
        };
        match v.iter_mut().find(|x| x.ssid == w.ssid) {
            Some(x) if w.em_uso || (!x.em_uso && w.sinal > x.sinal) => *x = w,
            Some(_) => {}
            None => v.push(w),
        }
    }
    // in use first, then the saved ones, then by signal
    v.sort_by_key(|w| (!w.em_uso, w.salva.is_none(), 255 - w.sinal as i32));
    v
}

fn ler() -> Leitura {
    let mut l = Leitura { wifi_ligado: nm(&["-t", "-f", "WIFI", "radio"]).map(|s| s.trim() == "enabled").unwrap_or(false), ..Default::default() };
    for linha in nm(&["-t", "-f", "NAME,TYPE,AUTOCONNECT,ACTIVE,DEVICE", "connection", "show"]).unwrap_or_default().lines() {
        let f = campos(linha);
        if f.len() >= 5 {
            l.conexoes.push(Conexao { nome: f[0].clone(), tipo: f[1].clone(), autoconnect: f[2] == "yes", ativa: f[3] == "yes" });
        }
    }
    // a saved Wi-Fi connection's network (its name may differ from it)
    let mut salvas = HashMap::new();
    for c in l.conexoes.iter().filter(|c| c.tipo == "802-3-ethernet") {
        let m = nm(&["-g", "connection.metered", "connection", "show", &c.nome]).map(|s| s.trim() == "yes").unwrap_or(false);
        l.limitada.insert(c.nome.clone(), m);
    }
    for c in l.conexoes.iter().filter(|c| c.tipo == "802-11-wireless") {
        let ssid = nm(&["-g", "802-11-wireless.ssid", "connection", "show", &c.nome]).map(|s| s.trim().to_string()).unwrap_or_default();
        salvas.insert(if ssid.is_empty() { c.nome.clone() } else { ssid }, c.nome.clone());
        let m = nm(&["-g", "connection.metered", "connection", "show", &c.nome]).map(|s| s.trim() == "yes").unwrap_or(false);
        l.limitada.insert(c.nome.clone(), m);
    }
    if l.wifi_ligado {
        // ("auto": NetworkManager scans again when its list is older than 30 s)
        l.redes = ler_redes(&nm(&["-t", "-f", "IN-USE,SSID,SIGNAL,SECURITY,RATE,FREQ", "device", "wifi", "list", "--rescan", "auto"]).unwrap_or_default(), &salvas);
    }
    for linha in nm(&["-t", "-f", "DEVICE,TYPE,STATE,CONNECTION", "device"]).unwrap_or_default().lines() {
        let f = campos(linha);
        if f.len() < 4 {
            continue;
        }
        match f[1].as_str() {
            "wifi" => l.wifi_dispositivo = f[0].clone(),
            "ethernet" | "veth" | "bridge" => l.cabos.push(Cabo { dispositivo: f[0].clone(), estado: f[2].clone(), conexao: f[3].clone(),
                virtual_: f[1] != "ethernet" || interface_virtual(&f[0]) }),
            _ => {}
        }
        if f[2].starts_with("connected") {
            let ip = nm(&["-g", "IP4.ADDRESS", "device", "show", &f[0]]).unwrap_or_default();
            l.ip.insert(f[0].clone(), ip.lines().next().unwrap_or("").split('/').next().unwrap_or("").to_string());
        }
    }
    l.sem_agentes = crate::servicos::kalangod_rpc("kalango/redes_agentes", json!({})).ok()
        .and_then(|v| v["sem_agentes"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())).unwrap_or_default();
    l
}

/// A cable's state in words.
fn estado_do_cabo(c: &Cabo) -> &'static str {
    if c.estado.starts_with("connected") {
        tr("Connected")
    } else if c.estado.contains("unavailable") {
        tr("No cable")
    } else if c.estado.contains("unmanaged") {
        tr("Managed by another program")
    } else {
        tr("Disconnected")
    }
}

/// "WPA3", "WPA2", "Aberta" from nmcli's security field.
fn seguranca(s: &str) -> String {
    if s.is_empty() || s == "--" {
        tr("Open network").to_string()
    } else if s.contains("WPA3") || s.contains("SAE") {
        "WPA3".into()
    } else if s.contains("WPA2") {
        "WPA2".into()
    } else if s.contains("WPA") {
        "WPA".into()
    } else {
        s.to_string()
    }
}

fn banda(freq: u32) -> &'static str {
    match freq {
        5925.. => "6 GHz",
        4900..=5924 => "5 GHz",
        1..=4899 => "2,4 GHz",
        _ => "",
    }
}

/// When the shell first saw each connection active (NetworkManager does not keep it): "Conectado há 3 h".
fn desde_path() -> std::path::PathBuf {
    std::env::var_os("XDG_STATE_HOME").map(std::path::PathBuf::from).filter(|p| p.is_absolute())
        .unwrap_or_else(|| glib::home_dir().join(".local/state")).join("kalango/rede-desde.json")
}

fn conectada_desde(nome: &str) -> i64 {
    let agora = glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0);
    let mut v: Value = std::fs::read(desde_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_else(|| json!({}));
    if let Some(t) = v[nome].as_i64() {
        return t;
    }
    v[nome] = json!(agora);
    if let Some(d) = desde_path().parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(desde_path(), v.to_string());
    agora
}

fn esquecer_desde(nome: &str) {
    let mut v: Value = std::fs::read(desde_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_else(|| json!({}));
    if let Some(o) = v.as_object_mut() {
        o.remove(nome);
    }
    let _ = std::fs::write(desde_path(), v.to_string());
}

pub(crate) fn ha(segundos: i64) -> String {
    match segundos {
        ..=59 => tr("just now").to_string(),
        60..=3599 => trf("{n} min ago", &[("n", (segundos / 60).to_string())]),
        3600..=86_399 => trf("{n} h ago", &[("n", (segundos / 3600).to_string())]),
        _ => trf("{n} days ago", &[("n", (segundos / 86_400).to_string())]),
    }
}

/// The signal's four bars (the connected network's in teal).
fn barras(sinal: u8, conectada: bool) -> gtk::Box {
    let b = caixa(Orientation::Horizontal, 2, "");
    b.set_valign(Align::Center);
    let n = match sinal { 75.. => 4, 50..=74 => 3, 25..=49 => 2, _ => 1 };
    for (k, h) in [4, 7, 10, 13].into_iter().enumerate() {
        let x = caixa(Orientation::Vertical, 0, "cfg-barra-rede");
        x.set_size_request(3, h);
        x.set_valign(Align::End);
        if k < n {
            x.add_css_class(if conectada { "verde" } else { "ativa" });
        }
        b.append(&x);
    }
    b
}

pub struct Rede {
    pub moldura: Rc<Moldura>,
    corpo: gtk::Stack,
    painel: gtk::Stack,
    // Wi-Fi
    grade: gtk::FlowBox,
    desligado: gtk::Box,
    p_nome: gtk::Label,
    p_estado: gtk::Label,
    p_ip: gtk::Label,
    p_ip_linha: gtk::Box,
    p_vel: gtk::Label,
    p_seg: gtk::Label,
    p_auto: gtk::Switch,
    p_auto_linha: gtk::Box,
    p_limitada: gtk::Switch,
    p_limitada_linha: gtk::Box,
    p_agentes: gtk::Switch,
    p_agentes_linha: gtk::Box,
    p_conectar: gtk::Button,
    p_esquecer: gtk::Button,
    p_erro: gtk::Label,
    escolhida: RefCell<Option<String>>,
    /// the cable chosen in the Ethernet tab (its device), and whether the virtual interfaces show
    cabo_escolhido: RefCell<Option<String>>,
    virtuais: Cell<bool>,
    // Ethernet, VPN
    cabos: gtk::Box,
    vpns: gtk::Box,
    e_info: gtk::Box,
    v_info: gtk::Box,
    // proxy
    proxy_modos: Vec<(&'static str, gtk::ToggleButton)>,
    proxy_host: gtk::Entry,
    proxy_porta: gtk::Entry,
    proxy_url: gtk::Entry,
    proxy_manual: gtk::Box,
    proxy_auto: gtk::Box,
    // firewall
    fw_sw: gtk::Switch,
    fw_estado: gtk::Label,
    fw_regras: gtk::Box,
    fw_porta: gtk::Entry,
    leitura: RefCell<Option<Leitura>>,
    lendo: Cell<bool>,
    ocupado: Rc<Cell<bool>>,
    timer: RefCell<Option<glib::SourceId>>,
}

fn rotulo_esq(t: &str, c: &str) -> gtk::Label {
    let l = rotulo(t, c);
    l.set_xalign(0.0);
    l
}

impl Rede {
    pub fn new(c: &Rc<crate::configuracoes::Configuracoes>) -> Rc<Rede> {
        let m = Moldura::new(tr("Network"), ABAS, tr("Selected network"));
        let corpo = gtk::Stack::new();
        corpo.set_vhomogeneous(false);
        corpo.set_hhomogeneous(false);
        m.corpo.append(&corpo);
        let painel = gtk::Stack::new();
        painel.set_vhomogeneous(false);
        painel.set_hhomogeneous(false);
        m.painel.append(&painel);
        // ---- Wi-Fi: the cards
        let wb = caixa(Orientation::Vertical, 14, "");
        let grade = gtk::FlowBox::new();
        grade.set_selection_mode(gtk::SelectionMode::None);
        grade.add_css_class("cfg-grade");
        grade.set_column_spacing(14);
        grade.set_row_spacing(14);
        grade.set_min_children_per_line(1);
        // (as many columns as fit, each card at least its width: not three on a wide screen)
        grade.set_max_children_per_line(24);
        grade.set_homogeneous(true);
        grade.set_hexpand(true);
        grade.set_valign(Align::Start);
        // Enter on a card (the flow box's child has the keyboard) is the card's click
        grade.connect_child_activated(|_, ch| if let Some(b) = ch.child().and_downcast::<gtk::Button>() { b.emit_clicked() });
        wb.append(&grade);
        let desligado = caixa(Orientation::Vertical, 10, "cfg-cartao");
        desligado.set_halign(Align::Start);
        desligado.append(&rotulo_esq(tr("Wi-Fi is off"), "cfg-valor"));
        let ligar = gtk::Button::with_label(tr("Turn Wi-Fi on"));
        ligar.add_css_class("cfg-botao-principal");
        ligar.set_halign(Align::Start);
        desligado.append(&ligar);
        desligado.set_visible(false);
        wb.append(&desligado);
        corpo.add_named(&wb, Some("wifi"));
        // ---- Wi-Fi: the panel
        let pw = caixa(Orientation::Vertical, 12, "");
        pw.append(&rotulo_cartao("wifi", tr("SELECTED NETWORK")));
        let p_nome = rotulo_esq("", "cfg-rede-nome");
        p_nome.set_ellipsize(gtk::pango::EllipsizeMode::End);
        pw.append(&p_nome);
        let (l, p_estado) = linha_valor(tr("State"));
        pw.append(&l);
        let (p_ip_linha, p_ip) = linha_valor(tr("IP address"));
        pw.append(&p_ip_linha);
        let (l, p_vel) = linha_valor(tr("Speed"));
        pw.append(&l);
        let (l, p_seg) = linha_valor(tr("Security"));
        pw.append(&l);
        let div = caixa(Orientation::Horizontal, 0, "cfg-div");
        div.set_size_request(-1, 1);
        pw.append(&div);
        let (p_auto_linha, p_auto) = linha_interruptor(tr("Connect automatically"));
        pw.append(&p_auto_linha);
        let (p_limitada_linha, p_limitada) = linha_interruptor(tr("Metered network (save data)"));
        pw.append(&p_limitada_linha);
        let (p_agentes_linha, p_agentes) = linha_interruptor(tr("Allow agents to use this network"));
        pw.append(&p_agentes_linha);
        let p_erro = rotulo_esq("", "cfg-erro");
        p_erro.set_wrap(true);
        p_erro.set_visible(false);
        pw.append(&p_erro);
        let p_conectar = gtk::Button::with_label(tr("Connect"));
        p_conectar.add_css_class("cfg-botao-principal");
        p_conectar.add_css_class("largo");
        pw.append(&p_conectar);
        let p_esquecer = gtk::Button::with_label(tr("Forget network"));
        p_esquecer.add_css_class("cfg-botao-perigo");
        pw.append(&p_esquecer);
        painel.add_named(&pw, Some("wifi"));
        // ---- Ethernet
        let cabos = caixa(Orientation::Vertical, 14, "");
        corpo.add_named(&cabos, Some("ethernet"));
        let e_info = caixa(Orientation::Vertical, 12, "");
        painel.add_named(&e_info, Some("ethernet"));
        // ---- VPN
        let vb = caixa(Orientation::Vertical, 14, "");
        let vpns = caixa(Orientation::Vertical, 10, "");
        vb.append(&vpns);
        let importar = gtk::Button::with_label(tr("Import a VPN…"));
        importar.add_css_class("cfg-botao");
        importar.set_halign(Align::Start);
        vb.append(&importar);
        corpo.add_named(&vb, Some("vpn"));
        let v_info = caixa(Orientation::Vertical, 12, "");
        painel.add_named(&v_info, Some("vpn"));
        // ---- proxy
        let pc = cartao("");
        pc.set_valign(Align::Start);
        pc.append(&rotulo_esq(tr("PROXY"), "cfg-rotulo"));
        let seg = caixa(Orientation::Horizontal, 2, "cfg-segmentado");
        seg.set_homogeneous(true);
        let mut proxy_modos: Vec<(&'static str, gtk::ToggleButton)> = Vec::new();
        for (id, nome) in [("none", tr("None")), ("manual", tr("Manual")), ("auto", tr("Automatic"))] {
            let b = gtk::ToggleButton::with_label(nome);
            b.add_css_class("cfg-seg");
            if let Some((_, p)) = proxy_modos.first() {
                b.set_group(Some(p));
            }
            seg.append(&b);
            proxy_modos.push((id, b));
        }
        pc.append(&seg);
        let proxy_manual = caixa(Orientation::Horizontal, 8, "");
        let proxy_host = gtk::Entry::new();
        proxy_host.add_css_class("cfg-campo");
        proxy_host.set_placeholder_text(Some(tr("Address (proxy.example.com)")));
        proxy_host.set_hexpand(true);
        proxy_manual.append(&proxy_host);
        let proxy_porta = gtk::Entry::new();
        proxy_porta.add_css_class("cfg-campo");
        proxy_porta.set_placeholder_text(Some(tr("Port")));
        proxy_porta.set_width_chars(6);
        proxy_manual.append(&proxy_porta);
        pc.append(&proxy_manual);
        let proxy_auto = caixa(Orientation::Horizontal, 8, "");
        let proxy_url = gtk::Entry::new();
        proxy_url.add_css_class("cfg-campo");
        proxy_url.set_placeholder_text(Some(tr("Configuration address (PAC)")));
        proxy_url.set_hexpand(true);
        proxy_auto.append(&proxy_url);
        pc.append(&proxy_auto);
        let salvar_proxy = gtk::Button::with_label(tr("Apply"));
        salvar_proxy.add_css_class("cfg-botao-principal");
        salvar_proxy.set_halign(Align::Start);
        pc.append(&salvar_proxy);
        corpo.add_named(&pc, Some("proxy"));
        let pp = caixa(Orientation::Vertical, 12, "");
        let ajuda = rotulo_esq(tr("Apps and the system's downloads go through the proxy. The agents' web reading does not: kalangod reaches pages directly, and refuses private addresses either way."), "cfg-sub");
        ajuda.set_wrap(true);
        pp.append(&ajuda);
        painel.add_named(&pp, Some("proxy"));
        // ---- firewall
        let fc = cartao("");
        fc.set_valign(Align::Start);
        let ft = caixa(Orientation::Horizontal, 8, "");
        let ftl = rotulo_esq(tr("FIREWALL"), "cfg-rotulo");
        ftl.set_hexpand(true);
        ftl.set_valign(Align::Center);
        ft.append(&ftl);
        let fw_sw = interruptor();
        ft.append(&fw_sw);
        fc.append(&ft);
        let fw_estado = rotulo_esq("", "cfg-sub");
        fw_estado.set_wrap(true);
        fc.append(&fw_estado);
        let fw_regras = caixa(Orientation::Vertical, 6, "");
        fc.append(&fw_regras);
        corpo.add_named(&fc, Some("firewall"));
        let pf = caixa(Orientation::Vertical, 12, "");
        pf.append(&rotulo_esq(tr("ALLOW A PORT"), "cfg-rotulo"));
        let fr = caixa(Orientation::Horizontal, 8, "");
        let fw_porta = gtk::Entry::new();
        fw_porta.add_css_class("cfg-campo");
        fw_porta.set_placeholder_text(Some(tr("Port (8080 or 8080/tcp)")));
        fw_porta.set_hexpand(true);
        fr.append(&fw_porta);
        let permitir = gtk::Button::with_label(tr("Allow"));
        permitir.add_css_class("cfg-botao");
        fr.append(&permitir);
        pf.append(&fr);
        let fa = rotulo_esq(tr("Changes to the firewall go through kalangod: the system asks for your approval, records them and can undo them."), "cfg-sub");
        fa.set_wrap(true);
        pf.append(&fa);
        painel.add_named(&pf, Some("firewall"));
        let r = Rc::new(Rede { moldura: m, corpo, painel, grade, desligado, p_nome, p_estado, p_ip, p_ip_linha, p_vel, p_seg, p_auto, p_auto_linha, p_limitada,
            p_limitada_linha, p_agentes, p_agentes_linha, p_conectar, p_esquecer, p_erro, escolhida: RefCell::default(), cabo_escolhido: RefCell::default(), virtuais: Cell::new(false), cabos, vpns, e_info,
            v_info, proxy_modos, proxy_host, proxy_porta, proxy_url, proxy_manual, proxy_auto, fw_sw, fw_estado, fw_regras, fw_porta, leitura: RefCell::default(),
            lendo: Cell::new(false), ocupado: Rc::default(), timer: RefCell::default() });
        r.ligar(c, &ligar, &importar, &salvar_proxy, &permitir);
        r
    }

    fn ligar(self: &Rc<Self>, c: &Rc<crate::configuracoes::Configuracoes>, ligar_wifi: &gtk::Button, importar: &gtk::Button, salvar_proxy: &gtk::Button,
        permitir: &gtk::Button) {
        let w = Rc::downgrade(self);
        self.moldura.ao_trocar_aba(move |id| if let Some(r) = w.upgrade() { r.aba(id) });
        ligar_wifi.connect_clicked(|_| {
            std::thread::spawn(|| crate::sistema::wifi(true));
        });
        let w = Rc::downgrade(self);
        self.p_auto.connect_state_set(move |_, on| {
            if let Some(r) = w.upgrade() {
                r.mudar_conexao("connection.autoconnect", if on { "yes" } else { "no" });
            }
            glib::Propagation::Proceed
        });
        let w = Rc::downgrade(self);
        self.p_limitada.connect_state_set(move |_, on| {
            if let Some(r) = w.upgrade() {
                r.mudar_conexao("connection.metered", if on { "yes" } else { "no" });
            }
            glib::Propagation::Proceed
        });
        let w = Rc::downgrade(self);
        self.p_agentes.connect_state_set(move |_, on| {
            if let Some(r) = w.upgrade().filter(|r| !r.lendo.get()) {
                if let Some(nome) = r.salva_escolhida() {
                    // (kalangod keeps it; its web tool follows it)
                    let w = Rc::downgrade(&r);
                    let n = nome.clone();
                    crate::lateral::em_segundo_plano(move || crate::servicos::kalangod_rpc("kalango/redes_agentes", json!({"rede": n, "permitir": on})), move |res| {
                        if depurando() {
                            eprintln!("debug rede: agentes {} em {nome}: {}", if on { "permitidos" } else { "proibidos" }, if res.is_ok() { "ok" } else { "erro" });
                        }
                        if let (Some(r), Err(e)) = (w.upgrade(), res) {
                            r.erro(&e);
                        }
                    });
                }
            }
            glib::Propagation::Proceed
        });
        let w = Rc::downgrade(self);
        self.p_conectar.connect_clicked(move |_| if let Some(r) = w.upgrade() { r.conectar() });
        let w = Rc::downgrade(self);
        self.p_esquecer.connect_clicked(move |_| {
            let Some(r) = w.upgrade() else { return };
            let Some(nome) = r.salva_escolhida() else { return };
            esquecer_desde(&nome);
            r.acao(vec!["connection".into(), "delete".into(), nome.clone()], format!("esquecida {nome}"));
        });
        let w = Rc::downgrade(self);
        importar.connect_clicked(move |_| {
            let Some(r) = w.upgrade() else { return };
            let w = Rc::downgrade(&r);
            let filtros = vec![crate::arquivos::seletor::Filtro::de("VPN", &["*.ovpn", "*.conf"]), crate::arquivos::seletor::Filtro::de(tr("All files"), &["*"])];
            crate::arquivos::seletor::abrir_arquivo(tr("Settings"), tr("Import a VPN"), None, filtros, None, move |p| {
                let Some(r) = w.upgrade() else { return };
                let tipo = if p.extension().is_some_and(|e| e == "conf") { "wireguard" } else { "openvpn" };
                r.acao(vec!["connection".into(), "import".into(), "type".into(), tipo.into(), "file".into(), p.display().to_string()], format!("importada {}", p.display()));
            });
        });
        for (_, b) in &self.proxy_modos {
            let w = Rc::downgrade(self);
            b.connect_toggled(move |_| if let Some(r) = w.upgrade() { r.proxy_campos() });
        }
        let w = Rc::downgrade(self);
        salvar_proxy.connect_clicked(move |_| if let Some(r) = w.upgrade() { r.salvar_proxy() });
        let w = Rc::downgrade(self);
        self.fw_sw.connect_state_set(move |s, on| {
            let Some(r) = w.upgrade().filter(|r| !r.lendo.get()) else { return glib::Propagation::Proceed };
            s.set_sensitive(false);
            let w = Rc::downgrade(&r);
            crate::lateral::em_segundo_plano(move || crate::servicos::kalangod_chamar("firewall.ligar", json!({"ligado": on})), move |res| {
                if let Some(r) = w.upgrade() {
                    r.fw_sw.set_sensitive(true);
                    if let Err(e) = res {
                        r.fw_estado.set_text(&e);
                    }
                    r.ler_firewall();
                }
            });
            glib::Propagation::Proceed
        });
        let w = Rc::downgrade(self);
        permitir.connect_clicked(move |_| {
            let Some(r) = w.upgrade() else { return };
            let porta = r.fw_porta.text().trim().to_string();
            if porta.is_empty() {
                return;
            }
            let w = Rc::downgrade(&r);
            crate::lateral::em_segundo_plano(move || crate::servicos::kalangod_chamar("firewall.regra", json!({"acao": "allow", "porta": porta})), move |res| {
                if let Some(r) = w.upgrade() {
                    if let Err(e) = res {
                        r.fw_estado.set_text(&e);
                    } else {
                        r.fw_porta.set_text("");
                    }
                    r.ler_firewall();
                }
            });
        });
        crate::configuracoes::registrar_posicao(c, "cfg rede agentes", self.p_agentes.upcast_ref());
        crate::configuracoes::registrar_posicao(c, "cfg rede esquecer", self.p_esquecer.upcast_ref());
        crate::configuracoes::registrar_posicao(c, "cfg rede conectar", self.p_conectar.upcast_ref());
        crate::configuracoes::registrar_posicao(c, "cfg rede limitada", self.p_limitada.upcast_ref());
        for (id, _) in ABAS {
            if let Some(b) = self.moldura.botao_aba(id) {
                crate::configuracoes::registrar_posicao(c, &format!("cfg rede aba {id}"), b.upcast_ref());
            }
        }
    }

    pub fn compactar(&self, c: bool) {
        self.moldura.compactar(c);
    }

    fn aba(self: &Rc<Self>, id: &str) {
        if depurando() {
            eprintln!("debug rede: aba {id}");
        }
        self.corpo.set_visible_child_name(id);
        self.painel.set_visible_child_name(id);
        self.moldura.painel_titulo.set_text(match id {
            "ethernet" => tr("Cable"),
            "vpn" => tr("VPN"),
            "proxy" => tr("Proxy"),
            "firewall" => tr("Firewall"),
            _ => tr("Selected network"),
        });
        match id {
            "proxy" => self.ler_proxy(),
            "firewall" => self.ler_firewall(),
            _ => {}
        }
    }

    /// Reads the networks now and every 3 s while the page shows.
    pub fn mostrar(self: &Rc<Self>) {
        self.atualizar();
        if self.timer.borrow().is_some() {
            return;
        }
        let w = Rc::downgrade(self);
        *self.timer.borrow_mut() = Some(glib::timeout_add_local(Duration::from_secs(3), move || {
            let Some(r) = w.upgrade() else { return glib::ControlFlow::Break };
            if !r.moldura.raiz.is_mapped() {
                r.timer.borrow_mut().take();
                return glib::ControlFlow::Break;
            }
            r.atualizar();
            glib::ControlFlow::Continue
        }));
    }

    fn atualizar(self: &Rc<Self>) {
        if self.ocupado.replace(true) {
            return;
        }
        let w = Rc::downgrade(self);
        let ocupado = self.ocupado.clone();
        crate::lateral::em_segundo_plano(ler, move |l| {
            ocupado.set(false);
            if let Some(r) = w.upgrade() {
                r.mostrar_leitura(l);
            }
        });
    }

    fn mostrar_leitura(self: &Rc<Self>, l: Leitura) {
        // the title's line: what this computer is on
        let conectada = l.redes.iter().find(|r| r.em_uso);
        let cabo = l.cabos.iter().find(|c| !c.virtual_ && c.estado.starts_with("connected"));
        self.moldura.sub.set_text(&match (conectada, cabo) {
            (Some(r), _) if !r.taxa.is_empty() => trf("Wi-Fi connected · {rate}", &[("rate", r.taxa.clone())]),
            (Some(_), _) => tr("Wi-Fi connected").to_string(),
            (None, Some(_)) => tr("Cable connected").to_string(),
            _ if !l.wifi_ligado => tr("Wi-Fi off").to_string(),
            _ => tr("Not connected").to_string(),
        });
        // the cards (made again only when the networks change)
        let antes = self.leitura.borrow().as_ref().map(|a| a.redes.clone());
        if antes.as_ref() != Some(&l.redes) || self.grade.first_child().is_none() {
            while let Some(x) = self.grade.first_child() {
                self.grade.remove(&x);
            }
            for rede in &l.redes {
                self.grade.insert(&self.cartao_rede(rede), -1);
            }
        }
        self.desligado.set_visible(!l.wifi_ligado);
        // the chosen network (the connected one at first)
        if self.escolhida.borrow().as_ref().is_none_or(|e| !l.redes.iter().any(|r| r.ssid == *e)) {
            *self.escolhida.borrow_mut() = l.redes.first().map(|r| r.ssid.clone());
        }
        self.cabos_render(&l);
        self.vpns_render(&l);
        let sem_mudar = antes.is_some();
        *self.leitura.borrow_mut() = Some(l);
        self.painel_render();
        self.marcar_escolhida();
        // (for the session test: the cards where they are now — they move as the panel and the window do)
        if depurando() {
            let mut x = self.grade.first_child();
            while let Some(f) = x {
                if let Some(b) = f.first_child() {
                    crate::central::posicao_debug(&format!("cfg rede cartao {}", b.widget_name()), &b);
                }
                x = f.next_sibling();
            }
        }
        if depurando() && !sem_mudar {
            let l = self.leitura.borrow();
            let l = l.as_ref().unwrap();
            eprintln!("debug rede: {} redes, em uso {}", l.redes.len(), l.redes.iter().find(|r| r.em_uso).map(|r| r.ssid.as_str()).unwrap_or("-"));
        }
    }

    fn cartao_rede(self: &Rc<Self>, r: &Wifi) -> gtk::Button {
        let b = gtk::Button::new();
        b.add_css_class("cfg-rede");
        b.set_size_request(262, 95);
        b.set_widget_name(&r.ssid);
        let col = caixa(Orientation::Vertical, 10, "");
        let topo = caixa(Orientation::Horizontal, 8, "");
        topo.append(&barras(r.sinal, r.em_uso));
        let esp = caixa(Orientation::Horizontal, 0, "");
        esp.set_hexpand(true);
        topo.append(&esp);
        if !r.seguranca.is_empty() && r.seguranca != "--" {
            topo.append(&ui_icon_fitted("cadeado-d", "#8E8F99", 16));
        }
        col.append(&topo);
        let n = rotulo_esq(&r.ssid, "cfg-rede-ssid");
        n.set_ellipsize(gtk::pango::EllipsizeMode::End);
        col.append(&n);
        let (estado, classe) = if r.em_uso {
            (tr("Connected"), "cfg-estado")
        } else if r.salva.is_some() {
            (tr("Saved"), "cfg-sub")
        } else if r.seguranca.is_empty() || r.seguranca == "--" {
            (tr("Open network"), "cfg-sub")
        } else {
            (tr("Protected"), "cfg-sub")
        };
        col.append(&rotulo_esq(estado, classe));
        b.set_child(Some(&col));
        let (w, ssid) = (Rc::downgrade(self), r.ssid.clone());
        b.connect_clicked(move |_| {
            if let Some(r) = w.upgrade() {
                *r.escolhida.borrow_mut() = Some(ssid.clone());
                r.p_erro.set_visible(false);
                r.painel_render();
                r.marcar_escolhida();
                r.moldura.mostrar_painel(true);
            }
        });
        b
    }

    fn marcar_escolhida(&self) {
        let e = self.escolhida.borrow().clone().unwrap_or_default();
        let mut x = self.grade.first_child();
        while let Some(f) = x {
            if let Some(b) = f.first_child() {
                if b.widget_name() == e {
                    b.add_css_class("escolhida");
                } else {
                    b.remove_css_class("escolhida");
                }
            }
            x = f.next_sibling();
        }
    }

    fn rede_escolhida(&self) -> Option<Wifi> {
        let e = self.escolhida.borrow().clone()?;
        self.leitura.borrow().as_ref()?.redes.iter().find(|r| r.ssid == e).cloned()
    }

    fn salva_escolhida(&self) -> Option<String> {
        self.rede_escolhida()?.salva
    }

    fn painel_render(&self) {
        self.lendo.set(true);
        let l = self.leitura.borrow();
        let Some(l) = l.as_ref() else { self.lendo.set(false); return };
        let r = self.rede_escolhida();
        let vazia = r.is_none();
        for w in [self.p_auto_linha.upcast_ref::<gtk::Widget>(), self.p_limitada_linha.upcast_ref(), self.p_agentes_linha.upcast_ref()] {
            w.set_visible(!vazia);
        }
        let Some(r) = r else {
            self.p_nome.set_text(if l.wifi_ligado { tr("No network nearby") } else { tr("Wi-Fi off") });
            for x in [&self.p_estado, &self.p_vel, &self.p_seg] {
                x.set_text("—");
            }
            self.p_ip_linha.set_visible(false);
            self.p_conectar.set_visible(false);
            self.p_esquecer.set_visible(false);
            self.lendo.set(false);
            return;
        };
        self.p_nome.set_text(&r.ssid);
        self.p_estado.set_text(&if r.em_uso {
            let desde = r.salva.as_deref().map(conectada_desde).unwrap_or(0);
            let agora = glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0);
            trf("Connected {ago}", &[("ago", ha(agora - desde))])
        } else if r.salva.is_some() {
            tr("Saved").to_string()
        } else {
            tr("Available").to_string()
        });
        let ip = l.ip.get(&l.wifi_dispositivo).cloned().filter(|_| r.em_uso).unwrap_or_default();
        self.p_ip.set_text(&ip);
        self.p_ip_linha.set_visible(!ip.is_empty());
        let b = banda(r.freq);
        self.p_vel.set_text(&match (r.taxa.is_empty(), b.is_empty()) {
            (false, false) => format!("{} · {b}", r.taxa),
            (false, true) => r.taxa.clone(),
            (true, _) => b.to_string(),
        });
        self.p_seg.set_text(&seguranca(&r.seguranca));
        let salva = r.salva.clone();
        let conexao = salva.as_ref().and_then(|n| l.conexoes.iter().find(|c| c.nome == *n));
        // the saved network's settings (a network never connected has none yet)
        for w in [self.p_auto_linha.upcast_ref::<gtk::Widget>(), self.p_limitada_linha.upcast_ref(), self.p_agentes_linha.upcast_ref()] {
            w.set_visible(salva.is_some());
        }
        if let Some(c) = conexao {
            self.p_auto.set_active(c.autoconnect);
            self.p_auto.set_state(c.autoconnect);
            let m = l.limitada.get(&c.nome).copied().unwrap_or(false);
            self.p_limitada.set_active(m);
            self.p_limitada.set_state(m);
            let ag = !l.sem_agentes.contains(&c.nome);
            self.p_agentes.set_active(ag);
            self.p_agentes.set_state(ag);
        }
        self.p_conectar.set_visible(!r.em_uso);
        self.p_esquecer.set_visible(salva.is_some());
        self.lendo.set(false);
        // (for the session test: the panel's controls where they are now)
        if depurando() {
            for (n, w) in [("cfg rede agentes", self.p_agentes.upcast_ref::<gtk::Widget>()), ("cfg rede esquecer", self.p_esquecer.upcast_ref()),
                ("cfg rede conectar", self.p_conectar.upcast_ref()), ("cfg rede limitada", self.p_limitada.upcast_ref())] {
                if w.is_visible() && w.parent().is_some_and(|p| p.is_visible()) {
                    crate::central::posicao_debug(n, w);
                }
            }
        }
    }

    fn erro(&self, e: &str) {
        self.p_erro.set_text(e);
        self.p_erro.set_visible(!e.is_empty());
    }

    /// A setting of the chosen network's saved connection.
    fn mudar_conexao(self: &Rc<Self>, chave: &str, valor: &str) {
        if self.lendo.get() {
            return;
        }
        let Some(nome) = self.salva_escolhida() else { return };
        self.acao(vec!["connection".into(), "modify".into(), nome.clone(), chave.into(), valor.into()], format!("{chave} {valor} em {nome}"));
    }

    /// Runs nmcli off the interface thread and reads again.
    fn acao(self: &Rc<Self>, args: Vec<String>, o_que: String) {
        self.erro("");
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(move || {
            let a: Vec<&str> = args.iter().map(String::as_str).collect();
            nm(&a)
        }, move |res| {
            let Some(r) = w.upgrade() else { return };
            match res {
                Ok(_) => {
                    if depurando() {
                        eprintln!("debug rede: {o_que}");
                    }
                }
                Err(e) => r.erro(&e),
            }
            r.atualizar();
        });
    }

    /// Connects to the chosen network: a saved one goes up; a protected one never joined asks for its password in
    /// the system's dialog (design "Senha do Wi-Fi"), and asks again when the network refuses it.
    fn conectar(self: &Rc<Self>) {
        let Some(rede) = self.rede_escolhida() else { return };
        if rede.salva.is_none() && !rede.seguranca.is_empty() && rede.seguranca != "--" {
            self.pedir_senha(&rede, false);
            return;
        }
        let args: Vec<String> = match &rede.salva {
            Some(nome) => vec!["connection".into(), "up".into(), nome.clone()],
            None => vec!["device".into(), "wifi".into(), "connect".into(), rede.ssid.clone()],
        };
        self.conectar_com(rede.ssid.clone(), args, None);
    }

    fn pedir_senha(self: &Rc<Self>, rede: &Wifi, recusada: bool) {
        let w = Rc::downgrade(self);
        let ssid = rede.ssid.clone();
        let req = crate::wifi_password::Request { ssid: ssid.clone(), security: seguranca(&rede.seguranca), enterprise: rede.seguranca.contains("802.1X"), wrong: recusada };
        crate::wifi_password::ask(req, move |resposta| {
            let (Some(r), Some(a)) = (w.upgrade(), resposta) else { return };
            let mut args: Vec<String> = vec!["device".into(), "wifi".into(), "connect".into(), ssid.clone(), "password".into(), a.password.clone()];
            if let Some(i) = a.identity.clone() {
                args.extend(["identity".into(), i]);
            }
            r.conectar_com(ssid, args, Some((a.autoconnect, a.agents)));
        });
    }

    /// nmcli in the background; `escolhas`: what the password dialog asked (connect automatically, agents allowed),
    /// applied to the new connection once it is up.
    fn conectar_com(self: &Rc<Self>, ssid: String, args: Vec<String>, escolhas: Option<(bool, bool)>) {
        self.p_conectar.set_sensitive(false);
        self.p_conectar.set_label(tr("Connecting…"));
        self.erro("");
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(move || {
            let a: Vec<&str> = args.iter().map(String::as_str).collect();
            let r = nm(&a);
            if let (Ok(_), Some((auto, agentes))) = (&r, escolhas) {
                if !auto {
                    let _ = nm(&["connection", "modify", &ssid, "connection.autoconnect", "no"]);
                }
                if !agentes {
                    let _ = crate::servicos::kalangod_rpc("kalango/redes_agentes", json!({"rede": ssid, "permitir": false}));
                }
            }
            (ssid, r)
        }, move |(ssid, res)| {
            let Some(r) = w.upgrade() else { return };
            r.p_conectar.set_sensitive(true);
            r.p_conectar.set_label(tr("Connect"));
            match res {
                Ok(_) => {
                    if depurando() {
                        eprintln!("debug rede: conectada {ssid}");
                    }
                }
                Err(e) => {
                    if depurando() {
                        eprintln!("debug rede: não conectou {ssid}: {e}");
                    }
                    let recusada = e.contains("Secrets were required") || e.contains("password");
                    match r.rede_escolhida().filter(|x| recusada && x.ssid == ssid) {
                        // the password was wrong: the dialog asks again, in coral
                        Some(rede) => r.pedir_senha(&rede, true),
                        None => r.erro(&e),
                    }
                }
            }
            r.atualizar();
        });
    }

    /// The Ethernet tab, as the Wi-Fi one (design "Configurações · Rede · Ethernet"): this computer's ports as cards (the
    /// containers' and virtual machines' interfaces behind a link), the chosen one in the panel.
    fn cabos_render(self: &Rc<Self>, l: &Leitura) {
        while let Some(x) = self.cabos.first_child() {
            self.cabos.remove(&x);
        }
        let (fisicos, virtuais): (Vec<&Cabo>, Vec<&Cabo>) = l.cabos.iter().partition(|c| !c.virtual_);
        // the chosen one: the person's, else the connected port, else the first one
        let escolhido = self.cabo_escolhido.borrow().clone().filter(|d| l.cabos.iter().any(|c| c.dispositivo == *d))
            .or_else(|| fisicos.iter().find(|c| c.estado.starts_with("connected")).or(fisicos.first()).map(|c| c.dispositivo.clone()))
            .or_else(|| virtuais.first().map(|c| c.dispositivo.clone()));
        *self.cabo_escolhido.borrow_mut() = escolhido.clone();
        let grade_de = |lista: &[&Cabo]| {
            let g = gtk::FlowBox::new();
            g.set_selection_mode(gtk::SelectionMode::None);
            g.add_css_class("cfg-grade");
            g.set_column_spacing(14);
            g.set_row_spacing(14);
            g.set_min_children_per_line(1);
            g.set_max_children_per_line(24);
            g.set_homogeneous(true);
            g.set_hexpand(true);
            g.set_valign(Align::Start);
            g.connect_child_activated(|_, ch| if let Some(b) = ch.child().and_downcast::<gtk::Button>() { b.emit_clicked() });
            for c in lista {
                g.insert(&self.cartao_cabo(c, escolhido.as_deref() == Some(c.dispositivo.as_str())), -1);
            }
            g
        };
        if fisicos.is_empty() {
            self.cabos.append(&rotulo_esq(tr("This computer has no network cable port."), "cfg-sub"));
        } else {
            self.cabos.append(&grade_de(&fisicos));
        }
        if !virtuais.is_empty() {
            let b = gtk::Button::new();
            b.add_css_class("cfg-link-discreto");
            b.set_halign(Align::Start);
            let linha = caixa(Orientation::Horizontal, 6, "");
            let aberto = self.virtuais.get();
            linha.append(&crate::conversa::ui_icon(if aberto { "chevron-baixo" } else { "seta-direita" }, "#8E8F99", 12));
            linha.append(&rotulo(&if aberto { tr("Hide virtual interfaces").to_string() } else { trf("Show virtual interfaces ({n})", &[("n", virtuais.len().to_string())]) }, "cfg-link-texto"));
            linha.append(&rotulo(tr("· containers and virtual machines"), "cfg-sub"));
            b.set_child(Some(&linha));
            let w = Rc::downgrade(self);
            b.connect_clicked(move |_| {
                let Some(r) = w.upgrade() else { return };
                r.virtuais.set(!r.virtuais.get());
                let l = r.leitura.borrow().clone();
                if let Some(l) = l {
                    r.cabos_render(&l);
                }
            });
            crate::central::posicao_debug("cfg rede virtuais", b.upcast_ref());
            self.cabos.append(&b);
            if aberto {
                self.cabos.append(&grade_de(&virtuais));
            }
        }
        self.cabo_painel(l);
        if depurando() {
            eprintln!("debug rede: cabos {} físicos, {} virtuais{}", fisicos.len(), virtuais.len(), if self.virtuais.get() { " (à vista)" } else { "" });
        }
    }

    /// A cable's card: as a Wi-Fi network's (its glyph, its name, its state).
    fn cartao_cabo(self: &Rc<Self>, c: &Cabo, escolhido: bool) -> gtk::Button {
        let b = gtk::Button::new();
        b.add_css_class("cfg-rede");
        if escolhido {
            b.add_css_class("escolhida");
        }
        b.set_size_request(262, 95);
        b.set_widget_name(&c.dispositivo);
        let ligado = c.estado.starts_with("connected");
        let col = caixa(Orientation::Vertical, 10, "");
        let topo = caixa(Orientation::Horizontal, 8, "");
        topo.append(&ui_icon_fitted("rede-cabo", if ligado { "#A5A1E2" } else { "#8E8F99" }, 16));
        col.append(&topo);
        let nome = if c.virtual_ { c.dispositivo.clone() } else { trf("Cable · {device}", &[("device", c.dispositivo.clone())]) };
        let n = rotulo_esq(&nome, "cfg-rede-ssid");
        n.set_ellipsize(gtk::pango::EllipsizeMode::End);
        col.append(&n);
        col.append(&rotulo_esq(estado_do_cabo(c), if ligado { "cfg-estado" } else { "cfg-sub" }));
        b.set_child(Some(&col));
        let (w, d) = (Rc::downgrade(self), c.dispositivo.clone());
        b.connect_clicked(move |_| {
            let Some(r) = w.upgrade() else { return };
            *r.cabo_escolhido.borrow_mut() = Some(d.clone());
            let l = r.leitura.borrow().clone();
            if let Some(l) = l {
                r.cabos_render(&l);
            }
            r.moldura.mostrar_painel(true);
            if depurando() {
                eprintln!("debug rede: cabo escolhido {d}");
            }
        });
        crate::central::posicao_debug(&format!("cfg rede cabo {}", c.dispositivo), b.upcast_ref());
        b
    }

    /// The panel: the chosen cable — its state, address, speed, hardware address and connection, the same settings as a
    /// Wi-Fi network's, and connect or disconnect.
    fn cabo_painel(self: &Rc<Self>, l: &Leitura) {
        while let Some(x) = self.e_info.first_child() {
            self.e_info.remove(&x);
        }
        let Some(c) = self.cabo_escolhido.borrow().clone().and_then(|d| l.cabos.iter().find(|c| c.dispositivo == d).cloned()) else { return };
        self.e_info.append(&rotulo_cartao("rede-cabo", tr("SELECTED CABLE")));
        let nome = if c.virtual_ { c.dispositivo.clone() } else { trf("Cable · {device}", &[("device", c.dispositivo.clone())]) };
        let n = rotulo_esq(&nome, "cfg-rede-nome");
        n.set_ellipsize(gtk::pango::EllipsizeMode::End);
        self.e_info.append(&n);
        let rd = |f: &str| std::fs::read_to_string(format!("/sys/class/net/{}/{f}", c.dispositivo)).map(|s| s.trim().to_string()).unwrap_or_default();
        let conexao = (!c.conexao.is_empty() && c.conexao != "--").then(|| c.conexao.clone());
        for (k, v) in [(tr("State"), estado_do_cabo(&c).to_string()), (tr("IP address"), l.ip.get(&c.dispositivo).cloned().filter(|s| !s.is_empty()).unwrap_or_else(|| "—".into())),
            (tr("Speed"), rd("speed").parse::<i64>().ok().filter(|v| *v > 0).map(|v| format!("{v} Mb/s")).unwrap_or_else(|| "—".into())),
            (tr("Hardware address"), Some(rd("address")).filter(|s| !s.is_empty()).unwrap_or_else(|| "—".into())),
            (tr("Connection"), conexao.clone().unwrap_or_else(|| "—".into()))] {
            let (r, val) = linha_valor(k);
            val.set_text(&v);
            self.e_info.append(&r);
        }
        if let Some(nome) = conexao.clone() {
            let div = caixa(Orientation::Horizontal, 0, "cfg-div");
            div.set_size_request(-1, 1);
            self.e_info.append(&div);
            let cx = l.conexoes.iter().find(|x| x.nome == nome);
            for (rotulo_l, chave, ligado) in [(tr("Connect automatically"), "connection.autoconnect", cx.is_some_and(|x| x.autoconnect)),
                (tr("Metered network (save data)"), "connection.metered", l.limitada.get(&nome).copied().unwrap_or(false))] {
                let (linha, sw) = linha_interruptor(rotulo_l);
                sw.set_active(ligado);
                sw.set_state(ligado);
                let metered = chave == "connection.metered";
                let (w, n, chave) = (Rc::downgrade(self), nome.clone(), chave.to_string());
                sw.connect_state_set(move |_, on| {
                    if let Some(r) = w.upgrade() {
                        r.acao(vec!["connection".into(), "modify".into(), n.clone(), chave.clone(), if on { "yes".into() } else { "no".into() }], format!("{chave} {} em {n}", if on { "yes" } else { "no" }));
                    }
                    glib::Propagation::Proceed
                });
                if metered {
                    crate::central::posicao_debug("cfg cabo limitada", sw.upcast_ref());
                }
                self.e_info.append(&linha);
            }
            let (linha, sw) = linha_interruptor(tr("Allow agents to use this network"));
            let ag = !l.sem_agentes.contains(&nome);
            sw.set_active(ag);
            sw.set_state(ag);
            let (w, n) = (Rc::downgrade(self), nome.clone());
            sw.connect_state_set(move |_, on| {
                let Some(r) = w.upgrade() else { return glib::Propagation::Proceed };
                let (w, n2) = (Rc::downgrade(&r), n.clone());
                crate::lateral::em_segundo_plano(move || crate::servicos::kalangod_rpc("kalango/redes_agentes", json!({"rede": n2, "permitir": on})), move |res| {
                    if let (Some(r), Err(e)) = (w.upgrade(), res) {
                        r.erro(&e);
                    }
                });
                glib::Propagation::Proceed
            });
            self.e_info.append(&linha);
        }
        let ligado = c.estado.starts_with("connected");
        let sem_cabo = c.estado.contains("unavailable");
        if sem_cabo && !c.virtual_ {
            let dica = rotulo_esq(tr("Plug a network cable into this port."), "cfg-sub");
            dica.set_wrap(true);
            self.e_info.append(&dica);
        }
        let b = gtk::Button::with_label(if ligado { tr("Disconnect") } else { tr("Connect") });
        b.add_css_class(if ligado { "cfg-botao" } else { "cfg-botao-principal" });
        b.add_css_class("largo");
        b.set_sensitive(!sem_cabo && !c.estado.contains("unmanaged"));
        let (w, d) = (Rc::downgrade(self), c.dispositivo.clone());
        b.connect_clicked(move |_| if let Some(r) = w.upgrade() {
            r.acao(vec!["device".into(), if ligado { "disconnect".into() } else { "connect".into() }, d.clone()], format!("cabo {d}"));
        });
        crate::central::posicao_debug("cfg cabo conectar", b.upcast_ref());
        self.e_info.append(&b);
    }

    fn vpns_render(self: &Rc<Self>, l: &Leitura) {
        while let Some(x) = self.vpns.first_child() {
            self.vpns.remove(&x);
        }
        while let Some(x) = self.v_info.first_child() {
            self.v_info.remove(&x);
        }
        let vpns: Vec<&Conexao> = l.conexoes.iter().filter(|c| c.tipo == "vpn" || c.tipo == "wireguard").collect();
        if vpns.is_empty() {
            self.vpns.append(&rotulo_esq(tr("No VPN yet. Import one from its file (OpenVPN .ovpn or WireGuard .conf)."), "cfg-sub"));
        }
        for c in vpns {
            let (linha, sw) = linha_interruptor(&c.nome);
            linha.add_css_class("cfg-cartao");
            linha.set_size_request(400, -1);
            linha.set_halign(Align::Start);
            sw.set_active(c.ativa);
            sw.set_state(c.ativa);
            let (w, n) = (Rc::downgrade(self), c.nome.clone());
            sw.connect_state_set(move |_, on| {
                if let Some(r) = w.upgrade().filter(|r| !r.lendo.get()) {
                    r.acao(vec!["connection".into(), if on { "up".into() } else { "down".into() }, n.clone()], format!("vpn {n} {on}"));
                }
                glib::Propagation::Proceed
            });
            self.vpns.append(&linha);
            let (r, v) = linha_valor(&c.nome);
            v.set_text(&format!("{} · {}", c.tipo, if c.ativa { tr("connected") } else { tr("off") }));
            self.v_info.append(&r);
        }
    }

    // ------------------------------------------------------------ proxy (GNOME's settings: the apps follow them)

    fn proxy_campos(&self) {
        let modo = self.proxy_modos.iter().find(|(_, b)| b.is_active()).map(|(m, _)| *m).unwrap_or("none");
        self.proxy_manual.set_visible(modo == "manual");
        self.proxy_auto.set_visible(modo == "auto");
    }

    fn ler_proxy(&self) {
        let Some(s) = proxy_settings("org.gnome.system.proxy") else {
            for (_, b) in &self.proxy_modos {
                b.set_sensitive(false);
            }
            return;
        };
        let modo = s.string("mode").to_string();
        for (m, b) in &self.proxy_modos {
            if *m == modo {
                b.set_active(true);
            }
        }
        self.proxy_url.set_text(&s.string("autoconfig-url"));
        if let Some(h) = proxy_settings("org.gnome.system.proxy.http") {
            self.proxy_host.set_text(&h.string("host"));
            let p = h.int("port");
            self.proxy_porta.set_text(&if p > 0 { p.to_string() } else { String::new() });
        }
        self.proxy_campos();
    }

    fn salvar_proxy(&self) {
        let modo = self.proxy_modos.iter().find(|(_, b)| b.is_active()).map(|(m, _)| *m).unwrap_or("none");
        let (host, porta, url) = (self.proxy_host.text().trim().to_string(), self.proxy_porta.text().trim().parse::<i32>().unwrap_or(0), self.proxy_url.text().trim().to_string());
        // the development session runs on someone's desktop: its settings stay as they are
        if std::env::var_os("KALANGO_SHELL_DEV").is_some() {
            if depurando() {
                eprintln!("debug rede: [sessão de teste] proxy {modo} {host}:{porta} {url} não foi gravado");
            }
            return;
        }
        let Some(s) = proxy_settings("org.gnome.system.proxy") else { return };
        let _ = s.set_string("mode", modo);
        let _ = s.set_string("autoconfig-url", &url);
        for esquema in ["org.gnome.system.proxy.http", "org.gnome.system.proxy.https"] {
            if let Some(h) = proxy_settings(esquema) {
                let _ = h.set_string("host", &host);
                let _ = h.set_int("port", porta);
            }
        }
    }

    // ------------------------------------------------------------ firewall (through kalangod)

    fn ler_firewall(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(|| crate::servicos::kalangod_chamar("firewall.estado", json!({})), move |res| {
            let Some(r) = w.upgrade() else { return };
            r.lendo.set(true);
            while let Some(x) = r.fw_regras.first_child() {
                r.fw_regras.remove(&x);
            }
            match res {
                Ok(v) => {
                    let ligado = v["ativo"].as_bool().or(v["ligado"].as_bool()).unwrap_or(false);
                    r.fw_sw.set_active(ligado);
                    r.fw_sw.set_state(ligado);
                    r.fw_sw.set_sensitive(true);
                    r.fw_estado.set_text(if ligado { tr("On: only the connections you allow come in.") } else { tr("Off: nothing blocks incoming connections.") });
                    for regra in v["regras"].as_array().cloned().unwrap_or_default() {
                        let t = regra.as_str().map(str::to_string).unwrap_or_else(|| regra.to_string());
                        r.fw_regras.append(&rotulo_esq(&t, "cfg-mono"));
                    }
                }
                Err(e) => {
                    r.fw_sw.set_sensitive(false);
                    r.fw_estado.set_text(&trf("The system did not answer: {e}", &[("e", e)]));
                }
            }
            r.lendo.set(false);
        });
    }
}

fn proxy_settings(esquema: &str) -> Option<gio::Settings> {
    let fonte = gio::SettingsSchemaSource::default()?;
    fonte.lookup(esquema, true)?;
    Some(gio::Settings::new(esquema))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nmcli_fields_keep_escaped_colons() {
        assert_eq!(campos(r"*:Casa\:5G:80:WPA2:540 Mbit/s:5180 MHz"), vec!["*", "Casa:5G", "80", "WPA2", "540 Mbit/s", "5180 MHz"]);
    }

    #[test]
    fn networks_one_card_each_in_use_and_saved_first() {
        let lista = " :Vizinho:40:WPA2:130 Mbit/s:2437 MHz\n*:MGP:70:WPA3:866 Mbit/s:5180 MHz\n :Café:55::54 Mbit/s:2412 MHz\n :Vizinho:60:WPA2:130 Mbit/s:2462 MHz\n :Escritório:20:WPA2:65 Mbit/s:2412 MHz\n";
        let mut salvas = HashMap::new();
        salvas.insert("Escritório".to_string(), "Escritório".to_string());
        salvas.insert("MGP".to_string(), "MGP".to_string());
        let v = ler_redes(lista, &salvas);
        let nomes: Vec<&str> = v.iter().map(|w| w.ssid.as_str()).collect();
        assert_eq!(nomes, vec!["MGP", "Escritório", "Vizinho", "Café"]);
        assert_eq!(v[0].taxa, "866 Mb/s");
        assert_eq!(v[2].sinal, 60, "the strongest of a name");
        assert_eq!(banda(v[0].freq), "5 GHz");
        assert_eq!(seguranca(&v[0].seguranca), "WPA3");
    }
}
