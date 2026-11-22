//! Máquinas virtuais do Kalango VirtualDesk como ferramentas `vm.*`.
//!
//! O trabalho é do próprio VirtualDesk: o kalangod roda `kalango-virtualdesk ferramenta NOME JSON` como a pessoa que
//! chamou (o mesmo uid e a casa dela, onde ficam as máquinas) e devolve o que ele responde. Aqui ficam o contrato
//! (nível, efeito, reversibilidade), a checagem dos caminhos de arquivo do computador (pastas permitidas e
//! protegidas, como em `arquivos.*`) e a pergunta "precisa aprovar?" — que segue o que a pessoa escolheu em
//! VirtualDesk › Ajustes › O que os agentes podem fazer (`kalango-virtualdesk ferramenta --aprovacao`). O desligar e o
//! apagar pedem sempre. Tudo entra na linha do tempo pelo fluxo comum de `server::call`, com o inverso que o
//! VirtualDesk devolve (Desfazer).

use super::{n_, obj, s, tr, trf, Ctx, Out, Tool};
use crate::peer::Peer;
use crate::policy::{Effect, Level};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::Command;

pub fn tools() -> Vec<Tool> {
    use Effect::*;
    use Level::*;
    let t = |name, description, level, effect, reversible, schema: fn() -> Value, run| Tool { name, description, level, effect, reversible, schema, run };
    vec![
        t("vm.listar", n_("Virtual machines of Kalango VirtualDesk and the state of each (on, paused, off), with processors, memory, folders and snapshots"), Observar, Leitura, false, none, listar),
        t("vm.captura_de_tela", n_("A picture (PNG) of the screen of a running virtual machine"), Usuario, Leitura, false, id_only, captura),
        t("vm.ajustes", n_("Reads or changes a virtual machine's processors, memory, network, sound, clipboard or CD; says what applies now and what at the next start"), Usuario, Escreve, true, ajustes_schema, ajustes),
        t("vm.ligar", n_("Turns a virtual machine on (or resumes its saved state)"), Usuario, Escreve, true, id_only, ligar),
        t("vm.pausar", n_("Pauses a running virtual machine"), Usuario, Escreve, true, id_only, pausar),
        t("vm.continuar", n_("Continues a paused virtual machine"), Usuario, Escreve, true, id_only, continuar),
        t("vm.snapshot", n_("Snapshots of a virtual machine: list, take, go back (the current state is kept first) or delete"), Usuario, Escreve, true,
            || obj(json!({"id": id_prop(), "acao": {"type": "string", "enum": ["listar", "tirar", "voltar", "apagar"]}, "nome": {"type": "string"}, "tag": {"type": "string"}}), &["id", "acao"]), snapshot),
        t("vm.criar", n_("Creates a virtual machine, optionally starting from an installation disc (ISO) of yours"), Usuario, Escreve, true,
            || obj(json!({"nome": {"type": "string"}, "iso": {"type": "string", "description": "caminho de um .iso seu"}, "sistema": {"type": "string"}, "cpus": {"type": "integer", "minimum": 1}, "memoria_mb": {"type": "integer", "minimum": 256}, "disco_gb": {"type": "integer", "minimum": 1}}), &["nome"]), criar),
        t("vm.clonar", n_("Makes a disposable copy of a template machine for an agent's sandbox, already running from a snapshot (no folder of yours goes along; the internet only, not your home network, unless rede says otherwise)"), Usuario, Escreve, false,
            || obj(json!({"id": id_prop(), "nome": {"type": "string"}, "snapshot": {"type": "string"}, "rede": rede_prop()}), &["id"]), clonar),
        t("vm.pastas", n_("Shares a folder of yours with a virtual machine (read only unless asked), stops sharing, or lists"), Usuario, Escreve, true,
            || obj(json!({"id": id_prop(), "acao": {"type": "string", "enum": ["listar", "compartilhar", "parar"]}, "caminho": {"type": "string"}, "etiqueta": {"type": "string"}, "somente_leitura": {"type": "boolean"}}), &["id", "acao"]), pastas),
        t("vm.copiar_arquivo", n_("Copies a file of yours into a running virtual machine, or out of it into your folders (up to 64 MB)"), Usuario, Escreve, false,
            || obj(json!({"id": id_prop(), "de": {"type": "string"}, "para": {"type": "string"}, "sentido": {"type": "string", "enum": ["para_maquina", "da_maquina"]}}), &["id", "de", "para"]), copiar),
        t("vm.executar", n_("Runs a shell command inside a running virtual machine through its guest agent, and returns the code and the output"), Usuario, Destrutiva, false,
            || obj(json!({"id": id_prop(), "comando": {"type": "string"}, "tempo_s": {"type": "integer", "minimum": 1, "maximum": 3600}}), &["id", "comando"]), executar),
        t("vm.desligar", n_("Turns a virtual machine off (asks its system; at once with forcar)"), Usuario, Destrutiva, true,
            || obj(json!({"id": id_prop(), "forcar": {"type": "boolean"}, "espera_s": {"type": "integer", "minimum": 1, "maximum": 300}}), &["id"]), desligar),
        t("vm.apagar", n_("Deletes a turned-off virtual machine and its disks"), Usuario, Destrutiva, false, id_only, apagar),
    ]
}

