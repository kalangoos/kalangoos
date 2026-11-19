//! Bringing a person's things from another system on this computer (the first use's "Quer trazer as suas coisas?"):
//! the disks' other systems — a Linux beside this one (its /home) or Windows (its Users) —, what each person there has
//! (the usual folders, a browser profile), and copying them into the caller's home, as theirs.
//!
//! The other system is only ever mounted read-only, and nothing in this home is overwritten (what exists stays). The
//! copy runs apart (it can be hundreds of GB): `importar.copiar` starts it and `importar.estado` says how far it is.
//! KALANGOD_IMPORTAR_RAIZ (tests): a folder that stands for another system's disk, already "mounted"; the copy then
//! goes to a "casa" folder beside it.

use super::*;
use std::path::{Path, PathBuf};

pub fn tools() -> Vec<Tool> {
    let t = |name, description, level, effect, reversible, schema: fn() -> Value, run: Run| Tool { name, description, level, effect, reversible, schema, run };
    use Effect::*;
    use Level::*;
    vec![
        t("importar.procurar", n_("Other systems on this computer's disks and what each person there has (folders, browser)"), Observar, Leitura, false, none, find),
        t("importar.copiar", n_("Copies a person's folders and browser from another system on this computer into your home (read-only there; nothing here is overwritten)"),
            Sistema, Escreve, false,
            || obj(json!({"dispositivo": {"type": "string", "description": "a partition from importar.procurar"}, "usuario": {"type": "string"},
                "arquivos": {"type": "boolean"}, "navegador": {"type": "boolean"}}), &["dispositivo", "usuario"]), copy),
        t("importar.estado", n_("How far the copy from another system is"), Observar, Leitura, false, none, status),
    ]
}

/// The usual folders: (the kind in user-dirs.dirs, the names it has in English and in Portuguese).
const FOLDERS: &[(&str, &[&str])] = &[
    ("DOCUMENTS", &["Documents", "Documentos"]),
    ("PICTURES", &["Pictures", "Imagens"]),
    ("MUSIC", &["Music", "Música", "Musica"]),
    ("VIDEOS", &["Videos", "Vídeos", "Videos"]),
    ("DOWNLOAD", &["Downloads", "Transferências"]),
    ("DESKTOP", &["Desktop", "Área de Trabalho", "Área de trabalho"]),
];

/// A browser profile: (its name, where it is in a Linux home, where it is in a Windows one).
const BROWSERS: &[(&str, &str, &str)] = &[
    ("Firefox", ".mozilla/firefox", "AppData/Roaming/Mozilla/Firefox"),
    ("Chrome", ".config/google-chrome", "AppData/Local/Google/Chrome/User Data"),
    ("Chromium", ".config/chromium", "AppData/Local/Chromium/User Data"),
    ("Edge", ".config/microsoft-edge", "AppData/Local/Microsoft/Edge/User Data"),
];

fn test_root() -> Option<PathBuf> {
    std::env::var_os("KALANGOD_IMPORTAR_RAIZ").map(PathBuf::from).filter(|p| p.is_dir())
}

/// A partition of another system: ext4/btrfs/xfs (Linux) or NTFS (Windows), not this system's own.
struct Part {
    dev: String,
    fstype: String,
    used: u64,
    mounted: Option<String>,
}

