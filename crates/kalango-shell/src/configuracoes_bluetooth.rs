//! Configurações › Bluetooth (design "Configurações · Bluetooth"): the switch, the person's devices (paired: connected
//! or not, with their battery), the ones nearby (found by a scan while the page shows) with "Parear", and the chosen
//! one in the panel — when it connected, its battery, its audio codec, connecting, disconnecting and forgetting it.
//! Everything through BlueZ (bluetoothctl, as the person) and, for headphones, PipeWire.
//!
//! The design's headphone has controls of its own brand (noise cancelling, pausing when taken off, switching between
//! the person's devices): those exist only on such devices, through their makers' protocols, and show only for a
//! device that offers them — none here, so they stay hidden. "Usar como microfone em chamadas" is the system's own:
//! WirePlumber switches a headset to its call profile when an app records.

use crate::configuracoes::{depurando, interruptor};
use crate::configuracoes_moldura::{linha_interruptor, linha_valor, Moldura};
use crate::conversa::{ui_icon, ui_icon_fitted};
use crate::superficies::{caixa, rotulo};
use gtk::prelude::*;
use gtk::{glib, Align, Orientation};
use kalango_i18n::{tr, trf};
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

/// BlueZ's command (a stand-in in the session test).
pub fn bluetoothctl() -> String {
    std::env::var("KALANGO_BLUETOOTHCTL").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "bluetoothctl".into())
}

/// Whether this computer has a Bluetooth adapter (or the session test's stand-in): without one, `bluetoothctl`
/// waits for a controller forever — every poll left another one running (seen in a VM: five of them, 23 MB each).
pub fn adapter_present() -> bool {
    std::env::var_os("KALANGO_BLUETOOTHCTL").is_some_and(|s| !s.is_empty())
        || std::fs::read_dir("/sys/class/bluetooth").is_ok_and(|mut d| d.any(|e| e.is_ok_and(|e| e.file_name().to_string_lossy().starts_with("hci"))))
}

fn bt(args: &[&str]) -> Result<String, String> {
    if !adapter_present() {
        return Err("no Bluetooth adapter".into());
    }
    let o = crate::sistema::output_timeout(std::process::Command::new(bluetoothctl()).args(args).env("LC_ALL", "C"), crate::sistema::STATUS_LIMIT)
        .map_err(|e| e.to_string())?;
    let saida = String::from_utf8_lossy(&o.stdout).into_owned();
    // bluetoothctl answers 0 even when it fails: its "Failed"/"not available" lines say so
    if !o.status.success() || saida.lines().any(|l| l.contains("Failed to") || l.contains("not available")) {
        return Err(saida.lines().find(|l| l.contains("Failed") || l.contains("not available")).unwrap_or(saida.trim()).trim().to_string());
    }
    Ok(saida)
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Aparelho {
    pub mac: String,
    pub nome: String,
    /// BlueZ's icon name: audio-headset, input-mouse, input-keyboard, phone, video-display…
    pub icone: String,
    pub pareado: bool,
    pub conectado: bool,
    pub bateria: Option<u8>,
}

#[derive(Clone, Debug, Default)]
struct Leitura {
    existe: bool,
    ligado: bool,
    nome: String,
    aparelhos: Vec<Aparelho>,
    codecs: HashMap<String, String>,
}

/// One device's `info`.
pub fn ler_info(mac: &str, info: &str) -> Aparelho {
    let campo = |k: &str| info.lines().find_map(|l| l.trim().strip_prefix(k).map(|v| v.trim().to_string()));
    Aparelho {
        mac: mac.to_string(),
        nome: campo("Alias:").or_else(|| campo("Name:")).unwrap_or_else(|| mac.to_string()),
        icone: campo("Icon:").unwrap_or_default(),
        pareado: campo("Paired:").is_some_and(|v| v == "yes"),
        conectado: campo("Connected:").is_some_and(|v| v == "yes"),
        bateria: campo("Battery Percentage:").and_then(|v| v.split('(').nth(1)?.trim_end_matches(')').parse().ok()),
    }
}

fn ler() -> Leitura {
    let mut l = Leitura::default();
    if let Ok(s) = bt(&["show"]) {
        l.existe = s.contains("Controller");
        l.ligado = s.contains("Powered: yes");
        l.nome = s.lines().find_map(|x| x.trim().strip_prefix("Alias:").map(|v| v.trim().to_string())).unwrap_or_default();
    }
    if l.ligado {
        for linha in bt(&["devices"]).unwrap_or_default().lines() {
            let f: Vec<&str> = linha.splitn(3, ' ').collect();
            if f.len() >= 2 && f[0] == "Device" {
                if let Ok(info) = bt(&["info", f[1]]) {
                    l.aparelhos.push(ler_info(f[1], &info));
                }
            }
        }
    }
    // the headphones' codec, from PipeWire (its sinks carry the device's address)
    if let Ok(o) = std::process::Command::new("pactl").args(["list", "sinks"]).env("LC_ALL", "C").output() {
        let t = String::from_utf8_lossy(&o.stdout);
        let (mut mac, mut codec) = (String::new(), String::new());
        for linha in t.lines().chain(std::iter::once("Sink #")) {
            let x = linha.trim();
            if x.starts_with("Sink #") {
                if !mac.is_empty() && !codec.is_empty() {
                    l.codecs.insert(mac.to_uppercase(), codec.to_uppercase());
                }
                mac.clear();
                codec.clear();
            } else if let Some(v) = x.strip_prefix("api.bluez5.address = ") {
                mac = v.trim_matches('"').to_string();
            } else if let Some(v) = x.strip_prefix("api.bluez5.codec = ") {
                codec = v.trim_matches('"').to_string();
            }
        }
    }
    l
}

/// The device's glyph and its tile's colour (the design: accent for the person's, blue for those nearby).
fn glifo(icone: &str) -> &'static str {
    match icone {
        i if i.contains("audio") || i.contains("headset") || i.contains("headphone") => "fone",
        i if i.contains("mouse") => "mouse-d",
        i if i.contains("keyboard") => "teclado-d",
        i if i.contains("phone") => "celular-d",
        i if i.contains("video") || i.contains("display") || i.contains("camera") => "video-d",
        _ => "bluetooth",
    }
}

