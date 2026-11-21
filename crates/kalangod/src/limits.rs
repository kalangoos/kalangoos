//! Limites por tarefa de agente: cada programa que o kalangod executa em nome de um agente roda num cgroup próprio
//! (memória, CPU, número de processos e tempo), dentro de `/kalango.slice/kalango-agentes.slice`, que é o cgroup
//! que a guarda eBPF reconhece como "agente".
//!
//! Três caminhos, escolhidos sozinhos:
//! - root com systemd: `systemd-run --scope --slice=kalango-agentes.slice` (o jeito certo numa distro com systemd);
//! - root sem systemd (VM de teste, contêiner): cgroup v2 direto em /sys/fs/cgroup;
//! - sem root (desenvolvimento): `systemd-run --user --scope`.
//!
//! Em todos, o programa passa antes pelo trampolim `kalangod __tarefa`, que entra no cgroup (quando é o caso),
//! larga o root, liga `no_new_privs`, aplica o Landlock nas pastas permitidas e só então executa o programa.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct Limits {
    pub memoria_mb: u64,
    /// 100 = um núcleo inteiro.
    pub cpu_pct: u64,
    pub processos: u64,
    pub tempo_s: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { memoria_mb: 2048, cpu_pct: 200, processos: 256, tempo_s: 600 }
    }
}

impl Limits {
    /// O agente pode pedir limites menores, nunca maiores que os da configuração.
    pub fn narrowed(&self, a: &Value) -> Limits {
        let pick = |k: &str, max: u64| a.get(k).and_then(Value::as_u64).map(|v| v.clamp(1, max)).unwrap_or(max);
        Limits { memoria_mb: pick("memoria_mb", self.memoria_mb), cpu_pct: pick("cpu_pct", self.cpu_pct), processos: pick("processos", self.processos), tempo_s: pick("tempo_s", self.tempo_s) }
    }
}

pub const CGROOT: &str = "/sys/fs/cgroup";
pub const AGENTES: &str = "kalango.slice/kalango-agentes.slice";
const MAX_SAIDA: usize = 512 * 1024;
static NEXT: AtomicU64 = AtomicU64::new(1);

pub fn agents_dir() -> PathBuf {
    Path::new(CGROOT).join(AGENTES)
}

fn systemd_running() -> bool {
    Path::new("/run/systemd/system").is_dir()
}

/// Garante que o cgroup dos agentes exista. Só como root.
pub fn prepare() -> std::io::Result<()> {
    if systemd_running() {
        // com systemd, a árvore é dele: o slice vem da unidade dist/kalango-agentes.slice
        if !agents_dir().is_dir() {
            let _ = Command::new("systemctl").args(["start", "kalango-agentes.slice"]).status();
        }
        return if agents_dir().is_dir() { Ok(()) } else { Err(std::io::Error::other("kalango-agentes.slice não está instalado/ativo")) };
    }
    let mut cur = PathBuf::from(CGROOT);
    for part in AGENTES.split('/') {
        let _ = std::fs::write(cur.join("cgroup.subtree_control"), "+cpu +memory +pids");
        cur.push(part);
        std::fs::create_dir_all(&cur)?;
    }
    std::fs::write(cur.join("cgroup.subtree_control"), "+cpu +memory +pids")
}

/// (id, nível) do cgroup dos agentes: o id é o inode do diretório no cgroupfs; o nível conta a partir da raiz (0).
pub fn agents_cgroup_id() -> std::io::Result<(u64, u64)> {
    prepare()?;
    let md = std::fs::metadata(agents_dir())?;
    Ok((md.ino(), AGENTES.split('/').count() as u64))
}

/// O que o trampolim recebe (um argumento JSON).
#[derive(Serialize, Deserialize, Default)]
struct Plano {
    cgroup: Option<PathBuf>,
    uid: Option<u32>,
    gid: Option<u32>,
    landlock: Option<Landlock>,
    seccomp: Option<Seccomp>,
    /// Quando presente, o programa recebe só estas variáveis de ambiente.
    ambiente: Option<std::collections::BTreeMap<String, String>>,
    programa: String,
    argumentos: Vec<String>,
    pasta: Option<PathBuf>,
}

