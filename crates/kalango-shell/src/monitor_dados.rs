//! What the System Monitor reads, once a second, off the interface's thread: the processor, the memory, the GPU
//! (nvidia-smi), the network interfaces and the disks (/proc), and every process the session can see.

use std::collections::HashMap;
use std::path::Path;

/// The kernel's clock ticks per second (sysconf(_SC_CLK_TCK) on Linux).
pub(crate) const TICKS: f64 = 100.0;
/// /proc/[pid]/statm counts pages.
const PAGE: u64 = 4096;

#[derive(Clone, Default, Debug)]
pub(crate) struct Proc {
    pub pid: i32,
    pub ppid: i32,
    pub uid: u32,
    /// the program's name ("cargo"), and its command line ("cargo build --release")
    pub name: String,
    pub cmd: String,
    /// user + system clock ticks so far
    pub ticks: u64,
    /// when it started, in ticks since boot
    pub start: u64,
    pub threads: u32,
    pub rss: u64,
    /// bytes read and written so far (None: not ours to read)
    pub io: Option<(u64, u64)>,
    pub stopped: bool,
}

#[derive(Clone, Default, Debug)]
pub(crate) struct Gpu {
    pub util: f64,
    pub vram_used_mb: f64,
    pub vram_total_mb: f64,
    pub temp: f64,
}

#[derive(Clone, Default, Debug)]
pub(crate) struct Snap {
    /// seconds, monotonic
    pub at: f64,
    pub cpu_total: u64,
    pub cpu_idle: u64,
    pub cores: usize,
    pub ghz: f64,
    pub temp: Option<f64>,
    pub mem_total: u64,
    pub mem_avail: u64,
    pub mem_cached: u64,
    pub gpu: Option<Gpu>,
    /// the GPU memory each process holds (MB)
    pub gpu_mem: HashMap<i32, f64>,
    /// interfaces: name, bytes received, bytes sent
    pub net: Vec<(String, u64, u64)>,
    /// whole disks: name, sectors read, sectors written (512 bytes each)
    pub disks: Vec<(String, u64, u64)>,
    pub procs: Vec<Proc>,
    /// seconds since boot (process start times are counted from it)
    pub uptime: f64,
}

fn read(p: impl AsRef<Path>) -> String {
    std::fs::read_to_string(p).unwrap_or_default()
}

/// One /proc/[pid]/stat line: (name, state, ppid, utime+stime, threads, start).
pub(crate) fn parse_stat(s: &str) -> Option<(String, char, i32, u64, u32, u64)> {
    // (the name is in parentheses and may hold spaces or parentheses itself: up to the last ")")
    let open = s.find('(')?;
    let close = s.rfind(')')?;
    let name = s.get(open + 1..close)?.to_string();
    let rest: Vec<&str> = s.get(close + 2..)?.split_whitespace().collect();
    // rest[0] = state (field 3); utime is field 14, stime 15, num_threads 20, starttime 22
    let f = |n: usize| rest.get(n - 3).and_then(|v| v.parse::<u64>().ok());
    Some((name, rest.first()?.chars().next()?, f(4)? as i32, f(14)? + f(15)?, f(20)? as u32, f(22)?))
}

fn procs() -> Vec<Proc> {
    let mut out = Vec::new();
    let Ok(dir) = std::fs::read_dir("/proc") else { return out };
    for e in dir.flatten() {
        let Some(pid) = e.file_name().to_str().and_then(|n| n.parse::<i32>().ok()) else { continue };
        let base = e.path();
        let Some((name, state, ppid, ticks, threads, start)) = parse_stat(&read(base.join("stat"))) else { continue };
        let rss = read(base.join("statm")).split_whitespace().nth(1).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0) * PAGE;
        let uid = read(base.join("status")).lines().find_map(|l| l.strip_prefix("Uid:")?.split_whitespace().next()?.parse().ok()).unwrap_or(0);
        let io = std::fs::read_to_string(base.join("io")).ok().and_then(|t| {
            let get = |k: &str| t.lines().find_map(|l| l.strip_prefix(k)?.trim().parse::<u64>().ok());
            Some((get("read_bytes:")?, get("write_bytes:")?))
        });
        let cmd = read(base.join("cmdline")).split('\0').filter(|a| !a.is_empty()).collect::<Vec<_>>().join(" ");
        out.push(Proc { pid, ppid, uid, name, cmd, ticks, start, threads, rss, io, stopped: matches!(state, 'T' | 't') });
    }
    out
}

