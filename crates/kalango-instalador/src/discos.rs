//! The disks the installer can use (lsblk), what is on them (an installed system, by its partitions' labels and file
//! systems), and the plan the engine follows: which disk, how (beside the system there, or erasing the disk), how much
//! for KalangoOS, encrypted or not, and the account.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Default)]
pub struct Partition {
    pub path: String,
    pub size: u64,
    /// its first sector (512 bytes): where it is made again when it gives up space
    pub start: u64,
    pub fstype: String,
    pub label: String,
    pub mounted: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Disk {
    pub path: String,
    pub size: u64,
    pub model: String,
    /// "NVMe", "SSD", "HDD", "USB"
    pub kind: String,
    pub removable: bool,
    pub partitions: Vec<Partition>,
    /// the system on it ("Ubuntu 24.04", "Windows"), when there is one
    pub system: Option<String>,
    /// bytes the system there uses now (for "beside it")
    pub system_used: u64,
    /// the disk the live session runs from (never offered)
    pub live: bool,
}

impl Disk {
    pub fn title(&self) -> String {
        format!("{} {}", self.kind, human(self.size))
    }
}

pub fn human(bytes: u64) -> String {
    let b = bytes as f64;
    let (v, u) = if b >= 1e12 { (b / 1e12, "TB") } else if b >= 1e9 { (b / 1e9, "GB") } else { (b / 1e6, "MB") };
    let s = if v >= 100.0 { format!("{v:.0}") } else { format!("{v:.1}").replace('.', ",") };
    format!("{} {u}", s.trim_end_matches(",0"))
}

fn as_u64(v: &Value) -> u64 {
    v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok())).unwrap_or(0)
}

/// The system a set of partitions holds, by what lsblk says of them.
fn system_of(parts: &[Partition]) -> Option<String> {
    for p in parts {
        let l = p.label.to_lowercase();
        if p.fstype == "ntfs" && (l.is_empty() || l.contains("windows") || l.contains("basic data")) {
            return Some("Windows".into());
        }
    }
    for p in parts {
        let l = p.label.to_lowercase();
        if p.fstype == "ext4" || p.fstype == "btrfs" || p.fstype == "xfs" {
            for name in ["ubuntu", "fedora", "debian", "mint", "arch", "pop", "kalango"] {
                if l.contains(name) {
                    let mut c = name.chars();
                    return Some(c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default());
                }
            }
            // the running system's own root: its name from os-release
            if p.mounted.iter().any(|m| m == "/") {
                let os = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
                let n = os.lines().find_map(|l| l.strip_prefix("PRETTY_NAME=")).map(|v| v.trim_matches('"').to_string());
                return n.map(|n| n.replace(" LTS", "").split(" (").next().unwrap_or("").to_string());
            }
            // a Linux without a label (Ubuntu's root has none) boots from this disk's EFI partition; a disk without one
            // holds data
            if parts.iter().any(|x| x.fstype == "vfat") {
                return Some("Linux".into());
            }
        }
    }
    None
}

/// The disks from lsblk's JSON (KALANGO_INSTALADOR_LSBLK: a file instead of the command, for tests).
pub fn read() -> Vec<Disk> {
    let raw = match std::env::var("KALANGO_INSTALADOR_LSBLK") {
        Ok(f) => std::fs::read_to_string(f).unwrap_or_default(),
        Err(_) => std::process::Command::new("lsblk").args(["-J", "-b", "-o", "NAME,PATH,SIZE,START,TYPE,FSTYPE,LABEL,PARTLABEL,MODEL,TRAN,ROTA,RM,MOUNTPOINTS,FSUSED"])
            .env("LC_ALL", "C").output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default(),
    };
    parse(&raw)
}

