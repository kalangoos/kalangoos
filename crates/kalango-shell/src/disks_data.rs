//! What the Disks app knows of the computer's drives: read from UDisks2 on the system bus (drives, their
//! partitions, file systems, encryption, SMART health), with the space used of what is mounted and the read and
//! write rates from /proc/diskstats. The actions go to UDisks2 too, which asks polkit (the person authorizes).
//! In the session test (`KALANGO_DISCOS_TESTE`, a JSON list of drives) the drives come from that file and the
//! actions are only written down (`KALANGO_DISCOS_LOG`): no real disk is ever touched by a test.

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Part {
    /// UDisks2's object (the block device of the partition).
    pub object: String,
    pub device: String,
    pub number: u32,
    pub offset: u64,
    pub size: u64,
    /// The file system ("ext4", "vfat", "ntfs"…), or "" (none, or locked).
    pub fs: String,
    pub label: String,
    /// The partition's own name (GPT), or its kind ("EFI").
    pub name: String,
    /// "LUKS2", "LUKS1"… when encrypted.
    pub encrypted: String,
    pub efi: bool,
    pub mount: Option<String>,
    pub used: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Health {
    pub smart: bool,
    pub failing: bool,
    pub warnings: Vec<String>,
    /// % of its rated life used (NVMe), when known.
    pub wear: Option<u8>,
    pub temp_c: Option<f64>,
    pub hours: Option<u64>,
    pub bad_sectors: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Drive {
    pub object: String,
    /// The kernel's name ("nvme0n1", "sda"), for its rates.
    pub device: String,
    pub vendor: String,
    pub model: String,
    pub size: u64,
    pub serial: String,
    /// "nvme", "sata", "usb", "sdio"…
    pub bus: String,
    pub rotational: bool,
    pub removable: bool,
    pub health: Health,
    pub parts: Vec<Part>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Healthy,
    Attention,
    Removable,
}

/// "1 TB", "500 GB", "64 GB": a drive's size as sold (decimal, rounded).
pub fn size_label(b: u64) -> String {
    let gb = b as f64 / 1e9;
    if gb >= 1000.0 {
        let tb = gb / 1000.0;
        if (tb - tb.round()).abs() < 0.15 { format!("{} TB", tb.round()) } else { format!("{:.1} TB", tb).replace('.', ",") }
    } else if gb >= 1.0 {
        format!("{} GB", gb.round())
    } else {
        format!("{} MB", (b as f64 / 1e6).round())
    }
}

/// "187 GB": a size in use (decimal).
pub fn amount(b: u64) -> String {
    let gb = b as f64 / 1e9;
    if gb >= 1000.0 { format!("{:.1} TB", gb / 1000.0).replace('.', ",") } else if gb >= 10.0 { format!("{} GB", gb.round()) } else if gb >= 1.0 { format!("{:.1} GB", gb).replace('.', ",") } else { format!("{} MB", (b as f64 / 1e6).round()) }
}

/// "14.200": thousands with a dot, as written in Portuguese.
pub fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push('.');
        }
        out.push(c);
    }
    out
}

impl Drive {
    /// "NVMe 1 TB", "SSD 2 TB", "HDD 500 GB", or for a removable one its brand and size ("Kingston 64 GB").
    pub fn title(&self) -> String {
        if self.removable {
            let brand = self.vendor.split_whitespace().next().filter(|v| !v.is_empty())
                .or_else(|| self.model.split(['_', ' ']).next()).unwrap_or("USB");
            let mut b = brand.to_lowercase();
            if let Some(f) = b.get_mut(0..1) {
                f.make_ascii_uppercase();
            }
            return format!("{b} {}", size_label(self.size));
        }
        let kind = if self.bus == "nvme" { "NVMe" } else if self.rotational { "HDD" } else { "SSD" };
        format!("{kind} {}", size_label(self.size))
    }

