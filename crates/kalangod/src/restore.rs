//! Pontos de restauração, criados antes de toda ação do nível sistema.
//!
//! Cada ponto guarda a configuração do sistema (/etc) e a lista exata de pacotes instalados (nome e versão):
//! - raiz em btrfs: snapshot somente leitura da raiz (via snapper quando existe, senão `btrfs subvolume snapshot`);
//! - outros sistemas de arquivos (ext4…): cópia compactada do /etc.
//!
//! Restaurar um ponto devolve o /etc ao estado do ponto (inclusive apagando o que foi criado depois) e reinstala ou
//! remove pacotes até a lista bater. Antes de restaurar, um novo ponto é criado: a restauração também se desfaz.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Point {
    pub id: String,
    pub time: u64,
    pub descricao: String,
    /// "btrfs", "snapper" ou "arquivos"
    pub tipo: String,
    /// Caminho do snapshot (btrfs) ou número do snapper.
    pub snapshot: Option<String>,
    pub pacotes: usize,
    pub bytes: u64,
}

pub fn root_fs() -> String {
    fs_of(Path::new("/"))
}

/// Tipo do sistema de arquivos do ponto de montagem mais específico que contém `p`.
pub fn fs_of(p: &Path) -> String {
    let mounts = std::fs::read_to_string("/proc/mounts").unwrap_or_default();
    let mut best: Option<(usize, String)> = None;
    for l in mounts.lines() {
        let f: Vec<&str> = l.split_whitespace().collect();
        if f.len() < 3 {
            continue;
        }
        let mp = f[1].replace("\\040", " ");
        if crate::policy::is_within(p, Path::new(&mp)) && best.as_ref().map(|b| mp.len() >= b.0).unwrap_or(true) {
            best = Some((mp.len(), f[2].to_string()));
        }
    }
    best.map(|b| b.1).unwrap_or_default()
}

pub fn which(cmd: &str) -> bool {
    std::env::var_os("PATH").map(|p| std::env::split_paths(&p).any(|d| d.join(cmd).is_file())).unwrap_or(false)
}

fn run(prog: &str, args: &[&str]) -> Result<String, String> {
    let o = Command::new(prog).args(args).env("LC_ALL", "C").output().map_err(|e| format!("{prog}: {e}"))?;
    if !o.status.success() {
        return Err(format!("{prog} {}: {}", args.join(" "), String::from_utf8_lossy(&o.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&o.stdout).into_owned())
}

pub fn store(cfg: &crate::policy::Config) -> Store<'_> {
    Store { state_dir: &cfg.state_dir, root: &cfg.restore_root, max: cfg.pontos_max }
}

pub struct Store<'a> {
    pub state_dir: &'a Path,
    pub root: &'a Path,
    pub max: usize,
}