pub fn parse(raw: &str) -> Vec<Disk> {
    let Ok(v) = serde_json::from_str::<Value>(raw) else { return vec![] };
    let mut out = Vec::new();
    for d in v["blockdevices"].as_array().cloned().unwrap_or_default() {
        if d["type"] != "disk" || d["name"].as_str().unwrap_or("").starts_with("zram") || d["name"].as_str().unwrap_or("").starts_with("loop") {
            continue;
        }
        let size = as_u64(&d["size"]);
        if size < 8 << 30 {
            continue;
        }
        let flag = |k: &str| d[k] == true || d[k] == "1";
        let name = d["name"].as_str().unwrap_or("");
        let kind = if d["tran"] == "nvme" || name.starts_with("nvme") { "NVMe" } else if d["tran"] == "usb" || flag("rm") { "USB" } else if flag("rota") { "HDD" } else { "SSD" };
        let mut partitions = Vec::new();
        let mut used = 0;
        let mut collect = |p: &Value, partitions: &mut Vec<Partition>| {
            let mounted: Vec<String> = p["mountpoints"].as_array().into_iter().flatten().filter_map(|m| m.as_str().map(str::to_string)).collect();
            used += as_u64(&p["fsused"]);
            partitions.push(Partition { path: p["path"].as_str().unwrap_or("").into(), size: as_u64(&p["size"]), start: as_u64(&p["start"]), fstype: p["fstype"].as_str().unwrap_or("").into(),
                label: p["label"].as_str().or(p["partlabel"].as_str()).unwrap_or("").into(), mounted });
        };
        for p in d["children"].as_array().cloned().unwrap_or_default() {
            collect(&p, &mut partitions);
            // an encrypted or LVM partition: what is inside counts as its content
            for c in p["children"].as_array().cloned().unwrap_or_default() {
                collect(&c, &mut partitions);
            }
        }
        // the live session's own medium (its root is a squashfs or an overlay on it): never offered
        let live = partitions.iter().any(|p| p.mounted.iter().any(|m| m.starts_with("/run/live") || m.starts_with("/cdrom") || m == "/run/initramfs/live"))
            || partitions.iter().any(|p| p.fstype == "iso9660");
        let system = system_of(&partitions);
        out.push(Disk { path: d["path"].as_str().unwrap_or("").into(), size, model: d["model"].as_str().unwrap_or("").trim().into(), kind: kind.into(),
            removable: flag("rm"), partitions, system, system_used: used, live });
    }
    out
}

/// How KalangoOS goes on the disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Mode {
    /// beside the system there: its last partition shrinks, KalangoOS takes the space freed
    #[default]
    Beside,
    /// the whole disk for KalangoOS
    Erase,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Plan {
    pub disk: String,
    pub disk_title: String,
    pub mode: Mode,
    /// bytes for KalangoOS (beside: what is taken from the system there)
    pub size: u64,
    pub encrypt: bool,
    pub locale: String,
    pub keyboard: String,
    pub name: String,
    pub user: String,
    pub hostname: String,
    #[serde(skip_serializing)]
    pub password: String,
    pub autologin: bool,
    pub fingerprint_later: bool,
    pub avatar: Option<String>,
}

/// A user name as the system takes it (lower case, letters, digits, - and _, starting with a letter).
pub fn valid_user(u: &str) -> bool {
    let mut c = u.chars();
    c.next().is_some_and(|f| f.is_ascii_lowercase()) && u.len() <= 32 && u.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        && !["root", "convidado", "greeter", "daemon", "bin", "sys", "nobody"].contains(&u)
}