fn e_audio(icone: &str) -> bool {
    icone.contains("audio") || icone.contains("headset") || icone.contains("headphone")
}

/// When the shell first saw each device connected (BlueZ does not keep it): "conectado há 2 h".
fn desde_path() -> std::path::PathBuf {
    std::env::var_os("XDG_STATE_HOME").map(std::path::PathBuf::from).filter(|p| p.is_absolute())
        .unwrap_or_else(|| glib::home_dir().join(".local/state")).join("kalango/bluetooth-desde.json")
}

fn conectado_desde(mac: &str, conectado: bool) -> i64 {
    let agora = glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0);
    let mut v: Value = std::fs::read(desde_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_else(|| json!({}));
    let t = v[mac].as_i64();
    match (conectado, t) {
        (true, Some(t)) => t,
        (true, None) | (false, Some(_)) => {
            if conectado {
                v[mac] = json!(agora);
            } else if let Some(o) = v.as_object_mut() {
                o.remove(mac);
            }
            if let Some(d) = desde_path().parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let _ = std::fs::write(desde_path(), v.to_string());
            agora
        }
        (false, None) => agora,
    }
}

/// WirePlumber 0.4's policy: a headset switches to its call profile when an app records (the default), or not.
fn microfone_path() -> std::path::PathBuf {
    glib::user_config_dir().join("wireplumber/policy.lua.d/51-kalango-microfone.lua")
}

fn microfone_ligado() -> bool {
    std::fs::read_to_string(microfone_path()).map(|t| !t.contains("= false")).unwrap_or(true)
}

fn tile(glifo_: &str, cor: &str, classe: &str, lado: i32) -> gtk::Box {
    tile_px(glifo_, cor, classe, lado, 16)
}