impl Store<'_> {
    fn dir(&self) -> PathBuf {
        self.state_dir.join("pontos")
    }

    /// Pacotes só entram quando a raiz é a raiz real do sistema (o dpkg é o do sistema).
    fn packages_enabled(&self) -> bool {
        self.root == Path::new("/") && which("dpkg-query")
    }

    pub fn list(&self) -> Vec<Point> {
        let mut v: Vec<Point> = std::fs::read_dir(self.dir()).into_iter().flatten().flatten()
            .filter_map(|e| std::fs::read_to_string(e.path().join("ponto.json")).ok())
            .filter_map(|t| serde_json::from_str(&t).ok()).collect();
        v.sort_by(|a: &Point, b: &Point| a.id.cmp(&b.id));
        v
    }

    pub fn get(&self, id: &str) -> Option<Point> {
        self.list().into_iter().find(|p| p.id == id)
    }

    /// Cria um ponto. Em simulação, só descreve o que faria.
    pub fn create(&self, descricao: &str, dry_run: bool) -> Result<Point, String> {
        let fs = fs_of(self.root);
        let tipo = if fs == "btrfs" && self.root == Path::new("/") && which("snapper") { "snapper" } else if fs == "btrfs" { "btrfs" } else { "arquivos" };
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = format!("{}-{}-{}", crate::timeline::now(), std::process::id(), SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst));
        if dry_run {
            return Ok(Point { id: format!("{id}-simulado"), time: crate::timeline::now(), descricao: descricao.into(), tipo: tipo.into(), snapshot: None, pacotes: 0, bytes: 0 });
        }
        let d = self.dir().join(&id);
        std::fs::create_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))?;
        let res = (|| -> Result<Point, String> {
            let mut pacotes = 0;
            if self.packages_enabled() {
                let list = run("dpkg-query", &["-W", "-f", "${db:Status-Abbrev}\t${Package}:${Architecture}\t${Version}\n"])?;
                let ii: String = list.lines().filter(|l| l.starts_with("ii")).map(|l| format!("{}\n", l.split_once('\t').map(|x| x.1).unwrap_or(""))).collect();
                pacotes = ii.lines().count();
                std::fs::write(d.join("pacotes.tsv"), ii).map_err(|e| e.to_string())?;
            }
            let snapshot = match tipo {
                "snapper" => Some(run("snapper", &["create", "--print-number", "--cleanup-algorithm", "number", "--description", descricao])?.trim().to_string()),
                "btrfs" => {
                    let base = self.root.join(".kalango-pontos");
                    std::fs::create_dir_all(&base).map_err(|e| e.to_string())?;
                    let snap = base.join(&id);
                    run("btrfs", &["subvolume", "snapshot", "-r", &self.root.to_string_lossy(), &snap.to_string_lossy()])?;
                    Some(snap.to_string_lossy().into_owned())
                }
                _ => {
                    let tar = d.join("etc.tar.gz");
                    // --numeric-owner: donos e permissões exatamente como estavam
                    run("tar", &["--numeric-owner", "--xattrs", "-C", &self.root.to_string_lossy(), "-czpf", &tar.to_string_lossy(), "etc"])?;
                    None
                }
            };
            let bytes = std::fs::metadata(d.join("etc.tar.gz")).map(|m| m.len()).unwrap_or(0);
            Ok(Point { id: id.clone(), time: crate::timeline::now(), descricao: descricao.into(), tipo: tipo.into(), snapshot, pacotes, bytes })
        })();
        match res {
            Ok(p) => {
                std::fs::write(d.join("ponto.json"), serde_json::to_string_pretty(&p).unwrap()).map_err(|e| e.to_string())?;
                self.prune();
                Ok(p)
            }
            Err(e) => {
                let _ = std::fs::remove_dir_all(&d);
                Err(e)
            }
        }
    }

    /// Mantém só os `max` pontos mais recentes.
    fn prune(&self) {
        let list = self.list();
        if list.len() <= self.max {
            return;
        }
        for p in &list[..list.len() - self.max] {
            let _ = self.delete(p);
        }
    }

    fn delete(&self, p: &Point) -> Result<(), String> {
        match (p.tipo.as_str(), &p.snapshot) {
            ("btrfs", Some(s)) => {
                run("btrfs", &["subvolume", "delete", s])?;
            }
            ("snapper", Some(n)) => {
                run("snapper", &["delete", n])?;
            }
            _ => {}
        }
        std::fs::remove_dir_all(self.dir().join(&p.id)).map_err(|e| e.to_string())
    }

    fn etc_source(&self, p: &Point, staging: &Path) -> Result<PathBuf, String> {
        match (p.tipo.as_str(), &p.snapshot) {
            ("btrfs", Some(s)) => Ok(PathBuf::from(s).join("etc")),
            ("snapper", Some(n)) => Ok(PathBuf::from(format!("/.snapshots/{n}/snapshot/etc"))),
            _ => {
                std::fs::create_dir_all(staging).map_err(|e| e.to_string())?;
                let tar = self.dir().join(&p.id).join("etc.tar.gz");
                run("tar", &["--numeric-owner", "--xattrs", "-C", &staging.to_string_lossy(), "-xzpf", &tar.to_string_lossy()])?;
                Ok(staging.join("etc"))
            }
        }
    }

    /// O que restaurar `id` faria: arquivos do /etc a apagar, trocar e recriar; pacotes a instalar e remover.
    pub fn plan(&self, id: &str) -> Result<(Point, Value, PathBuf, PathBuf), String> {
        let p = self.get(id).ok_or(kalango_i18n::trf("point {id} does not exist", &[("id", id.to_string())]))?;
        let staging = self.state_dir.join("restaurando").join(&p.id);
        let _ = std::fs::remove_dir_all(&staging);
        let src = self.etc_source(&p, &staging)?;
        let dst = self.root.join("etc");
        let (old, new) = (tree(&src), tree(&dst));
        let apagar: Vec<&String> = new.keys().filter(|k| !old.contains_key(*k)).collect();
        let recriar: Vec<&String> = old.keys().filter(|k| !new.contains_key(*k)).collect();
        let trocar: Vec<&String> = old.iter().filter(|(k, v)| new.get(*k).is_some_and(|n| n != *v)).map(|(k, _)| k).collect();
        let (instalar, remover) = self.package_diff(&p)?;
        let plano = json!({
            "etc": {"apagar": apagar, "recriar": recriar, "trocar": trocar},
            "pacotes": {"instalar": instalar, "remover": remover},
        });
        Ok((p, plano, src, staging))
    }

    fn package_diff(&self, p: &Point) -> Result<(Vec<String>, Vec<String>), String> {
        let file = self.dir().join(&p.id).join("pacotes.tsv");
        if !self.packages_enabled() || !file.exists() {
            return Ok((vec![], vec![]));
        }
        let parse = |t: &str| -> BTreeMap<String, String> { t.lines().filter_map(|l| l.split_once('\t')).map(|(a, b)| (a.to_string(), b.to_string())).collect() };
        let saved = parse(&std::fs::read_to_string(&file).map_err(|e| e.to_string())?);
        let cur_raw = run("dpkg-query", &["-W", "-f", "${db:Status-Abbrev}\t${Package}:${Architecture}\t${Version}\n"])?;
        let cur = parse(&cur_raw.lines().filter(|l| l.starts_with("ii")).filter_map(|l| l.split_once('\t').map(|x| x.1)).collect::<Vec<_>>().join("\n"));
        let instalar = saved.iter().filter(|(k, v)| cur.get(*k) != Some(*v)).map(|(k, v)| format!("{k}={v}")).collect();
        let remover = cur.keys().filter(|k| !saved.contains_key(*k)).cloned().collect();
        Ok((instalar, remover))
    }

    /// Restaura o ponto `id`. Devolve o plano executado.
    pub fn restore(&self, id: &str, dry_run: bool) -> Result<Value, String> {
        let (p, plano, src, staging) = self.plan(id)?;
        if dry_run {
            let _ = std::fs::remove_dir_all(&staging);
            return Ok(json!({"simulado": true, "ponto": p, "plano": plano}));
        }
        let dst = self.root.join("etc");
        // 1. apaga o que não existia no ponto (mais fundo primeiro)
        let mut apagar: Vec<String> = plano["etc"]["apagar"].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(str::to_string)).collect();
        apagar.sort_by_key(|k| std::cmp::Reverse(k.matches('/').count()));
        for rel in &apagar {
            let t = dst.join(rel);
            let md = std::fs::symlink_metadata(&t);
            let _ = match md {
                Ok(m) if m.is_dir() => std::fs::remove_dir_all(&t),
                Ok(_) => std::fs::remove_file(&t),
                Err(_) => Ok(()),
            };
        }
        // 2. copia o conteúdo do ponto por cima, preservando dono, modo, horários e links
        run("cp", &["-a", "--reflink=auto", "--remove-destination", "-T", &src.to_string_lossy(), &dst.to_string_lossy()])?;
        let _ = std::fs::remove_dir_all(&staging);
        // 3. pacotes
        let mut saida_pacotes = String::new();
        let rem: Vec<&str> = plano["pacotes"]["remover"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
        let ins: Vec<&str> = plano["pacotes"]["instalar"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
        if !rem.is_empty() {
            let mut a = vec!["remove", "-y", "-q"];
            a.extend(&rem);
            saida_pacotes += &apt(&a)?;
        }
        if !ins.is_empty() {
            let mut a = vec!["install", "-y", "-q", "--allow-downgrades", "--no-install-recommends"];
            a.extend(&ins);
            saida_pacotes += &apt(&a)?;
        }
        Ok(json!({"ponto": p, "plano": plano, "saida_pacotes": saida_pacotes}))
    }
}