fn none() -> Value {
    super::none()
}

fn id_prop() -> Value {
    json!({"type": "string", "description": "id ou nome da máquina"})
}

/// A machine's network (VirtualDesk's three modes; "casa" is the same as "nat").
fn rede_prop() -> Value {
    json!({"type": "string", "enum": ["internet", "nenhuma", "nat", "casa"],
        "description": "internet: só a internet, sem a rede de casa nem este computador; nenhuma: sem rede; nat ou casa: a internet e a rede de casa"})
}

fn id_only() -> Value {
    obj(json!({"id": id_prop()}), &["id"])
}

fn ajustes_schema() -> Value {
    obj(
        json!({"id": id_prop(), "cpus": {"type": "integer", "minimum": 1}, "memoria_mb": {"type": "integer", "minimum": 256}, "aninhada": {"type": "boolean"},
            "area_de_transferencia": {"type": "boolean"}, "som": {"type": "boolean"}, "microfone": {"type": "boolean"}, "tela_cheia_ao_abrir": {"type": "boolean"},
            "rede": rede_prop(), "cd": {"type": "string", "description": "caminho de um .iso seu; vazio ejeta"}, "novo_nome": {"type": "string"}}),
        &["id"],
    )
}

/// The VirtualDesk program: `$KALANGO_VIRTUALDESK`, the system's, the person's ~/.local/bin, or the PATH.
pub fn program(peer: &Peer) -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("KALANGO_VIRTUALDESK").map(PathBuf::from).filter(|p| p.is_file()) {
        return Some(p);
    }
    let fixed = [PathBuf::from("/usr/bin/kalango-virtualdesk"), PathBuf::from("/usr/local/bin/kalango-virtualdesk"), peer.passwd_home.join(".local/bin/kalango-virtualdesk")];
    if let Some(p) = fixed.into_iter().find(|p| p.is_file()) {
        return Some(p);
    }
    std::env::var_os("PATH").and_then(|path| std::env::split_paths(&path).map(|d| d.join("kalango-virtualdesk")).find(|p| p.is_file()))
}

/// This kalangod's own socket (set when it starts serving).
pub static OWN_SOCKET: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

/// Whether this kalangod is a test session (its config names a test `home`): only then do VirtualDesk's test
/// variables reach what it runs.
pub static TEST_SESSION: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// VirtualDesk's variables for tests: another home in place of the one in passwd, another library. Inherited by a
/// development kalangod from whoever started it, they would move the agents' rules and machines somewhere an agent
/// may write — so they are removed from what kalangod runs unless it is itself a test session.
const VIRTUALDESK_TEST_ENV: &[&str] = &["KALANGO_VIRTUALDESK_TEST_HOME", "KALANGO_VIRTUALDESK_DIR"];

fn strip_test_env(cmd: &mut Command, test_session: bool) {
    if !test_session {
        for k in VIRTUALDESK_TEST_ENV {
            cmd.env_remove(k);
        }
    }
}

/// The proofs of the calls kalangod is making to VirtualDesk now: proof → (uid, tool). Each is answered once, for that
/// person and that tool, and dropped when the call ends.
static PROOFS: std::sync::Mutex<Vec<(String, u32, String)>> = std::sync::Mutex::new(Vec::new());