pub fn valid_hostname(h: &str) -> bool {
    !h.is_empty() && h.len() <= 63 && !h.starts_with('-') && !h.ends_with('-') && h.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// A user name from a person's name ("Ana Lima" → "ana").
pub fn user_from(name: &str) -> String {
    let first = name.split_whitespace().next().unwrap_or("").to_lowercase();
    let folded: String = first.chars().map(|c| match c {
        'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
        'é' | 'ê' | 'è' => 'e',
        'í' | 'ì' => 'i',
        'ó' | 'ô' | 'õ' | 'ò' => 'o',
        'ú' | 'ü' | 'ù' => 'u',
        'ç' => 'c',
        c => c,
    }).filter(|c| c.is_ascii_alphanumeric()).collect();
    folded
}

/// How strong a password is: 0 weak, 1 fair, 2 good, 3 strong.
pub fn strength(p: &str) -> u8 {
    let kinds = [p.chars().any(|c| c.is_lowercase()), p.chars().any(|c| c.is_uppercase()), p.chars().any(|c| c.is_ascii_digit()), p.chars().any(|c| !c.is_alphanumeric())]
        .iter().filter(|k| **k).count();
    match (p.chars().count(), kinds) {
        (0..=5, _) => 0,
        (6..=7, _) => 1,
        (8..=11, k) if k >= 3 => 2,
        (8..=11, _) => 1,
        (_, k) if k >= 2 => 3,
        _ => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LSBLK: &str = r#"{"blockdevices": [
      {"name":"nvme0n1","path":"/dev/nvme0n1","size":1000204886016,"type":"disk","fstype":null,"label":null,"partlabel":null,"model":"Samsung SSD 980","tran":"nvme","rota":false,"rm":false,"mountpoints":[null],"fsused":null,
       "children":[{"name":"nvme0n1p1","path":"/dev/nvme0n1p1","size":536870912,"type":"part","fstype":"vfat","label":null,"partlabel":"EFI System Partition","mountpoints":["/boot/efi"],"fsused":"6291456"},
                   {"name":"nvme0n1p2","path":"/dev/nvme0n1p2","size":999667941376,"start":1050624,"type":"part","fstype":"ext4","label":null,"partlabel":null,"mountpoints":["/"],"fsused":"312000000000"}]},
      {"name":"sda","path":"/dev/sda","size":2000398934016,"type":"disk","fstype":null,"model":"Crucial MX500","tran":"sata","rota":false,"rm":false,"mountpoints":[null],
       "children":[{"name":"sda1","path":"/dev/sda1","size":2000397885440,"type":"part","fstype":"ext4","label":"dados","mountpoints":["/mnt/dados"],"fsused":"900000000000"}]},
      {"name":"sdb","path":"/dev/sdb","size":32015679488,"type":"disk","model":"Pendrive","tran":"usb","rota":false,"rm":true,"mountpoints":[null],
       "children":[{"name":"sdb1","path":"/dev/sdb1","size":4000000000,"type":"part","fstype":"iso9660","label":"KalangoOS","mountpoints":["/run/live/medium"]}]},
      {"name":"zram0","path":"/dev/zram0","size":8000000000,"type":"disk"}
    ]}"#;

    #[test]
    fn disks_their_kind_and_what_is_on_them() {
        let d = parse(LSBLK);
        assert_eq!(d.len(), 3);
        assert_eq!(d[0].title(), "NVMe 1 TB");
        assert!(d[0].system.is_some());
        assert_eq!(d[0].system_used, 312_006_291_456);
        assert_eq!(d[0].partitions[1].start, 1_050_624);
        assert_eq!(d[1].title(), "SSD 2 TB");
        // ext4 without an EFI partition beside it: data, not a system
        assert_eq!(d[1].system, None);
        assert!(d[2].live && d[2].removable);
    }

    #[test]
    fn accounts_and_passwords() {
        assert_eq!(user_from("Ana Lima"), "ana");
        assert_eq!(user_from("Ícaro"), "icaro");
        assert!(valid_user("ana") && !valid_user("Ana") && !valid_user("root") && !valid_user("9ana"));
        assert!(valid_hostname("ana-MS-7C95") && !valid_hostname("-x"));
        assert_eq!(strength("abc"), 0);
        assert_eq!(strength("abcdefg"), 1);
        assert_eq!(strength("Abcdef12"), 2);
        assert_eq!(strength("Lua-cheia-2026"), 3);
    }
}
