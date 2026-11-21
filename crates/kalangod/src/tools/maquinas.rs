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
        t("vm.clonar", n_("Makes a disposable copy of a template machine for an agent's sandbox, already running from a snapshot (no folder of yours goes along)"), Usuario, Escreve, false,
            || obj(json!({"id": id_prop(), "nome": {"type": "string"}, "snapshot": {"type": "string"}, "rede": {"type": "string", "enum": ["nat", "nenhuma"]}}), &["id"]), clonar),
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

fn id_only() -> Value {
    obj(json!({"id": id_prop()}), &["id"])
}

fn ajustes_schema() -> Value {
    obj(
        json!({"id": id_prop(), "cpus": {"type": "integer", "minimum": 1}, "memoria_mb": {"type": "integer", "minimum": 256}, "aninhada": {"type": "boolean"},
            "area_de_transferencia": {"type": "boolean"}, "som": {"type": "boolean"}, "microfone": {"type": "boolean"}, "tela_cheia_ao_abrir": {"type": "boolean"},
            "rede": {"type": "string", "enum": ["nat", "nenhuma"]}, "cd": {"type": "string", "description": "caminho de um .iso seu; vazio ejeta"}, "novo_nome": {"type": "string"}}),
        &["id"],
    )
}

/// The VirtualDesk program: `$KALANGO_VIRTUALDESK`, the system's, the person's ~/.local/bin, or the PATH.
pub fn program(peer: &Peer) -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("KALANGO_VIRTUALDESK").map(PathBuf::from).filter(|p| p.is_file()) {
        return Some(p);
    }
    let fixed = [PathBuf::from("/usr/bin/kalango-virtualdesk"), PathBuf::from("/usr/local/bin/kalango-virtualdesk"), peer.home.join(".local/bin/kalango-virtualdesk")];
    if let Some(p) = fixed.into_iter().find(|p| p.is_file()) {
        return Some(p);
    }
    std::env::var_os("PATH").and_then(|path| std::env::split_paths(&path).map(|d| d.join("kalango-virtualdesk")).find(|p| p.is_file()))
}

/// Runs VirtualDesk as the caller and reads its one JSON line.
fn virtualdesk(peer: &Peer, args: &[&str]) -> Result<Value, String> {
    let exe = program(peer).ok_or_else(|| tr("Kalango VirtualDesk is not installed").to_string())?;
    let mut cmd = Command::new(exe);
    cmd.args(args).env("KALANGO_CHAMADOR", &peer.exe);
    if nix::unistd::geteuid().is_root() && peer.uid != 0 {
        use std::os::unix::process::CommandExt;
        // as the person, with only what VirtualDesk needs from the environment: their home (the machines) and their
        // runtime folder (the machines' sockets)
        cmd.uid(peer.uid).gid(peer.gid).env_clear().env("HOME", &peer.home).env("USER", &peer.user)
            .env("XDG_RUNTIME_DIR", format!("/run/user/{}", peer.uid)).env("PATH", "/usr/local/bin:/usr/bin:/bin").env("LANG", std::env::var("LANG").unwrap_or_else(|_| "C.UTF-8".into()))
            .env("KALANGO_CHAMADOR", &peer.exe)
            .current_dir(&peer.home);
    }
    let o = cmd.output().map_err(|e| e.to_string())?;
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
    let v = virtualdesk(c.peer, &["ferramenta", name, &a.to_string()])?;
    let summary = v["resumo"].as_str().unwrap_or(name).to_string();
    let inverse = v.get("inverso").filter(|i| !i.is_null()).cloned();
    let mut value = v["valor"].clone();
    if !value.is_object() {
        value = json!({ "resultado": value });
    }
    Ok(Out { value, summary, inverse })
}

fn clonar(c: &Ctx, a: &Value) -> Result<Out, String> {
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
    call(c, "vm.ligar", a)
}
fn pausar(c: &Ctx, a: &Value) -> Result<Out, String> {
    call(c, "vm.pausar", a)
}
fn continuar(c: &Ctx, a: &Value) -> Result<Out, String> {
    call(c, "vm.continuar", a)
}
fn snapshot(c: &Ctx, a: &Value) -> Result<Out, String> {
    s(a, "acao")?;
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
    } else {
        path_arg(c, &mut a, "de")?;
    }
    call(c, "vm.copiar_arquivo", &a)
}
fn executar(c: &Ctx, a: &Value) -> Result<Out, String> {
    s(a, "comando")?;
    call(c, "vm.executar", a)
}
fn desligar(c: &Ctx, a: &Value) -> Result<Out, String> {
    call(c, "vm.desligar", a)
}
fn apagar(c: &Ctx, a: &Value) -> Result<Out, String> {
    call(c, "vm.apagar", a)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    }
}
