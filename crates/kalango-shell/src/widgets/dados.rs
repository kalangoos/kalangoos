//! What the metric widgets show, read from the system every 2 s off the interface thread: CPU (/proc/stat, cpufreq),
//! GPU (nvidia-smi, or the amdgpu files), memory (/proc/meminfo), network (the active NetworkManager connection and
//! /proc/net/dev), the CPU temperature (hwmon), uptime and the disks (df). Five minutes of history are kept. The
//! sampler runs while some widget on screen listens.

use gtk::glib;
use gtk::prelude::*;
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::process::Command;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// 5 minutes at one sample every 2 s.
pub const HISTORICO: usize = 150;

#[derive(Clone, Debug, Default)]
pub struct Gpu {
    pub nome: String,
    pub uso: f64,
    pub temp: Option<f64>,
    /// used and total, in bytes
    pub vram: Option<(f64, f64)>,
}

#[derive(Clone, Debug, Default)]
pub struct Rede {
    pub nome: String,
    /// "wifi", "cabo" or "" (unknown)
    pub tipo: String,
    pub rx: f64,
    pub tx: f64,
}

#[derive(Clone, Debug, Default)]
pub struct Disco {
    pub nome: String,
    pub usado: f64,
    pub total: f64,
}

#[derive(Clone, Debug, Default)]
pub struct Amostra {
    /// 0…1
    pub cpu: f64,
    pub nucleos: usize,
    pub ghz: Option<f64>,
    pub gpu: Option<Gpu>,
    /// used and total, in bytes
    pub mem: (f64, f64),
    pub rede: Option<Rede>,
    pub temp: Option<f64>,
    pub ligado_s: u64,
    pub discos: Vec<Disco>,
    pub hist_cpu: VecDeque<f64>,
    pub hist_gpu: VecDeque<f64>,
    pub hist_mem: VecDeque<f64>,
    pub hist_rx: VecDeque<f64>,
}

/// What one reading needs from the previous one (counters) and keeps between readings (slow things).
#[derive(Clone, Default)]
struct Contadores {
    cpu: Option<crate::sistema::Cpu>,
    rede: Option<(String, u64, u64, Instant)>,
    conexao: Option<(String, String, String, Instant)>,
    discos: Option<(Vec<Disco>, Instant)>,
}

fn ler(arq: &str) -> Option<String> {
    std::fs::read_to_string(arq).ok()
}

fn nucleos() -> usize {
    ler("/proc/stat").map(|s| s.lines().filter(|l| l.starts_with("cpu") && l.as_bytes().get(3).is_some_and(u8::is_ascii_digit)).count()).unwrap_or(0)
}

fn ghz() -> Option<f64> {
    let mut v = Vec::new();
    for e in std::fs::read_dir("/sys/devices/system/cpu").ok()?.flatten() {
        let n = e.file_name();
        if n.to_string_lossy().starts_with("cpu") {
            if let Some(k) = ler(&format!("{}/cpufreq/scaling_cur_freq", e.path().display())).and_then(|s| s.trim().parse::<f64>().ok()) {
                v.push(k);
            }
        }
    }
    (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64 / 1e6)
}

/// "NVIDIA GeForce RTX 3090" → "RTX 3090".
fn nome_curto(n: &str) -> String {
    let n = n.trim();
    for marca in ["RTX", "GTX", "Quadro", "Radeon", "Arc", "Tesla"] {
        if let Some(i) = n.find(marca) {
            return n[i..].to_string();
        }
    }
    n.trim_start_matches("NVIDIA ").trim_start_matches("GeForce ").to_string()
}

