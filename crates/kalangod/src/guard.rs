//! Carrega a guarda eBPF (ebpf/guarda.bpf.c) e transforma os eventos em registros da linha do tempo.
//!
//! Precisa de root e do eBPF LSM ativo (`bpf` em /sys/kernel/security/lsm). Sem isso o kalangod continua
//! funcionando com Landlock e as verificações próprias, e `kernel.seguranca` mostra que a guarda está desligada.

use crate::limits;
use crate::timeline::{Entry, Timeline};
use aya::maps::{Array, HashMap as BpfHashMap, RingBuf};
use aya::programs::Lsm;
use aya::{Btf, Ebpf, Pod};
use serde_json::json;
use std::os::fd::AsRawFd;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

static BYTES: &[u8] = aya::include_bytes_aligned!(concat!(env!("OUT_DIR"), "/guarda.bpf.o"));
pub static ATIVA: AtomicBool = AtomicBool::new(false);

#[repr(C)]
#[derive(Clone, Copy)]
struct ChaveInode {
    dev: u64,
    ino: u64,
}
unsafe impl Pod for ChaveInode {}

#[repr(C)]
#[derive(Clone, Copy)]
struct Evento {
    tipo: u32,
    bloqueado: u32,
    pid: u32,
    uid: u32,
    cgroup: u64,
    dev: u64,
    ino: u64,
    flags: u32,
    nome: [u8; 64],
}

pub fn bpf_lsm_active() -> bool {
    std::fs::read_to_string("/sys/kernel/security/lsm").map(|s| s.trim().split(',').any(|x| x == "bpf")).unwrap_or(false)
}

/// Casas dos usuários (para expandir `~` das pastas protegidas para todos eles).
fn homes() -> Vec<PathBuf> {
    std::fs::read_to_string("/etc/passwd").unwrap_or_default().lines().filter_map(|l| {
        let f: Vec<&str> = l.split(':').collect();
        let uid: u32 = f.get(2)?.parse().ok()?;
        let h = PathBuf::from(f.get(5)?);
        (uid == 0 || (1000..60000).contains(&uid)).then_some(h)
    }).collect()
}

pub fn expand_all(patterns: &[String]) -> Vec<PathBuf> {
    let hs = homes();
    let mut out = Vec::new();
    for p in patterns {
        if p == "~" || p.starts_with("~/") {
            out.extend(hs.iter().map(|h| crate::policy::expand(p, h)));
        } else {
            out.push(PathBuf::from(p));
        }
    }
    out
}

struct Guarda {
    bpf: Ebpf,
    conhecidas: std::collections::HashSet<(u64, u64)>,
}

impl Guarda {
    fn config(&mut self, k: u32, v: u64) -> Result<(), String> {
        let mut cfg: Array<_, u64> = Array::try_from(self.bpf.map_mut("CONFIG").ok_or("mapa CONFIG")?).map_err(|e| e.to_string())?;
        cfg.set(k, v, 0).map_err(|e| e.to_string())
    }