fn parts() -> Vec<Part> {
    let Ok(raw) = exec("lsblk", &["-J", "-b", "-o", "PATH,TYPE,FSTYPE,MOUNTPOINTS,FSUSED,SIZE"]) else { return vec![] };
    let v: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
    let mut out = Vec::new();
    let mut stack: Vec<Value> = v["blockdevices"].as_array().cloned().unwrap_or_default();
    while let Some(d) = stack.pop() {
        stack.extend(d["children"].as_array().cloned().unwrap_or_default());
        let fstype = d["fstype"].as_str().unwrap_or("");
        if !["ext4", "btrfs", "xfs", "ntfs", "ntfs3"].contains(&fstype) || d["type"] != "part" && d["type"] != "crypt" && d["type"] != "lvm" {
            continue;
        }
        let mounts: Vec<String> = d["mountpoints"].as_array().into_iter().flatten().filter_map(|m| m.as_str().map(str::to_string)).collect();
        // this system's own (its root, its /home, /boot) is not "another system"
        if mounts.iter().any(|m| m == "/" || m == "/home" || m.starts_with("/boot") || m == "/target") {
            continue;
        }
        let num = |k: &str| d[k].as_u64().or_else(|| d[k].as_str().and_then(|s| s.parse().ok())).unwrap_or(0);
        if num("size") < 4 << 30 {
            continue;
        }
        out.push(Part { dev: d["path"].as_str().unwrap_or("").into(), fstype: fstype.into(), used: num("fsused"), mounted: mounts.into_iter().next() });
    }
    out.sort_by(|a, b| a.dev.cmp(&b.dev));
    out
}

/// Mounts a partition read-only where only kalangod looks (or uses where it already is). Returns the place and
/// whether it was mounted here (to unmount after).
fn mount_ro(p: &Part) -> Result<(PathBuf, bool), String> {
    if let Some(m) = &p.mounted {
        return Ok((PathBuf::from(m), false));
    }
    let dir = PathBuf::from("/run/kalangod/importar").join(p.dev.trim_start_matches("/dev/").replace('/', "-"));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let d = dir.display().to_string();
    // never written: ext4 without replaying its journal, xfs without recovery, NTFS by the kernel's driver
    let r = match p.fstype.as_str() {
        "ext4" => exec("mount", &["-o", "ro,noload", &p.dev, &d]),
        "xfs" => exec("mount", &["-o", "ro,norecovery", &p.dev, &d]),
        "ntfs" | "ntfs3" => exec("mount", &["-t", "ntfs3", "-o", "ro", &p.dev, &d]).or_else(|_| exec("mount", &["-t", "ntfs-3g", "-o", "ro", &p.dev, &d])),
        _ => exec("mount", &["-o", "ro", &p.dev, &d]),
    };
    r.map(|_| (dir, true))
}

fn unmount(dir: &Path) {
    let _ = exec("umount", &[&dir.display().to_string()]);
}

/// The system a mounted partition holds and where its people's homes are.
fn system_of(root: &Path) -> (String, Option<PathBuf>, bool) {
    if root.join("Windows").is_dir() || root.join("Users").is_dir() && !root.join("home").is_dir() {
        return ("Windows".into(), Some(root.join("Users")).filter(|p| p.is_dir()), true);
    }
    let os = std::fs::read_to_string(root.join("etc/os-release")).unwrap_or_default();
    let name = os.lines().find_map(|l| l.strip_prefix("NAME=")).map(|v| v.trim_matches('"').to_string());
    if root.join("home").is_dir() {
        return (name.unwrap_or_else(|| "Linux".into()), Some(root.join("home")), false);
    }
    // a /home partition on its own: the people's folders are at its top
    let homes = std::fs::read_dir(root).ok().map(|d| d.flatten().any(|e| e.path().join(".config").is_dir() || e.path().join(".bashrc").is_file())).unwrap_or(false);
    ("Linux".into(), homes.then(|| root.to_path_buf()), false)
}

fn people(homes: &Path, windows: bool) -> Vec<PathBuf> {
    let skip = ["lost+found", "Public", "Default", "Default User", "All Users", "desktop.ini", "Todos os Usuários", "Usuário Padrão"];
    let mut v: Vec<PathBuf> = std::fs::read_dir(homes).ok().into_iter().flatten().flatten().map(|e| e.path())
        .filter(|p| p.is_dir() && !p.is_symlink() && !skip.contains(&p.file_name().and_then(|n| n.to_str()).unwrap_or("")))
        .filter(|p| !windows || p.join("Documents").is_dir() || p.join("Desktop").is_dir())
        .collect();
    v.sort();
    v
}