fn tile_px(glifo_: &str, cor: &str, classe: &str, lado: i32, px: i32) -> gtk::Box {
    let t = caixa(Orientation::Horizontal, 0, "cfg-bt-tile");
    t.add_css_class(classe);
    t.set_size_request(lado, lado);
    t.set_valign(Align::Center);
    let g = ui_icon_fitted(glifo_, cor, px);
    g.set_hexpand(true);
    t.append(&g);
    // (the glyph centres itself by expanding; the tile keeps its size)
    t.set_hexpand(false);
    t.set_halign(Align::Start);
    t
}

pub struct Bluetooth {
    pub moldura: Rc<Moldura>,
    sw: gtk::Switch,
    sw_sub: gtk::Label,
    meus: gtk::Box,
    perto: gtk::Box,
    procurando: gtk::Box,
    meus_cartao: gtk::Box,
    perto_cartao: gtk::Box,
    p_vazio: gtk::Label,
    p_detalhes: gtk::Box,
    p_rotulo: gtk::Box,
    p_tile: gtk::Box,
    p_nome: gtk::Label,
    p_estado: gtk::Label,
    p_bateria: gtk::Label,
    p_bateria_linha: gtk::Box,
    p_codec: gtk::Label,
    p_codec_linha: gtk::Box,
    p_mic: gtk::Switch,
    p_mic_linha: gtk::Box,
    p_conectar: gtk::Button,
    p_esquecer: gtk::Button,
    p_erro: gtk::Label,
    escolhido: RefCell<Option<String>>,
    leitura: RefCell<Option<Leitura>>,
    lendo: Cell<bool>,
    ocupado: Rc<Cell<bool>>,
    timer: RefCell<Option<glib::SourceId>>,
    /// the last scan (a new one every 25 s while the page shows)
    procura: Cell<i64>,
}

/// This page's labels: the glyph in its 16 px frame (the design draws these icons in frames), 6 px to the text.
fn rotulo16(glifo_: &str, texto: &str) -> gtk::Box {
    let r = caixa(Orientation::Horizontal, 6, "");
    r.append(&ui_icon(glifo_, "#8E8F99", 16));
    r.append(&rotulo(texto, "cfg-rotulo"));
    r
}

fn esquerda(t: &str, c: &str) -> gtk::Label {
    let l = rotulo(t, c);
    l.set_xalign(0.0);
    l
}