fn new_proof(uid: u32, tool: &str) -> String {
    let mut b = [0u8; 16];
    let _ = std::fs::File::open("/dev/urandom").and_then(|mut f| std::io::Read::read_exact(&mut f, &mut b));
    let p: String = b.iter().map(|x| format!("{x:02x}")).collect();
    PROOFS.lock().unwrap_or_else(|e| e.into_inner()).push((p.clone(), uid, tool.to_string()));
    p
}

fn forget_proof(p: &str) {
    PROOFS.lock().unwrap_or_else(|e| e.into_inner()).retain(|x| x.0 != p);
}

/// VirtualDesk asks: did kalangod start this call (proof), for this person (the asking process's uid, by the kernel),
/// of this tool? Yes once; then the proof is gone.
pub fn redeem_proof(proof: &str, uid: u32, tool: &str) -> bool {
    let mut l = PROOFS.lock().unwrap_or_else(|e| e.into_inner());
    match l.iter().position(|x| x.0 == proof && x.1 == uid && x.2 == tool) {
        Some(i) => {
            l.remove(i);
            true
        }
        None => false,
    }
}

/// Runs VirtualDesk as the caller and reads its one JSON line.
fn virtualdesk(peer: &Peer, args: &[&str]) -> Result<Value, String> {
    virtualdesk_on(peer, args, None)
}

/// As `virtualdesk`, saying which network kept off the agents this computer is on (`KALANGO_REDE_SEM_AGENTES`):
/// VirtualDesk then refuses, in the one function every start goes through, to start a machine with network for an
/// agent — whatever the call (turning on, a snapshot with memory, a command that starts it).
fn virtualdesk_on(peer: &Peer, args: &[&str], blocked: Option<&str>) -> Result<Value, String> {
    let exe = program(peer).ok_or_else(|| tr("Kalango VirtualDesk is not installed").to_string())?;
    let mut cmd = Command::new(exe);
    cmd.args(args).env("KALANGO_CHAMADOR", &peer.exe);
    if nix::unistd::geteuid().is_root() && peer.uid != 0 {
        use std::os::unix::process::CommandExt;
        // as the person, with only what VirtualDesk needs from the environment: their home (the machines) and their
        // runtime folder (the machines' sockets)
        cmd.uid(peer.uid).gid(peer.gid).env_clear().env("HOME", &peer.passwd_home).env("USER", &peer.user)
            .env("XDG_RUNTIME_DIR", format!("/run/user/{}", peer.uid)).env("PATH", "/usr/local/bin:/usr/bin:/bin").env("LANG", std::env::var("LANG").unwrap_or_else(|_| "C.UTF-8".into()))
            .env("KALANGO_CHAMADOR", &peer.exe)
            .current_dir(&peer.passwd_home);
    }
    strip_test_env(&mut cmd, TEST_SESSION.get().copied().unwrap_or(false));
    cmd.env("KALANGO_REDE_SEM_AGENTES", blocked.unwrap_or(""));
    // where VirtualDesk asks about this call (a development kalangod's socket; the system's is found by itself)
    if let Some(s) = OWN_SOCKET.get() {
        cmd.env("KALANGOD_SOCKET", s);
    }
    // the one-time proof that this call is kalangod's: on VirtualDesk's standard input, which only it reads
    let tool = args.get(1).copied().unwrap_or("").to_string();
    let proof = new_proof(peer.uid, &tool);
    cmd.stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        let _ = writeln!(stdin, "prova {proof}");
    }
    let o = child.wait_with_output().map_err(|e| e.to_string());
    forget_proof(&proof);
    let o = o?;
    let line = String::from_utf8_lossy(&o.stdout);
    let v: Value = serde_json::from_str(line.trim()).map_err(|_| trf("Kalango VirtualDesk did not answer: {e}", &[("e", String::from_utf8_lossy(&o.stderr).trim().to_string())]))?;
    match v.get("erro").and_then(Value::as_str) {
        Some(e) => Err(e.to_string()),
        None => Ok(v),
    }
}