#[derive(Serialize, Deserialize, Default)]
pub struct Landlock {
    /// Leitura e execução.
    pub leitura: Vec<PathBuf>,
    /// Leitura e escrita.
    pub escrita: Vec<PathBuf>,
    /// Only listing what a folder holds (the names, not the files): a folder on the way to a protected one.
    #[serde(default)]
    pub listar: Vec<PathBuf>,
    /// Rede TCP (Landlock ABI 4+): `None` não restringe; `Some(portas)` só deixa conectar nessas portas.
    #[serde(default)]
    pub portas: Option<Vec<u16>>,
}

/// Perfil seccomp aplicado logo antes do exec (vale para o programa e tudo o que ele iniciar).
#[derive(Serialize, Deserialize, Default, Clone, Copy)]
pub struct Seccomp {
    /// Sem rede: sockets de internet (IPv4, IPv6, pacotes crus) são negados.
    pub sem_rede: bool,
    /// No local sockets (AF_UNIX): no D-Bus (the session and system buses), no systemd (its private socket, `systemd-run
    /// --user`, `systemctl --user start`, D-Bus activation of a user service), no other program's socket — the ways a
    /// command could have something started outside its own cgroup, where it would outlive the task and the rules it
    /// runs under. `socketpair` (a pipe between a process and its children) still works.
    #[serde(default)]
    pub sem_unix: bool,
}

pub struct Pedido<'a> {
    pub programa: &'a str,
    pub argumentos: &'a [String],
    pub uid: u32,
    pub gid: u32,
    pub limites: Limits,
    pub landlock: Option<Landlock>,
    pub seccomp: Option<Seccomp>,
    pub ambiente: Option<std::collections::BTreeMap<String, String>>,
    pub pasta: Option<PathBuf>,
    pub dry_run: bool,
}

/// Comando pronto para rodar dentro de um cgroup limitado, com o caminho do cgroup (quando conhecido).
pub struct Preparado {
    pub cmd: Command,
    pub dir: Option<PathBuf>,
    pub modo: &'static str,
    descricao: String,
    /// the systemd unit (scope) of the task
    unit: String,
}


/// Monta o comando: escolhe systemd (root), cgroupfs direto (root sem systemd) ou systemd do usuário.
pub fn prepare_command(p: Pedido, prefixo: &str) -> Result<Preparado, String> {
    let lim = &p.limites;
    // (started by systemd-run: the file on disk; started here: this very program, even if its file was replaced)
    let me = crate::sandbox::exe_on_disk()?;
    let root = nix::unistd::geteuid().is_root();
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    let unit = format!("{prefixo}-{}-{n}", std::process::id());
    let mut plano = Plano {
        landlock: p.landlock, seccomp: p.seccomp, ambiente: p.ambiente, programa: p.programa.into(), argumentos: p.argumentos.to_vec(), pasta: p.pasta, ..Default::default()
    };
    let (mut cmd, dir, modo): (Command, Option<PathBuf>, &'static str);
    if root && systemd_running() {
        prepare().map_err(|e| format!("cgroup dos agentes: {e}"))?;
        let mut c = Command::new("systemd-run");
        c.args(["--scope", "--quiet", "--collect", "--slice=kalango-agentes.slice"]).arg(format!("--unit={unit}"));
        if p.uid != 0 {
            c.arg(format!("--uid={}", p.uid)).arg(format!("--gid={}", p.gid));
        }
        props(&mut c, lim);
        c.arg("--").arg(&me);
        cmd = c;
        dir = Some(agents_dir().join(format!("{unit}.scope")));
        modo = "systemd";
    } else if root {
        prepare().map_err(|e| format!("cgroup dos agentes: {e}"))?;
        let d = agents_dir().join(&unit);
        std::fs::create_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))?;
        let w = |f: &str, v: String| std::fs::write(d.join(f), v).map_err(|e| format!("{f}: {e}"));
        w("memory.max", format!("{}", lim.memoria_mb * 1024 * 1024))?;
        let _ = w("memory.swap.max", "0".into());
        w("pids.max", lim.processos.to_string())?;
        w("cpu.max", format!("{} 100000", lim.cpu_pct * 1000))?;
        plano.cgroup = Some(d.clone());
        if p.uid != 0 {
            plano.uid = Some(p.uid);
            plano.gid = Some(p.gid);
        }
        cmd = Command::new(crate::sandbox::self_exe()?);
        dir = Some(d);
        modo = "cgroupfs";
    } else if crate::restore::which("systemd-run") {
        let mut c = Command::new("systemd-run");
        c.args(["--user", "--scope", "--quiet", "--collect", "--slice=kalango-agentes.slice"]).arg(format!("--unit={unit}"));
        // the person's user manager is on the user bus; a nested session (dbus-run-session: development, tests) has its
        // own session bus, where there is no systemd
        if let Some(bus) = std::env::var_os("XDG_RUNTIME_DIR").map(|d| PathBuf::from(d).join("bus")).filter(|b| b.exists()) {
            c.env("DBUS_SESSION_BUS_ADDRESS", format!("unix:path={}", bus.display()));
        }
        props(&mut c, lim);
        c.arg("--").arg(&me);
        cmd = c;
        // o escopo fica sob o gerenciador do usuário: …/user@UID.service/kalango.slice/kalango-agentes.slice/UNIT.scope
        dir = user_manager_cgroup().map(|m| m.join(AGENTES).join(format!("{unit}.scope")));
        modo = "systemd --user";
    } else {
        return Err(kalango_i18n::tr("no root and no systemd-run: there is no way to limit the task, so it does not run").into());
    }
    let descricao = format!("{} {}", plano.programa, plano.argumentos.join(" "));
    cmd.arg("__tarefa").arg(serde_json::to_string(&plano).unwrap());
    Ok(Preparado { cmd, dir, modo, descricao, unit })
}

