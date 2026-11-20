//! Configurações › Som (design "Configurações · Som"): the outputs and the inputs as PipeWire has them (pw-dump),
//! the one in use chosen by a click (wpctl set-default), its volume and balance, the microphone's level following it
//! live while the page shows (pw-record, read and dropped: nothing is kept), and in the panel each app's volume and
//! the system's behaviour — the music lower during calls, a sound when the volume changes, the system's sounds.
//!
//! Spatial audio and the AI noise reduction exist only where the system has them (a spatializer, the RNNoise filter):
//! here they show for a device that has them, and stay hidden otherwise. The agents do not speak yet: their voice
//! has no setting until they do.

use crate::configuracoes::{depurando, interruptor};
use crate::configuracoes_moldura::Moldura;
use crate::conversa::ui_icon;
use crate::superficies::{caixa, rotulo};
use gtk::prelude::*;
use gtk::{glib, Align, Orientation};
use kalango_i18n::{tr, trf};
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io::Read;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// PipeWire's commands (stand-ins in the session test).
pub fn comando(nome: &str) -> String {
    let var = match nome {
        "pw-dump" => "KALANGO_PW_DUMP",
        "wpctl" => "KALANGO_WPCTL",
        "pw-cli" => "KALANGO_PW_CLI",
        "pw-record" => "KALANGO_PW_RECORD",
        _ => return nome.to_string(),
    };
    std::env::var(var).ok().filter(|s| !s.is_empty()).unwrap_or_else(|| nome.to_string())
}