impl Bluetooth {
    pub fn new(c: &Rc<crate::configuracoes::Configuracoes>) -> Rc<Bluetooth> {
        let m = Moldura::new(tr("Bluetooth"), &[], tr("Device"));
        m.busca_grande();
        // the switch
        let topo = caixa(Orientation::Horizontal, 12, "cfg-cartao");
        topo.add_css_class("cfg-bt-chave");
        topo.append(&ui_icon("bluetooth", "#A5A1E2", 16));
        let tx = caixa(Orientation::Vertical, 1, "");
        tx.set_hexpand(true);
        tx.set_valign(Align::Center);
        tx.append(&esquerda(tr("Bluetooth"), "cfg-texto-13"));
        let sw_sub = esquerda("", "cfg-sub");
        tx.append(&sw_sub);
        topo.append(&tx);
        let sw = interruptor();
        topo.append(&sw);
        m.corpo.append(&topo);
        // my devices
        let meus_cartao = caixa(Orientation::Vertical, 4, "cfg-cartao");
        meus_cartao.append(&rotulo16("bluetooth", tr("MY DEVICES")));
        let meus = caixa(Orientation::Vertical, 4, "");
        meus_cartao.append(&meus);
        m.corpo.append(&meus_cartao);
        // nearby
        let perto_cartao = caixa(Orientation::Vertical, 4, "cfg-cartao");
        perto_cartao.append(&rotulo16("busca-d", tr("NEARBY")));
        let perto = caixa(Orientation::Vertical, 4, "");
        perto_cartao.append(&perto);
        let procurando = caixa(Orientation::Horizontal, 7, "cfg-bt-procurando");
        procurando.append(&ui_icon("anel", "#8E8F99", 16));
        procurando.append(&rotulo(tr("looking for devices…"), "cfg-sub"));
        procurando.set_visible(false);
        perto_cartao.append(&procurando);
        m.corpo.append(&perto_cartao);
        // the panel
        let p_vazio = esquerda(tr("Choose a device to see it here."), "cfg-sub");
        p_vazio.set_wrap(true);
        m.painel.append(&p_vazio);
        let p_detalhes = caixa(Orientation::Vertical, 12, "");
        let p_rotulo = caixa(Orientation::Horizontal, 5, "");
        p_detalhes.append(&p_rotulo);
        let cab = caixa(Orientation::Horizontal, 14, "");
        let p_tile = caixa(Orientation::Horizontal, 0, "");
        cab.append(&p_tile);
        let ct = caixa(Orientation::Vertical, 2, "");
        ct.set_valign(Align::Center);
        let p_nome = esquerda("", "cfg-bt-nome");
        p_nome.set_ellipsize(gtk::pango::EllipsizeMode::End);
        ct.append(&p_nome);
        let p_estado = esquerda("", "cfg-estado");
        ct.append(&p_estado);
        cab.append(&ct);
        p_detalhes.append(&cab);
        let dados = caixa(Orientation::Vertical, 8, "");
        let (p_bateria_linha, p_bateria) = linha_valor(tr("Battery"));
        dados.append(&p_bateria_linha);
        let (p_codec_linha, p_codec) = linha_valor(tr("Codec"));
        dados.append(&p_codec_linha);
        p_detalhes.append(&dados);
        let div = caixa(Orientation::Horizontal, 0, "cfg-div");
        div.set_size_request(-1, 1);
        p_detalhes.append(&div);
        let (p_mic_linha, p_mic) = linha_interruptor(tr("Use as microphone in calls"));
        p_mic_linha.add_css_class("cfg-bt-opcao");
        p_detalhes.append(&p_mic_linha);
        let p_erro = esquerda("", "cfg-erro");
        p_erro.set_wrap(true);
        p_erro.set_visible(false);
        p_detalhes.append(&p_erro);
        let botoes = caixa(Orientation::Horizontal, 8, "");
        botoes.set_homogeneous(true);
        let p_conectar = gtk::Button::with_label(tr("Disconnect"));
        p_conectar.add_css_class("cfg-botao");
        p_conectar.add_css_class("largo");
        botoes.append(&p_conectar);
        let p_esquecer = gtk::Button::with_label(tr("Forget"));
        p_esquecer.add_css_class("cfg-botao-perigo");
        botoes.append(&p_esquecer);
        p_detalhes.append(&botoes);
        m.painel.append(&p_detalhes);
        let b = Rc::new(Bluetooth { moldura: m, sw, sw_sub, meus, perto, procurando, meus_cartao, perto_cartao, p_vazio, p_detalhes, p_rotulo, p_tile, p_nome,
            p_estado, p_bateria, p_bateria_linha, p_codec, p_codec_linha, p_mic, p_mic_linha, p_conectar, p_esquecer, p_erro, escolhido: RefCell::default(),
            leitura: RefCell::default(), lendo: Cell::new(false), ocupado: Rc::default(), timer: RefCell::default(), procura: Cell::new(0) });
        b.ligar(c);
        b
    }

    fn ligar(self: &Rc<Self>, c: &Rc<crate::configuracoes::Configuracoes>) {
        let w = Rc::downgrade(self);
        self.sw.connect_state_set(move |_, on| {
            if let Some(b) = w.upgrade().filter(|b| !b.lendo.get()) {
                b.acao(vec!["power".into(), if on { "on".into() } else { "off".into() }], format!("ligado {on}"));
            }
            glib::Propagation::Proceed
        });
        let w = Rc::downgrade(self);
        self.p_conectar.connect_clicked(move |_| {
            let Some(b) = w.upgrade() else { return };
            let Some(a) = b.aparelho() else { return };
            b.acao(vec![if a.conectado { "disconnect".into() } else { "connect".into() }, a.mac.clone()],
                format!("{} {}", if a.conectado { "desconectado" } else { "conectado" }, a.nome));
        });
        let w = Rc::downgrade(self);
        self.p_esquecer.connect_clicked(move |_| {
            let Some(b) = w.upgrade() else { return };
            let Some(a) = b.aparelho() else { return };
            *b.escolhido.borrow_mut() = None;
            b.acao(vec!["remove".into(), a.mac.clone()], format!("esquecido {}", a.nome));
        });
        let w = Rc::downgrade(self);
        self.p_mic.connect_state_set(move |_, on| {
            if let Some(b) = w.upgrade().filter(|b| !b.lendo.get()) {
                b.mudar_microfone(on);
            }
            glib::Propagation::Proceed
        });
        crate::configuracoes::registrar_posicao(c, "cfg bt chave", self.sw.upcast_ref());
        crate::configuracoes::registrar_posicao(c, "cfg bt conectar", self.p_conectar.upcast_ref());
        crate::configuracoes::registrar_posicao(c, "cfg bt esquecer", self.p_esquecer.upcast_ref());
    }