/// Executa uma tarefa limitada e devolve o resultado (código, saída, memória máxima, se estourou memória ou tempo).
pub fn run_task(p: Pedido) -> Result<Value, String> {
    let lim = p.limites.clone();
    let dry_run = p.dry_run;
    let Preparado { mut cmd, dir, modo, descricao, unit } = prepare_command(p, "kalango-tarefa")?;
    if dry_run {
        return Ok(json!({"simulado": true, "comando": descricao, "modo": modo, "limites": lim}));
    }
    let lim = &lim;
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).env("LC_ALL", "C");
    // o escopo do systemd some assim que a tarefa morre; o contador do slice pai (hierárquico) fica
    let parent_events = dir.as_ref().and_then(|d| d.parent()).map(|p| p.join("memory.events"));
    let parent_oom_before = parent_events.as_ref().map(|p| events(p, "oom_kill")).unwrap_or(0);
    let start = Instant::now();
    let mut child = cmd.spawn().map_err(|e| format!("iniciar tarefa: {e}"))?;
    let out_t = reader(child.stdout.take().unwrap());
    let err_t = reader(child.stderr.take().unwrap());
    let deadline = start + Duration::from_secs(lim.tempo_s);
    let (mut peak, mut oom, mut timed_out) = (0u64, 0u64, false);
    let status = loop {
        if let Some(d) = &dir {
            peak = peak.max(read_u64(&d.join("memory.peak")).or_else(|| read_u64(&d.join("memory.current"))).unwrap_or(0));
            oom = oom.max(events(&d.join("memory.events"), "oom_kill"));
        }
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) => {}
            Err(e) => return Err(e.to_string()),
        }
        if Instant::now() > deadline && !timed_out {
            timed_out = true;
            match &dir {
                Some(d) if std::fs::write(d.join("cgroup.kill"), "1").is_ok() => {}
                _ => {
                    let _ = child.kill();
                }
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    if let Some(d) = &dir {
        oom = oom.max(events(&d.join("memory.events"), "oom_kill"));
    }
    // nothing the task started outlives it (a detached `setsid` loop could otherwise keep working after the command
    // returned — round 2 of the review raced a folder swap that way): its whole cgroup is killed, in every mode
    end_scope(dir.as_deref(), modo, &unit);
    let (saida, cortada_o) = out_t.join().unwrap_or_default();
    let (erros, cortada_e) = err_t.join().unwrap_or_default();
    let sinal = status.signal();
    if sinal == Some(9) && oom == 0 {
        if let Some(p) = &parent_events {
            oom = events(p, "oom_kill").saturating_sub(parent_oom_before);
        }
    }
    Ok(json!({
        "codigo": status.code(), "sinal": sinal, "saida": saida, "erros": erros, "saida_cortada": cortada_o || cortada_e,
        "tempo_ms": start.elapsed().as_millis() as u64, "memoria_pico_mb": peak / (1024 * 1024),
        "estourou_memoria": oom > 0, "estourou_tempo": timed_out, "limites": lim, "modo": modo,
    }))
}

/// Kills every process left in a task's cgroup (`cgroup.kill`), and stops its systemd scope when the cgroup's folder
/// is not known; a cgroupfs folder is removed.
fn end_scope(dir: Option<&Path>, modo: &str, unit: &str) {
    let killed = dir.is_some_and(|d| std::fs::write(d.join("cgroup.kill"), "1").is_ok());
    if !killed && modo != "cgroupfs" {
        let mut c = Command::new("systemctl");
        if modo == "systemd --user" {
            c.arg("--user");
            if let Some(bus) = std::env::var_os("XDG_RUNTIME_DIR").map(|d| PathBuf::from(d).join("bus")).filter(|b| b.exists()) {
                c.env("DBUS_SESSION_BUS_ADDRESS", format!("unix:path={}", bus.display()));
            }
        }
        let _ = c.args(["kill", "--signal=SIGKILL", &format!("{unit}.scope")]).stdout(Stdio::null()).stderr(Stdio::null()).status();
    }
    if let (Some(d), "cgroupfs") = (dir, modo) {
        for _ in 0..40 {
            if std::fs::remove_dir(d).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}

fn user_manager_cgroup() -> Option<PathBuf> {
    let cg = std::fs::read_to_string("/proc/self/cgroup").ok()?;
    let path = cg.lines().find_map(|l| l.strip_prefix("0::"))?;
    let i = path.find(".service")?;
    let base = &path[..i + ".service".len()];
    base.contains("/user@").then(|| Path::new(CGROOT).join(base.trim_start_matches('/')))
}

fn props(c: &mut Command, lim: &Limits) {
    c.arg(format!("--property=MemoryMax={}M", lim.memoria_mb))
        .arg("--property=MemorySwapMax=0")
        .arg(format!("--property=TasksMax={}", lim.processos))
        .arg(format!("--property=CPUQuota={}%", lim.cpu_pct))
        .arg(format!("--property=RuntimeMaxSec={}", lim.tempo_s + 5));
}

fn reader<R: Read + Send + 'static>(mut r: R) -> std::thread::JoinHandle<(String, bool)> {
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 8192];
        let mut cut = false;
        while let Ok(n) = r.read(&mut chunk) {
            if n == 0 {
                break;
            }
            if buf.len() < MAX_SAIDA {
                buf.extend_from_slice(&chunk[..n.min(MAX_SAIDA - buf.len())]);
            } else {
                cut = true;
            }
        }
        (String::from_utf8_lossy(&buf).into_owned(), cut)
    })
}