fn gpu() -> Option<Gpu> {
    if let Ok(o) = Command::new("nvidia-smi").args(["--query-gpu=name,utilization.gpu,temperature.gpu,memory.used,memory.total", "--format=csv,noheader,nounits"]).output() {
        if o.status.success() {
            let s = String::from_utf8_lossy(&o.stdout);
            let c: Vec<&str> = s.lines().next()?.split(',').map(str::trim).collect();
            let num = |i: usize| c.get(i).and_then(|v| v.parse::<f64>().ok());
            return Some(Gpu {
                nome: nome_curto(c.first().copied().unwrap_or("")),
                uso: num(1).unwrap_or(0.0) / 100.0,
                temp: num(2),
                vram: num(3).zip(num(4)).map(|(u, t)| (u * 1048576.0, t * 1048576.0)),
            });
        }
    }
    // amdgpu
    for e in std::fs::read_dir("/sys/class/drm").ok()?.flatten() {
        let d = e.path().join("device");
        if let Some(u) = ler(&format!("{}/gpu_busy_percent", d.display())).and_then(|s| s.trim().parse::<f64>().ok()) {
            let n = |f: &str| ler(&format!("{}/{f}", d.display())).and_then(|s| s.trim().parse::<f64>().ok());
            let temp = std::fs::read_dir(d.join("hwmon")).ok().and_then(|mut it| it.next()).and_then(|h| h.ok())
                .and_then(|h| ler(&format!("{}/temp1_input", h.path().display()))).and_then(|s| s.trim().parse::<f64>().ok()).map(|t| t / 1000.0);
            return Some(Gpu { nome: "Radeon".into(), uso: u / 100.0, temp, vram: n("mem_info_vram_used").zip(n("mem_info_vram_total")) });
        }
    }
    None
}

/// The graphics card now (one reading, for a page that asks once).
pub fn gpu_now() -> Option<Gpu> {
    gpu()
}

/// The memory free now, in bytes.
pub fn free_memory() -> f64 {
    let (used, total) = memoria();
    total - used
}

fn memoria() -> (f64, f64) {
    let s = ler("/proc/meminfo").unwrap_or_default();
    let kb = |k: &str| s.lines().find(|l| l.starts_with(k)).and_then(|l| l.split_whitespace().nth(1)).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0) * 1024.0;
    let total = kb("MemTotal:");
    (total - kb("MemAvailable:"), total)
}

/// The active connection: (name, kind, device). NetworkManager first; else the device of the default route.
fn conexao() -> Option<(String, String, String)> {
    if let Ok(o) = Command::new("nmcli").args(["-t", "-f", "NAME,TYPE,DEVICE", "connection", "show", "--active"]).output() {
        for l in String::from_utf8_lossy(&o.stdout).lines() {
            // NAME may contain escaped colons ("\:")
            let campos: Vec<String> = l.replace("\\:", "\u{1}").split(':').map(|c| c.replace('\u{1}', ":")).collect();
            let (Some(n), Some(t), Some(d)) = (campos.first(), campos.get(1), campos.get(2)) else { continue };
            let tipo = match t.as_str() {
                "802-11-wireless" => "wifi",
                "802-3-ethernet" => "cabo",
                _ => continue,
            };
            return Some((n.clone(), tipo.into(), d.clone()));
        }
    }
    let rota = ler("/proc/net/route")?;
    let dev = rota.lines().skip(1).find(|l| l.split_whitespace().nth(1) == Some("00000000"))?.split_whitespace().next()?.to_string();
    let tipo = if std::path::Path::new(&format!("/sys/class/net/{dev}/wireless")).exists() { "wifi" } else { "cabo" };
    Some((dev.clone(), tipo.into(), dev))
}

fn bytes_rede(dev: &str) -> Option<(u64, u64)> {
    let s = ler("/proc/net/dev")?;
    let l = s.lines().find(|l| l.trim_start().starts_with(&format!("{dev}:")))?;
    let c: Vec<u64> = l.split(':').nth(1)?.split_whitespace().filter_map(|v| v.parse().ok()).collect();
    Some((*c.first()?, *c.get(8)?))
}

fn temp_cpu() -> Option<f64> {
    for e in std::fs::read_dir("/sys/class/hwmon").ok()?.flatten() {
        let n = ler(&format!("{}/name", e.path().display())).unwrap_or_default();
        if ["coretemp", "k10temp", "zenpower", "cpu_thermal"].contains(&n.trim()) {
            if let Some(t) = ler(&format!("{}/temp1_input", e.path().display())).and_then(|s| s.trim().parse::<f64>().ok()) {
                return Some(t / 1000.0);
            }
        }
    }
    None
}