/// Whether this call asks the person first (the choices in VirtualDesk's settings; turning off and deleting always).
pub fn needs_approval(peer: &Peer, name: &str, args: &Value) -> Result<Option<String>, String> {
    let v = virtualdesk(peer, &["ferramenta", "--aprovacao", name, &args.to_string()])?;
    // turning off or deleting asks, except a disposable sandbox copy (VirtualDesk knows which machines are those: it
    // made them with vm.clonar, and nothing of the person's is in them)
    if matches!(name, "vm.desligar" | "vm.apagar") && v["pedir"].as_bool() != Some(false) {
        return Ok(Some(tr("turning off or deleting a machine always asks").into()));
    }
    Ok(v["pedir"].as_bool().unwrap_or(true).then(|| v["motivo"].as_str().unwrap_or("").to_string()))
}

/// A path of this computer in the arguments: absolute, inside the allowed folders and outside the protected ones.
fn path_arg(c: &Ctx, a: &mut Value, k: &str) -> Result<(), String> {
    checked_arg(c, a, k, crate::sandbox::check_path)
}

/// As `path_arg`, for a path the machine may change (a folder shared into it, a file copied out of it): also not a
/// folder that holds a protected one — ~/.config shared into a machine would let what runs there write
/// ~/.config/kalango.
fn change_arg(c: &Ctx, a: &mut Value, k: &str) -> Result<(), String> {
    checked_arg(c, a, k, crate::sandbox::check_change_path)
}

fn checked_arg(c: &Ctx, a: &mut Value, k: &str, check: fn(&str, &Peer, &[String], &[String]) -> Result<PathBuf, String>) -> Result<(), String> {
    if let Some(raw) = a.get(k).and_then(Value::as_str).filter(|r| !r.is_empty()) {
        let p = check(raw, c.peer, &c.rule.allowed_paths, &c.cfg.protected_paths)?;
        a[k] = json!(p);
    }
    Ok(())
}

fn call(c: &Ctx, name: &str, a: &Value) -> Result<Out, String> {
    let blocked = crate::redes::bloqueada(&c.cfg.state_dir, c.peer.uid);
    let v = virtualdesk_on(c.peer, &["ferramenta", name, &a.to_string()], blocked.as_deref())?;
    let summary = v["resumo"].as_str().unwrap_or(name).to_string();
    let inverse = v.get("inverso").filter(|i| !i.is_null()).cloned();
    let mut value = v["valor"].clone();
    if !value.is_object() {
        value = json!({ "resultado": value });
    }
    Ok(Out { value, summary, inverse })
}

/// A network the person keeps the agents off (Settings › Network) is off for the virtual machines too: a copy
/// "internet only" still goes out through it. A machine without network ("nenhuma") is fine.
fn network_allowed_for(state_dir: &std::path::Path, peer: &Peer, machine_net: &str) -> Result<(), String> {
    if machine_net == "nenhuma" {
        return Ok(());
    }
    match crate::redes::bloqueada(state_dir, peer.uid) {
        Some(rede) => Err(trf("agents may not use the network “{rede}” (Settings › Network), and this machine would go out through it", &[("rede", rede)])),
        None => Ok(()),
    }
}

fn network_allowed(c: &Ctx, machine_net: &str) -> Result<(), String> {
    network_allowed_for(&c.cfg.state_dir, c.peer, machine_net)
}

/// The network of the machine a call means, as VirtualDesk resolves it (by id first, then by name: the one source of
/// truth, so a copy named like another machine never makes this check the wrong one); "nat" when it cannot be told.
fn machine_network_of(peer: &Peer, a: &Value) -> String {
    virtualdesk(peer, &["ferramenta", "--maquina", &a.to_string()])
        .ok()
        .and_then(|m| m["rede"].as_str().map(String::from))
        .unwrap_or_else(|| "nat".into())
}

/// For the calls that make a machine use the network: refused on a network kept off the agents.
fn check_network_of(c: &Ctx, a: &Value) -> Result<(), String> {
    if crate::redes::bloqueada(&c.cfg.state_dir, c.peer.uid).is_none() {
        return Ok(());
    }
    network_allowed(c, &machine_network_of(c.peer, a))
}