fn read_u64(p: &Path) -> Option<u64> {
    std::fs::read_to_string(p).ok()?.trim().parse().ok()
}

fn events(p: &Path, key: &str) -> u64 {
    std::fs::read_to_string(p).ok().and_then(|s| s.lines().find_map(|l| l.strip_prefix(key).and_then(|v| v.trim().parse().ok()))).unwrap_or(0)
}

/// Trampolim (`kalangod __tarefa PLANO`). Tudo acontece antes do exec: não há instante em que o programa rode fora
/// do cgroup, com mais privilégio ou sem o Landlock.
pub fn task_main(arg: &str) -> ! {
    let fail = |m: String| -> ! {
        eprintln!("kalangod __tarefa: {m}");
        std::process::exit(126)
    };
    let plano: Plano = serde_json::from_str(arg).unwrap_or_else(|e| fail(kalango_i18n::trf("invalid plan: {e}", &[("e", e.to_string())])));
    if let Some(d) = &plano.cgroup {
        if let Err(e) = std::fs::write(d.join("cgroup.procs"), "0") {
            fail(format!("entrar no cgroup: {e}"));
        }
    }
    if let (Some(uid), Some(gid)) = (plano.uid, plano.gid) {
        use nix::unistd::{setgid, setgroups, setuid, Gid, Uid};
        if let Err(e) = setgroups(&[]).and_then(|_| setgid(Gid::from_raw(gid))).and_then(|_| setuid(Uid::from_raw(uid))) {
            fail(kalango_i18n::trf("switch user: {e}", &[("e", e.to_string())]));
        }
    }
    if let Some(d) = &plano.pasta {
        if let Err(e) = std::env::set_current_dir(d) {
            fail(format!("{}: {e}", d.display()));
        }
    }
    if let Some(env) = &plano.ambiente {
        // nada do ambiente de quem chamou (tokens, chaves) passa adiante sem estar declarado
        for (k, _) in std::env::vars_os() {
            std::env::remove_var(k);
        }
        for (k, v) in env {
            std::env::set_var(k, v);
        }
    }
    if let Some(l) = &plano.landlock {
        if let Err(e) = landlock_task(l) {
            fail(e);
        }
    } else {
        // mesmo sem Landlock, o programa não ganha privilégio por setuid
        unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) };
    }
    if let Some(sc) = plano.seccomp {
        if let Err(e) = seccomp_task(sc) {
            fail(format!("seccomp: {e}"));
        }
    }
    let prog = std::ffi::CString::new(plano.programa.clone()).unwrap_or_else(|_| fail("programa inválido".into()));
    let mut argv = vec![prog.clone()];
    argv.extend(plano.argumentos.iter().map(|a| std::ffi::CString::new(a.as_str()).unwrap_or_default()));
    let e = nix::unistd::execvp(&prog, &argv).unwrap_err();
    fail(format!("{}: {e}", plano.programa))
}