    /// "Samsung 990 · PCIe 4.0 · número de série oculto" (the serial is never shown).
    pub fn model_line(&self, hidden_serial: &str) -> String {
        let model = [self.vendor.trim(), self.model.trim()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" ").replace('_', " ");
        let bus = match self.bus.as_str() { "nvme" => "NVMe", "usb" => "USB", "sata" | "ata" => "SATA", "sdio" => "SD", b => b };
        [model.as_str(), bus, if self.serial.is_empty() { "" } else { hidden_serial }].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" · ")
    }

    pub fn is_system(&self) -> bool {
        self.parts.iter().any(|p| matches!(p.mount.as_deref(), Some("/") | Some("/boot") | Some("/boot/efi")))
    }

    pub fn encrypted(&self) -> bool {
        self.parts.iter().any(|p| !p.encrypted.is_empty())
    }

    /// The share of its mounted space in use (0 – 1), when anything is mounted.
    pub fn fullness(&self) -> Option<f64> {
        let (used, total) = self.parts.iter().filter(|p| p.mount.is_some()).filter_map(|p| p.used.map(|u| (u, p.size))).fold((0u64, 0u64), |a, b| (a.0 + b.0, a.1 + b.1));
        (total > 0).then(|| used as f64 / total as f64)
    }

    /// Where it keeps backups (a partition labelled or mounted as such), if anywhere.
    pub fn backup_mount(&self) -> Option<String> {
        self.parts.iter().filter_map(|p| p.mount.clone().map(|m| (p, m)))
            .find(|(p, m)| [p.label.as_str(), p.name.as_str(), m.as_str()].iter().any(|s| s.to_lowercase().contains("backup")))
            .map(|(_, m)| m)
    }

    pub fn status(&self) -> Status {
        if self.removable {
            return Status::Removable;
        }
        let h = &self.health;
        let worn = h.wear.is_some_and(|w| w >= 80) || h.bad_sectors.is_some_and(|b| b > 0)
            || (self.rotational && h.hours.is_some_and(|x| x >= 12_000));
        if h.failing || !h.warnings.is_empty() || worn { Status::Attention } else { Status::Healthy }
    }

    pub fn total_used(&self) -> u64 {
        self.parts.iter().filter_map(|p| p.used).sum()
    }

    pub fn free_space(&self) -> u64 {
        self.parts.iter().filter_map(|p| p.used.map(|u| p.size.saturating_sub(u))).max().unwrap_or(0)
    }
}

impl Part {
    /// "Ubuntu", "KalangoOS", "EFI", "Dados", or "Partição 3".
    pub fn title(&self, nth: &str) -> String {
        if self.efi {
            return "EFI".into();
        }
        [self.label.as_str(), self.name.as_str()].iter().find(|s| !s.trim().is_empty()).map(|s| s.to_string()).unwrap_or_else(|| nth.to_string())
    }
}

// ----------------------------------------------------------------------------------------------- reading

/// The test's drives (when that file is there).
fn test_file() -> Option<PathBuf> {
    std::env::var_os("KALANGO_DISCOS_TESTE").map(PathBuf::from).filter(|f| f.exists())
}

/// The drives, from UDisks2 (or the test's file). Runs in a background thread.
pub fn read() -> Result<Vec<Drive>, String> {
    if let Some(f) = test_file() {
        let s = std::fs::read_to_string(&f).map_err(|e| e.to_string())?;
        return serde_json::from_str(&s).map_err(|e| e.to_string());
    }
    let bus = gio::bus_get_sync(gio::BusType::System, gio::Cancellable::NONE).map_err(|e| e.to_string())?;
    let r = bus.call_sync(Some("org.freedesktop.UDisks2"), "/org/freedesktop/UDisks2", "org.freedesktop.DBus.ObjectManager", "GetManagedObjects",
        None, Some(glib::VariantTy::new("(a{oa{sa{sv}}})").unwrap()), gio::DBusCallFlags::NONE, 5000, gio::Cancellable::NONE).map_err(|e| e.to_string())?;
    let all: HashMap<String, HashMap<String, HashMap<String, glib::Variant>>> = r.child_value(0).iter().filter_map(|e| {
        let path = e.child_value(0).str()?.to_string();
        let ifaces = e.child_value(1).iter().filter_map(|i| {
            let name = i.child_value(0).str()?.to_string();
            let props = i.child_value(1).iter().filter_map(|p| Some((p.child_value(0).str()?.to_string(), p.child_value(1).as_variant()?))).collect();
            Some((name, props))
        }).collect();
        Some((path, ifaces))
    }).collect();
    Ok(parse(&bus, &all))
}