    pub fn compactar(&self, c: bool) {
        self.moldura.compactar(c);
    }

    /// Reads now and every 3 s while the page shows (and keeps a scan going).
    pub fn mostrar(self: &Rc<Self>) {
        self.atualizar();
        if self.timer.borrow().is_some() {
            return;
        }
        let w = Rc::downgrade(self);
        *self.timer.borrow_mut() = Some(glib::timeout_add_local(Duration::from_secs(3), move || {
            let Some(b) = w.upgrade() else { return glib::ControlFlow::Break };
            if !b.moldura.raiz.is_mapped() {
                b.timer.borrow_mut().take();
                return glib::ControlFlow::Break;
            }
            b.atualizar();
            glib::ControlFlow::Continue
        }));
    }

    fn atualizar(self: &Rc<Self>) {
        if self.ocupado.replace(true) {
            return;
        }
        // a scan for devices nearby, now and then (it runs on its own for 20 s)
        let agora = glib::monotonic_time() / 1_000_000;
        let ligado = self.leitura.borrow().as_ref().is_some_and(|l| l.ligado);
        if ligado && agora - self.procura.get() > 25 {
            self.procura.set(agora);
            // "procurando dispositivos…" while it runs
            self.procurando.set_visible(true);
            let w = Rc::downgrade(self);
            crate::lateral::em_segundo_plano(|| {
                let _ = bt(&["--timeout", "20", "scan", "on"]);
            }, move |_| {
                if let Some(b) = w.upgrade() {
                    b.procurando.set_visible(false);
                }
            });
        }
        let w = Rc::downgrade(self);
        let ocupado = self.ocupado.clone();
        crate::lateral::em_segundo_plano(ler, move |l| {
            ocupado.set(false);
            if let Some(b) = w.upgrade() {
                b.mostrar_leitura(l);
            }
        });
    }

    fn mostrar_leitura(self: &Rc<Self>, l: Leitura) {
        self.lendo.set(true);
        let conectados = l.aparelhos.iter().filter(|a| a.conectado).count();
        self.moldura.sub.set_text(&if !l.existe {
            tr("This computer has no Bluetooth").to_string()
        } else if l.ligado {
            trf("On · visible as {name}", &[("name", l.nome.clone())])
        } else {
            tr("Off").to_string()
        });
        self.sw.set_sensitive(l.existe);
        self.sw.set_active(l.ligado);
        self.sw.set_state(l.ligado);
        self.sw_sub.set_text(&match (l.ligado, conectados) {
            (false, _) => tr("Off").to_string(),
            (true, 0) => tr("Nothing connected").to_string(),
            (true, 1) => tr("1 device connected").to_string(),
            (true, n) => trf("{n} devices connected", &[("n", n.to_string())]),
        });
        let antes = self.leitura.borrow().as_ref().map(|a| a.aparelhos.clone());
        if antes.as_ref() != Some(&l.aparelhos) {
            self.listas(&l);
        }
        self.meus_cartao.set_visible(l.ligado);
        self.perto_cartao.set_visible(l.ligado);
        if self.escolhido.borrow().as_ref().is_none_or(|m| !l.aparelhos.iter().any(|a| a.mac == *m && a.pareado)) {
            // the first of the list: the connected ones first, by name
            let mut meus: Vec<&Aparelho> = l.aparelhos.iter().filter(|a| a.pareado).collect();
            meus.sort_by_key(|a| (!a.conectado, a.nome.to_lowercase()));
            *self.escolhido.borrow_mut() = meus.first().map(|a| a.mac.clone());
        }
        let primeira = antes.is_none();
        *self.leitura.borrow_mut() = Some(l);
        self.lendo.set(false);
        self.painel_render();
        self.marcar();
        if depurando() {
            let mut x = self.meus.first_child();
            while let Some(f) = x {
                crate::central::posicao_debug(&format!("cfg bt linha {}", f.widget_name()), &f);
                x = f.next_sibling();
            }
            let mut x = self.perto.first_child();
            while let Some(f) = x {
                if let Some(b) = f.last_child() {
                    crate::central::posicao_debug(&format!("cfg bt parear {}", b.widget_name()), &b);
                }
                x = f.next_sibling();
            }
        }
        if depurando() && primeira {
            let l = self.leitura.borrow();
            let l = l.as_ref().unwrap();
            eprintln!("debug bluetooth: {} meus, {} por perto, {} conectados", l.aparelhos.iter().filter(|a| a.pareado).count(),
                l.aparelhos.iter().filter(|a| !a.pareado).count(), conectados);
        }
    }