fn landlock_task(l: &Landlock) -> Result<(), String> {
    use landlock::{Access, AccessFs, AccessNet, NetPort, PathBeneath, PathFd, Ruleset, RulesetAttr, RulesetCreatedAttr, RulesetStatus, ABI};
    let abi = ABI::V3;
    let mut rs = Ruleset::default().handle_access(AccessFs::from_all(abi)).map_err(|e| e.to_string())?;
    if l.portas.is_some() {
        rs = rs.handle_access(AccessNet::from_all(ABI::V4)).map_err(|e| e.to_string())?;
    }
    let mut rs = rs.create().map_err(|e| e.to_string())?;
    for (paths, access) in [(&l.listar, AccessFs::ReadDir.into()), (&l.leitura, AccessFs::from_read(abi)), (&l.escrita, AccessFs::from_all(abi))] {
        for p in paths {
            if let Ok(fd) = PathFd::new(p) {
                // regras em arquivos (ex.: /dev/null) só aceitam direitos de arquivo
                let a = if p.is_dir() { access } else { access & AccessFs::from_file(abi) };
                rs = rs.add_rule(PathBeneath::new(fd, a)).map_err(|e| e.to_string())?;
            }
        }
    }
    for port in l.portas.iter().flatten() {
        rs = rs.add_rule(NetPort::new(*port, AccessNet::ConnectTcp)).map_err(|e| e.to_string())?;
    }
    // restrict_self também liga no_new_privs
    let st = rs.restrict_self().map_err(|e| e.to_string())?;
    match st.ruleset {
        RulesetStatus::NotEnforced => Err("Landlock indisponível neste kernel: recusado".into()),
        // portas pedidas mas o kernel não restringe rede (Landlock < 4): não roda sem a proteção prometida
        RulesetStatus::PartiallyEnforced if l.portas.is_some() => Err(kalango_i18n::tr("this kernel does not restrict network by port (Landlock ABI 4): refused").into()),
        _ => Ok(()),
    }
}