/// The processor's temperature: the k10temp / coretemp / zenpower sensor, else the first thermal zone.
fn cpu_temp() -> Option<f64> {
    for h in std::fs::read_dir("/sys/class/hwmon").ok()?.flatten() {
        let name = read(h.path().join("name"));
        if ["k10temp", "coretemp", "zenpower", "cpu_thermal"].contains(&name.trim()) {
            if let Ok(v) = read(h.path().join("temp1_input")).trim().parse::<f64>() {
                return Some(v / 1000.0);
            }
        }
    }
    read("/sys/class/thermal/thermal_zone0/temp").trim().parse::<f64>().ok().map(|v| v / 1000.0)
}

/// The NVIDIA GPU (nvidia-smi): use, video memory, temperature, and the memory each process holds.
fn nvidia() -> Option<(Gpu, HashMap<i32, f64>)> {
    let run = |args: &[&str]| std::process::Command::new("nvidia-smi").args(args).output().ok().filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
    let o = run(&["--query-gpu=utilization.gpu,memory.used,memory.total,temperature.gpu", "--format=csv,noheader,nounits"])?;
    let f: Vec<&str> = o.lines().next()?.split(',').map(str::trim).collect();
    let gpu = Gpu {
        util: f.first()?.parse().ok()?,
        vram_used_mb: f.get(1)?.parse().ok()?,
        vram_total_mb: f.get(2)?.parse().ok()?,
        temp: f.get(3).and_then(|t| t.parse().ok()).unwrap_or(0.0),
    };
    let mut per = HashMap::new();
    if let Some(o) = run(&["--query-compute-apps=pid,used_memory", "--format=csv,noheader,nounits"]) {
        for l in o.lines() {
            let mut p = l.split(',').map(str::trim);
            if let (Some(Ok(pid)), Some(Ok(mb))) = (p.next().map(str::parse::<i32>), p.next().map(str::parse::<f64>)) {
                *per.entry(pid).or_insert(0.0) += mb;
            }
        }
    }
    Some((gpu, per))
}

/// Everything at once (`gpu`: ask nvidia-smi too — it takes a few tens of milliseconds).
pub(crate) fn snap(gpu: bool) -> Snap {
    let stat = read("/proc/stat");
    let cpu: Vec<u64> = stat.lines().next().unwrap_or("").split_whitespace().skip(1).filter_map(|v| v.parse().ok()).collect();
    let (cores, ghz) = crate::sistema::nucleos_ghz();
    let mem = read("/proc/meminfo");
    let kb = |k: &str| mem.lines().find_map(|l| l.strip_prefix(k)?.split_whitespace().next()?.parse::<u64>().ok()).unwrap_or(0) * 1024;
    let net = read("/proc/net/dev").lines().skip(2).filter_map(|l| {
        let (name, rest) = l.split_once(':')?;
        let v: Vec<u64> = rest.split_whitespace().filter_map(|x| x.parse().ok()).collect();
        let name = name.trim().to_string();
        (name != "lo").then(|| (name, *v.first().unwrap_or(&0), *v.get(8).unwrap_or(&0)))
    }).collect();
    // (whole disks only — /sys/block lists them, partitions are inside)
    let disks = read("/proc/diskstats").lines().filter_map(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        let name = f.get(2)?.to_string();
        (Path::new("/sys/block").join(&name).exists() && !name.starts_with("loop") && !name.starts_with("ram") && !name.starts_with("zram"))
            .then(|| (name, f.get(5).and_then(|v| v.parse().ok()).unwrap_or(0), f.get(9).and_then(|v| v.parse().ok()).unwrap_or(0)))
    }).collect();
    let (gpu, gpu_mem) = if gpu { nvidia().map(|(g, m)| (Some(g), m)).unwrap_or_default() } else { (None, HashMap::new()) };
    let uptime = read("/proc/uptime").split_whitespace().next().and_then(|v| v.parse().ok()).unwrap_or(0.0);
    Snap {
        at: gtk::glib::monotonic_time() as f64 / 1e6,
        cpu_total: cpu.iter().take(8).sum(),
        cpu_idle: cpu.get(3).copied().unwrap_or(0) + cpu.get(4).copied().unwrap_or(0),
        cores,
        ghz,
        temp: cpu_temp(),
        mem_total: kb("MemTotal:"),
        mem_avail: kb("MemAvailable:"),
        mem_cached: kb("Cached:") + kb("Buffers:"),
        gpu,
        gpu_mem,
        net,
        disks,
        procs: procs(),
        uptime,
    }
}