    fn listas(self: &Rc<Self>, l: &Leitura) {
        for b in [&self.meus, &self.perto] {
            while let Some(x) = b.first_child() {
                b.remove(&x);
            }
        }
        let mut meus: Vec<&Aparelho> = l.aparelhos.iter().filter(|a| a.pareado).collect();
        meus.sort_by_key(|a| (!a.conectado, a.nome.to_lowercase()));
        if meus.is_empty() {
            self.meus.append(&esquerda(tr("No device paired yet."), "cfg-sub"));
        }
        for a in meus {
            self.meus.append(&self.linha_aparelho(a, l));
        }
        for a in l.aparelhos.iter().filter(|a| !a.pareado) {
            self.perto.append(&self.linha_perto(a));
        }
    }

    fn linha_aparelho(self: &Rc<Self>, a: &Aparelho, l: &Leitura) -> gtk::Button {
        let b = gtk::Button::new();
        b.add_css_class("cfg-bt-linha");
        b.set_widget_name(&a.mac);
        let r = caixa(Orientation::Horizontal, 12, "");
        r.append(&tile(glifo(&a.icone), if a.conectado { "#A5A1E2" } else { "#8E8F99" }, if a.conectado { "acento" } else { "apagado" }, 34));
        let t = caixa(Orientation::Vertical, 2, "");
        t.set_hexpand(true);
        t.set_valign(Align::Center);
        let n = esquerda(&a.nome, "cfg-nome-13");
        n.set_ellipsize(gtk::pango::EllipsizeMode::End);
        t.append(&n);
        let estado = if a.conectado {
            match l.codecs.get(&a.mac.to_uppercase()) {
                Some(c) => trf("connected · {codec} audio", &[("codec", c.clone())]),
                None => tr("connected").to_string(),
            }
        } else {
            tr("paired · not connected").to_string()
        };
        t.append(&esquerda(&estado, if a.conectado { "cfg-estado" } else { "cfg-sub" }));
        r.append(&t);
        if let Some(p) = a.bateria {
            let bat = caixa(Orientation::Vertical, 4, "");
            bat.set_valign(Align::Center);
            bat.set_size_request(90, -1);
            let v = rotulo(&format!("{p}%"), "cfg-sub");
            v.set_xalign(1.0);
            bat.append(&v);
            let barra = gtk::LevelBar::for_interval(0.0, 100.0);
            barra.add_css_class("cfg-bt-bateria");
            barra.set_value(f64::from(p));
            bat.append(&barra);
            r.append(&bat);
        }
        r.append(&ui_icon("chevron", "#7A7B88", 16));
        b.set_child(Some(&r));
        let (w, mac) = (Rc::downgrade(self), a.mac.clone());
        b.connect_clicked(move |_| {
            if let Some(s) = w.upgrade() {
                *s.escolhido.borrow_mut() = Some(mac.clone());
                s.p_erro.set_visible(false);
                s.painel_render();
                s.marcar();
                s.moldura.mostrar_painel(true);
            }
        });
        b
    }

