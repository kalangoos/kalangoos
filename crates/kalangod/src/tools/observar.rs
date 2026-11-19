//! Nível observar: estado do sistema, sem conteúdo de arquivos pessoais.

use super::*;
use std::path::Path;

pub fn tools() -> Vec<Tool> {
    vec![
        Tool { name: "sistema.info", description: n_("System, kernel, uptime, load and memory"), level: Level::Observar, effect: Effect::Leitura, reversible: false, schema: none, run: info },
        Tool { name: "relogio.agora", description: n_("The date and time now on this computer: the day, the weekday, the hour, the time zone and whether the clock is synchronised"), level: Level::Observar, effect: Effect::Leitura, reversible: false, schema: none, run: clock_now },
        Tool { name: "kernel.seguranca", description: n_("Kernel security features KalangoOS uses: active LSMs, Landlock, eBPF, BTF, cgroups v2"), level: Level::Observar, effect: Effect::Leitura, reversible: false, schema: none, run: kernel_security },
        Tool { name: "servicos.listar", description: n_("systemd services and their states"), level: Level::Observar, effect: Effect::Leitura, reversible: false, schema: none, run: services_list },
        Tool { name: "servicos.estado", description: n_("Detailed state of a service"), level: Level::Observar, effect: Effect::Leitura, reversible: false,
            schema: || obj(json!({"nome": {"type": "string", "description": "ex.: NetworkManager.service"}}), &["nome"]), run: service_status },
        Tool { name: "discos.uso", description: n_("Space used by the mounted file systems"), level: Level::Observar, effect: Effect::Leitura, reversible: false, schema: none, run: disks },
        Tool { name: "rede.interfaces", description: n_("Network interfaces, state and addresses"), level: Level::Observar, effect: Effect::Leitura, reversible: false, schema: none, run: net },
        Tool { name: "processos.listar", description: n_("Processes using the most memory or CPU"), level: Level::Observar, effect: Effect::Leitura, reversible: false,
            schema: || obj(json!({"ordenar": {"type": "string", "enum": ["memoria", "cpu"]}, "limite": {"type": "integer", "minimum": 1, "maximum": 200}}), &[]), run: procs },
        Tool { name: "registros.ler", description: n_("Last lines of the system journal"), level: Level::Observar, effect: Effect::Leitura, reversible: false,
            schema: || obj(json!({"unidade": {"type": "string"}, "linhas": {"type": "integer", "minimum": 1, "maximum": 500}, "prioridade": {"type": "string", "enum": ["erro", "aviso", "info"]}}), &[]), run: journal },
        Tool { name: "pacotes.listar", description: n_("Installed packages (apt), with an optional filter"), level: Level::Observar, effect: Effect::Leitura, reversible: false,
            schema: || obj(json!({"busca": {"type": "string"}}), &[]), run: packages },
        Tool { name: "sysctl.ler", description: n_("Reads a kernel parameter"), level: Level::Observar, effect: Effect::Leitura, reversible: false,
            schema: || obj(json!({"chave": {"type": "string", "description": "ex.: vm.swappiness"}}), &["chave"]), run: sysctl_get },
        Tool { name: "linha_do_tempo.listar", description: n_("Actions recorded by kalangod, newest first"), level: Level::Observar, effect: Effect::Leitura, reversible: false,
            schema: || obj(json!({"limite": {"type": "integer", "minimum": 1, "maximum": 500}}), &[]), run: timeline_list },
    ]
}

fn read(p: &str) -> String {
    std::fs::read_to_string(p).unwrap_or_default()
}

fn info(_: &Ctx, _: &Value) -> Result<Out, String> {
    let os = read("/etc/os-release").lines().find_map(|l| l.strip_prefix("PRETTY_NAME=").map(|v| v.trim_matches('"').to_string())).unwrap_or_default();
    let mem: std::collections::HashMap<String, u64> = read("/proc/meminfo")
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            Some((it.next()?.trim_end_matches(':').to_string(), it.next()?.parse().ok()?))
        })
        .collect();
    let up = read("/proc/uptime").split_whitespace().next().and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
    let v = json!({
        "sistema": os,
        "kernel": read("/proc/sys/kernel/osrelease").trim(),
        "maquina": read("/proc/sys/kernel/hostname").trim(),
        "ligado_ha_min": (up / 60.0).round(),
        "carga": read("/proc/loadavg").split_whitespace().take(3).collect::<Vec<_>>(),
        "memoria_mb": { "total": mem.get("MemTotal").copied().unwrap_or(0) / 1024, "disponivel": mem.get("MemAvailable").copied().unwrap_or(0) / 1024 },
        "nucleos": std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1),
    });
    Ok(Out::read(v, tr("system information")))
}