/// Before anyone is asked: a call that would start a machine with network (or make a copy with network) on a network
/// the person keeps the agents off is refused at once — asking, then refusing, would waste the person's answer. The
/// tools check again when they run.
pub fn precheck(state_dir: &std::path::Path, peer: &Peer, name: &str, a: &Value) -> Result<(), String> {
    let starts = match name {
        "vm.clonar" => return network_allowed_for(state_dir, peer, a["rede"].as_str().filter(|r| !r.is_empty()).unwrap_or("internet")),
        "vm.ligar" | "vm.continuar" | "vm.executar" => true,
        "vm.snapshot" => a["acao"].as_str() == Some("voltar"),
        _ => false,
    };
    if !starts || crate::redes::bloqueada(state_dir, peer.uid).is_none() {
        return Ok(());
    }
    network_allowed_for(state_dir, peer, &machine_network_of(peer, a))
}

fn clonar(c: &Ctx, a: &Value) -> Result<Out, String> {
    network_allowed(c, a["rede"].as_str().filter(|r| !r.is_empty()).unwrap_or("internet"))?;
    call(c, "vm.clonar", a)
}

fn listar(c: &Ctx, a: &Value) -> Result<Out, String> {
    call(c, "vm.listar", a)
}
fn captura(c: &Ctx, a: &Value) -> Result<Out, String> {
    call(c, "vm.captura_de_tela", a)
}
fn ajustes(c: &Ctx, a: &Value) -> Result<Out, String> {
    let mut a = a.clone();
    path_arg(c, &mut a, "cd")?;
    call(c, "vm.ajustes", &a)
}
fn ligar(c: &Ctx, a: &Value) -> Result<Out, String> {
    check_network_of(c, a)?;
    call(c, "vm.ligar", a)
}
fn pausar(c: &Ctx, a: &Value) -> Result<Out, String> {
    call(c, "vm.pausar", a)
}
fn continuar(c: &Ctx, a: &Value) -> Result<Out, String> {
    check_network_of(c, a)?;
    call(c, "vm.continuar", a)
}
fn snapshot(c: &Ctx, a: &Value) -> Result<Out, String> {
    // going back to a snapshot with memory starts a machine that is off
    if s(a, "acao")? == "voltar" {
        check_network_of(c, a)?;
    }
    call(c, "vm.snapshot", a)
}
fn criar(c: &Ctx, a: &Value) -> Result<Out, String> {
    let mut a = a.clone();
    path_arg(c, &mut a, "iso")?;
    call(c, "vm.criar", &a)
}
fn pastas(c: &Ctx, a: &Value) -> Result<Out, String> {
    let mut a = a.clone();
    change_arg(c, &mut a, "caminho")?;
    call(c, "vm.pastas", &a)
}
fn copiar(c: &Ctx, a: &Value) -> Result<Out, String> {
    let mut a = a.clone();
    // only the side on this computer is checked; inside the machine it is the machine's business
    if a["sentido"].as_str() == Some("da_maquina") {
        change_arg(c, &mut a, "para")?;
        // VirtualDesk writes into a staging file in the person's protected state (no agent reaches it), and kalangod
        // puts it in place without following links (`arquivos::put_in_place`): VirtualDesk follows a link on the path
        let dest = PathBuf::from(s(&a, "para")?);
        let name = dest.file_name().ok_or_else(|| tr("path without a name").to_string())?.to_os_string();
        let staging = c.peer.home.join(".local/state/kalango/vm-copias").join(format!("{}-{}", std::process::id(), crate::timeline::now()));
        std::fs::create_dir_all(&staging).map_err(|e| format!("{}: {e}", staging.display()))?;
        if nix::unistd::geteuid().is_root() {
            let _ = nix::unistd::chown(&staging, Some(nix::unistd::Uid::from_raw(c.peer.uid)), Some(nix::unistd::Gid::from_raw(c.peer.gid)));
        }
        let staged = staging.join(&name);
        a["para"] = json!(staged);
        let out = call(c, "vm.copiar_arquivo", &a);
        let placed = match &out {
            Ok(_) if staged.exists() => crate::tools::arquivos::put_in_place(c, &staged, &dest).map(|_| ()),
            _ => Ok(()),
        };
        let _ = std::fs::remove_dir_all(&staging);
        placed?;
        return out;
    }
    path_arg(c, &mut a, "de")?;
    call(c, "vm.copiar_arquivo", &a)
}
fn executar(c: &Ctx, a: &Value) -> Result<Out, String> {
    s(a, "comando")?;
    check_network_of(c, a)?;
    call(c, "vm.executar", a)
}
fn desligar(c: &Ctx, a: &Value) -> Result<Out, String> {
    call(c, "vm.desligar", a)
}
fn apagar(c: &Ctx, a: &Value) -> Result<Out, String> {
    call(c, "vm.apagar", a)
}