fn bytes_str(v: Option<&glib::Variant>) -> String {
    v.and_then(|v| v.get::<Vec<u8>>()).map(|b| String::from_utf8_lossy(&b).trim_end_matches('\0').to_string()).unwrap_or_default()
}

fn parse(bus: &gio::DBusConnection, all: &HashMap<String, HashMap<String, HashMap<String, glib::Variant>>>) -> Vec<Drive> {
    let s = |m: &HashMap<String, glib::Variant>, k: &str| m.get(k).and_then(|v| v.str().map(str::to_string)).unwrap_or_default();
    let u = |m: &HashMap<String, glib::Variant>, k: &str| m.get(k).and_then(|v| v.get::<u64>()).unwrap_or(0);
    let b = |m: &HashMap<String, glib::Variant>, k: &str| m.get(k).and_then(|v| v.get::<bool>()).unwrap_or(false);
    let o = |m: &HashMap<String, glib::Variant>, k: &str| m.get(k).and_then(|v| v.get::<glib::variant::ObjectPath>()).map(|p| p.to_string()).unwrap_or_default();
    let mut drives = Vec::new();
    for (path, ifaces) in all {
        let Some(d) = ifaces.get("org.freedesktop.UDisks2.Drive") else { continue };
        // (the drive's whole block device: its kernel name)
        let whole = all.iter().find(|(_, i)| i.get("org.freedesktop.UDisks2.Block").is_some_and(|bl| o(bl, "Drive") == *path) && !i.contains_key("org.freedesktop.UDisks2.Partition"));
        let device = whole.map(|(_, i)| bytes_str(i["org.freedesktop.UDisks2.Block"].get("PreferredDevice"))).unwrap_or_default()
            .trim_start_matches("/dev/").to_string();
        let mut health = Health::default();
        if let Some(a) = ifaces.get("org.freedesktop.UDisks2.Drive.Ata") {
            health.smart = b(a, "SmartSupported") && b(a, "SmartEnabled");
            health.failing = b(a, "SmartFailing");
            let t = a.get("SmartTemperature").and_then(|v| v.get::<f64>()).unwrap_or(0.0);
            health.temp_c = (t > 0.0).then_some(t - 273.15);
            let secs = u(a, "SmartPowerOnSeconds");
            health.hours = (secs > 0).then_some(secs / 3600);
            health.bad_sectors = a.get("SmartNumBadSectors").and_then(|v| v.get::<i64>()).filter(|n| *n >= 0);
        }
        if let Some(n) = ifaces.get("org.freedesktop.UDisks2.NVMe.Controller") {
            health.smart = true;
            health.warnings = n.get("SmartCriticalWarning").and_then(|v| v.get::<Vec<String>>()).unwrap_or_default();
            let t = n.get("SmartTemperature").and_then(|v| v.get::<u16>()).unwrap_or(0);
            health.temp_c = (t > 0).then_some(f64::from(t) - 273.15);
            let h = u(n, "SmartPowerOnHours");
            health.hours = (h > 0).then_some(h);
            // (the share of its life used: asked, it is not a property)
            if let Ok(r) = bus.call_sync(Some("org.freedesktop.UDisks2"), path, "org.freedesktop.UDisks2.NVMe.Controller", "SmartGetAttributes",
                Some(&(glib::VariantDict::new(None).end(),).to_variant()), None, gio::DBusCallFlags::NONE, 3000, gio::Cancellable::NONE) {
                let attrs = glib::VariantDict::new(Some(&r.child_value(0)));
                health.wear = attrs.lookup_value("percent_used", None).and_then(|v| v.get::<u8>().or_else(|| v.get::<i32>().map(|x| x as u8)));
            }
        }
        let mut parts = Vec::new();
        for (bpath, bi) in all {
            let (Some(bl), Some(p)) = (bi.get("org.freedesktop.UDisks2.Block"), bi.get("org.freedesktop.UDisks2.Partition")) else { continue };
            if o(bl, "Drive") != *path {
                continue;
            }
            let ptype = s(p, "Type").to_lowercase();
            let mut part = Part {
                object: bpath.clone(),
                device: bytes_str(bl.get("PreferredDevice")),
                number: p.get("Number").and_then(|v| v.get::<u32>()).unwrap_or(0),
                offset: u(p, "Offset"),
                size: u(p, "Size"),
                fs: s(bl, "IdType"),
                label: s(bl, "IdLabel"),
                name: s(p, "Name"),
                efi: ptype == "c12a7328-f81f-11d2-ba4b-00a0c93ec93b" || ptype == "0xef",
                ..Default::default()
            };
            // an encrypted one: its kind, and what is inside once unlocked
            let mut fs_obj = bi.clone();
            if bi.contains_key("org.freedesktop.UDisks2.Encrypted") {
                part.encrypted = match s(bl, "IdVersion").as_str() { "2" => "LUKS2".into(), "1" => "LUKS1".into(), _ => "LUKS".into() };
                let clear = o(&bi["org.freedesktop.UDisks2.Encrypted"], "CleartextDevice");
                if let Some(ci) = all.get(&clear) {
                    if let Some(cb) = ci.get("org.freedesktop.UDisks2.Block") {
                        part.fs = s(cb, "IdType");
                        if part.label.is_empty() {
                            part.label = s(cb, "IdLabel");
                        }
                    }
                    fs_obj = ci.clone();
                } else {
                    part.fs.clear();
                }
            }
            if let Some(f) = fs_obj.get("org.freedesktop.UDisks2.Filesystem") {
                let mounts: Vec<Vec<u8>> = f.get("MountPoints").and_then(|v| v.get()).unwrap_or_default();
                part.mount = mounts.first().map(|m| String::from_utf8_lossy(m).trim_end_matches('\0').to_string());
                if let Some(m) = &part.mount {
                    part.used = crate::configuracoes_sistema::statvfs(Path::new(m)).map(|(t, free)| t.saturating_sub(free));
                }
            }
            parts.push(part);
        }
        parts.sort_by_key(|p| p.offset);
        drives.push(Drive {
            object: path.clone(), device, vendor: s(d, "Vendor"), model: s(d, "Model"), size: u(d, "Size"), serial: s(d, "Serial"),
            bus: { let c = s(d, "ConnectionBus"); if c.is_empty() && path.contains("nvme") || ifaces.contains_key("org.freedesktop.UDisks2.NVMe.Controller") { "nvme".into() } else if c.is_empty() { "sata".into() } else { c } },
            rotational: d.get("RotationRate").and_then(|v| v.get::<i32>()).is_some_and(|r| r != 0),
            removable: b(d, "Removable") || b(d, "MediaRemovable"),
            health, parts,
        });
    }
    // the system's first, then by size
    drives.sort_by_key(|d| (!d.is_system(), d.removable, std::cmp::Reverse(d.size)));
    drives
}