/// The usual folders a home has: (kind, its path there).
fn folders_of(home: &Path) -> Vec<(&'static str, PathBuf)> {
    FOLDERS.iter().filter_map(|(kind, names)| names.iter().map(|n| home.join(n)).find(|p| p.is_dir()).map(|p| (*kind, p))).collect()
}

/// A browser's profile in a home and how many bookmarks it has (when that can be counted).
fn browsers_of(home: &Path, windows: bool) -> Vec<(&'static str, PathBuf, Option<u64>)> {
    let mut v = Vec::new();
    for (name, linux, win) in BROWSERS {
        let p = home.join(if windows { win } else { linux });
        if !p.is_dir() {
            continue;
        }
        let count = if *name == "Firefox" {
            firefox_bookmarks(&p)
        } else {
            std::fs::read_to_string(p.join("Default/Bookmarks")).ok().map(|t| t.matches("\"type\": \"url\"").count() as u64)
        };
        v.push((*name, p, count));
    }
    v
}

fn firefox_bookmarks(dir: &Path) -> Option<u64> {
    let ini = std::fs::read_to_string(dir.join("profiles.ini")).ok()?;
    let rel = ini.lines().find_map(|l| l.strip_prefix("Path="))?;
    let db = dir.join(rel).join("places.sqlite");
    // read without a lock (the file may be another system's, never written)
    let uri = format!("file:{}?immutable=1", db.display());
    exec("sqlite3", &[&uri, "select count(*) from moz_bookmarks where type=1"]).ok()?.trim().parse().ok()
}

fn find(_: &Ctx, _: &Value) -> Result<Out, String> {
    let mut systems = Vec::new();
    let candidates: Vec<(Part, Option<PathBuf>)> = match test_root() {
        Some(r) => vec![(Part { dev: "teste".into(), fstype: "ext4".into(), used: 312_000_000_000, mounted: Some(r.display().to_string()) }, None)],
        None => parts().into_iter().map(|p| (p, None)).collect(),
    };
    for (p, _) in candidates {
        let Ok((root, ours)) = mount_ro(&p) else { continue };
        let (system, homes, windows) = system_of(&root);
        if let Some(h) = homes {
            let ps: Vec<Value> = people(&h, windows).iter().map(|home| {
                let fs: Vec<Value> = folders_of(home).iter().map(|(k, path)| json!({"tipo": k, "nome": path.file_name().and_then(|n| n.to_str()).unwrap_or("")})).collect();
                let bs: Vec<Value> = browsers_of(home, windows).iter().map(|(n, _, c)| json!({"nome": n, "favoritos": c})).collect();
                json!({"nome": home.file_name().and_then(|n| n.to_str()).unwrap_or(""), "pastas": fs, "navegadores": bs})
            }).filter(|v| v["pastas"].as_array().is_some_and(|a| !a.is_empty()) || v["navegadores"].as_array().is_some_and(|a| !a.is_empty())).collect();
            if !ps.is_empty() {
                systems.push(json!({"dispositivo": p.dev, "sistema": system, "usado": p.used, "windows": windows, "pessoas": ps}));
            }
        }
        if ours {
            unmount(&root);
        }
    }
    let n = systems.len();
    Ok(Out::read(json!({"sistemas": systems}), trf("{n} other systems on the disks", &[("n", n.to_string())])))
}

fn status_file(uid: u32) -> PathBuf {
    let base = if test_root().is_some() { std::env::temp_dir() } else { PathBuf::from("/run/kalangod") };
    base.join(format!("importar-{uid}.json"))
}

fn write_status(uid: u32, v: &Value) {
    let f = status_file(uid);
    if let Some(d) = f.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(f, v.to_string());
}