/// Perfil dos conectores: o programa pode executar outros (node, python…), mas não depurar processos, montar,
/// carregar módulos ou eBPF, mexer em chaves do kernel, criar namespaces nem usar io_uring (que escaparia do filtro).
fn seccomp_task(p: Seccomp) -> Result<(), String> {
    use seccompiler::{apply_filter, BpfProgram, SeccompAction, SeccompCmpArgLen, SeccompCmpOp, SeccompCondition, SeccompFilter, SeccompRule, TargetArch};
    let deny = [
        libc::SYS_ptrace, libc::SYS_mount, libc::SYS_umount2, libc::SYS_pivot_root, libc::SYS_bpf, libc::SYS_kexec_load, libc::SYS_kexec_file_load,
        libc::SYS_init_module, libc::SYS_finit_module, libc::SYS_delete_module, libc::SYS_reboot, libc::SYS_swapon, libc::SYS_swapoff, libc::SYS_setns,
        libc::SYS_unshare, libc::SYS_process_vm_readv, libc::SYS_process_vm_writev, libc::SYS_perf_event_open, libc::SYS_keyctl, libc::SYS_add_key,
        libc::SYS_request_key, libc::SYS_chroot, libc::SYS_io_uring_setup, libc::SYS_userfaultfd, libc::SYS_open_by_handle_at, libc::SYS_name_to_handle_at,
    ];
    let mut rules: std::collections::BTreeMap<i64, Vec<SeccompRule>> = deny.iter().map(|s| (*s, vec![])).collect();
    let fam = |f: i32| SeccompRule::new(vec![SeccompCondition::new(0, SeccompCmpArgLen::Dword, SeccompCmpOp::Eq, f as u64).unwrap()]).unwrap();
    let mut socket_rules = Vec::new();
    if p.sem_rede {
        socket_rules.extend([fam(libc::AF_INET), fam(libc::AF_INET6), fam(libc::AF_PACKET)]);
    }
    if p.sem_unix {
        socket_rules.push(fam(libc::AF_UNIX));
    }
    if !socket_rules.is_empty() {
        rules.insert(libc::SYS_socket, socket_rules);
    }
    let filter = SeccompFilter::new(rules, SeccompAction::Allow, SeccompAction::Errno(libc::EPERM as u32), TargetArch::x86_64).map_err(|e| e.to_string())?;
    let prog: BpfProgram = filter.try_into().map_err(|e: seccompiler::BackendError| e.to_string())?;
    apply_filter(&prog).map_err(|e| e.to_string())
}

/// Filtro seccomp do processo auxiliar de arquivos: nega chamadas que nenhuma operação de arquivo precisa.
pub fn seccomp_helper() -> Result<(), String> {
    use seccompiler::{apply_filter, BpfProgram, SeccompAction, SeccompFilter, TargetArch};
    let deny = [
        libc::SYS_ptrace, libc::SYS_mount, libc::SYS_umount2, libc::SYS_pivot_root, libc::SYS_bpf, libc::SYS_kexec_load, libc::SYS_kexec_file_load,
        libc::SYS_init_module, libc::SYS_finit_module, libc::SYS_delete_module, libc::SYS_reboot, libc::SYS_swapon, libc::SYS_swapoff, libc::SYS_setns,
        libc::SYS_unshare, libc::SYS_process_vm_readv, libc::SYS_process_vm_writev, libc::SYS_perf_event_open, libc::SYS_keyctl, libc::SYS_add_key,
        libc::SYS_request_key, libc::SYS_execve, libc::SYS_execveat, libc::SYS_socket, libc::SYS_connect, libc::SYS_setuid, libc::SYS_setgid,
        libc::SYS_setreuid, libc::SYS_setresuid, libc::SYS_chroot, libc::SYS_io_uring_setup,
    ];
    let rules = deny.iter().map(|s| (*s, vec![])).collect();
    let filter = SeccompFilter::new(rules, SeccompAction::Allow, SeccompAction::Errno(libc::EPERM as u32), TargetArch::x86_64).map_err(|e| e.to_string())?;
    let prog: BpfProgram = filter.try_into().map_err(|e: seccompiler::BackendError| e.to_string())?;
    apply_filter(&prog).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn seccomp_do_auxiliar_bloqueia_exec_e_rede() {
        // o filtro vale só para a thread que o aplica (sem TSYNC): testa numa thread separada
        let r = std::thread::spawn(|| {
            unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) };
            super::seccomp_helper().unwrap();
            let sock = unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0) };
            let sock_err = std::io::Error::last_os_error().raw_os_error();
            let exec = std::process::Command::new("/bin/true").status();
            let file = std::fs::write(std::env::temp_dir().join(format!("kalango-seccomp-{}", std::process::id())), "ok");
            (sock, sock_err, exec.is_err(), file.is_ok())
        }).join().unwrap();
        let _ = std::fs::remove_file(std::env::temp_dir().join(format!("kalango-seccomp-{}", std::process::id())));
        assert_eq!(r.0, -1);
        assert_eq!(r.1, Some(libc::EPERM));
        assert!(r.2, "exec devia falhar");
        assert!(r.3, "operações de arquivo continuam funcionando");
    }
}