/// Sectors read and written so far, per kernel device name.
pub fn diskstats() -> HashMap<String, (u64, u64)> {
    if let Some(f) = std::env::var_os("KALANGO_DISCOS_TESTE_STATS") {
        // (the test's: a line per device, the counts growing a fixed step each reading)
        return std::fs::read_to_string(f).unwrap_or_default().lines().filter_map(|l| {
            let mut it = l.split_whitespace();
            Some((it.next()?.to_string(), (it.next()?.parse().ok()?, it.next()?.parse().ok()?)))
        }).collect();
    }
    std::fs::read_to_string("/proc/diskstats").unwrap_or_default().lines().filter_map(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        Some((f.get(2)?.to_string(), (f.get(5)?.parse().ok()?, f.get(9)?.parse().ok()?)))
    }).collect()
}

// ----------------------------------------------------------------------------------------------- acting

/// What the person (or the agent, once confirmed) asks of a drive.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum Action {
    /// Checks a partition's file system (unmounted), or else runs the drive's short SMART self-test.
    Check { drive: String, part: Option<String> },
    /// A copy of a partition (or the whole drive) into a file.
    Image { block: String, file: PathBuf },
    Resize { part: String, size: u64 },
    Format { part: String, fs: String, label: String },
    Mount { part: String },
    Unmount { part: String },
    Eject { drive: String },
}