/// A running QEMU of VirtualDesk that the network rule is about, from what /proc says of the process: the machine's
/// id (`-name …,process=kvd-ID`) and whether it has network (not `restrict=on`). Only sandbox copies (VirtualDesk
/// marks their QEMU with `KALANGO_VD_COPIA=1` in its environment) and any QEMU started from inside the agents' slice.
pub fn qemu_copy(cmdline: &str, environ: &str, cgroup: &str) -> Option<(String, bool)> {
    let args: Vec<&str> = cmdline.split('\0').collect();
    if !args.first().is_some_and(|a| a.rsplit('/').next().is_some_and(|n| n.starts_with("qemu-system"))) {
        return None;
    }
    let id = args.iter().find_map(|a| a.split(',').find_map(|p| p.strip_prefix("process=kvd-")))?.to_string();
    let copy = environ.split('\0').any(|v| v == "KALANGO_VD_COPIA=1");
    let agents = cgroup.contains("/kalango-agentes.slice");
    if !copy && !agents {
        return None;
    }
    let nic = args.windows(2).find(|w| w[0] == "-nic").map(|w| w[1]).unwrap_or("none");
    let network = nic != "none" && !nic.split(',').any(|o| o == "restrict=on");
    Some((id, network))
}

/// When this computer is on a network a person keeps the agents off, their running sandbox copies with network are
/// turned off (not paused: a paused copy could be continued; a copy turned off can only start again through the check
/// in VirtualDesk's start), and the timeline says so. The copies are found in /proc — wherever their sockets are (a
/// copy started with another runtime folder is still found) — by the mark VirtualDesk puts on their QEMU. Run every few
/// seconds by the server. Returns what it turned off.
pub fn stop_copies_on_blocked_networks(state_dir: &std::path::Path, timeline: &crate::timeline::Timeline) -> Vec<String> {
    use std::os::unix::fs::MetadataExt;
    let mut done = vec![];
    let me = nix::unistd::geteuid();
    let blocked: Vec<(u32, String)> = crate::redes::pessoas(state_dir)
        .into_iter()
        // a kalangod that is not root only acts for its own user
        .filter(|uid| me.is_root() || *uid == me.as_raw())
        .filter_map(|uid| crate::redes::bloqueada(state_dir, uid).map(|r| (uid, r)))
        .collect();
    if blocked.is_empty() {
        return done;
    }
    let Ok(rd) = std::fs::read_dir("/proc") else { return done };
    for e in rd.flatten() {
        let Ok(pid) = e.file_name().to_string_lossy().parse::<i32>() else { continue };
        let Ok(md) = std::fs::metadata(e.path()) else { continue };
        let Some((_, rede)) = blocked.iter().find(|(u, _)| *u == md.uid()) else { continue };
        let read = |f: &str| std::fs::read(e.path().join(f)).map(|b| String::from_utf8_lossy(&b).to_string()).unwrap_or_default();
        let Some((id, network)) = qemu_copy(&read("cmdline"), &read("environ"), &read("cgroup")) else { continue };
        if !network {
            continue;
        }
        let _ = nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), nix::sys::signal::Signal::SIGTERM);
        for _ in 0..40 {
            if !e.path().exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        if e.path().exists() {
            let _ = nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), nix::sys::signal::Signal::SIGKILL);
        }
        timeline.append(crate::timeline::Entry {
            id: 0,
            time: 0,
            caller: "kalangod (rede sem agentes)".into(),
            uid: md.uid(),
            pid: std::process::id() as i32,
            tool: "vm.desligar".into(),
            level: Level::Usuario.name().into(),
            args: json!({"id": id, "forcar": true, "pid": pid}),
            status: "ok".into(),
            summary: trf("turned {n} off: this computer is on “{rede}”, a network kept off the agents", &[("n", id.clone()), ("rede", rede.clone())]),
            inverse: None,
            restore_point: None,
            miniatura: None,
            startup: None,
            d1: None,
        });
        done.push(id);
    }
    done
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtualdesk_s_test_variables_reach_it_only_from_a_test_kalangod() {
        let removed = |test: bool| {
            let mut c = Command::new("true");
            c.env("KALANGO_VIRTUALDESK_TEST_HOME", "/tmp/x").env("KALANGO_VIRTUALDESK_DIR", "/tmp/y");
            strip_test_env(&mut c, test);
            c.get_envs().filter(|(_, v)| v.is_none()).map(|(k, _)| k.to_string_lossy().to_string()).collect::<Vec<_>>()
        };
        let mut r = removed(false);
        r.sort();
        assert_eq!(r, ["KALANGO_VIRTUALDESK_DIR", "KALANGO_VIRTUALDESK_TEST_HOME"], "a kalangod that is not a test session (root or not)");
        assert!(removed(true).is_empty(), "a test session keeps them");
    }

    #[test]
    fn a_proof_is_good_once_for_its_person_and_tool() {
        let p = new_proof(1000, "vm.ligar");
        assert_eq!(p.len(), 32);
        assert!(!redeem_proof(&p, 1000, "vm.ajustes"), "another tool");
        assert!(!redeem_proof(&p, 1001, "vm.ligar"), "another person");
        assert!(redeem_proof(&p, 1000, "vm.ligar"), "this call");
        assert!(!redeem_proof(&p, 1000, "vm.ligar"), "never twice (a replay)");
        // a proof forgotten when its call ended is no longer good
        let q = new_proof(1000, "vm.ligar");
        forget_proof(&q);
        assert!(!redeem_proof(&q, 1000, "vm.ligar"));
        assert!(!redeem_proof("", 1000, "vm.ligar"));
        assert_ne!(new_proof(1, "x"), new_proof(1, "x"), "each call its own");
    }

    #[test]
    fn copies_with_network_are_told_from_their_qemu() {
        let line = |nic: &str| format!("/usr/bin/qemu-system-x86_64\0-name\0caixa,process=kvd-caixa-1\0-nic\0{nic}\0-daemonize\0");
        let copy = "HOME=/home/ana\0KALANGO_VD_COPIA=1\0";
        let user = "0::/user.slice/user-1000.slice/session-2.scope\n";
        assert_eq!(qemu_copy(&line("user,id=net0,model=virtio-net-pci"), copy, user), Some(("caixa-1".into(), true)));
        assert_eq!(qemu_copy(&line("user,id=net0,model=virtio-net-pci,restrict=on"), copy, user), Some(("caixa-1".into(), false)));
        assert_eq!(qemu_copy(&line("user,id=net0"), "HOME=/home/ana\0", user), None, "the person's own machine");
        let agents = "0::/user.slice/user-1000.slice/user@1000.service/kalango.slice/kalango-agentes.slice/t.scope\n";
        assert_eq!(qemu_copy(&line("user,id=net0"), "", agents), Some(("caixa-1".into(), true)), "started from the agents' slice");
        assert_eq!(qemu_copy("/usr/bin/python3\0process=kvd-x\0", copy, user), None, "not a QEMU");
    }

    #[test]
    fn every_vm_tool_has_a_contract() {
        let all = tools();
        assert_eq!(all.len(), 14);
        for t in &all {
            assert!(t.name.starts_with("vm."));
            assert_eq!((t.schema)()["type"], "object");
        }
        let find = |n: &str| all.iter().find(|t| t.name == n).unwrap();
        assert_eq!(find("vm.listar").effect, Effect::Leitura);
        assert_eq!(find("vm.executar").effect, Effect::Destrutiva);
        assert_eq!(find("vm.apagar").effect, Effect::Destrutiva);
        assert!(find("vm.ligar").reversible);
        // the three network modes of VirtualDesk, for a sandbox copy and for a machine's settings
        for n in ["vm.clonar", "vm.ajustes"] {
            let rede = &(find(n).schema)()["properties"]["rede"]["enum"];
            for w in ["internet", "nenhuma", "nat", "casa"] {
                assert!(rede.as_array().unwrap().iter().any(|v| v == w), "{n}: {w}");
            }
        }
    }
}

