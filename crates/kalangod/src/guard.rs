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

/// Quantos cgroups de agentes o programa eBPF guarda (`MAX_AGENTES` em guarda.bpf.c): o do sistema e um por gerenciador de
/// usuário do systemd. Passar disso é caso de alerta, nunca de silêncio (ver [`plano_agentes`]).
const MAX_AGENTES: usize = 4096;

/// Protegidas dos agentes pela política do kalangod e pelo Landlock (`policy::KALANGO_PROTECTED`), mas NÃO pela guarda do kernel:
/// o serviço dos Fluxos roda no cgroup dos agentes (a política do kalangod o reconhece por ele) e é o dono do banco dos fluxos e das
/// aprovações; a guarda vê o cgroup, não quem é o serviço, e o impedia de abrir o próprio banco. Uma isenção por cgroup, por
/// programa ou por pid pode ser contornada por quem roda como a mesma pessoa (escrever no `cgroup.procs` do serviço, copiar ou
/// executar o programa com `LD_PRELOAD`); sem isenção não há o que contornar. O que fica de fora do kernel é só esta pasta.
pub const FORA_DA_GUARDA_DO_KERNEL: &[&str] = &["~/.local/share/kalango/fluxos"];

/// As pastas que a guarda do kernel protege: as da configuração menos [`FORA_DA_GUARDA_DO_KERNEL`].
pub fn para_o_kernel(patterns: Vec<String>) -> Vec<String> {
    patterns.into_iter().filter(|p| !FORA_DA_GUARDA_DO_KERNEL.contains(&p.as_str())).collect()
}

/// `false` quando há cgroups de agentes que a guarda não consegue cobrir (mapa cheio): `kernel.seguranca` mostra
/// `guarda_ebpf_completa: false` e a linha do tempo recebe um alerta.
pub static COMPLETA: AtomicBool = AtomicBool::new(true);

/// O que fazer para o mapa dos cgroups de agentes ficar igual ao que existe: ids a pôr, ids a tirar e quantos não cabem.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct PlanoAgentes {
    pub inserir: Vec<u64>,
    pub remover: Vec<u64>,
    /// Cgroups de agentes que não cabem no mapa: ficariam sem a guarda.
    pub descartados: usize,
}

/// Compara o que está no mapa (`atual`) com o que deve estar (`desejado`, na ordem de prioridade: o primeiro é o do
/// sistema), para um mapa de `capacidade` entradas. O que sobra além da capacidade é contado, não ignorado.
pub fn plano_agentes(atual: &std::collections::HashSet<u64>, desejado: &[u64], capacidade: usize) -> PlanoAgentes {
    let mut vistos = std::collections::HashSet::new();
    let unicos: Vec<u64> = desejado.iter().copied().filter(|&id| id != 0 && vistos.insert(id)).collect();
    let cabem: std::collections::HashSet<u64> = unicos.iter().copied().take(capacidade).collect();
    let mut inserir: Vec<u64> = unicos.iter().copied().filter(|id| cabem.contains(id) && !atual.contains(id)).collect();
    let mut remover: Vec<u64> = atual.iter().copied().filter(|id| !cabem.contains(id)).collect();
    inserir.sort_unstable();
    remover.sort_unstable();
    PlanoAgentes { inserir, remover, descartados: unicos.len() - cabem.len() }
}

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

/// (uid, casa) dos usuários (para expandir `~` e `%t` das pastas protegidas para todos eles).
fn users() -> Vec<(u32, PathBuf)> {
    std::fs::read_to_string("/etc/passwd").unwrap_or_default().lines().filter_map(|l| {
        let f: Vec<&str> = l.split(':').collect();
        let uid: u32 = f.get(2)?.parse().ok()?;
        let h = PathBuf::from(f.get(5)?);
        (uid == 0 || (1000..60000).contains(&uid)).then_some((uid, h))
    }).collect()
}

pub fn expand_all(patterns: &[String]) -> Vec<PathBuf> {
    expand_for(patterns, &users())
}

/// `~` is each user's home and `%t` each user's runtime folder (`/run/user/UID`, `policy::runtime_dir`): the folder of
/// the Mind's confirmation codes lives there, and is made after kalangod starts, so it is looked for again.
fn expand_for(patterns: &[String], users: &[(u32, PathBuf)]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for p in patterns {
        if p == "~" || p.starts_with("~/") {
            out.extend(users.iter().map(|(_, h)| crate::policy::expand(p, h)));
        } else if let Some(rest) = p.strip_prefix("%t/") {
            out.extend(users.iter().map(|(uid, _)| crate::policy::runtime_dir(*uid).join(rest)));
        } else {
            out.push(PathBuf::from(p));
        }
    }
    out
}