fn log_test(a: &Action) -> Option<Result<String, String>> {
    let f = std::env::var_os("KALANGO_DISCOS_LOG")?;
    use std::io::Write;
    let line = serde_json::to_string(a).unwrap_or_default();
    let r = std::fs::OpenOptions::new().create(true).append(true).open(&f).and_then(|mut h| writeln!(h, "{line}"));
    Some(r.map(|_| "ok (teste)".to_string()).map_err(|e| e.to_string()))
}

fn opts() -> glib::Variant {
    // (polkit may ask the person: the call waits for them)
    let d = glib::VariantDict::new(None);
    d.insert_value("auth.no_user_interaction", &false.to_variant());
    d.end()
}

fn call(object: &str, iface: &str, method: &str, args: glib::Variant, timeout_ms: i32) -> Result<glib::Variant, String> {
    let bus = gio::bus_get_sync(gio::BusType::System, gio::Cancellable::NONE).map_err(|e| e.to_string())?;
    bus.call_sync(Some("org.freedesktop.UDisks2"), object, iface, method, Some(&args), None, gio::DBusCallFlags::ALLOW_INTERACTIVE_AUTHORIZATION,
        timeout_ms, gio::Cancellable::NONE).map_err(|e| e.message().to_string())
}

/// Does it (a background thread): UDisks2 asks polkit, the person authorizes. A test only writes it down.
pub fn act(a: &Action, progress: &dyn Fn(f64)) -> Result<String, String> {
    if let Some(r) = log_test(a) {
        progress(1.0);
        return r;
    }
    match a {
        Action::Check { drive, part } => {
            if let Some(p) = part {
                if call(p, "org.freedesktop.UDisks2.Filesystem", "Check", glib::Variant::tuple_from_iter([opts()]), 600_000).is_ok() {
                    return Ok("checked".into());
                }
            }
            // (mounted, or no file system to check: the drive's own short self-test)
            call(drive, "org.freedesktop.UDisks2.NVMe.Controller", "SmartSelftestStart", ("short", opts()).to_variant(), 30_000)
                .or_else(|_| call(drive, "org.freedesktop.UDisks2.Drive.Ata", "SmartSelftestStart", ("short", opts()).to_variant(), 30_000))
                .map(|_| "selftest".into())
        }
        Action::Image { block, file } => image(block, file, progress),
        Action::Resize { part, size } => {
            call(part, "org.freedesktop.UDisks2.Filesystem", "Resize", (*size, opts()).to_variant(), 3_600_000)
                .or_else(|_| call(part, "org.freedesktop.UDisks2.Partition", "Resize", (*size, opts()).to_variant(), 600_000)).map(|_| "resized".into())
        }
        Action::Format { part, fs, label } => {
            let o = glib::VariantDict::new(Some(&opts()));
            o.insert_value("label", &label.to_variant());
            o.insert_value("update-partition-type", &true.to_variant());
            call(part, "org.freedesktop.UDisks2.Block", "Format", (fs.as_str(), o.end()).to_variant(), 3_600_000).map(|_| "formatted".into())
        }
        Action::Mount { part } => call(part, "org.freedesktop.UDisks2.Filesystem", "Mount", glib::Variant::tuple_from_iter([opts()]), 60_000)
            .map(|v| v.child_value(0).str().unwrap_or("").to_string()),
        Action::Unmount { part } => call(part, "org.freedesktop.UDisks2.Filesystem", "Unmount", glib::Variant::tuple_from_iter([opts()]), 60_000).map(|_| "unmounted".into()),
        Action::Eject { drive } => call(drive, "org.freedesktop.UDisks2.Drive", "PowerOff", glib::Variant::tuple_from_iter([opts()]), 60_000)
            .or_else(|_| call(drive, "org.freedesktop.UDisks2.Drive", "Eject", glib::Variant::tuple_from_iter([opts()]), 60_000)).map(|_| "ejected".into()),
    }
}