    fn linha_perto(self: &Rc<Self>, a: &Aparelho) -> gtk::Box {
        let r = caixa(Orientation::Horizontal, 12, "cfg-bt-linha");
        r.add_css_class("perto");
        r.append(&tile(glifo(&a.icone), "#7CC3DF", "azul", 34));
        let t = caixa(Orientation::Vertical, 2, "");
        t.set_hexpand(true);
        t.set_valign(Align::Center);
        let n = esquerda(&a.nome, "cfg-nome-13");
        n.set_ellipsize(gtk::pango::EllipsizeMode::End);
        t.append(&n);
        t.append(&esquerda(tr("ready to pair"), "cfg-sub"));
        r.append(&t);
        let parear = gtk::Button::with_label(tr("Pair"));
        parear.set_widget_name(&a.nome);
        parear.add_css_class("cfg-bt-parear");
        parear.set_valign(Align::Center);
        let (w, mac, nome) = (Rc::downgrade(self), a.mac.clone(), a.nome.clone());
        parear.connect_clicked(move |b| {
            let Some(s) = w.upgrade() else { return };
            b.set_sensitive(false);
            b.set_label(tr("Pairing…"));
            // pairing (through the shell's own BlueZ agent: a keyboard's or a phone's code shows in the pairing dialog),
            // trusting it (it reconnects by itself) and connecting
            let (mac, nome, w) = (mac.clone(), nome.clone(), Rc::downgrade(&s));
            let agente = crate::bluez_agent::connection();
            crate::lateral::em_segundo_plano(move || {
                match &agente {
                    Some(c) => crate::bluez_agent::pair_on(c, &mac),
                    None => bt(&["--agent", "NoInputNoOutput", "pair", &mac]).map(|_| ()),
                }
                .and_then(|_| bt(&["trust", &mac])).and_then(|_| bt(&["connect", &mac])).map(|_| mac)
            }, move |res| {
                let Some(s) = w.upgrade() else { return };
                match res {
                    Ok(mac) => {
                        *s.escolhido.borrow_mut() = Some(mac);
                        if depurando() {
                            eprintln!("debug bluetooth: pareado {nome}");
                        }
                    }
                    Err(e) => {
                        s.p_erro.set_text(&trf("Could not pair {name}: {e}", &[("name", nome.clone()), ("e", e)]));
                        s.p_erro.set_visible(true);
                    }
                }
                s.atualizar();
            });
        });
        r.append(&parear);
        r
    }

    fn marcar(&self) {
        let e = self.escolhido.borrow().clone().unwrap_or_default();
        let mut x = self.meus.first_child();
        while let Some(f) = x {
            if f.widget_name() == e {
                f.add_css_class("escolhida");
            } else {
                f.remove_css_class("escolhida");
            }
            x = f.next_sibling();
        }
    }

    fn aparelho(&self) -> Option<Aparelho> {
        let m = self.escolhido.borrow().clone()?;
        self.leitura.borrow().as_ref()?.aparelhos.iter().find(|a| a.mac == m).cloned()
    }