/// The local time now: (year, month, day, hour, minute, weekday 1 = Monday … 7 = Sunday, offset from UTC in minutes).
pub fn local_now() -> (i32, u32, u32, u32, u32, u32, i64) {
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        let wd = if tm.tm_wday == 0 { 7 } else { tm.tm_wday as u32 };
        (tm.tm_year + 1900, (tm.tm_mon + 1) as u32, tm.tm_mday as u32, tm.tm_hour as u32, tm.tm_min as u32, wd, tm.tm_gmtoff / 60)
    }
}

fn clock_now(_: &Ctx, _: &Value) -> Result<Out, String> {
    let (y, mo, d, h, mi, wd, off) = local_now();
    const DIAS: [&str; 7] = [n_("Monday"), n_("Tuesday"), n_("Wednesday"), n_("Thursday"), n_("Friday"), n_("Saturday"), n_("Sunday")];
    // the zone's name: /etc/timezone, or where /etc/localtime points
    let fuso = Some(read("/etc/timezone").trim().to_string()).filter(|z| !z.is_empty()).or_else(|| {
        std::fs::read_link("/etc/localtime").ok().map(|p| p.display().to_string()).and_then(|p| p.split("zoneinfo/").nth(1).map(str::to_string))
    }).unwrap_or_default();
    let sinal = if off < 0 { '-' } else { '+' };
    let utc = format!("{sinal}{:02}:{:02}", off.abs() / 60, off.abs() % 60);
    let sincronizado = exec("timedatectl", &["show", "-p", "NTPSynchronized", "--value"]).ok().map(|v| v.trim() == "yes");
    let v = json!({
        "data": format!("{y:04}-{mo:02}-{d:02}"), "hora": format!("{h:02}:{mi:02}"), "dia_da_semana": tr(DIAS[(wd - 1) as usize]),
        "fuso": fuso, "utc": utc, "sincronizado": sincronizado,
    });
    Ok(Out::read(v, trf("{date} {time} ({weekday})", &[("date", format!("{d:02}/{mo:02}/{y}")), ("time", format!("{h:02}:{mi:02}")), ("weekday", tr(DIAS[(wd - 1) as usize]).to_string())])))
}

/// Versão da ABI do Landlock (0 = indisponível).
pub fn landlock_abi() -> i64 {
    const LANDLOCK_CREATE_RULESET_VERSION: u32 = 1;
    unsafe { libc::syscall(libc::SYS_landlock_create_ruleset, std::ptr::null::<u8>(), 0usize, LANDLOCK_CREATE_RULESET_VERSION) }.max(0)
}

fn kernel_security(_: &Ctx, _: &Value) -> Result<Out, String> {
    let lsm = read("/sys/kernel/security/lsm");
    let lsms: Vec<&str> = lsm.trim().split(',').filter(|s| !s.is_empty()).collect();
    let cfg = format!("/boot/config-{}", read("/proc/sys/kernel/osrelease").trim());
    let cfg_has = |o: &str| read(&cfg).lines().any(|l| l == format!("CONFIG_{o}=y"));
    let v = json!({
        "kernel": read("/proc/sys/kernel/osrelease").trim(),
        "lsm_ativos": lsms,
        "landlock_abi": landlock_abi(),
        "bpf_lsm_compilado": cfg_has("BPF_LSM"),
        "bpf_lsm_ativo": lsms.contains(&"bpf"),
        "btf": Path::new("/sys/kernel/btf/vmlinux").exists(),
        "cgroups_v2": Path::new("/sys/fs/cgroup/cgroup.controllers").exists(),
        "fanotify_permissao": cfg_has("FANOTIFY_ACCESS_PERMISSIONS"),
        "sistema_de_arquivos_raiz": crate::restore::root_fs(),
        "guarda_ebpf": crate::guard::status(),
        "guarda_ebpf_ativa": crate::guard::ATIVA.load(std::sync::atomic::Ordering::SeqCst),
        "pronto_para_kalango": landlock_abi() >= 3 && Path::new("/sys/kernel/btf/vmlinux").exists() && lsms.contains(&"bpf"),
    });
    let ok = v["pronto_para_kalango"].as_bool().unwrap_or(false);
    Ok(Out::read(v, if ok { "kernel pronto para o KalangoOS" } else { "kernel sem todos os recursos (ver campos)" }))
}