/// The person's disks: real filesystems on real devices, one per device, "/" first.
fn discos() -> Vec<Disco> {
    let Ok(o) = Command::new("df").args(["-B1", "--output=source,fstype,size,used,target"]).output() else { return vec![] };
    let mut v: Vec<(String, Disco)> = Vec::new();
    for l in String::from_utf8_lossy(&o.stdout).lines().skip(1) {
        let c: Vec<&str> = l.split_whitespace().collect();
        let [src, fs, total, usado, alvo] = c[..] else { continue };
        if !src.starts_with("/dev/") || src.starts_with("/dev/loop") || alvo.starts_with("/boot") || alvo.starts_with("/snap") || alvo.contains("/snap/") {
            continue;
        }
        if !["ext4", "btrfs", "xfs", "f2fs", "ntfs", "ntfs3", "exfat", "vfat", "zfs", "bcachefs"].contains(&fs) {
            continue;
        }
        // the block device behind it (nvme0n1p2 → nvme0n1, sda1 → sda)
        let base = src.trim_start_matches("/dev/").trim_start_matches("mapper/").to_string();
        let disco = if base.starts_with("nvme") { base.split('p').next().unwrap_or(&base).to_string() } else { base.trim_end_matches(|c: char| c.is_ascii_digit()).to_string() };
        if v.iter().any(|(d, _)| *d == disco) {
            continue;
        }
        let tipo = if disco.starts_with("nvme") {
            "NVMe"
        } else if ler(&format!("/sys/block/{disco}/queue/rotational")).is_some_and(|r| r.trim() == "1") {
            "HDD"
        } else {
            "SSD"
        };
        let nome = if alvo == "/" {
            format!("{} · {tipo}", kalango_i18n::tr("System"))
        } else {
            let b = std::path::Path::new(alvo).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| alvo.to_string());
            format!("{b} · {tipo}")
        };
        let (Ok(t), Ok(u)) = (total.parse::<f64>(), usado.parse::<f64>()) else { continue };
        v.push((disco, Disco { nome, usado: u, total: t }));
    }
    v.sort_by_key(|(_, d)| !d.nome.starts_with(kalango_i18n::tr("System")));
    v.into_iter().map(|(_, d)| d).take(3).collect()
}

/// One reading (off the interface thread).
fn medir(mut c: Contadores) -> (Contadores, Amostra) {
    let mut a = Amostra::default();
    let cpu = crate::sistema::Cpu::ler();
    // (uso_desde is already a fraction)
    a.cpu = c.cpu.map(|antes| cpu.uso_desde(&antes)).unwrap_or(0.0);
    c.cpu = Some(cpu);
    a.nucleos = nucleos();
    a.ghz = ghz();
    a.gpu = gpu();
    a.mem = memoria();
    // the connection changes rarely: asked every 30 s
    if c.conexao.as_ref().is_none_or(|x| x.3.elapsed() > Duration::from_secs(30)) {
        c.conexao = conexao().map(|(n, t, d)| (n, t, d, Instant::now()));
    }
    if let Some((nome, tipo, dev, _)) = c.conexao.clone() {
        if let Some((rx, tx)) = bytes_rede(&dev) {
            let agora = Instant::now();
            let (vrx, vtx) = match &c.rede {
                Some((d, rx0, tx0, t0)) if *d == dev => {
                    let dt = agora.duration_since(*t0).as_secs_f64().max(0.1);
                    (rx.saturating_sub(*rx0) as f64 / dt, tx.saturating_sub(*tx0) as f64 / dt)
                }
                _ => (0.0, 0.0),
            };
            c.rede = Some((dev, rx, tx, agora));
            a.rede = Some(Rede { nome, tipo, rx: vrx, tx: vtx });
        }
    }
    a.temp = temp_cpu();
    a.ligado_s = ler("/proc/uptime").and_then(|s| s.split_whitespace().next().and_then(|v| v.parse::<f64>().ok())).unwrap_or(0.0) as u64;
    if c.discos.as_ref().is_none_or(|x| x.1.elapsed() > Duration::from_secs(30)) {
        c.discos = Some((discos(), Instant::now()));
    }
    a.discos = c.discos.as_ref().map(|d| d.0.clone()).unwrap_or_default();
    (c, a)
}

type Ouvinte = (glib::WeakRef<gtk::Widget>, Box<dyn Fn(&Amostra)>);

