//! The name of the volume a path is on (a USB drive's label), from the mount table and /dev/disk/by-label.

use std::path::Path;

fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let (mut out, mut i) = (Vec::new(), 0);
    while i < b.len() {
        let code = if b[i] == b'\\' {
            match (b.get(i + 1), s.get(i + 2..i + 4), s.get(i + 1..i + 4)) {
                (Some(b'x'), Some(h), _) => u8::from_str_radix(h, 16).ok().map(|c| (c, 4)),
                (_, _, Some(o)) => u8::from_str_radix(o, 8).ok().map(|c| (c, 4)),
                _ => None,
            }
        } else {
            None
        };
        match code {
            Some((c, n)) => {
                out.push(c);
                i += n;
            }
            None => {
                out.push(b[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The device a path is on: the mount with the longest mount point that is a prefix of the path.
pub fn device_of(path: &Path, mountinfo: &str) -> Option<String> {
    mountinfo.lines().filter_map(|l| {
        let f: Vec<&str> = l.split(' ').collect();
        let dash = f.iter().position(|x| *x == "-")?;
        Some((unescape(f.get(4)?), f.get(dash + 2)?.to_string()))
    }).filter(|(mp, _)| path.starts_with(mp)).max_by_key(|(mp, _)| mp.len()).map(|(_, dev)| dev)
}

pub fn label_of(path: &Path) -> Option<String> {
    let dev = device_of(path, &std::fs::read_to_string("/proc/self/mountinfo").ok()?)?;
    let real = std::fs::canonicalize(&dev).ok()?;
    std::fs::read_dir("/dev/disk/by-label").ok()?.flatten().find(|e| std::fs::canonicalize(e.path()).ok().as_deref() == Some(real.as_path()))
        .map(|e| unescape(&e.file_name().to_string_lossy()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_longest_mount_point_wins() {
        let mi = "22 1 8:2 / / rw - ext4 /dev/sda2 rw\n40 22 8:17 / /media/ana/PENDRIVE\\040AZUL rw - vfat /dev/sdb1 rw\n";
        assert_eq!(device_of(Path::new("/media/ana/PENDRIVE AZUL/modelos"), mi).as_deref(), Some("/dev/sdb1"));
        assert_eq!(device_of(Path::new("/home/ana"), mi).as_deref(), Some("/dev/sda2"));
    }

    #[test]
    fn escapes_are_read() {
        assert_eq!(unescape("PENDRIVE\\x20AZUL"), "PENDRIVE AZUL");
        assert_eq!(unescape("A\\040B"), "A B");
    }
}