fn services_list(_: &Ctx, _: &Value) -> Result<Out, String> {
    let raw = exec("systemctl", &["list-units", "--type=service", "--all", "--no-pager", "--output=json"])?;
    let v: Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    let list: Vec<Value> = v.as_array().cloned().unwrap_or_default().into_iter()
        .map(|u| json!({ "nome": u["unit"], "ativo": u["active"], "sub": u["sub"], "descricao": u["description"] })).collect();
    let n = list.len();
    Ok(Out::read(json!({ "servicos": list }), trf("{n} services", &[("n", n.to_string())])))
}

fn service_status(_: &Ctx, a: &Value) -> Result<Out, String> {
    let name = safe_name(s(a, "nome")?)?;
    let raw = exec("systemctl", &["show", name, "--no-pager", "-p", "Id,Description,ActiveState,SubState,UnitFileState,MainPID,MemoryCurrent,ActiveEnterTimestamp"])?;
    let map: serde_json::Map<String, Value> = raw.lines().filter_map(|l| l.split_once('=')).map(|(k, v)| (k.to_string(), Value::String(v.to_string()))).collect();
    Ok(Out::read(Value::Object(map), trf("state of {name}", &[("name", name.to_string())])))
}

/// A mount that is a disk the person can fill: not a kernel or memory file system, not an installation medium (a
/// DVD, the live image's ISO) and not mounted read-only — a read-only one is "100% full" by nature (the live session's
/// /cdrom said so in its summary).
fn fills_up(fstype: &str, options: &str) -> bool {
    const SKIP: &[&str] = &["proc", "sysfs", "tmpfs", "devtmpfs", "cgroup2", "securityfs", "pstore", "bpf", "debugfs", "tracefs", "configfs", "fusectl", "mqueue",
        "hugetlbfs", "autofs", "binfmt_misc", "squashfs", "nsfs", "efivarfs", "devpts", "ramfs", "overlay", "fuse.portal", "fuse.gvfsd-fuse", "iso9660", "udf"];
    !SKIP.contains(&fstype) && !options.split(',').any(|o| o == "ro")
}

fn disks(_: &Ctx, _: &Value) -> Result<Out, String> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for l in read("/proc/mounts").lines() {
        let f: Vec<&str> = l.split_whitespace().collect();
        if f.len() < 4 || !fills_up(f[2], f[3]) || !seen.insert(f[0].to_string()) {
            continue;
        }
        if let Ok(st) = nix::sys::statvfs::statvfs(f[1]) {
            let bs = st.fragment_size();
            let total = st.blocks() * bs;
            if total == 0 {
                continue;
            }
            let free = st.blocks_available() * bs;
            out.push(json!({ "dispositivo": f[0], "montado_em": f[1], "tipo": f[2], "total_gb": total / 1_000_000_000, "livre_gb": free / 1_000_000_000, "uso_pct": 100 - free * 100 / total }));
        }
    }
    Ok(Out::read(json!({ "discos": out }), tr("disk usage")))
}

fn net(_: &Ctx, _: &Value) -> Result<Out, String> {
    let addrs: Value = exec("ip", &["-j", "addr"]).ok().and_then(|r| serde_json::from_str(&r).ok()).unwrap_or(json!([]));
    let list: Vec<Value> = addrs.as_array().cloned().unwrap_or_default().into_iter().map(|i| json!({
        "nome": i["ifname"], "estado": i["operstate"], "mac": i["address"],
        "enderecos": i["addr_info"].as_array().map(|a| a.iter().map(|x| format!("{}/{}", x["local"].as_str().unwrap_or(""), x["prefixlen"])).collect::<Vec<_>>()).unwrap_or_default(),
    })).collect();
    Ok(Out::read(json!({ "interfaces": list }), tr("network interfaces")))
}