struct Guarda {
    bpf: Ebpf,
    conhecidas: std::collections::HashSet<(u64, u64)>,
    /// Os ids que estão em CG_AGENTES.
    agentes: std::collections::HashSet<u64>,
}

impl Guarda {
    /// Deixa o mapa CG_AGENTES com os cgroups de agentes de `desejado`. Devolve quantos não couberam (nunca em silêncio:
    /// quem chama avisa).
    fn sincronizar_agentes(&mut self, desejado: &[u64]) -> Result<usize, String> {
        let plano = plano_agentes(&self.agentes, desejado, MAX_AGENTES);
        let mut m: BpfHashMap<_, u64, u8> = BpfHashMap::try_from(self.bpf.map_mut("CG_AGENTES").ok_or("mapa CG_AGENTES")?).map_err(|e| e.to_string())?;
        // (primeiro entram os novos: um cgroup que continua não fica um instante sem proteção)
        for id in &plano.inserir {
            m.insert(id, 1, 0).map_err(|e| e.to_string())?;
            self.agentes.insert(*id);
        }
        for id in &plano.remover {
            let _ = m.remove(id);
            self.agentes.remove(id);
        }
        Ok(plano.descartados)
    }

    fn config(&mut self, k: u32, v: u64) -> Result<(), String> {
        let mut cfg: Array<_, u64> = Array::try_from(self.bpf.map_mut("CONFIG").ok_or("mapa CONFIG")?).map_err(|e| e.to_string())?;
        cfg.set(k, v, 0).map_err(|e| e.to_string())
    }

    /// Descobre como o kernel identifica cada pasta protegida: abre a pasta com a sonda ligada e lê o evento
    /// (o par dispositivo/inode do superbloco, que em btrfs e overlay difere do st_dev do stat()).
    fn probe(&mut self, rb: &mut RingBuf<aya::maps::MapData>, paths: &[PathBuf], timeline: &Timeline) -> Result<usize, String> {
        if !paths.iter().any(|p| p.exists()) {
            return Ok(0);
        }
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
        status: status.into(), summary, inverse: None, restore_point: None, miniatura: None, startup: None, d1: None,
    });
}

/// Estado mostrado em `kernel.seguranca`.
pub static ESTADO: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

pub fn status() -> String {
    let s = ESTADO.lock().unwrap().clone();
    if s.is_empty() { "desligada".into() } else { s }
}

/// Os cgroups de agentes que a guarda tem de conhecer: o do sistema primeiro, depois um por gerenciador de usuário.
fn desejados(sistema: u64, usuarios: &[(u64, u64)]) -> Vec<u64> {
    std::iter::once(sistema).chain(usuarios.iter().map(|(id, _)| *id)).collect()
}