struct Amostrador {
    ouvintes: RefCell<Vec<Ouvinte>>,
    ultima: RefCell<Option<Amostra>>,
    contadores: RefCell<Contadores>,
    historico: RefCell<HashMap<&'static str, VecDeque<f64>>>,
    rodando: std::cell::Cell<bool>,
}

thread_local! {
    static AMOSTRADOR: Rc<Amostrador> = Rc::new(Amostrador {
        ouvintes: RefCell::default(), ultima: RefCell::default(), contadores: RefCell::default(), historico: RefCell::default(),
        rodando: std::cell::Cell::new(false),
    });
}

fn passo(a: Rc<Amostrador>) {
    let c = a.contadores.borrow().clone();
    let a2 = a.clone();
    crate::lateral::em_segundo_plano(move || medir(c), move |(c, mut amostra)| {
        *a2.contadores.borrow_mut() = c;
        {
            let mut h = a2.historico.borrow_mut();
            let rx = amostra.rede.as_ref().map(|r| r.rx).unwrap_or(0.0);
            let mem = if amostra.mem.1 > 0.0 { amostra.mem.0 / amostra.mem.1 } else { 0.0 };
            for (k, v) in [("cpu", amostra.cpu), ("gpu", amostra.gpu.as_ref().map(|g| g.uso).unwrap_or(0.0)), ("mem", mem), ("rx", rx)] {
                let q = h.entry(k).or_default();
                q.push_back(v);
                while q.len() > HISTORICO {
                    q.pop_front();
                }
            }
            amostra.hist_cpu = h["cpu"].clone();
            amostra.hist_gpu = h["gpu"].clone();
            amostra.hist_mem = h["mem"].clone();
            amostra.hist_rx = h["rx"].clone();
        }
        a2.ouvintes.borrow_mut().retain(|(w, _)| w.upgrade().is_some());
        for (w, f) in a2.ouvintes.borrow().iter() {
            if w.upgrade().is_some_and(|w| w.is_mapped()) {
                f(&amostra);
            }
        }
        *a2.ultima.borrow_mut() = Some(amostra);
    });
}

/// `f` gets every reading while `w` exists (and the last one at once, when there is one).
pub fn ouvir(w: &impl IsA<gtk::Widget>, f: impl Fn(&Amostra) + 'static) {
    AMOSTRADOR.with(|a| {
        if let Some(u) = a.ultima.borrow().as_ref() {
            f(u);
        }
        a.ouvintes.borrow_mut().push((w.as_ref().downgrade(), Box::new(f)));
        if !a.rodando.replace(true) {
            passo(a.clone());
            let a = a.clone();
            glib::timeout_add_local(Duration::from_secs(2), move || {
                a.ouvintes.borrow_mut().retain(|(w, _)| w.upgrade().is_some());
                if a.ouvintes.borrow().is_empty() {
                    a.rodando.set(false);
                    return glib::ControlFlow::Break;
                }
                // nothing on screen listens (the agents center closed, the desktop under windows still counts): no
                // reading this time
                if a.ouvintes.borrow().iter().any(|(w, _)| w.upgrade().is_some_and(|w| w.is_mapped())) {
                    passo(a.clone());
                }
                glib::ControlFlow::Continue
            });
        }
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn gpu_names_are_short() {
        assert_eq!(super::nome_curto("NVIDIA GeForce RTX 3090"), "RTX 3090");
        assert_eq!(super::nome_curto("AMD Radeon RX 7900 XTX"), "Radeon RX 7900 XTX");
    }

    /// A real reading of this machine: values in their ranges.
    #[test]
    fn reads_the_system() {
        let (c, _) = super::medir(Default::default());
        // some work between the two readings, so the CPU is not idle
        let t = std::time::Instant::now();
        let mut x = 0u64;
        while t.elapsed() < std::time::Duration::from_millis(300) {
            x = x.wrapping_mul(31).wrapping_add(7);
        }
        std::hint::black_box(x);
        let (_, a) = super::medir(c);
        assert!(a.cpu > 0.0 && a.cpu <= 1.0, "cpu {}", a.cpu);
        assert!(a.nucleos > 0);
        assert!(a.mem.1 > 0.0 && a.mem.0 <= a.mem.1);
        assert!(a.discos.iter().all(|d| d.total > 0.0 && d.usado <= d.total));
    }
}