/// The open sockets of a process: (connected, listening ports) — from its file descriptors and the kernel's tables.
pub(crate) fn sockets(pids: &[i32]) -> (usize, Vec<u16>, Vec<String>) {
    let mut inodes = std::collections::HashSet::new();
    for pid in pids {
        let Ok(fds) = std::fs::read_dir(format!("/proc/{pid}/fd")) else { continue };
        for fd in fds.flatten() {
            if let Ok(t) = std::fs::read_link(fd.path()) {
                if let Some(i) = t.to_str().and_then(|s| s.strip_prefix("socket:[")).and_then(|s| s.strip_suffix(']')).and_then(|s| s.parse::<u64>().ok()) {
                    inodes.insert(i);
                }
            }
        }
    }
    let (mut connected, mut listening, mut remotes) = (0, Vec::new(), Vec::new());
    for (table, v6) in [("/proc/net/tcp", false), ("/proc/net/tcp6", true), ("/proc/net/udp", false), ("/proc/net/udp6", true)] {
        for l in read(table).lines().skip(1) {
            let f: Vec<&str> = l.split_whitespace().collect();
            let (Some(local), Some(remote), Some(st), Some(inode)) = (f.get(1), f.get(2), f.get(3), f.get(9).and_then(|v| v.parse::<u64>().ok())) else { continue };
            if !inodes.contains(&inode) {
                continue;
            }
            let port = |a: &str| a.rsplit(':').next().and_then(|p| u16::from_str_radix(p, 16).ok()).unwrap_or(0);
            let tcp = table.contains("tcp");
            if tcp && *st == "0A" {
                listening.push(port(local));
            } else if !tcp || *st == "01" {
                if port(remote) != 0 {
                    connected += 1;
                    if let Some(ip) = remote.split(':').next().and_then(|h| hex_ip(h, v6)) {
                        if !remotes.contains(&ip) {
                            remotes.push(ip);
                        }
                    }
                }
            }
        }
    }
    listening.sort_unstable();
    listening.dedup();
    (connected, listening, remotes)
}

/// The kernel's hex address (little-endian words) as text.
fn hex_ip(h: &str, v6: bool) -> Option<String> {
    if !v6 {
        let n = u32::from_str_radix(h, 16).ok()?;
        return Some(std::net::Ipv4Addr::from(n.swap_bytes()).to_string());
    }
    let mut b = [0u8; 16];
    for i in 0..4 {
        let w = u32::from_str_radix(h.get(i * 8..i * 8 + 8)?, 16).ok()?.swap_bytes();
        b[i * 4..i * 4 + 4].copy_from_slice(&w.to_be_bytes());
    }
    let a = std::net::Ipv6Addr::from(b);
    Some(a.to_ipv4_mapped().map(|v| v.to_string()).unwrap_or_else(|| a.to_string()))
}

#[cfg(test)]
mod tests {
    use super::{hex_ip, parse_stat};

    #[test]
    fn stat_lines_and_addresses() {
        let (name, state, ppid, ticks, threads, start) =
            parse_stat("1234 (Web Content (x)) S 1000 1234 1234 0 -1 4194560 1 0 0 0 150 50 0 0 20 0 27 0 9876 0 0").unwrap();
        assert_eq!((name.as_str(), state, ppid, ticks, threads, start), ("Web Content (x)", 'S', 1000, 200, 27, 9876));
        assert_eq!(hex_ip("0100007F", false).as_deref(), Some("127.0.0.1"));
        assert_eq!(hex_ip("0000000000000000FFFF00000100007F", true).as_deref(), Some("127.0.0.1"));
    }
}