fn executar(nome: &str, args: &[&str]) -> Result<String, String> {
    let o = crate::sistema::output_timeout(std::process::Command::new(comando(nome)).args(args).env("LC_ALL", "C"), crate::sistema::STATUS_LIMIT)
        .map_err(|e| e.to_string())?;
    if o.status.success() {
        Ok(String::from_utf8_lossy(&o.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&o.stderr).trim().to_string())
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct No {
    pub id: u32,
    pub classe: String,
    pub nome: String,
    pub descricao: String,
    /// "Bluetooth · LDAC", "Analógico", "HDMI", "USB", "Virtual"
    pub tipo: String,
    pub glifo: &'static str,
    /// the app of a stream
    pub app: String,
    pub icone_app: String,
    /// left and right (and more) volumes, linear
    pub canais: Vec<f64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Leitura {
    pub saidas: Vec<No>,
    pub entradas: Vec<No>,
    pub apps: Vec<No>,
    pub gravando: Vec<No>,
    pub saida_padrao: String,
    pub entrada_padrao: String,
    /// "Áudio espacial" on: its sink (the default) plays into these headphones (their node's name)
    pub spatial: Option<String>,
}

/// PipeWire's objects (`pw-dump`): the sinks, the sources (not the monitors of sinks), the apps playing and recording,
/// and the defaults.
pub fn ler_pw(v: &Value) -> Leitura {
    let objetos = v.as_array().cloned().unwrap_or_default();
    let props = |o: &Value| o["info"]["props"].clone();
    let dispositivos: HashMap<u64, Value> = objetos.iter().filter(|o| o["type"] == "PipeWire:Interface:Device").filter_map(|o| Some((o["id"].as_u64()?, props(o)))).collect();
    let mut l = Leitura::default();
    for o in &objetos {
        // (the real pw-dump has a metadata's name in its top-level "props"; older ones in "info")
        if o["type"] == "PipeWire:Interface:Metadata" && (o["props"]["metadata.name"] == "default" || props(o)["metadata.name"] == "default") {
            for m in o["metadata"].as_array().cloned().unwrap_or_default() {
                let nome = m["value"]["name"].as_str().unwrap_or("").to_string();
                match m["key"].as_str() {
                    Some("default.audio.sink") => l.saida_padrao = nome,
                    Some("default.audio.source") => l.entrada_padrao = nome,
                    _ => {}
                }
            }
        }
        if o["type"] != "PipeWire:Interface:Node" {
            continue;
        }
        let p = props(o);
        let classe = p["media.class"].as_str().unwrap_or("").to_string();
        // the spatial audio's own nodes are not devices or apps: its stream tells the headphones it plays into
        if let Some(n) = p["node.name"].as_str().filter(|n| n.starts_with(SPATIAL_NODE)) {
            if n == SPATIAL_STREAM {
                l.spatial = p["target.object"].as_str().map(str::to_string);
            }
            continue;
        }
        let disp = p["device.id"].as_u64().and_then(|d| dispositivos.get(&d)).cloned().unwrap_or(Value::Null);
        let nome = p["node.name"].as_str().unwrap_or("").to_string();
        let api = p["device.api"].as_str().or(disp["device.api"].as_str()).unwrap_or("");
        let bus = disp["device.bus"].as_str().unwrap_or("");
        let codec = p["api.bluez5.codec"].as_str().unwrap_or("");
        let entrada = classe == "Audio/Source";
        let tipo = if api == "bluez5" {
            if codec.is_empty() { "Bluetooth".to_string() } else { format!("Bluetooth · {}", codec.to_uppercase()) }
        } else if nome.contains("hdmi") || nome.contains("HDMI") {
            "HDMI".into()
        } else if bus == "usb" {
            "USB".into()
        } else if api == "alsa" {
            (if entrada { tr("Analog input") } else { tr("Analog") }).into()
        } else {
            tr("Virtual").into()
        };
        let forma = disp["device.form_factor"].as_str().or(p["device.form_factor"].as_str()).unwrap_or("");
        let glifo = if api == "bluez5" || forma == "headset" || forma == "headphone" {
            "fone"
        } else if tipo == "HDMI" {
            "monitor"
        } else if entrada {
            "microfone-d"
        } else {
            "volume"
        };
        let canais = o["info"]["params"]["Props"].as_array().and_then(|a| a.iter().find_map(|x| x["channelVolumes"].as_array().cloned()))
            .map(|a| a.iter().filter_map(Value::as_f64).collect()).unwrap_or_default();
        let no = No {
            id: o["id"].as_u64().unwrap_or(0) as u32,
            classe: classe.clone(),
            nome,
            descricao: p["node.description"].as_str().or(p["node.nick"].as_str()).unwrap_or("").to_string(),
            tipo,
            glifo,
            app: p["application.name"].as_str().unwrap_or("").to_string(),
            icone_app: p["application.icon-name"].as_str().or(p["application.process.binary"].as_str()).unwrap_or("").to_string(),
            canais,
        };
        match classe.as_str() {
            "Audio/Sink" => l.saidas.push(no),
            "Audio/Source" => l.entradas.push(no),
            "Stream/Output/Audio" => l.apps.push(no),
            "Stream/Input/Audio" => l.gravando.push(no),
            _ => {}
        }
    }
    // spatial audio as the default: the person chose the headphones
    if l.saida_padrao == SPATIAL_NODE {
        match l.spatial.clone() {
            Some(target) => l.saida_padrao = target,
            None => l.spatial = None,
        }
    } else {
        l.spatial = None;
    }
    l
}

fn ler() -> (Leitura, HashMap<u32, f64>) {
    let l = executar("pw-dump", &[]).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).map(|v| ler_pw(&v)).unwrap_or_default();
    // the volumes as the person sees them (wpctl's, not the linear ones)
    let mut volumes = HashMap::new();
    for n in l.saidas.iter().chain(&l.entradas).chain(&l.apps) {
        if let Ok(t) = executar("wpctl", &["get-volume", &n.id.to_string()]) {
            if let Some(v) = t.split_whitespace().nth(1).and_then(|x| x.parse::<f64>().ok()) {
                volumes.insert(n.id, if t.contains("MUTED") { 0.0 } else { v });
            }
        }
    }
    (l, volumes)
}

/// The settings of this page kept by the shell (configuracoes.json).
fn ajuste(chave: &str, padrao: bool) -> bool {
    crate::configuracoes::ajustes()[chave].as_bool().unwrap_or(padrao)
}

/// The system's sounds (notifications) at the chosen volume, through libcanberra; none in the session test.
pub fn tocar(som: &str) {
    play_at(som, crate::configuracoes::ajustes()["sons_do_sistema"].as_f64().unwrap_or(0.4));
}

/// A sound of the theme at this volume (0 – 1): the notifications have one of their own.
pub fn play_at(som: &str, v: f64) {
    // Accessibility › visual alerts: the screen flashes softly as well
    crate::descrever::piscar();
    if std::env::var("KALANGO_SONS").as_deref() == Ok("nao") {
        if depurando() {
            eprintln!("debug som: tocaria {som} ({}%)", (v * 100.0).round());
        }
        return;
    }
    if v <= 0.01 {
        return;
    }
    // (canberra's volume is in decibels)
    let db = (20.0 * v.log10()).max(-60.0);
    let som = som.to_string();
    std::thread::spawn(move || {
        let _ = std::process::Command::new("canberra-gtk-play").args(["-i", &som, "-V", &format!("{db:.1}")])
            .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status();
    });
}

/// Called after the shell changes the volume: the change's sound, if the person wants it.
pub fn ao_mudar_volume() {
    if ajuste("som_ao_mudar_volume", false) {
        tocar("audio-volume-change");
    }
}

thread_local! {
    /// the players lowered for a call: (player, its volume before)
    static ABAIXADOS: RefCell<Vec<(String, f64)>> = RefCell::default();
}

/// Every few seconds: during a call (an app recording while it plays — a voice call), the music players go down to a
/// quarter; after it, they come back.
pub fn vigiar_chamadas(shell: &crate::Shell) {
    if !ajuste("baixar_musica_em_chamadas", true) {
        return;
    }
    let players = shell.midia.clone();
    // (only with music playing, or lowered and waiting to come back: pw-dump writes the whole PipeWire graph, and
    // every 3 s with nothing playing it took a third of a CPU in a VM for nothing)
    if players.volumes_tocando().is_empty() && ABAIXADOS.with(|a| a.borrow().is_empty()) {
        return;
    }
    crate::lateral::em_segundo_plano(|| executar("pw-dump", &[]).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).map(|v| ler_pw(&v)), move |l| {
        let Some(l) = l else { return };
        let em_chamada = l.gravando.iter().any(|g| !g.app.is_empty() && l.apps.iter().any(|a| a.app == g.app) && !g.app.to_lowercase().contains("kalango"));
        ABAIXADOS.with(|a| {
            let mut a = a.borrow_mut();
            if em_chamada && a.is_empty() {
                for (nome, v) in players.volumes_tocando() {
                    players.definir_volume(&nome, v * 0.25);
                    a.push((nome, v));
                }
                if depurando() && !a.is_empty() {
                    eprintln!("debug som: chamada, música mais baixa ({})", a.len());
                }
            } else if !em_chamada && !a.is_empty() {
                for (nome, v) in a.drain(..) {
                    players.definir_volume(&nome, v);
                }
                if depurando() {
                    eprintln!("debug som: fim da chamada, música de volta");
                }
            }
        });
    });
}

// ------------------------------------------------------------ spatial audio

/// The spatial audio's sink, and the stream that plays it into the headphones.
const SPATIAL_NODE: &str = "kalango_espacial";
const SPATIAL_STREAM: &str = "kalango_espacial.saida";

/// PipeWire's own command: it runs a filter as a client of its own (a stand-in in the session test).
fn pipewire_cmd() -> String {
    std::env::var("KALANGO_PW_FILTRO").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "pipewire".into())
}

fn spatial_conf_path() -> std::path::PathBuf {
    std::env::var_os("XDG_STATE_HOME").map(std::path::PathBuf::from).filter(|p| p.is_absolute())
        .unwrap_or_else(|| glib::home_dir().join(".local/state")).join("kalango/audio-espacial.conf")
}