    fn painel_render(&self) {
        let Some(a) = self.aparelho() else {
            self.p_vazio.set_visible(true);
            self.p_detalhes.set_visible(false);
            return;
        };
        self.lendo.set(true);
        self.p_vazio.set_visible(false);
        self.p_detalhes.set_visible(true);
        let g = glifo(&a.icone);
        while let Some(x) = self.p_rotulo.first_child() {
            self.p_rotulo.remove(&x);
        }
        self.p_rotulo.append(&rotulo16(g, tr("SELECTED DEVICE")));
        while let Some(x) = self.p_tile.first_child() {
            self.p_tile.remove(&x);
        }
        // (the design's 18 px glyph with its 2.4 px line: the 16 px one drawn at 26)
        let t = tile_px(g, if a.conectado { "#A5A1E2" } else { "#8E8F99" }, if a.conectado { "acento" } else { "apagado" }, 56, 26);
        t.add_css_class("grande");
        self.p_tile.append(&t);
        self.p_nome.set_text(&a.nome);
        let agora = glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0);
        let desde = conectado_desde(&a.mac, a.conectado);
        self.p_estado.set_text(&if a.conectado { trf("connected {ago}", &[("ago", crate::configuracoes_rede::ha(agora - desde))]) } else { tr("not connected").to_string() });
        if a.conectado {
            self.p_estado.remove_css_class("cfg-sub");
            self.p_estado.add_css_class("cfg-estado");
        } else {
            self.p_estado.remove_css_class("cfg-estado");
            self.p_estado.add_css_class("cfg-sub");
        }
        self.p_bateria.set_text(&a.bateria.map(|b| format!("{b}%")).unwrap_or_default());
        self.p_bateria_linha.set_visible(a.bateria.is_some());
        let codec = self.leitura.borrow().as_ref().and_then(|l| l.codecs.get(&a.mac.to_uppercase()).cloned());
        self.p_codec.set_text(codec.as_deref().unwrap_or(""));
        self.p_codec_linha.set_visible(codec.is_some() && a.conectado);
        let audio = e_audio(&a.icone);
        self.p_mic_linha.set_visible(audio);
        let mic = microfone_ligado();
        self.p_mic.set_active(mic);
        self.p_mic.set_state(mic);
        self.p_conectar.set_label(if a.conectado { tr("Disconnect") } else { tr("Connect") });
        self.lendo.set(false);
        // (for the session test: the panel's buttons where they are now)
        if depurando() {
            crate::central::posicao_debug("cfg bt conectar", self.p_conectar.upcast_ref());
            crate::central::posicao_debug("cfg bt esquecer", self.p_esquecer.upcast_ref());
        }
    }

    /// Runs bluetoothctl off the interface thread and reads again.
    fn acao(self: &Rc<Self>, args: Vec<String>, o_que: String) {
        self.p_erro.set_visible(false);
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(move || {
            let a: Vec<&str> = args.iter().map(String::as_str).collect();
            bt(&a)
        }, move |res| {
            let Some(b) = w.upgrade() else { return };
            match res {
                Ok(_) => {
                    if depurando() {
                        eprintln!("debug bluetooth: {o_que}");
                    }
                }
                Err(e) => {
                    b.p_erro.set_text(&e);
                    b.p_erro.set_visible(true);
                }
            }
            b.atualizar();
        });
    }

    /// WirePlumber's policy file, and WirePlumber again so it takes it (outside the development session, which runs on
    /// someone's own sound).
    fn mudar_microfone(&self, on: bool) {
        let conteudo = format!("-- KalangoOS (Configurações › Bluetooth › \"Usar como microfone em chamadas\")\nbluetooth_policy.policy[\"media-role.use-headset-profile\"] = {on}\n");
        if std::env::var_os("KALANGO_SHELL_DEV").is_some() {
            if depurando() {
                eprintln!("debug bluetooth: [sessão de teste] microfone em chamadas {on} não foi gravado");
            }
            return;
        }
        let p = microfone_path();
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(&p, conteudo);
        std::thread::spawn(|| {
            let _ = std::process::Command::new("systemctl").args(["--user", "restart", "wireplumber"]).status();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_device_from_bluetoothctl_info() {
        let info = "Device C8:24:78:67:3D:CF (public)\n\tName: EDIFIER W800BT Pro\n\tAlias: EDIFIER W800BT Pro\n\tIcon: audio-headset\n\tPaired: yes\n\tConnected: yes\n\tBattery Percentage: 0x50 (80)\n";
        let a = ler_info("C8:24:78:67:3D:CF", info);
        assert_eq!(a.nome, "EDIFIER W800BT Pro");
        assert!(a.pareado && a.conectado);
        assert_eq!(a.bateria, Some(80));
        assert_eq!(glifo(&a.icone), "fone");
        assert!(e_audio(&a.icone));
        let b = ler_info("11:22", "Device 11:22\n\tName: Mouse\n\tIcon: input-mouse\n\tPaired: no\n\tConnected: no\n");
        assert!(!b.pareado && b.bateria.is_none());
        assert_eq!(glifo(&b.icone), "mouse-d");
    }
}