fn procs(_: &Ctx, a: &Value) -> Result<Out, String> {
    let by_cpu = a.get("ordenar").and_then(Value::as_str) == Some("cpu");
    let limit = a.get("limite").and_then(Value::as_u64).unwrap_or(15) as usize;
    let page = 4096u64;
    let mut list = Vec::new();
    for e in std::fs::read_dir("/proc").map_err(|e| e.to_string())?.flatten() {
        let Ok(pid) = e.file_name().to_string_lossy().parse::<i32>() else { continue };
        let stat = read(&format!("/proc/{pid}/stat"));
        let Some((head, rest)) = stat.rsplit_once(')') else { continue };
        let name = head.split_once('(').map(|x| x.1).unwrap_or("").to_string();
        let f: Vec<&str> = rest.split_whitespace().collect();
        if f.len() < 22 {
            continue;
        }
        let cpu_ticks: u64 = f[11].parse::<u64>().unwrap_or(0) + f[12].parse::<u64>().unwrap_or(0);
        let rss = f[21].parse::<u64>().unwrap_or(0) * page;
        list.push((pid, name, cpu_ticks, rss));
    }
    list.sort_by_key(|x| std::cmp::Reverse(if by_cpu { x.2 } else { x.3 }));
    let v: Vec<Value> = list.into_iter().take(limit).map(|(pid, n, c, r)| json!({ "pid": pid, "nome": n, "cpu_s": c / 100, "memoria_mb": r / 1_048_576 })).collect();
    Ok(Out::read(json!({ "processos": v }), tr("processes")))
}

fn journal(_: &Ctx, a: &Value) -> Result<Out, String> {
    let n = a.get("linhas").and_then(Value::as_u64).unwrap_or(50).to_string();
    let mut args = vec!["--no-pager", "-o", "short-iso", "-n", n.as_str()];
    let unit;
    if let Some(u) = a.get("unidade").and_then(Value::as_str) {
        unit = safe_name(u)?.to_string();
        args.extend(["-u", unit.as_str()]);
    }
    match a.get("prioridade").and_then(Value::as_str) {
        Some("erro") => args.extend(["-p", "err"]),
        Some("aviso") => args.extend(["-p", "warning"]),
        _ => {}
    }
    let raw = exec("journalctl", &args)?;
    Ok(Out::read(json!({ "linhas": raw.lines().collect::<Vec<_>>() }), tr("system logs")))
}

fn packages(_: &Ctx, a: &Value) -> Result<Out, String> {
    let q = a.get("busca").and_then(Value::as_str).unwrap_or("").to_lowercase();
    let raw = exec("dpkg-query", &["-W", "-f", "${Package}\t${Version}\t${db:Status-Abbrev}\n"])?;
    let list: Vec<Value> = raw.lines().filter_map(|l| {
        let f: Vec<&str> = l.split('\t').collect();
        (f.len() == 3 && f[2].starts_with("ii") && f[0].contains(&q)).then(|| json!({ "nome": f[0], "versao": f[1] }))
    }).take(500).collect();
    let n = list.len();
    Ok(Out::read(json!({ "pacotes": list }), trf("{n} packages", &[("n", n.to_string())])))
}

pub fn sysctl_path(key: &str) -> Result<String, String> {
    let k = safe_name(key)?;
    if k.contains("..") || k.contains('/') {
        return Err(kalango_i18n::trf("invalid key: {k}", &[("k", k.to_string())]));
    }
    Ok(format!("/proc/sys/{}", k.replace('.', "/")))
}

fn sysctl_get(_: &Ctx, a: &Value) -> Result<Out, String> {
    let key = s(a, "chave")?;
    let v = std::fs::read_to_string(sysctl_path(key)?).map_err(|e| format!("{key}: {e}"))?;
    Ok(Out::read(json!({ "chave": key, "valor": v.trim() }), format!("{key} = {}", v.trim())))
}

fn timeline_list(c: &Ctx, a: &Value) -> Result<Out, String> {
    let limit = a.get("limite").and_then(Value::as_u64).unwrap_or(50) as usize;
    let mut all = c.timeline.all();
    // cada usuário vê só o que é dele; root vê tudo
    if c.peer.uid != 0 {
        all.retain(|e| e.uid == c.peer.uid);
    }
    all.reverse();
    all.truncate(limit);
    let n = all.len();
    // structuredContent do MCP é sempre um objeto
    Ok(Out::read(json!({"registros": all}), trf("{n} records", &[("n", n.to_string())])))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_disks_that_fill_up_count() {
        // the system's disk and a data disk count
        assert!(fills_up("ext4", "rw,relatime"));
        assert!(fills_up("vfat", "rw,relatime,fmask=0077"));
        // the live session's ISO, its image and its memory layer never do
        assert!(!fills_up("iso9660", "ro,noatime,nojoliet"));
        assert!(!fills_up("squashfs", "ro,noatime"));
        assert!(!fills_up("overlay", "rw,noatime,lowerdir=/x"));
        // a disk mounted read-only is full by nature
        assert!(!fills_up("ext4", "ro,relatime"));
        // ("errors=remount-ro" is not read-only)
        assert!(fills_up("ext4", "rw,errors=remount-ro"));
    }
}