/// A disk image: UDisks2 opens the device for reading (polkit), the bytes go to the file.
fn image(block: &str, file: &Path, progress: &dyn Fn(f64)) -> Result<String, String> {
    use std::io::{Read, Write};
    use std::os::fd::FromRawFd;
    let bus = gio::bus_get_sync(gio::BusType::System, gio::Cancellable::NONE).map_err(|e| e.to_string())?;
    let size = bus.call_sync(Some("org.freedesktop.UDisks2"), block, "org.freedesktop.DBus.Properties", "Get",
        Some(&("org.freedesktop.UDisks2.Block", "Size").to_variant()), None, gio::DBusCallFlags::NONE, 5000, gio::Cancellable::NONE)
        .ok().and_then(|v| v.child_value(0).as_variant()).and_then(|v| v.get::<u64>()).unwrap_or(0);
    let (_, fds) = bus.call_with_unix_fd_list_sync(Some("org.freedesktop.UDisks2"), block, "org.freedesktop.UDisks2.Block", "OpenForBackup",
        Some(&glib::Variant::tuple_from_iter([opts()])), None, gio::DBusCallFlags::ALLOW_INTERACTIVE_AUTHORIZATION, 120_000,
        None::<&gio::UnixFDList>, gio::Cancellable::NONE).map_err(|e| e.message().to_string())?;
    let fd = fds.ok_or("no fd")?.get(0).map_err(|e| e.to_string())?;
    let mut src = unsafe { std::fs::File::from_raw_fd(fd) };
    let mut dst = std::fs::File::create(file).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; 4 << 20];
    let mut done = 0u64;
    loop {
        let n = src.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        dst.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        done += n as u64;
        if size > 0 {
            progress(done as f64 / size as f64);
        }
    }
    Ok(file.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drive(bus: &str, rot: bool, size: u64) -> Drive {
        Drive { bus: bus.into(), rotational: rot, size, ..Default::default() }
    }

    #[test]
    fn titles_and_sizes() {
        assert_eq!(drive("nvme", false, 1_000_204_886_016).title(), "NVMe 1 TB");
        assert_eq!(drive("sata", false, 2_000_398_934_016).title(), "SSD 2 TB");
        assert_eq!(drive("sata", true, 500_107_862_016).title(), "HDD 500 GB");
        let mut k = drive("usb", false, 61_872_793_600);
        k.removable = true;
        k.vendor = "KINGSTON".into();
        assert_eq!(k.title(), "Kingston 62 GB");
        assert_eq!(thousands(14_200), "14.200");
        assert_eq!(amount(187_000_000_000), "187 GB");
        assert_eq!(size_label(599_000_000_000), "599 GB");
    }

    #[test]
    fn health_and_backups() {
        let mut d = drive("sata", true, 500_000_000_000);
        d.health.hours = Some(14_200);
        assert_eq!(d.status(), Status::Attention);
        d.health.hours = Some(1_000);
        assert_eq!(d.status(), Status::Healthy);
        d.parts.push(Part { label: "Backup".into(), mount: Some("/media/x/Backup".into()), size: 100, used: Some(40), ..Default::default() });
        assert_eq!(d.backup_mount().as_deref(), Some("/media/x/Backup"));
        assert_eq!(d.fullness(), Some(0.4));
    }
}