/// "Áudio espacial" for headphones: a crossfeed made only of PipeWire's own filters (nothing to install) — each ear
/// also hears the other channel, low-passed and 0.3 ms later, as from speakers in front of the person.
pub fn spatial_conf(target: &str, name: &str) -> String {
    format!(r#"# KalangoOS (Configurações › Som › "Áudio espacial"): run as `pipewire -c` (a client of its own)
context.properties = {{ log.level = 0 }}
context.spa-libs = {{ audio.convert.* = audioconvert/libspa-audioconvert support.* = support/libspa-support }}
context.modules = [
  {{ name = libpipewire-module-rt args = {{ }} flags = [ ifexists nofail ] }}
  {{ name = libpipewire-module-protocol-native }}
  {{ name = libpipewire-module-client-node }}
  {{ name = libpipewire-module-adapter }}
  {{ name = libpipewire-module-filter-chain
    args = {{
      node.description = "{name}"
      media.name = "{name}"
      filter.graph = {{
        nodes = [
          {{ type = builtin name = eL label = copy }}
          {{ type = builtin name = eR label = copy }}
          {{ type = builtin name = pL label = bq_lowpass control = {{ "Freq" = 700.0 "Q" = 0.5 }} }}
          {{ type = builtin name = pR label = bq_lowpass control = {{ "Freq" = 700.0 "Q" = 0.5 }} }}
          {{ type = builtin name = aL label = delay config = {{ "max-delay" = 0.01 }} control = {{ "Delay (s)" = 0.0003 }} }}
          {{ type = builtin name = aR label = delay config = {{ "max-delay" = 0.01 }} control = {{ "Delay (s)" = 0.0003 }} }}
          {{ type = builtin name = mL label = mixer control = {{ "Gain 1" = 0.8 "Gain 2" = 0.3 }} }}
          {{ type = builtin name = mR label = mixer control = {{ "Gain 1" = 0.8 "Gain 2" = 0.3 }} }}
        ]
        links = [
          {{ output = "eL:Out" input = "mL:In 1" }}
          {{ output = "eL:Out" input = "pL:In" }}
          {{ output = "pL:Out" input = "aL:In" }}
          {{ output = "aL:Out" input = "mR:In 2" }}
          {{ output = "eR:Out" input = "mR:In 1" }}
          {{ output = "eR:Out" input = "pR:In" }}
          {{ output = "pR:Out" input = "aR:In" }}
          {{ output = "aR:Out" input = "mL:In 2" }}
        ]
        inputs = [ "eL:In" "eR:In" ]
        outputs = [ "mL:Out" "mR:Out" ]
      }}
      audio.channels = 2
      audio.position = [ FL FR ]
      capture.props = {{ node.name = "{SPATIAL_NODE}" media.class = Audio/Sink }}
      playback.props = {{ node.name = "{SPATIAL_STREAM}" node.passive = true target.object = "{target}" stream.dont-remix = true }}
    }}
  }}
]
"#, target = target.replace('"', ""), name = name.replace('"', ""))
}

/// The filter's processes still running from before (a shell that started again): the same configuration file.
fn spatial_processes() -> Vec<i32> {
    let conf = spatial_conf_path().to_string_lossy().into_owned();
    std::fs::read_dir("/proc").map(|d| d.flatten().filter_map(|e| {
        let pid = e.file_name().to_str()?.parse::<i32>().ok()?;
        let cmd = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
        let args: Vec<String> = cmd.split(|b| *b == 0).map(|a| String::from_utf8_lossy(a).into_owned()).collect();
        (args.iter().any(|a| a == "-c") && args.iter().any(|a| *a == conf)).then_some(pid)
    }).collect()).unwrap_or_default()
}

fn stop_spatial() {
    for pid in spatial_processes() {
        let _ = std::process::Command::new("kill").arg(pid.to_string()).status();
    }
}

/// Starts the filter playing into these headphones; the shell makes its sink the default once PipeWire has it.
fn start_spatial(target: &str) -> Result<(), String> {
    stop_spatial();
    let f = spatial_conf_path();
    if let Some(d) = f.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    std::fs::write(&f, spatial_conf(target, tr("Spatial audio"))).map_err(|e| e.to_string())?;
    std::process::Command::new(pipewire_cmd()).arg("-c").arg(&f)
        .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
        .spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// The id of a node by its name, from PipeWire now.
fn node_id(nome: &str) -> Option<u32> {
    let v: Value = serde_json::from_str(&executar("pw-dump", &[]).ok()?).ok()?;
    v.as_array()?.iter().find(|o| o["info"]["props"]["node.name"] == nome).and_then(|o| o["id"].as_u64()).map(|i| i as u32)
}

/// On: the filter starts and its sink becomes the default (kept: it starts again with the session). Off: the
/// headphones are the default again and the filter ends.
pub fn set_spatial(target: Option<String>) -> Result<(), String> {
    match target {
        Some(a) => {
            start_spatial(&a)?;
            crate::configuracoes::salvar_ajuste("spatial_audio", json!(a));
            for _ in 0..30 {
                if let Some(id) = node_id(SPATIAL_NODE) {
                    executar("wpctl", &["set-default", &id.to_string()])?;
                    return Ok(());
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(tr("PipeWire did not start the spatial audio").to_string())
        }
        None => {
            let antes = crate::configuracoes::ajustes()["spatial_audio"].as_str().map(str::to_string);
            if let Some(id) = antes.as_deref().and_then(node_id) {
                let _ = executar("wpctl", &["set-default", &id.to_string()]);
            }
            crate::configuracoes::salvar_ajuste("spatial_audio", Value::Null);
            stop_spatial();
            Ok(())
        }
    }
}

/// At the session's start: the spatial audio the person left on (WirePlumber remembers its sink as the default).
/// The development session runs on someone's own sound: there it starts only when turned on.
pub fn resume_spatial() {
    let Some(target) = crate::configuracoes::ajustes()["spatial_audio"].as_str().map(str::to_string) else { return };
    if std::env::var_os("KALANGO_SHELL_DEV").is_some() && std::env::var_os("KALANGO_PW_FILTRO").is_none_or(|v| v.is_empty()) {
        return;
    }
    if spatial_processes().is_empty() {
        if let Err(e) = start_spatial(&target) {
            eprintln!("kalango-shell: áudio espacial: {e}");
        }
    }
}

/// The RNNoise filter (for "Redução de ruído com IA"), where it is installed.
fn rnnoise() -> Option<std::path::PathBuf> {
    ["/usr/lib/ladspa/librnnoise_ladspa.so", "/usr/lib/x86_64-linux-gnu/ladspa/librnnoise_ladspa.so", "/usr/local/lib/ladspa/librnnoise_ladspa.so"]
        .iter().map(std::path::PathBuf::from).find(|p| p.is_file())
}

fn ruido_path() -> std::path::PathBuf {
    glib::user_config_dir().join("pipewire/pipewire.conf.d/99-kalango-reducao-de-ruido.conf")
}

/// "Redução de ruído com IA": PipeWire's filter chain with RNNoise makes a clean microphone ("Microfone (sem
/// ruído)") from the one in use; PipeWire starts again to take it (outside the development session, which runs on
/// someone's own sound).
fn mudar_ruido(on: bool) {
    let Some(plugin) = rnnoise() else { return };
    if std::env::var_os("KALANGO_SHELL_DEV").is_some() {
        if depurando() {
            eprintln!("debug som: [sessão de teste] redução de ruído {on} não foi gravada");
        }
        return;
    }
    let p = ruido_path();
    if on {
        let conf = format!(r#"# KalangoOS (Configurações › Som › "Redução de ruído com IA")
context.modules = [
  {{ name = libpipewire-module-filter-chain
    args = {{
      node.description = "{desc}"
      media.name = "{desc}"
      filter.graph = {{ nodes = [ {{ type = ladspa name = rnnoise plugin = "{plugin}" label = noise_suppressor_mono control = {{ "VAD Threshold (%)" = 50.0 }} }} ] }}
      capture.props = {{ node.name = "capture.kalango_rnnoise" node.passive = true audio.rate = 48000 }}
      playback.props = {{ node.name = "kalango_rnnoise_source" media.class = Audio/Source audio.rate = 48000 }}
    }}
  }}
]
"#, desc = tr("Microphone (no noise)"), plugin = plugin.display());
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(&p, conf);
    } else {
        let _ = std::fs::remove_file(&p);
    }
    std::thread::spawn(|| {
        let _ = std::process::Command::new("systemctl").args(["--user", "restart", "pipewire", "pipewire-pulse", "wireplumber"]).status();
    });
}

// ------------------------------------------------------------ the page

pub struct Som {
    pub moldura: Rc<Moldura>,
    saidas: gtk::Box,
    entradas: gtk::Box,
    volume: gtk::Scale,
    volume_txt: gtk::Label,
    equilibrio: gtk::Scale,
    equilibrio_linha: gtk::Box,
    spatial: gtk::Switch,
    spatial_row: gtk::Box,
    nivel: gtk::DrawingArea,
    ruido_linha: gtk::Box,
    ruido: gtk::Switch,
    apps: gtk::Box,
    sistema: gtk::Scale,
    baixar: gtk::Switch,
    som_volume: gtk::Switch,
    leitura: RefCell<Option<(Leitura, HashMap<u32, f64>)>>,
    lendo: Cell<bool>,
    /// the microphone's level now (0 – 1000), from the reading thread
    medida: Arc<AtomicU32>,
    /// the reading: its process and the source it reads
    medidor: RefCell<Option<(std::process::Child, u32)>>,
    timer: RefCell<Option<glib::SourceId>>,
    quadros: RefCell<Option<glib::SourceId>>,
}

fn esquerda(t: &str, c: &str) -> gtk::Label {
    let l = rotulo(t, c);
    l.set_xalign(0.0);
    l
}

fn rotulo16(glifo: &str, texto: &str) -> gtk::Box {
    let r = caixa(Orientation::Horizontal, 6, "");
    r.append(&ui_icon(glifo, "#8E8F99", 16));
    r.append(&rotulo(texto, "cfg-rotulo"));
    r
}

fn escala(min: f64, max: f64) -> gtk::Scale {
    let s = gtk::Scale::with_range(Orientation::Horizontal, min, max, 0.01);
    s.add_css_class("cfg-escala");
    s.set_draw_value(false);
    s.set_hexpand(true);
    s
}

/// The design's line with a title (and a line under it) and a control at the right, 10 px in.
fn linha(titulo: &str, sub: Option<&str>, controle: &impl IsA<gtk::Widget>) -> gtk::Box {
    let r = caixa(Orientation::Horizontal, 12, "cfg-tela-opcao");
    let t = caixa(Orientation::Vertical, 1, "");
    t.set_hexpand(true);
    t.set_valign(Align::Center);
    t.append(&esquerda(titulo, "cfg-texto-13"));
    if let Some(s) = sub {
        t.append(&esquerda(s, "cfg-sub"));
        r.add_css_class("alta");
    }
    r.append(&t);
    controle.set_valign(Align::Center);
    r.append(controle);
    r
}

impl Som {
    pub fn new(c: &Rc<crate::configuracoes::Configuracoes>) -> Rc<Som> {
        let m = Moldura::new(tr("Sound"), &[], tr("Apps and behaviour"));
        m.busca_grande();
        // output
        let cs = caixa(Orientation::Vertical, 12, "cfg-cartao");
        cs.append(&rotulo16("volume", tr("OUTPUT")));
        let saidas = caixa(Orientation::Horizontal, 10, "");
        saidas.set_homogeneous(true);
        cs.append(&saidas);
        let vr = caixa(Orientation::Horizontal, 10, "");
        vr.append(&ui_icon("volume", "#8E8F99", 16));
        let volume = escala(0.0, 1.0);
        vr.append(&volume);
        let volume_txt = rotulo("", "cfg-sub");
        volume_txt.set_width_chars(4);
        volume_txt.set_xalign(1.0);
        vr.append(&volume_txt);
        cs.append(&vr);
        let opcoes = caixa(Orientation::Vertical, 2, "");
        let equilibrio = gtk::Scale::with_range(Orientation::Horizontal, -1.0, 1.0, 0.05);
        equilibrio.add_css_class("cfg-escala");
        equilibrio.set_draw_value(false);
        equilibrio.set_size_request(200, -1);
        let equilibrio_linha = linha(tr("Balance"), None, &equilibrio);
        opcoes.append(&equilibrio_linha);
        let spatial = interruptor();
        let spatial_row = linha(tr("Spatial audio"), Some(tr("available for these headphones")), &spatial);
        spatial_row.set_visible(false);
        opcoes.append(&spatial_row);
        cs.append(&opcoes);
        m.corpo.append(&cs);
        // input
        let ce = caixa(Orientation::Vertical, 12, "cfg-cartao");
        ce.append(&rotulo16("microfone-d", tr("INPUT")));
        let entradas = caixa(Orientation::Horizontal, 10, "");
        entradas.set_homogeneous(true);
        ce.append(&entradas);
        let nr = caixa(Orientation::Horizontal, 3, "");
        nr.append(&ui_icon("microfone-d", "#8E8F99", 16));
        let nivel = gtk::DrawingArea::new();
        nivel.set_content_height(18);
        nivel.set_hexpand(true);
        nr.append(&nivel);
        ce.append(&nr);
        let ruido = interruptor();
        let ruido_linha = linha(tr("AI noise reduction"), Some(tr("done by the local filter, without sending audio")), &ruido);
        ruido_linha.set_visible(rnnoise().is_some());
        ce.append(&ruido_linha);
        m.corpo.append(&ce);
        // the panel: each app's volume, the behaviour
        let ca = caixa(Orientation::Vertical, 4, "cfg-cartao");
        ca.append(&rotulo16("ajustes-d", tr("VOLUME PER APP")));
        let apps = caixa(Orientation::Vertical, 4, "");
        ca.append(&apps);
        let sistema = escala(0.0, 1.0);
        sistema.set_size_request(200, -1);
        // (200 wide, as the design's: the name takes the rest)
        sistema.set_hexpand(false);
        let sr = caixa(Orientation::Horizontal, 12, "cfg-tela-opcao");
        sr.append(&ui_icon("sino-d", "#8E8F99", 16));
        let st = esquerda(tr("System sounds"), "cfg-texto-13");
        st.set_hexpand(true);
        sr.append(&st);
        sistema.set_valign(Align::Center);
        sr.append(&sistema);
        ca.append(&sr);
        m.painel.append(&ca);
        let cc = caixa(Orientation::Vertical, 2, "cfg-cartao");
        cc.append(&rotulo16("configuracoes", tr("BEHAVIOUR")));
        let baixar = interruptor();
        cc.append(&linha(tr("Lower the music during calls"), None, &baixar));
        let som_volume = interruptor();
        cc.append(&linha(tr("Sound when the volume changes"), None, &som_volume));
        m.painel.append(&cc);
        let s = Rc::new(Som { moldura: m, saidas, entradas, volume, volume_txt, equilibrio, equilibrio_linha, spatial, spatial_row, nivel, ruido_linha, ruido, apps, sistema, baixar, som_volume,
            leitura: RefCell::default(), lendo: Cell::new(false), medida: Arc::new(AtomicU32::new(0)), medidor: RefCell::default(), timer: RefCell::default(),
            quadros: RefCell::default() });
        s.ligar(c);
        s
    }

    fn ligar(self: &Rc<Self>, c: &Rc<crate::configuracoes::Configuracoes>) {
        let w = Rc::downgrade(self);
        self.volume.connect_value_changed(move |sc| {
            let Some(s) = w.upgrade().filter(|s| !s.lendo.get()) else { return };
            let v = sc.value();
            s.volume_txt.set_text(&format!("{}%", (v * 100.0).round()));
            // the device's own volume (with spatial audio on, the default is the filter: the headphones are behind it)
            let target = s.leitura.borrow().as_ref().filter(|(l, _)| l.spatial.is_some())
                .and_then(|(l, _)| l.saidas.iter().find(|n| n.nome == l.saida_padrao).map(|n| n.id.to_string()))
                .unwrap_or_else(|| "@DEFAULT_AUDIO_SINK@".into());
            std::thread::spawn(move || {
                let _ = executar("wpctl", &["set-volume", &target, &format!("{v:.2}")]);
            });
            ao_mudar_volume();
        });
        let w = Rc::downgrade(self);
        self.equilibrio.connect_value_changed(move |sc| {
            let Some(s) = w.upgrade().filter(|s| !s.lendo.get()) else { return };
            s.equilibrar(sc.value());
        });
        let w = Rc::downgrade(self);
        self.nivel.set_draw_func(move |_, cr, w_, h_| if let Some(s) = w.upgrade() { s.desenhar_nivel(cr, f64::from(w_), f64::from(h_)) });
        let w = Rc::downgrade(self);
        self.spatial.connect_state_set(move |_, on| {
            let Some(s) = w.upgrade().filter(|s| !s.lendo.get()) else { return glib::Propagation::Proceed };
            let target = s.leitura.borrow().as_ref().map(|(l, _)| l.saida_padrao.clone()).filter(|a| !a.is_empty());
            let w = Rc::downgrade(&s);
            crate::lateral::em_segundo_plano(move || set_spatial(if on { target.clone() } else { None }).map(|_| target), move |r| {
                if depurando() {
                    match &r {
                        Ok(a) => eprintln!("debug som: áudio espacial {} ({})", if on { "ligado" } else { "desligado" }, a.clone().unwrap_or_default()),
                        Err(e) => eprintln!("debug som: áudio espacial: {e}"),
                    }
                }
                if let Some(s) = w.upgrade() {
                    if let Err(e) = r {
                        s.moldura.sub.set_text(&e);
                    }
                    s.atualizar();
                }
            });
            glib::Propagation::Proceed
        });
        let w = Rc::downgrade(self);
        self.ruido.connect_state_set(move |_, on| {
            if w.upgrade().is_some_and(|s| !s.lendo.get()) {
                mudar_ruido(on);
            }
            glib::Propagation::Proceed
        });
        let w = Rc::downgrade(self);
        self.sistema.connect_value_changed(move |sc| {
            if w.upgrade().is_some_and(|s| !s.lendo.get()) {
                crate::configuracoes::salvar_ajuste("sons_do_sistema", json!((sc.value() * 100.0).round() / 100.0));
            }
        });
        for (sw, chave) in [(&self.baixar, "baixar_musica_em_chamadas"), (&self.som_volume, "som_ao_mudar_volume")] {
            let w = Rc::downgrade(self);
            sw.connect_state_set(move |_, on| {
                if w.upgrade().is_some_and(|s| !s.lendo.get()) {
                    crate::configuracoes::salvar_ajuste(chave, json!(on));
                    if depurando() {
                        eprintln!("debug som: {chave} {on}");
                    }
                }
                glib::Propagation::Proceed
            });
        }
        crate::configuracoes::registrar_posicao(c, "cfg som volume", self.volume.upcast_ref());
        crate::configuracoes::registrar_posicao(c, "cfg som espacial", self.spatial.upcast_ref());
        crate::configuracoes::registrar_posicao(c, "cfg som baixar", self.baixar.upcast_ref());
        crate::configuracoes::registrar_posicao(c, "cfg som ao mudar", self.som_volume.upcast_ref());
    }

    pub fn compactar(&self, c: bool) {
        self.moldura.compactar(c);
    }

    /// Reads now and every 2 s while the page shows; the microphone's level is followed live.
    pub fn mostrar(self: &Rc<Self>) {
        self.atualizar();
        if self.timer.borrow().is_some() {
            return;
        }
        let w = Rc::downgrade(self);
        *self.timer.borrow_mut() = Some(glib::timeout_add_local(Duration::from_secs(2), move || {
            let Some(s) = w.upgrade() else { return glib::ControlFlow::Break };
            if !s.moldura.raiz.is_mapped() {
                s.timer.borrow_mut().take();
                s.parar_medidor();
                return glib::ControlFlow::Break;
            }
            s.atualizar();
            glib::ControlFlow::Continue
        }));
        // the meter's frames (60 a second, as the design says)
        let w = Rc::downgrade(self);
        *self.quadros.borrow_mut() = Some(glib::timeout_add_local(Duration::from_millis(16), move || {
            let Some(s) = w.upgrade() else { return glib::ControlFlow::Break };
            if !s.moldura.raiz.is_mapped() {
                s.quadros.borrow_mut().take();
                return glib::ControlFlow::Break;
            }
            s.nivel.queue_draw();
            glib::ControlFlow::Continue
        }));
    }

    fn atualizar(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        crate::lateral::em_segundo_plano(ler, move |l| {
            if let Some(s) = w.upgrade() {
                s.mostrar_leitura(l);
            }
        });
    }

    fn mostrar_leitura(self: &Rc<Self>, (l, volumes): (Leitura, HashMap<u32, f64>)) {
        let mudou = self.leitura.borrow().as_ref().map(|(a, _)| a.clone()) != Some(l.clone());
        let saida = l.saidas.iter().find(|n| n.nome == l.saida_padrao).cloned();
        let entrada = l.entradas.iter().find(|n| n.nome == l.entrada_padrao).cloned();
        self.moldura.sub.set_text(&format!("{} · {}",
            trf("Output: {name}", &[("name", saida.as_ref().map(|n| n.descricao.clone()).unwrap_or_else(|| tr("none").into()))]),
            trf("Input: {name}", &[("name", entrada.as_ref().map(|n| n.descricao.clone()).unwrap_or_else(|| tr("none").into()))])));
        if mudou {
            self.cartoes(&self.saidas, &l.saidas, &l.saida_padrao);
            self.cartoes(&self.entradas, &l.entradas, &l.entrada_padrao);
            self.apps_render(&l, &volumes);
        }
        self.lendo.set(true);
        if let Some(n) = &saida {
            let v = volumes.get(&n.id).copied().unwrap_or(0.0);
            if !self.volume.has_css_class("arrastando") {
                self.volume.set_value(v.min(1.0));
            }
            self.volume_txt.set_text(&format!("{}%", (v * 100.0).round()));
            // balance: the right channel against the left
            let eq = match n.canais.as_slice() {
                [e, d, ..] if (e + d) > 0.0 => (d - e) / e.max(*d),
                _ => 0.0,
            };
            self.equilibrio.set_value(eq);
            self.equilibrio_linha.set_visible(n.canais.len() >= 2);
        }
        // spatial audio: for headphones
        self.spatial_row.set_visible(saida.as_ref().is_some_and(|n| n.glifo == "fone"));
        self.spatial.set_active(l.spatial.is_some());
        self.spatial.set_state(l.spatial.is_some());
        self.sistema.set_value(crate::configuracoes::ajustes()["sons_do_sistema"].as_f64().unwrap_or(0.4));
        self.ruido_linha.set_visible(rnnoise().is_some());
        let ruido = ruido_path().is_file();
        self.ruido.set_active(ruido);
        self.ruido.set_state(ruido);
        for (sw, chave, padrao) in [(&self.baixar, "baixar_musica_em_chamadas", true), (&self.som_volume, "som_ao_mudar_volume", false)] {
            let on = ajuste(chave, padrao);
            sw.set_active(on);
            sw.set_state(on);
        }
        self.lendo.set(false);
        // the meter follows the input in use
        if let Some(e) = &entrada {
            self.medir(e.id);
        }
        let primeira = self.leitura.borrow().is_none();
        *self.leitura.borrow_mut() = Some((l, volumes));
        if depurando() && (primeira || mudou) {
            let l = self.leitura.borrow();
            let (l, _) = l.as_ref().unwrap();
            eprintln!("debug som: {} saídas, {} entradas, {} apps; saída {}", l.saidas.len(), l.entradas.len(), l.apps.len(), l.saida_padrao);
        }
    }

    /// The devices as the design's cards (the one in use outlined); a click makes one the default.
    fn cartoes(self: &Rc<Self>, caixa_: &gtk::Box, nos: &[No], padrao: &str) {
        while let Some(x) = caixa_.first_child() {
            caixa_.remove(&x);
        }
        if nos.is_empty() {
            caixa_.append(&esquerda(tr("No device"), "cfg-sub"));
        }
        for n in nos {
            let b = gtk::Button::new();
            b.add_css_class("cfg-som-cartao");
            b.set_valign(Align::Start);
            if n.nome == padrao {
                b.add_css_class("escolhido");
            }
            let col = caixa(Orientation::Vertical, 6, "");
            col.append(&ui_icon(n.glifo, if n.nome == padrao { "#A5A1E2" } else { "#8E8F99" }, 16));
            let t = esquerda(&n.descricao, "cfg-nome-13");
            t.set_ellipsize(gtk::pango::EllipsizeMode::End);
            col.append(&t);
            col.append(&esquerda(&n.tipo, "cfg-sub"));
            for x in [col.first_child()].into_iter().flatten() {
                x.set_halign(Align::Start);
            }
            b.set_child(Some(&col));
            let (w, id, desc) = (Rc::downgrade(self), n.id, n.descricao.clone());
            b.connect_clicked(move |_| {
                let Some(s) = w.upgrade() else { return };
                let w = Rc::downgrade(&s);
                let desc = desc.clone();
                crate::lateral::em_segundo_plano(move || executar("wpctl", &["set-default", &id.to_string()]), move |r| {
                    if depurando() {
                        eprintln!("debug som: padrão {desc} ({})", if r.is_ok() { "ok" } else { "erro" });
                    }
                    if let Some(s) = w.upgrade() {
                        s.atualizar();
                    }
                });
            });
            if depurando() {
                crate::central::posicao_debug(&format!("cfg som cartao {}", n.descricao), b.upcast_ref());
            }
            caixa_.append(&b);
        }
    }

    fn apps_render(self: &Rc<Self>, l: &Leitura, volumes: &HashMap<u32, f64>) {
        while let Some(x) = self.apps.first_child() {
            self.apps.remove(&x);
        }
        if l.apps.is_empty() {
            let v = esquerda(tr("No app playing sound now."), "cfg-sub");
            v.set_margin_start(10);
            v.set_margin_top(6);
            v.set_margin_bottom(6);
            self.apps.append(&v);
        }
        for a in &l.apps {
            let r = caixa(Orientation::Horizontal, 12, "cfg-som-app");
            let ic = gtk::Image::from_icon_name(if a.icone_app.is_empty() { "audio-x-generic" } else { &a.icone_app });
            ic.set_pixel_size(21);
            ic.set_size_request(26, 26);
            r.append(&ic);
            let n = esquerda(if a.app.is_empty() { &a.descricao } else { &a.app }, "cfg-texto-13");
            n.set_size_request(80, -1);
            n.set_ellipsize(gtk::pango::EllipsizeMode::End);
            n.set_max_width_chars(1);
            r.append(&n);
            let sc = escala(0.0, 1.0);
            sc.set_value(volumes.get(&a.id).copied().unwrap_or(1.0).min(1.0));
            let id = a.id;
            sc.connect_value_changed(move |sc| {
                let v = sc.value();
                std::thread::spawn(move || {
                    let _ = executar("wpctl", &["set-volume", &id.to_string(), &format!("{v:.2}")]);
                });
            });
            r.append(&sc);
            self.apps.append(&r);
        }
    }

    /// The balance: the louder side keeps the volume, the other goes down by as much as asked.
    fn equilibrar(&self, b: f64) {
        let Some(n) = self.leitura.borrow().as_ref().and_then(|(l, _)| l.saidas.iter().find(|n| n.nome == l.saida_padrao).cloned()) else { return };
        let base = n.canais.iter().cloned().fold(0.0f64, f64::max);
        if base <= 0.0 || n.canais.len() < 2 {
            return;
        }
        let (e, d) = (base * (1.0 - b.max(0.0)), base * (1.0 + b.min(0.0)));
        let mut canais = vec![e, d];
        canais.extend(n.canais.iter().skip(2).cloned());
        let props = format!("{{ channelVolumes: [ {} ] }}", canais.iter().map(|c| format!("{c:.3}")).collect::<Vec<_>>().join(", "));
        let id = n.id.to_string();
        std::thread::spawn(move || {
            let _ = executar("pw-cli", &["set-param", &id, "Props", &props]);
        });
    }

    // ------------------------------------------------------------ the microphone's level

    /// Starts reading the input (again when it changes): 8 kHz mono, its peak every 1/60 s; nothing is kept.
    fn medir(&self, id: u32) {
        if self.medidor.borrow().as_ref().is_some_and(|(_, i)| *i == id) {
            return;
        }
        self.parar_medidor();
        let Ok(mut filho) = std::process::Command::new(comando("pw-record")).args(["--target", &id.to_string(), "--rate", "8000", "--channels", "1", "--format", "s16", "-"])
            .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn() else { return };
        let Some(mut saida) = filho.stdout.take() else { return };
        let medida = self.medida.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 266];
            while let Ok(n) = saida.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let pico = buf[..n - n % 2].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]).unsigned_abs()).max().unwrap_or(0);
                // a quick rise, a slower fall
                let novo = (u32::from(pico) * 1000 / 32768).min(1000);
                let antes = medida.load(Ordering::Relaxed);
                medida.store(if novo > antes { novo } else { antes.saturating_sub(antes / 8).max(novo) }, Ordering::Relaxed);
            }
            medida.store(0, Ordering::Relaxed);
        });
        *self.medidor.borrow_mut() = Some((filho, id));
        if depurando() {
            eprintln!("debug som: medidor na entrada {id}");
        }
    }

    fn parar_medidor(&self) {
        if let Some((mut f, _)) = self.medidor.borrow_mut().take() {
            let _ = f.kill();
            let _ = f.wait();
        }
        self.medida.store(0, Ordering::Relaxed);
    }

    /// The design's meter: 8 × 10 bars 3 px apart, lit up to the level — teal, then gold at the top.
    fn desenhar_nivel(&self, cr: &gtk::cairo::Context, w: f64, h: f64) {
        let n = ((w + 3.0) / 11.0).floor().max(1.0) as usize;
        let acesas = (self.medida.load(Ordering::Relaxed) as f64 / 1000.0 * n as f64 * 1.6).round() as usize;
        let y = (h - 10.0) / 2.0;
        for i in 0..n {
            let x = i as f64 * 11.0 + 3.0;
            let cor = if i < acesas { if i as f64 >= n as f64 * 0.6 { "#E5C98C" } else { "#8ED2C3" } } else { "#24252D" };
            let (r, g, b) = crate::tema::rgb(cor);
            cr.set_source_rgb(r, g, b);
            cr.new_sub_path();
            let rr = 2.0;
            cr.arc(x + 8.0 - rr, y + rr, rr, -std::f64::consts::FRAC_PI_2, 0.0);
            cr.arc(x + 8.0 - rr, y + 10.0 - rr, rr, 0.0, std::f64::consts::FRAC_PI_2);
            cr.arc(x + rr, y + 10.0 - rr, rr, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
            cr.arc(x + rr, y + rr, rr, std::f64::consts::PI, 1.5 * std::f64::consts::PI);
            cr.close_path();
            let _ = cr.fill();
        }
    }
}