/// Cobertura incompleta nunca passa em silêncio: `kernel.seguranca` mostra `guarda_ebpf_completa: false` e a linha do
/// tempo recebe um alerta (uma vez por mudança de situação).
fn avisar_cobertura(descartados: usize, total: usize, timeline: &Timeline) {
    let completa = descartados == 0;
    if COMPLETA.swap(completa, Ordering::SeqCst) == completa {
        return;
    }
    if !completa {
        let resumo = kalango_i18n::trf("guard covers only {n} of {m} agent cgroups", &[("n", (total - descartados).to_string()), ("m", total.to_string())]);
        eprintln!("kalangod: {resumo}");
        timeline.append(Entry {
            id: 0, time: 0, caller: "kernel (guarda eBPF)".into(), uid: 0, pid: 0, tool: "auditoria.guarda_incompleta".into(), level: "auditoria".into(),
            args: json!({"cobertos": total - descartados, "total": total}),
            status: "alerta".into(), summary: resumo, inverse: None, restore_point: None, miniatura: None, startup: None, d1: None,
        });
    }
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
    let patterns = para_o_kernel(patterns);
    if !nix::unistd::geteuid().is_root() {
        return Err(kalango_i18n::tr("off: kalangod is not running as root").into());
    }
    if !bpf_lsm_active() {
        return Err(kalango_i18n::tr("off: eBPF LSM inactive ('bpf' is missing from the kernel lsm= list)").into());
    }
    let (agentes, nivel) = limits::agents_cgroup_id().map_err(|e| format!("cgroup dos agentes: {e}"))?;
    let bpf = Ebpf::load(BYTES).map_err(|e| format!("carregar eBPF: {e}"))?;
    let mut g = Guarda { bpf, conhecidas: Default::default(), agentes: Default::default() };
    let btf = Btf::from_sys_fs().map_err(|e| kalango_i18n::trf("kernel BTF: {e}", &[("e", e.to_string())]))?;
    for (prog, hook) in [("guarda_abrir", "file_open"), ("guarda_executar", "bprm_check_security")] {
        let p: &mut Lsm = g.bpf.program_mut(prog).ok_or(format!("programa {prog} ausente"))?.try_into().map_err(|e| format!("{prog}: {e}"))?;
        p.load(hook, &btf).map_err(|e| format!("{prog}: {e}"))?;
        p.attach().map_err(|e| format!("{prog}: {e}"))?;
    }
    let mut rb = RingBuf::try_from(g.bpf.take_map("EVENTOS").ok_or("mapa EVENTOS")?).map_err(|e| e.to_string())?;
    let paths = expand_all(&patterns);
    let n = g.probe(&mut rb, &paths, &timeline)?;
    let mut usuarios = limits::user_agents_cgroup_ids();
    let descartados = g.sincronizar_agentes(&desejados(agentes, &usuarios))?;
    avisar_cobertura(descartados, usuarios.len() + 1, &timeline);
    ATIVA.store(true, Ordering::SeqCst);
    let msg = format!("ativa: cgroup dos agentes {agentes} (nível {nivel}), {n} pastas protegidas no kernel");
    eprintln!("kalangod: {}", kalango_i18n::trf("eBPF guard {msg}", &[("msg", msg.to_string())]));
    *ESTADO.lock().unwrap() = msg;
    std::thread::spawn(move || {
        let fd = rb.as_raw_fd();
        let mut last = std::time::Instant::now();
        let mut cg = agentes;
        let runtime = expand_all(&patterns.iter().filter(|p| p.starts_with("%t/")).cloned().collect::<Vec<_>>());
        loop {
            let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
            unsafe { libc::poll(&mut pfd, 1, 1000) };
            // (um gerenciador de usuário novo, e o cgroup dos agentes dele, entram na guarda em até 1 s)
            let atual = limits::user_agents_cgroup_ids();
            if atual != usuarios {
                if let Ok(d) = g.sincronizar_agentes(&desejados(cg, &atual)) {
                    avisar_cobertura(d, atual.len() + 1, &timeline);
                    usuarios = atual;
                }
            }
            while let Some(item) = rb.next() {
                if let Some(e) = parse(&item) {
                    if e.tipo != 3 {
                        record(&e, &timeline);
                    }
                }
            }
            // the folders in the runtime area (the Mind's confirmation codes) appear, and are made again with a new inode,
            // while kalangod runs: looked for every second (a stat and an open each), so the time one is open to an
            // agent before the kernel knows it is as short as this loop can make it
            let _ = g.probe(&mut rb, &runtime, &timeline);
            if last.elapsed().as_secs() >= 15 {
                last = std::time::Instant::now();
                // pastas protegidas criadas depois (ex.: ~/.ssh de um usuário novo) e cgroup recriado
                let _ = g.probe(&mut rb, &expand_all(&patterns), &timeline);
                if let Ok((id, _)) = limits::agents_cgroup_id() {
                    if id != cg {
                        if let Ok(d) = g.sincronizar_agentes(&desejados(id, &usuarios)) {
                            avisar_cobertura(d, usuarios.len() + 1, &timeline);
                            cg = id;
                        }
                    }
                }
                *ESTADO.lock().unwrap() = format!("ativa: cgroup dos agentes {cg} (nível {nivel}), {} pastas protegidas no kernel", g.conhecidas.len());
            }
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{expand_for, para_o_kernel, plano_agentes, FORA_DA_GUARDA_DO_KERNEL, MAX_AGENTES};
    use std::collections::HashSet;
    use std::path::PathBuf;

    /// The Fluxos service lives in the agents' cgroup and owns its folder: the kernel guard (which sees cgroups, not who is
    /// who) left it without its own database. That folder alone is out of the kernel guard, and stays protected from the agents
    /// by the policy and Landlock; everything else, the Mind's folders and ~/.ssh included, stays in the kernel guard.
    #[test]
    fn only_the_fluxos_folder_is_left_out_of_the_kernel_guard_and_it_stays_protected_by_policy() {
        let tudo: Vec<String> = ["~/.ssh", "~/.local/share/kalango/fluxos", "~/.local/share/kalango/mind", "%t/kalango/mind-confirm", "/etc/shadow"].iter().map(|x| x.to_string()).collect();
        let ficam = para_o_kernel(tudo.clone());
        assert_eq!(ficam.len(), tudo.len() - 1);
        assert!(!ficam.iter().any(|p| FORA_DA_GUARDA_DO_KERNEL.contains(&p.as_str())));
        for p in ["~/.ssh", "~/.local/share/kalango/mind", "%t/kalango/mind-confirm", "/etc/shadow"] {
            assert!(ficam.iter().any(|x| x == p), "{p} must stay in the kernel guard");
        }
        for p in FORA_DA_GUARDA_DO_KERNEL {
            assert!(crate::policy::KALANGO_PROTECTED.contains(p), "{p} stays protected for agents by the policy and Landlock");
        }
        assert_eq!(FORA_DA_GUARDA_DO_KERNEL.len(), 1);
    }

    /// The kernel guard is told every user's confirmation-code folder, whether it exists yet or not (the probe looks
    /// again every second), and the person's home folders as before.
    #[test]
    fn the_guard_expands_the_runtime_folder_for_every_user() {
        let users = vec![(0, PathBuf::from("/root")), (1000, PathBuf::from("/home/ana")), (1001, PathBuf::from("/home/bia"))];
        let got = expand_for(&["%t/kalango/mind-confirm".into(), "~/.ssh".into(), "/etc/shadow".into()], &users);
        let run = |u: u32| crate::policy::runtime_dir(u).join("kalango/mind-confirm");
        for u in [0, 1000, 1001] {
            assert!(got.contains(&run(u)), "{got:?}");
        }
        assert!(got.contains(&PathBuf::from("/home/ana/.ssh")) && got.contains(&PathBuf::from("/etc/shadow")));
        assert_eq!(got.len(), 7);
    }

    /// The kernel's verifier once refused the whole guard ("infinite loop detected") because the search through the agents'
    /// cgroups was a `for` whose counter lived on the stack. The search has no loop: one constant ancestor level per step
    /// and one lookup in the hash of agents' cgroups. (The real proof is the VM test, `agents_guard` in
    /// imagem/testar-iso.py: the verifier only runs in the kernel.)
    #[test]
    fn the_ebpf_program_has_no_loop_and_no_per_user_limit() {
        let src = include_str!("../ebpf/guarda.bpf.c");
        let start = src.find("static __always_inline int eh_agente").expect("eh_agente");
        let body = &src[start..src[start..].find("\n}\n").unwrap() + start];
        assert!(!body.contains("for (") && !body.contains("while ("), "a loop in eh_agente:\n{body}");
        for n in 1..=8 {
            assert!(body.contains(&format!("NIVEL({n})")), "ancestor level {n} not tested");
        }
        assert!(src.contains("CG_AGENTES") && !src.contains("MAX_USUARIOS") && !src.contains("CG_USUARIOS"), "the per-user array is gone");
        assert!(src.contains(&format!("#define MAX_AGENTES {MAX_AGENTES}")), "the map size the program declares is the one kalangod plans for");
    }

    /// The old array held 64 users and the rest went unprotected, with no warning. Now every cgroup that fits is registered
    /// (a hash of 4096), and what does not fit is counted so that the caller can say it.
    #[test]
    fn a_hundred_users_all_get_the_guard_and_an_overflow_is_counted() {
        let sistema = 1u64;
        let usuarios: Vec<u64> = (1000..1100).collect();
        let desejado: Vec<u64> = std::iter::once(sistema).chain(usuarios.iter().copied()).collect();
        let p = plano_agentes(&HashSet::new(), &desejado, MAX_AGENTES);
        assert_eq!(p.inserir.len(), 101, "the system cgroup and all 100 users");
        assert_eq!(p.descartados, 0);
        // (more than the map holds: the system's comes first and is never the one left out; the rest is counted)
        let muitos: Vec<u64> = (1..=(MAX_AGENTES as u64 + 10)).collect();
        let p = plano_agentes(&HashSet::new(), &muitos, MAX_AGENTES);
        assert_eq!(p.inserir.len(), MAX_AGENTES);
        assert!(p.inserir.contains(&1));
        assert_eq!(p.descartados, 10, "the ones that do not fit are reported, not ignored");
    }

    #[test]
    fn the_plan_adds_the_new_ones_removes_the_gone_ones_and_ignores_zero_and_repeats() {
        let atual: HashSet<u64> = [1, 2, 3].into_iter().collect();
        let p = plano_agentes(&atual, &[1, 3, 3, 0, 4], MAX_AGENTES);
        assert_eq!(p.inserir, vec![4]);
        assert_eq!(p.remover, vec![2]);
        assert_eq!(p.descartados, 0);
        assert_eq!(plano_agentes(&atual, &[1, 2, 3], MAX_AGENTES), Default::default(), "nothing to do when equal");
    }
}