fn status(c: &Ctx, _: &Value) -> Result<Out, String> {
    let v: Value = std::fs::read(status_file(c.peer.uid)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(json!({"fase": "nada"}));
    let s = v["fase"].as_str().unwrap_or("").to_string();
    Ok(Out::read(v, s))
}

/// The caller's own folder for a kind (their user-dirs.dirs), else the name it had there.
fn dest_for(home: &Path, kind: &str, fallback: &str) -> PathBuf {
    let dirs = std::fs::read_to_string(home.join(".config/user-dirs.dirs")).unwrap_or_default();
    dirs.lines().find_map(|l| l.strip_prefix(&format!("XDG_{kind}_DIR=")))
        .map(|v| v.trim_matches('"').replace("$HOME", &home.display().to_string()))
        .map(PathBuf::from)
        .filter(|p| p != home)
        .unwrap_or_else(|| home.join(fallback))
}

fn copy(c: &Ctx, a: &Value) -> Result<Out, String> {
    let dev = s(a, "dispositivo")?.to_string();
    let user = s(a, "usuario")?.to_string();
    if user.contains('/') || user.starts_with('.') || user.is_empty() {
        return Err(tr("invalid user name").into());
    }
    let (want_files, want_browser) = (a["arquivos"].as_bool().unwrap_or(true), a["navegador"].as_bool().unwrap_or(true));
    let part = match test_root() {
        Some(r) if dev == "teste" => Part { dev: dev.clone(), fstype: "ext4".into(), used: 0, mounted: Some(r.display().to_string()) },
        _ => parts().into_iter().find(|p| p.dev == dev).ok_or_else(|| trf("{device} is not another system's disk", &[("device", dev.clone())]))?,
    };
    let (root, ours) = mount_ro(&part)?;
    let (system, homes, windows) = system_of(&root);
    let Some(src) = homes.map(|h| h.join(&user)).filter(|p| p.is_dir()) else {
        if ours {
            unmount(&root);
        }
        return Err(trf("{user} has no home on {device}", &[("user", user.clone()), ("device", dev.clone())]));
    };
    // (tests: the copy goes to a "casa" beside the stand-in disk, never the person's real home)
    let home = match test_root() {
        Some(r) => r.with_file_name("casa"),
        None => c.peer.home.clone(),
    };
    let owner = format!("{}:{}", c.peer.uid, c.peer.gid);
    // what goes where: the folders into the caller's own, a browser profile only where there is none yet
    let mut plan: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut skipped = Vec::new();
    if want_files {
        for (kind, p) in folders_of(&src) {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
            plan.push((p, dest_for(&home, kind, &name)));
        }
    }
    if want_browser {
        for (name, p, _) in browsers_of(&src, windows) {
            let linux = BROWSERS.iter().find(|b| b.0 == name).map(|b| b.1).unwrap_or("");
            let dest = home.join(linux);
            if windows {
                // a Windows profile does not run here: its bookmarks go into this browser's, when it has none
                let bm = p.join("Default/Bookmarks");
                let to = dest.join("Default/Bookmarks");
                if bm.is_file() && !to.exists() && name != "Firefox" {
                    plan.push((bm, to));
                } else {
                    skipped.push(name.to_string());
                }
            } else if dest.exists() {
                skipped.push(name.to_string());
            } else {
                plan.push((p, dest));
            }
        }
    }
    let commands: Vec<String> = plan.iter().map(|(f, t)| format!("rsync -a --ignore-existing --chown={owner} {}{} {}", f.display(), if f.is_dir() { "/" } else { "" }, t.display())).collect();
    let uid = c.peer.uid;
    let summary = trf("bringing {n} folders from {system}", &[("n", plan.len().to_string()), ("system", system.clone())]);
    if c.cfg.dry_run {
        if ours {
            unmount(&root);
        }
        write_status(uid, &json!({"fase": "terminou", "feito": 0, "total": 0, "simulado": commands, "pulados": skipped}));
        return Ok(Out { value: json!({"simulado": commands, "pulados": skipped, "sistema": system}), summary, inverse: None });
    }
    // the copy, apart: how much there is, then each piece with rsync's own progress; at the end the disk goes away
    let total_plan = plan.clone();
    let skipped_out = skipped.clone();
    write_status(uid, &json!({"fase": "medindo", "feito": 0, "total": 0}));
    std::thread::spawn(move || {
        let total: u64 = total_plan.iter().map(|(f, _)| exec("du", &["-sb", &f.display().to_string()]).ok()
            .and_then(|o| o.split_whitespace().next().and_then(|n| n.parse().ok())).unwrap_or(0)).sum();
        let mut done = 0u64;
        let mut error = None;
        for (from, to) in &total_plan {
            if let Some(d) = if from.is_dir() { Some(to.as_path()) } else { to.parent() } {
                let _ = std::fs::create_dir_all(d);
                let _ = exec("chown", &[&owner, &d.display().to_string()]);
            }
            let src_arg = format!("{}{}", from.display(), if from.is_dir() { "/" } else { "" });
            let child = std::process::Command::new("rsync").args(["-a", "--ignore-existing", &format!("--chown={owner}"), "--info=progress2", "--no-inc-recursive", &src_arg, &to.display().to_string()])
                .env("LC_ALL", "C").stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn();
            let Ok(mut child) = child else { error = Some("rsync".to_string()); break };
            let mut piece = 0u64;
            if let Some(out) = child.stdout.take() {
                use std::io::Read;
                // progress2 rewrites one line with \r: "  1,234,567  12%  ..."
                let mut buf = Vec::new();
                for b in out.bytes().map_while(Result::ok) {
                    if b == b'\r' || b == b'\n' {
                        let line = String::from_utf8_lossy(&buf).to_string();
                        if let Some(n) = line.split_whitespace().next().and_then(|n| n.replace(',', "").parse::<u64>().ok()) {
                            piece = n;
                            write_status(uid, &json!({"fase": "copiando", "feito": done + piece, "total": total, "atual": to.display().to_string()}));
                        }
                        buf.clear();
                    } else {
                        buf.push(b);
                    }
                }
            }
            let ok = child.wait().map(|s| s.success() || s.code() == Some(23) || s.code() == Some(24)).unwrap_or(false);
            done += piece;
            if !ok {
                error = Some(to.display().to_string());
            }
        }
        if ours {
            unmount(&root);
        }
        write_status(uid, &json!({"fase": "terminou", "feito": done, "total": total, "erro": error, "pulados": skipped}));
    });
    Ok(Out { value: json!({"comandos": commands, "pulados": skipped_out, "sistema": system}), summary, inverse: None })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_linux_beside_and_its_people() {
        let d = std::env::temp_dir().join(format!("kalangod-importar-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("etc")).unwrap();
        std::fs::write(d.join("etc/os-release"), "NAME=\"Ubuntu\"\nVERSION=\"24.04\"\n").unwrap();
        for p in ["home/ana/Documentos", "home/ana/Imagens", "home/ana/.mozilla/firefox", "home/lost+found"] {
            std::fs::create_dir_all(d.join(p)).unwrap();
        }
        let (system, homes, windows) = system_of(&d);
        assert_eq!(system, "Ubuntu");
        assert!(!windows);
        let ps = people(&homes.unwrap(), false);
        assert_eq!(ps.len(), 1);
        let kinds: Vec<&str> = folders_of(&ps[0]).iter().map(|(k, _)| *k).collect();
        assert_eq!(kinds, vec!["DOCUMENTS", "PICTURES"]);
        assert_eq!(browsers_of(&ps[0], false).iter().map(|b| b.0).collect::<Vec<_>>(), vec!["Firefox"]);
        // the caller's own folder for the kind
        let me = d.join("eu");
        std::fs::create_dir_all(me.join(".config")).unwrap();
        std::fs::write(me.join(".config/user-dirs.dirs"), "XDG_DOCUMENTS_DIR=\"$HOME/Documentos\"\nXDG_DESKTOP_DIR=\"$HOME\"\n").unwrap();
        assert_eq!(dest_for(&me, "DOCUMENTS", "Documents"), me.join("Documentos"));
        assert_eq!(dest_for(&me, "DESKTOP", "Desktop"), me.join("Desktop"));
        let _ = std::fs::remove_dir_all(&d);
    }
}