impl Drop for Som {
    fn drop(&mut self) {
        self.parar_medidor();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spatial_audio_is_not_a_device_and_shows_the_headphones_as_the_output() {
        let v = json!([
            {"id": 60, "type": "PipeWire:Interface:Node", "info": {"props": {"media.class": "Audio/Sink", "node.name": "bluez_output.X", "node.description": "Fone", "device.api": "bluez5"}}},
            {"id": 95, "type": "PipeWire:Interface:Node", "info": {"props": {"media.class": "Audio/Sink", "node.name": "kalango_espacial", "node.description": "Áudio espacial"}}},
            {"id": 96, "type": "PipeWire:Interface:Node", "info": {"props": {"media.class": "Stream/Output/Audio", "node.name": "kalango_espacial.saida", "target.object": "bluez_output.X"}}},
            {"id": 90, "type": "PipeWire:Interface:Metadata", "info": {"props": {"metadata.name": "default"}}, "metadata": [{"key": "default.audio.sink", "value": {"name": "kalango_espacial"}}]}
        ]);
        let l = ler_pw(&v);
        assert_eq!(l.saidas.len(), 1);
        assert!(l.apps.is_empty());
        assert_eq!(l.saida_padrao, "bluez_output.X");
        assert_eq!(l.spatial.as_deref(), Some("bluez_output.X"));
        // the filter's configuration plays into those headphones, made of PipeWire's own filters
        let c = spatial_conf("bluez_output.X", "Áudio espacial");
        assert!(c.contains("target.object = \"bluez_output.X\"") && c.contains("label = bq_lowpass") && !c.contains("ladspa"));
    }

    #[test]
    fn the_real_pw_dump_keeps_the_metadata_name_at_the_top() {
        // (as pw-dump 1.0 writes it: "props" next to "metadata", no "info")
        let v = json!([
            {"id": 42, "type": "PipeWire:Interface:Node", "info": {"props": {"media.class": "Audio/Source", "node.name": "alsa_input.usb-fifine", "node.description": "fifine Microphone"}}},
            {"id": 55, "type": "PipeWire:Interface:Node", "info": {"props": {"media.class": "Audio/Source", "node.name": "alsa_input.usb-emeet", "node.description": "EMEET SmartCam"}}},
            {"id": 36, "type": "PipeWire:Interface:Metadata", "props": {"metadata.name": "default"},
             "metadata": [{"subject": 0, "key": "default.audio.source", "type": "Spa:String:JSON", "value": {"name": "alsa_input.usb-fifine"}}]}
        ]);
        let l = ler_pw(&v);
        assert_eq!(l.entrada_padrao, "alsa_input.usb-fifine");
        assert_eq!(l.entradas.iter().find(|n| n.nome == l.entrada_padrao).map(|n| n.id), Some(42));
    }

    #[test]
    fn pipewire_objects_become_devices_apps_and_defaults() {
        let v: Value = serde_json::from_str(r#"[
          {"id": 40, "type": "PipeWire:Interface:Device", "info": {"props": {"device.api": "alsa", "device.bus": "usb", "device.form_factor": "microphone"}}},
          {"id": 41, "type": "PipeWire:Interface:Device", "info": {"props": {"device.api": "bluez5", "device.form_factor": "headset"}}},
          {"id": 60, "type": "PipeWire:Interface:Node", "info": {"props": {"media.class": "Audio/Sink", "node.name": "bluez_output.C8", "node.description": "Fone Lunar", "device.id": 41, "device.api": "bluez5", "api.bluez5.codec": "ldac"},
             "params": {"Props": [{"channelVolumes": [0.4, 0.4]}]}}},
          {"id": 61, "type": "PipeWire:Interface:Node", "info": {"props": {"media.class": "Audio/Sink", "node.name": "alsa_output.hdmi-stereo", "node.description": "Monitor MSI"}}},
          {"id": 70, "type": "PipeWire:Interface:Node", "info": {"props": {"media.class": "Audio/Source", "node.name": "alsa_input.usb-yeti", "node.description": "Microfone USB", "device.id": 40, "device.api": "alsa"}}},
          {"id": 80, "type": "PipeWire:Interface:Node", "info": {"props": {"media.class": "Stream/Output/Audio", "application.name": "Discord", "application.icon-name": "discord"}}},
          {"id": 81, "type": "PipeWire:Interface:Node", "info": {"props": {"media.class": "Stream/Input/Audio", "application.name": "Discord"}}},
          {"id": 90, "type": "PipeWire:Interface:Metadata", "info": {"props": {"metadata.name": "default"}},
           "metadata": [{"key": "default.audio.sink", "value": {"name": "bluez_output.C8"}}, {"key": "default.audio.source", "value": {"name": "alsa_input.usb-yeti"}}]}
        ]"#).unwrap();
        let l = ler_pw(&v);
        assert_eq!(l.saidas.len(), 2);
        assert_eq!(l.saidas[0].tipo, "Bluetooth · LDAC");
        assert_eq!(l.saidas[0].glifo, "fone");
        assert_eq!(l.saidas[0].canais, vec![0.4, 0.4]);
        assert_eq!(l.saidas[1].tipo, "HDMI");
        assert_eq!(l.entradas[0].tipo, "USB");
        assert_eq!(l.entradas[0].glifo, "microfone-d");
        assert_eq!((l.saida_padrao.as_str(), l.entrada_padrao.as_str()), ("bluez_output.C8", "alsa_input.usb-yeti"));
        assert_eq!(l.apps[0].app, "Discord");
        assert_eq!(l.gravando[0].app, "Discord");
    }
}
