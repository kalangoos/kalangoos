//! Estado e controles reais do sistema para os Controles e o "Hoje": rede (NetworkManager), Bluetooth (BlueZ),
//! som (PipeWire/WirePlumber), modo de energia (power-profiles-daemon), baterias (UPower), brilho (backlight),
//! CPU (/proc/stat) e GPU (nvidia-smi). As leituras rodam fora da thread da interface.
//!
//! Hoje usa as ferramentas de linha de comando de cada serviço (nmcli, bluetoothctl, wpctl, powerprofilesctl,
//! upower); a troca por D-Bus direto não muda a interface deste módulo.

use gtk::glib;
use std::process::Command;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Rede {
    pub wifi_ligado: bool,
    pub conectada: Option<String>,
    pub sinal: u8,
    pub aviao: bool,
    pub cabo: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Dispositivo {
    pub nome: String,
    pub tipo: String,
    pub bateria: Option<u8>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bluetooth {
    pub existe: bool,
    pub ligado: bool,
    pub conectados: Vec<Dispositivo>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Som {
    pub nivel: f64,
    pub mudo: bool,
    pub existe: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sistema {
    pub rede: Rede,
    pub bt: Bluetooth,
    pub som: Som,
    /// `power-saver`, `balanced`, `performance` (vazio = sem power-profiles-daemon)
    pub perfil: String,
    pub perfis: Vec<String>,
    /// bateria do computador (notebook), em %, e se está carregando
    pub bateria: Option<(u8, bool)>,
    /// brilho de 0 a 1 (None = a tela não oferece)
    pub brilho: Option<f64>,
    /// night light: available (wlsunset installed) and on
    pub luz_noturna: Option<bool>,
    /// tried, and this screen has no gamma control
    pub luz_noturna_indisponivel: bool,
    /// the person's agent engines: how many, and how many are paused
    pub agentes: (usize, usize),
}

fn sh(prog: &str, args: &[&str]) -> Option<String> {
    // (NetworkManager's command can be a stand-in: the session test's)
    let prog = match prog {
        "nmcli" => crate::configuracoes_rede::nmcli(),
        "bluetoothctl" => crate::configuracoes_bluetooth::bluetoothctl(),
        "wpctl" => crate::configuracoes_som::comando("wpctl"),
        "powerprofilesctl" => crate::configuracoes_energia::powerprofilesctl(),
        p => p.to_string(),
    };
    let o = Command::new(prog).args(args).env("LC_ALL", "C").output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).into_owned())
}

fn tem(prog: &str) -> bool {
    std::env::var_os("PATH").map(|p| std::env::split_paths(&p).any(|d| d.join(prog).is_file())).unwrap_or(false)
}

pub fn ler() -> Sistema {
    let mut s = Sistema::default();
    // rede
    if let Some(g) = sh("nmcli", &["-t", "-f", "WIFI,WWAN", "radio"]) {
        let f: Vec<&str> = g.trim().split(':').collect();
        s.rede.wifi_ligado = f.first() == Some(&"enabled");
    }
    if let Some(a) = sh("nmcli", &["-t", "-f", "NETWORKING", "general"]) {
        s.rede.aviao = a.trim() == "disabled";
    }
    if let Some(d) = sh("nmcli", &["-t", "-f", "TYPE,STATE,CONNECTION", "device"]) {
        for l in d.lines() {
            let f: Vec<&str> = l.splitn(3, ':').collect();
            if f.len() == 3 && f[1] == "connected" {
                match f[0] {
                    "wifi" => s.rede.conectada = Some(f[2].to_string()),
                    "ethernet" => s.rede.cabo = true,
                    _ => {}
                }
            }
        }
    }
    if s.rede.conectada.is_some() {
        if let Some(w) = sh("nmcli", &["-t", "-f", "IN-USE,SIGNAL", "device", "wifi", "list", "--rescan", "no"]) {
            s.rede.sinal = w.lines().find(|l| l.starts_with('*')).and_then(|l| l.split(':').nth(1)?.parse().ok()).unwrap_or(0);
        }
    }
    // Bluetooth
    if let Some(b) = sh("bluetoothctl", &["show"]) {
        s.bt.existe = b.contains("Controller");
        s.bt.ligado = b.contains("Powered: yes");
    }
    if s.bt.ligado {
        if let Some(d) = sh("bluetoothctl", &["devices", "Connected"]) {
            for l in d.lines() {
                let f: Vec<&str> = l.splitn(3, ' ').collect();
                if f.len() == 3 {
                    let info = sh("bluetoothctl", &["info", f[1]]).unwrap_or_default();
                    let tipo = info.lines().find_map(|l| l.trim().strip_prefix("Icon: ")).unwrap_or("").to_string();
                    let bateria = info.lines().find_map(|l| l.split('(').nth(1)?.strip_suffix(')')?.parse().ok());
                    s.bt.conectados.push(Dispositivo { nome: f[2].to_string(), tipo, bateria });
                }
            }
        }
    }
    // som
    if let Some(v) = sh("wpctl", &["get-volume", "@DEFAULT_AUDIO_SINK@"]) {
        s.som.existe = true;
        s.som.nivel = v.split_whitespace().nth(1).and_then(|x| x.parse().ok()).unwrap_or(0.0);
        s.som.mudo = v.contains("MUTED");
    }
    // energia
    if let Some(p) = sh("powerprofilesctl", &["get"]) {
        s.perfil = p.trim().to_string();
        s.perfis = sh("powerprofilesctl", &["list"]).unwrap_or_default().lines()
            .filter_map(|l| l.trim().trim_start_matches('*').trim().strip_suffix(':').map(str::to_string)).collect();
    }
    // bateria do computador
    if let Some(u) = sh("upower", &["-i", "/org/freedesktop/UPower/devices/DisplayDevice"]) {
        let presente = u.lines().any(|l| l.trim() == "power supply:         yes" || l.contains("power supply:") && l.contains("yes"));
        let pct = u.lines().find_map(|l| l.trim().strip_prefix("percentage:").map(|x| x.trim().trim_end_matches('%').replace(',', ".")));
        let carregando = u.lines().any(|l| l.contains("state:") && (l.contains("charging") && !l.contains("discharging") || l.contains("fully-charged")));
        if presente && u.contains("is-present:          yes") {
            if let Some(p) = pct.and_then(|p| p.parse::<f64>().ok()) {
                s.bateria = Some((p.round() as u8, carregando));
            }
        }
    }
    s.luz_noturna = tem("wlsunset").then(night_light_on);
    s.luz_noturna_indisponivel = night_light_unavailable();
    s.agentes = {
        let pids = agent_pids();
        let paused = pids.iter().filter(|p| is_stopped(**p)).count();
        (pids.len(), paused)
    };
    // brilho
    if let Some(d) = std::fs::read_dir("/sys/class/backlight").ok().and_then(|mut r| r.next()).and_then(|e| e.ok()) {
        let ler = |f: &str| std::fs::read_to_string(d.path().join(f)).ok().and_then(|t| t.trim().parse::<f64>().ok());
        if let (Some(b), Some(m)) = (ler("brightness"), ler("max_brightness")) {
            if m > 0.0 && tem("brightnessctl") {
                s.brilho = Some(b / m);
            }
        }
    }
    s
}

// ------------------------------------------------------------------ ações (todas sem shell)

pub fn wifi(ligar: bool) {
    let _ = sh("nmcli", &["radio", "wifi", if ligar { "on" } else { "off" }]);
}

pub fn aviao(ligar: bool) {
    // modo avião: desliga toda a rede sem fio e o Bluetooth
    let _ = sh("nmcli", &["radio", "all", if ligar { "off" } else { "on" }]);
    let _ = sh("bluetoothctl", &["power", if ligar { "off" } else { "on" }]);
}

pub fn bluetooth(ligar: bool) {
    let _ = sh("bluetoothctl", &["power", if ligar { "on" } else { "off" }]);
}

pub fn volume(nivel: f64) {
    let _ = sh("wpctl", &["set-volume", "-l", "1.0", "@DEFAULT_AUDIO_SINK@", &format!("{:.2}", nivel.clamp(0.0, 1.0))]);
    crate::configuracoes_som::ao_mudar_volume();
}

pub fn mudo(m: bool) {
    let _ = sh("wpctl", &["set-mute", "@DEFAULT_AUDIO_SINK@", if m { "1" } else { "0" }]);
}

pub fn perfil(p: &str) {
    let _ = sh("powerprofilesctl", &["set", p]);
}

pub fn brilho(v: f64) {
    let _ = sh("brightnessctl", &["set", &format!("{}%", (v.clamp(0.05, 1.0) * 100.0).round())]);
}

// ------------------------------------------------------------------ luz noturna

static NIGHT_LIGHT: std::sync::Mutex<Option<std::process::Child>> = std::sync::Mutex::new(None);
/// The compositor has no gamma control on this screen (wlsunset quits at once): night light is unavailable.
static NO_GAMMA: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn night_light_unavailable() -> bool {
    NO_GAMMA.load(std::sync::atomic::Ordering::Relaxed)
}

fn night_light_on() -> bool {
    let mut g = NIGHT_LIGHT.lock().unwrap();
    match g.as_mut().map(|c| c.try_wait()) {
        Some(Ok(None)) => true,
        Some(_) => {
            *g = None;
            false
        }
        None => false,
    }
}

/// Night light: warm colors (the person's warmth, 3500 K unless chosen in Settings › Display) through the
/// compositor's gamma control (wlsunset). Off restores the colors (wlsunset resets the gamma when it ends).
pub fn luz_noturna(on: bool) {
    let temp = crate::configuracoes::ajustes()["luz_noturna_temperatura"].as_u64().map(|t| t.clamp(2500, 5500) as u32).unwrap_or(3500);
    luz_noturna_com(on, temp);
}

/// With this warmth (kelvin): again when it changes.
pub fn luz_noturna_com(on: bool, temp: u32) {
    night_light_with("wlsunset", on, temp);
}

pub fn luz_noturna_ligada() -> bool {
    night_light_on()
}

fn night_light_with(bin: &str, on: bool, temp: u32) {
    let mut g = NIGHT_LIGHT.lock().unwrap();
    if let Some(mut c) = g.take() {
        let _ = c.kill();
        let _ = c.wait();
    }
    if on {
        // low and high temperatures almost equal: warm all the time, until the shell turns it off
        let debug = std::env::var_os("KALANGO_SHELL_DEBUG").is_some();
        let err = if debug { std::process::Stdio::inherit() } else { std::process::Stdio::null() };
        let (t, t2) = (temp.to_string(), (temp + 1).to_string());
        *g = Command::new(bin).args(["-t", t.as_str(), "-T", t2.as_str()]).stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null()).stderr(err).spawn().ok();
        // without gamma control (a nested session, some virtual screens) wlsunset quits right away
        std::thread::sleep(std::time::Duration::from_millis(500));
        if let Some(Ok(Some(_))) = g.as_mut().map(|c| c.try_wait()) {
            *g = None;
            NO_GAMMA.store(true, std::sync::atomic::Ordering::Relaxed);
            if debug {
                eprintln!("debug controles: luz noturna indisponível: o compositor não oferece controle de gama nesta tela");
            }
        }
    }
}

/// Seconds until the next 06:00 (night light turns itself off then).
pub fn until_morning() -> u64 {
    let now = glib::DateTime::now_local();
    let Ok(now) = now else { return 8 * 3600 };
    let mut six = glib::DateTime::from_local(now.year(), now.month(), now.day_of_month(), 6, 0, 0.0).unwrap_or(now.clone());
    if six.to_unix() <= now.to_unix() {
        six = six.add_days(1).unwrap_or(six);
    }
    (six.to_unix() - now.to_unix()).max(60) as u64
}

// ------------------------------------------------------------------ pausar agentes

/// The person's agent engines (`kalango-agente --json`): the side bar conversation and the command bar's answers.
pub fn agent_pids() -> Vec<i32> {
    let me = std::fs::metadata("/proc/self").map(|m| { use std::os::unix::fs::MetadataExt; m.uid() }).unwrap_or(u32::MAX);
    let mut out = Vec::new();
    for e in std::fs::read_dir("/proc").into_iter().flatten().flatten() {
        let Ok(pid) = e.file_name().to_string_lossy().parse::<i32>() else { continue };
        let owner = { use std::os::unix::fs::MetadataExt; e.metadata().map(|m| m.uid()).unwrap_or(u32::MAX) };
        if owner != me {
            continue;
        }
        let Ok(cmd) = std::fs::read(format!("/proc/{pid}/cmdline")) else { continue };
        let args: Vec<&[u8]> = cmd.split(|b| *b == 0).collect();
        let exe_ok = args.first().map(|a| a.ends_with(b"kalango-agente")).unwrap_or(false);
        if exe_ok && args.iter().any(|a| *a == b"--json") {
            out.push(pid);
        }
    }
    out
}

pub fn is_stopped(pid: i32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()
        .and_then(|s| s.rsplit_once(") ").map(|(_, rest)| rest.starts_with('T') || rest.starts_with('t'))).unwrap_or(false)
}

/// Pause (freeze where it is) or resume one process.
pub fn pausar_processo(pid: i32, pause: bool) {
    extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    const SIGCONT: i32 = 18;
    const SIGSTOP: i32 = 19;
    if pid > 0 {
        unsafe {
            kill(pid, if pause { SIGSTOP } else { SIGCONT });
        }
    }
}

/// Pause (freeze where they are) or resume the person's agents.
pub fn pausar_agentes(pause: bool) {
    for pid in agent_pids() {
        pausar_processo(pid, pause);
    }
}

// ------------------------------------------------------------------ desempenho

/// Uso de CPU entre duas leituras de /proc/stat.
#[derive(Default, Clone, Copy)]
pub struct Cpu {
    total: u64,
    ocioso: u64,
}

impl Cpu {
    pub fn ler() -> Cpu {
        let t = std::fs::read_to_string("/proc/stat").unwrap_or_default();
        let v: Vec<u64> = t.lines().next().unwrap_or("").split_whitespace().skip(1).filter_map(|x| x.parse().ok()).collect();
        Cpu { total: v.iter().sum(), ocioso: v.get(3).copied().unwrap_or(0) + v.get(4).copied().unwrap_or(0) }
    }
    pub fn uso_desde(&self, antes: &Cpu) -> f64 {
        let dt = self.total.saturating_sub(antes.total) as f64;
        if dt == 0.0 { 0.0 } else { 1.0 - self.ocioso.saturating_sub(antes.ocioso) as f64 / dt }
    }
}

pub fn nucleos_ghz() -> (usize, f64) {
    let info = std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    let mhz: Vec<f64> = info.lines().filter_map(|l| l.strip_prefix("cpu MHz")?.split(':').nth(1)?.trim().parse().ok()).collect();
    let n = mhz.len().max(1);
    (n, mhz.iter().cloned().fold(0.0, f64::max) / 1000.0)
}

/// (uso %, temperatura °C, nome) da GPU NVIDIA.
pub fn gpu() -> Option<(u8, u8, String)> {
    let o = sh("nvidia-smi", &["--query-gpu=utilization.gpu,temperature.gpu,name", "--format=csv,noheader,nounits"])?;
    let f: Vec<&str> = o.lines().next()?.split(", ").collect();
    Some((f.first()?.trim().parse().ok()?, f.get(1)?.trim().parse().ok()?, f.get(2)?.trim().replace("NVIDIA GeForce ", "")))
}

#[cfg(test)]
mod tests {
    #[test]
    fn pausing_freezes_only_agent_engines_and_resumes_them() {
        // a stand-in engine: a process whose name and arguments look like `kalango-agente --json`
        use std::os::unix::process::CommandExt;
        // "; true" keeps the shell itself running (no exec of the last command)
        let mut child = std::process::Command::new("/bin/sh").arg0("/usr/libexec/kalango-agente")
            .args(["-c", "sleep 30; true", "x", "--json"]).spawn().unwrap();
        let mut other = std::process::Command::new("/bin/sleep").arg("30").spawn().unwrap();
        let pid = child.id() as i32;
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(super::agent_pids().contains(&pid));
        assert!(!super::agent_pids().contains(&(other.id() as i32)));
        super::pausar_agentes(true);
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(super::is_stopped(pid));
        assert!(!super::is_stopped(other.id() as i32));
        super::pausar_agentes(false);
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(!super::is_stopped(pid));
        let _ = child.kill();
        let _ = other.kill();
        let _ = child.wait();
        let _ = other.wait();
    }

    #[test]
    fn night_light_runs_and_stops_or_reports_no_gamma() {
        let dir = std::env::temp_dir().join(format!("kalango-noturna-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ok = dir.join("wlsunset-ok");
        let sem_gama = dir.join("wlsunset-sem-gama");
        // written by a child: a script this process held open for writing could be inherited by another test's fork
        // and fail to run ("text file busy") when the tests run in parallel
        crate::pam::tests::escrever_por_filho(&ok, "#!/bin/sh\nexec sleep 30\n");
        crate::pam::tests::escrever_por_filho(&sem_gama, "#!/bin/sh\necho 'gamma_control_manager missing' >&2\nexit 1\n");
        // a screen with gamma control: it keeps running until turned off
        super::night_light_with(ok.to_str().unwrap(), true, 3500);
        assert!(super::night_light_on() && !super::night_light_unavailable());
        super::night_light_with(ok.to_str().unwrap(), false, 3500);
        assert!(!super::night_light_on());
        // a screen without: it says so, and nothing is left running
        super::night_light_with(sem_gama.to_str().unwrap(), true, 3500);
        assert!(!super::night_light_on() && super::night_light_unavailable());
        super::NO_GAMMA.store(false, std::sync::atomic::Ordering::Relaxed);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn morning_is_within_a_day() {
        let s = super::until_morning();
        assert!((60..=24 * 3600).contains(&s));
    }
}