    /// Descobre como o kernel identifica cada pasta protegida: abre a pasta com a sonda ligada e lê o evento
    /// (o par dispositivo/inode do superbloco, que em btrfs e overlay difere do st_dev do stat()).
    fn probe(&mut self, rb: &mut RingBuf<aya::maps::MapData>, paths: &[PathBuf], timeline: &Timeline) -> Result<usize, String> {
        let mut want = std::collections::HashMap::new();
        self.config(2, std::process::id() as u64)?;
        for p in paths {
            if let Ok(md) = std::fs::metadata(p) {
                want.insert(md.ino(), p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                let _ = std::fs::File::open(p); // dispara o file_open com a sonda
            }
        }
        self.config(2, 0)?;
        let mut found = Vec::new();
        while let Some(item) = rb.next() {
            let Some(e) = parse(&item) else { continue };
            let nome = String::from_utf8_lossy(&e.nome[..e.nome.iter().position(|&b| b == 0).unwrap_or(64)]).into_owned();
            if e.tipo == 3 && want.get(&e.ino).is_some_and(|n| n.is_empty() || *n == nome) {
                found.push((e.dev, e.ino));
            } else if e.tipo != 3 {
                record(&e, timeline);
            }
        }
        let mut prot: BpfHashMap<_, ChaveInode, u8> = BpfHashMap::try_from(self.bpf.map_mut("PROTEGIDAS").ok_or("mapa PROTEGIDAS")?).map_err(|e| e.to_string())?;
        let mut novas = 0;
        for (dev, ino) in found {
            if self.conhecidas.insert((dev, ino)) {
                prot.insert(ChaveInode { dev, ino }, 1, 0).map_err(|e| e.to_string())?;
                novas += 1;
            }
        }
        Ok(novas)
    }
}

fn parse(item: &[u8]) -> Option<Evento> {
    (item.len() >= std::mem::size_of::<Evento>()).then(|| unsafe { std::ptr::read_unaligned(item.as_ptr() as *const Evento) })
}

fn record(e: &Evento, timeline: &Timeline) {
    let nome = String::from_utf8_lossy(&e.nome[..e.nome.iter().position(|&b| b == 0).unwrap_or(64)]).into_owned();
    let (tool, status, summary) = match (e.tipo, e.bloqueado) {
        (1, 1) => ("auditoria.bloqueio", "bloqueado", kalango_i18n::trf("kernel blocked opening '{name}' (protected folder)", &[("name", nome.clone())])),
        (1, _) => ("auditoria.escrita", "auditoria", kalango_i18n::trf("opened '{name}' for writing", &[("name", nome.clone())])),
        _ => ("auditoria.execucao", "auditoria", kalango_i18n::trf("ran {name}", &[("name", nome.clone())])),
    };
    timeline.append(Entry {
        id: 0, time: 0, caller: "kernel (guarda eBPF)".into(), uid: e.uid, pid: e.pid as i32, tool: tool.into(), level: "auditoria".into(),
        args: json!({"inode": e.ino, "dispositivo": e.dev, "cgroup": e.cgroup, "nome": nome}),
        status: status.into(), summary, inverse: None, restore_point: None, miniatura: None,
    });
}

/// Estado mostrado em `kernel.seguranca`.
pub static ESTADO: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

pub fn status() -> String {
    let s = ESTADO.lock().unwrap().clone();
    if s.is_empty() { "desligada".into() } else { s }
}

/// Inicia a guarda numa thread própria. Devolve erro (sem derrubar o kalangod) se não for possível.
pub fn start(patterns: Vec<String>, timeline: Arc<Timeline>) -> Result<(), String> {
    let r = start_inner(patterns, timeline);
    if let Err(e) = &r {
        *ESTADO.lock().unwrap() = e.clone();
    }
    r
}

fn start_inner(patterns: Vec<String>, timeline: Arc<Timeline>) -> Result<(), String> {
    if !nix::unistd::geteuid().is_root() {
        return Err(kalango_i18n::tr("off: kalangod is not running as root").into());
    }
    if !bpf_lsm_active() {
        return Err(kalango_i18n::tr("off: eBPF LSM inactive ('bpf' is missing from the kernel lsm= list)").into());
    }
    let (agentes, nivel) = limits::agents_cgroup_id().map_err(|e| format!("cgroup dos agentes: {e}"))?;
    let bpf = Ebpf::load(BYTES).map_err(|e| format!("carregar eBPF: {e}"))?;
    let mut g = Guarda { bpf, conhecidas: Default::default() };
    let btf = Btf::from_sys_fs().map_err(|e| kalango_i18n::trf("kernel BTF: {e}", &[("e", e.to_string())]))?;
    for (prog, hook) in [("guarda_abrir", "file_open"), ("guarda_executar", "bprm_check_security")] {
        let p: &mut Lsm = g.bpf.program_mut(prog).ok_or(format!("programa {prog} ausente"))?.try_into().map_err(|e| format!("{prog}: {e}"))?;
        p.load(hook, &btf).map_err(|e| format!("{prog}: {e}"))?;
        p.attach().map_err(|e| format!("{prog}: {e}"))?;
    }
    let mut rb = RingBuf::try_from(g.bpf.take_map("EVENTOS").ok_or("mapa EVENTOS")?).map_err(|e| e.to_string())?;
    let paths = expand_all(&patterns);
    let n = g.probe(&mut rb, &paths, &timeline)?;
    g.config(0, agentes)?;
    g.config(1, nivel)?;
    ATIVA.store(true, Ordering::SeqCst);
    let msg = format!("ativa: cgroup dos agentes {agentes} (nível {nivel}), {n} pastas protegidas no kernel");
    eprintln!("kalangod: {}", kalango_i18n::trf("eBPF guard {msg}", &[("msg", msg.to_string())]));
    *ESTADO.lock().unwrap() = msg;
    std::thread::spawn(move || {
        let fd = rb.as_raw_fd();
        let mut last = std::time::Instant::now();
        let mut cg = agentes;
        loop {
            let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
            unsafe { libc::poll(&mut pfd, 1, 1000) };
            while let Some(item) = rb.next() {
                if let Some(e) = parse(&item) {
                    if e.tipo != 3 {
                        record(&e, &timeline);
                    }
                }
            }
            if last.elapsed().as_secs() >= 15 {
                last = std::time::Instant::now();
                // pastas protegidas criadas depois (ex.: ~/.ssh de um usuário novo) e cgroup recriado
                let _ = g.probe(&mut rb, &expand_all(&patterns), &timeline);
                if let Ok((id, _)) = limits::agents_cgroup_id() {
                    if id != cg && g.config(0, id).is_ok() {
                        cg = id;
                    }
                }
                *ESTADO.lock().unwrap() = format!("ativa: cgroup dos agentes {cg} (nível {nivel}), {} pastas protegidas no kernel", g.conhecidas.len());
            }
        }
    });
    Ok(())
}