fn apt(args: &[&str]) -> Result<String, String> {
    let o = Command::new("apt-get").args(args).env("DEBIAN_FRONTEND", "noninteractive").env("LC_ALL", "C").output().map_err(|e| e.to_string())?;
    if !o.status.success() {
        return Err(format!("apt-get {}: {}", args[0], String::from_utf8_lossy(&o.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&o.stdout).into_owned())
}

/// Árvore de arquivos relativa a `base`, com uma "assinatura" de cada entrada (tipo, modo, dono, tamanho, conteúdo).
fn tree(base: &Path) -> BTreeMap<String, String> {
    use std::os::unix::fs::MetadataExt;
    let mut out = BTreeMap::new();
    let mut stack = vec![base.to_path_buf()];
    let mut seen = BTreeSet::new();
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let path = e.path();
            let Ok(m) = std::fs::symlink_metadata(&path) else { continue };
            let rel = path.strip_prefix(base).unwrap().to_string_lossy().into_owned();
            let sig = if m.file_type().is_symlink() {
                format!("l {}", std::fs::read_link(&path).map(|l| l.to_string_lossy().into_owned()).unwrap_or_default())
            } else if m.is_dir() {
                if seen.insert(m.ino()) {
                    stack.push(path.clone());
                }
                format!("d {:o} {}:{}", m.mode(), m.uid(), m.gid())
            } else {
                let h = std::fs::read(&path).map(|b| fnv(&b)).unwrap_or(0);
                format!("f {:o} {}:{} {} {h:x}", m.mode(), m.uid(), m.gid(), m.len())
            };
            out.insert(rel, sig);
        }
    }
    out
}

fn fnv(b: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for x in b {
        h ^= *x as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}
