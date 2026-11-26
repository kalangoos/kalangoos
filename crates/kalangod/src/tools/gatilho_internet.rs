//! O gatilho de entrada pela internet (kalango-fluxos) e o túnel que o expõe, como ferramentas `gatilho_internet.*` e
//! `tunel.*`. É uma porta para o mundo, então:
//!
//! - criar um gatilho, ligá-lo, girar a chave dele e abrir um túnel pedem SEMPRE a aprovação da pessoa, com o texto
//!   exato do que vai existir (nunca "sempre permitir": o pedido não olha as aprovações permanentes);
//! - o trabalho é do serviço `kalango-fluxosd` (da pessoa, não do kalangod, que é privilegiado): o kalangod roda
//!   `kalango-fluxosd chamar internet.* JSON` como a pessoa que chamou;
//! - a chave de um gatilho nunca volta para um agente: quem quer vê-la pede ao serviço (`kalango-fluxosd gatilho
//!   mostrar --segredo`), na própria sessão;
//! - `tunel.autorizar` é a pergunta que o serviço faz antes de abrir o túnel (por qualquer caminho: a pessoa na
//!   linha de comando, o atalho do shell ou o agente com `tunel.abrir`): mostra o que fica acessível, por qual
//!   provedor e quais gatilhos, e entra na linha do tempo. Fechar nunca pede.

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
        t("gatilho_internet.listar", n_("Internet webhook triggers: address path, level, limits and whether each is on (never the key)"), Observar, Leitura, false, super::none, listar),
        t("gatilho_internet.mostrar", n_("One internet webhook trigger: its address, limits and how a call is signed (never the key)"), Observar, Leitura, false, id_only, mostrar),
        t("gatilho_internet.chamadas", n_("The latest calls to the internet webhook triggers: status and size only, never the content"), Observar, Leitura, false,
            || obj(json!({"id": {"type": "string"}, "limite": {"type": "integer", "minimum": 1, "maximum": 200}}), &[]), chamadas),
        t("gatilho_internet.criar", n_("Creates an internet webhook trigger for a flow (a public address that starts it, signed; created OFF); the person approves exactly what it is"), Sistema, Escreve, true, criar_schema, criar),
        t("gatilho_internet.ativar", n_("Turns an internet webhook trigger on; the person approves, seeing its address, level and limits"), Sistema, Escreve, true, id_only, ativar),
        t("gatilho_internet.editar", n_("Changes the level an internet webhook trigger gives the flow when it starts from outside; raising it asks the person, lowering never does"), Sistema, Escreve, true,
            || obj(json!({"id": {"type": "string"}, "nivel": {"type": "string", "enum": ["observar", "usuario", "sistema"]}}), &["id", "nivel"]), editar),
        t("gatilho_internet.autorizar_nivel", n_("The person's approval to give an internet trigger a level above the lowest (asked by the service when nobody else asked); changes nothing by itself"), Sistema, Escreve, false,
            || obj(json!({"id": {"type": "string"}, "de": {"type": "string", "enum": ["observar", "usuario", "sistema", "total"]}, "para": {"type": "string", "enum": ["observar", "usuario", "sistema", "total"]}}), &["id", "de", "para"]), autorizar_nivel),
        t("gatilho_internet.autorizar_ampliacao", n_("The person's approval to make an internet trigger accept more callers (public without a key, or without its address list); changes nothing by itself"), Sistema, Escreve, false,
            || obj(json!({"id": {"type": "string"}, "o_que": {"type": "string", "enum": ["public", "addresses", "enable"]}}), &["id", "o_que"]), autorizar_ampliacao),
        t("gatilho_internet.desativar", n_("Turns an internet webhook trigger off (its address stops answering at once)"), Usuario, Escreve, true, id_only, desativar),
        t("gatilho_internet.girar_segredo", n_("Replaces the key of an internet webhook trigger; the old one stops working and the new one is not shown to the agent"), Sistema, Escreve, false, id_only, girar),
        t("gatilho_internet.apagar", n_("Deletes an internet webhook trigger and its key"), Usuario, Destrutiva, false, id_only, apagar),
        t("tunel.estado", n_("The internet tunnel: open or closed, its public address, since when, and whether it is healthy"), Observar, Leitura, false, super::none, estado),
        t("tunel.abrir", n_("Opens a tunnel to the internet (Cloudflare) so the webhook triggers answer from outside; the person approves first"), Usuario, Escreve, true,
            || obj(json!({"modo": {"type": "string", "enum": ["rapido", "nomeado"]}, "dominio": {"type": "string"}, "ociosidade_s": {"type": "integer", "minimum": 60, "maximum": 604800}}), &["modo"]), abrir),
        t("tunel.fechar", n_("Closes the internet tunnel (never asks)"), Observar, Escreve, false, super::none, fechar),
        t("tunel.fechar_todos", n_("The kill switch: closes every internet tunnel at once (never asks)"), Observar, Escreve, false, super::none, fechar),
        t("tunel.autorizar", n_("The person's approval to open an internet tunnel: shows what becomes reachable, through which provider and which triggers; changes nothing by itself"), Sistema, Escreve, false,
            || obj(json!({"modo": {"type": "string", "enum": ["rapido", "nomeado"]}, "provedor": {"type": "string"}, "dominio": {"type": "string"}, "porta": {"type": "integer", "minimum": 1, "maximum": 65535},
                "ociosidade_s": {"type": ["integer", "null"]}, "gatilhos": {"type": "array"}}), &["modo", "provedor", "porta"]), autorizar),
    ]
}

fn id_only() -> Value {
    obj(json!({"id": {"type": "string", "description": "the trigger's id (see gatilho_internet.listar)"}}), &["id"])
}

fn criar_schema() -> Value {
    obj(
        json!({
            "fluxo": {"type": "string", "description": "the id of the flow the address starts"},
            "nome": {"type": "string", "description": "a name for the trigger (optional; the flow's name when empty)"},
            "metodos": {"type": "array", "items": {"type": "string", "enum": ["GET", "POST", "PUT", "PATCH", "DELETE"]}},
            "corpo_max_bytes": {"type": "integer", "minimum": 1, "maximum": 1048576},
            "por_minuto": {"type": "integer", "minimum": 1, "maximum": 600},
            "tipos": {"type": "array", "items": {"type": "string"}, "description": "accepted content types"},
            "resposta": {"type": "string", "enum": ["imediata", "esperar"]},
            "espera_s": {"type": "integer", "minimum": 1, "maximum": 120},
            "ips": {"type": "array", "items": {"type": "string"}, "description": "addresses or networks allowed to call (empty: anyone with the signature)"},
            "sem_assinatura": {"type": "boolean", "description": "public, WITHOUT signature: only at the lowest level; avoid"},
            "nivel": {"type": "string", "enum": ["observar", "usuario", "sistema", "total"], "description": "the highest level the flow may use when started from outside (default: observar)"},
            "fila_max": {"type": "integer", "minimum": 1, "maximum": 1000}
        }),
        &["fluxo"],
    )
}

// ------------------------------------------------------------------ o serviço

fn program(peer: &Peer) -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("KALANGO_FLUXOSD").map(PathBuf::from).filter(|p| p.is_file()) {
        return Some(p);
    }
    let fixed = [PathBuf::from("/usr/lib/kalango/kalango-fluxosd"), peer.passwd_home.join(".local/bin/kalango-fluxosd")];
    if let Some(p) = fixed.into_iter().find(|p| p.is_file()) {
        return Some(p);
    }
    std::env::var_os("PATH").and_then(|path| std::env::split_paths(&path).map(|d| d.join("kalango-fluxosd")).find(|p| p.is_file()))
}

fn socket(peer: &Peer) -> String {
    std::env::var("KALANGO_FLUXOS_SOCKET").unwrap_or_else(|_| format!("/run/user/{}/kalango/fluxos.sock", peer.uid))
}

/// Whether the process on the other end of the connected socket runs as `uid` (SO_PEERCRED).
pub fn peer_is(s: &std::os::unix::net::UnixStream, uid: u32) -> bool {
    nix::sys::socket::getsockopt(s, nix::sys::socket::sockopt::PeerCredentials).map(|c| c.uid() == uid).unwrap_or(false)
}

/// One JSON-RPC call over the service's socket (what the service's own command does, without starting any program). The
/// socket must be a socket of this person: another owner's, or a link, is not spoken to.
pub fn call_socket(path: &str, owner: Option<u32>, method: &str, params: &Value) -> Result<Value, String> {
    call_socket_with(path, owner, owner, method, params, Deadlines { connect: std::time::Duration::from_secs(10), whole: std::time::Duration::from_secs(150) })
}

/// How long a call to the service may take: to get connected, and from the first byte written to the last one read.
#[derive(Clone, Copy)]
pub struct Deadlines {
    pub connect: std::time::Duration,
    pub whole: std::time::Duration,
}

const MAX_REPLY: usize = 1 << 20;

/// `call_socket`, with the owner of the socket FILE and the uid that must be on the other end of the CONNECTION told apart
/// (they are the same person's in production), and with the deadlines of the call.
pub fn call_socket_with(path: &str, file_owner: Option<u32>, peer_uid: Option<u32>, method: &str, params: &Value, d: Deadlines) -> Result<Value, String> {
    use std::io::{Read, Write};
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    let md = std::fs::symlink_metadata(path).map_err(|e| trf("the agents platform is not running: {e}", &[("e", e.to_string())]))?;
    if !md.file_type().is_socket() || file_owner.is_some_and(|u| md.uid() != u) {
        return Err(tr("the service's socket is not the person's own").into());
    }
    let started = std::time::Instant::now();
    let mut s = connect_by(path, started + d.connect)?;
    // the path was looked at, then connected to: it may have been swapped in between. The kernel says who is on the other end
    // of what was really connected, and only the person's own service is spoken to
    if let Some(uid) = peer_uid {
        if !peer_is(&s, uid) {
            return Err(tr("the service's socket is not the person's own").into());
        }
    }
    let deadline = std::time::Instant::now() + d.whole;
    let left = |at: std::time::Instant| at.checked_duration_since(std::time::Instant::now()).filter(|d| !d.is_zero()).ok_or_else(|| tr("the agents platform did not answer in time").to_string());
    let _ = s.set_write_timeout(Some(left(deadline)?));
    writeln!(s, "{}", json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params})).map_err(|e| e.to_string())?;
    // the whole answer has one deadline (a read each time with what is left of it), and at most a mebibyte: a service that
    // sends a byte now and then, or never stops, does not keep this thread
    let mut line: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        s.set_read_timeout(Some(left(deadline)?)).map_err(|e| e.to_string())?;
        let n = match s.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => return Err(tr("the agents platform did not answer in time").into()),
            Err(e) => return Err(e.to_string()),
        };
        match chunk[..n].iter().position(|b| *b == b'\n') {
            Some(i) => {
                line.extend_from_slice(&chunk[..i]);
                break;
            }
            None => line.extend_from_slice(&chunk[..n]),
        }
        if line.len() > MAX_REPLY {
            return Err(tr("the agents platform's answer is too large").into());
        }
    }
    if line.len() > MAX_REPLY {
        return Err(tr("the agents platform's answer is too large").into());
    }
    let v: Value = serde_json::from_slice(&line).map_err(|_| tr("the agents platform did not answer").to_string())?;
    match v.get("error") {
        Some(e) => Err(e["message"].as_str().unwrap_or("error").to_string()),
        None => Ok(v["result"].clone()),
    }
}

/// Connects without ever waiting for the other side: a socket whose queue is full answers "try again" at once, and this tries
/// until `until`. (A plain connect on a full queue waits for as long as the service does not accept.)
fn connect_by(path: &str, until: std::time::Instant) -> Result<std::os::unix::net::UnixStream, String> {
    use nix::sys::socket::{connect, socket, AddressFamily, SockFlag, SockType, UnixAddr};
    use std::os::fd::AsRawFd;
    let addr = UnixAddr::new(path).map_err(|e| e.to_string())?;
    loop {
        let fd = socket(AddressFamily::Unix, SockType::Stream, SockFlag::SOCK_NONBLOCK | SockFlag::SOCK_CLOEXEC, None).map_err(|e| e.to_string())?;
        match connect(fd.as_raw_fd(), &addr) {
            Ok(()) => {
                let s = std::os::unix::net::UnixStream::from(fd);
                s.set_nonblocking(false).map_err(|e| e.to_string())?;
                return Ok(s);
            }
            Err(nix::errno::Errno::EAGAIN) | Err(nix::errno::Errno::EINTR) => {
                if std::time::Instant::now() >= until {
                    return Err(tr("the agents platform is busy: it did not answer in time").into());
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Err(e) => return Err(e.to_string()),
        }
    }
}

/// One call to the service, as the caller (its uid, its runtime folder). As root (the system service) the call goes over
/// the person's socket, as root: the service tells kalangod by that uid. Nothing is ever started as root from a place the
/// person can write.
fn service(peer: &Peer, method: &str, params: &Value) -> Result<Value, String> {
    #[cfg(test)]
    if let Some(r) = STAND_IN.with(|s| s.borrow().as_ref().map(|f| f(method, params))) {
        return r;
    }
    if nix::unistd::geteuid().is_root() && peer.uid != 0 {
        return call_socket(&socket(peer), Some(peer.uid), method, params);
    }
    let exe = program(peer).ok_or_else(|| tr("the agents platform (kalango-fluxos) is not installed").to_string())?;
    let mut cmd = Command::new(exe);
    cmd.args(["chamar", method, &params.to_string(), "--socket", &socket(peer)]);
    // (not root here: a development session, where the service is this user's own: nothing to drop)
    let o = cmd.output().map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(String::from_utf8_lossy(&o.stdout).trim())
        .map_err(|_| trf("the agents platform did not answer: {e}", &[("e", String::from_utf8_lossy(&o.stderr).trim().to_string())]))?;
    match v.get("error") {
        Some(e) => Err(e["message"].as_str().unwrap_or("error").to_string()),
        None => Ok(v["result"].clone()),
    }
}

/// A service answering this test thread's calls, in place of the program and its socket (nothing shared with other tests).
#[cfg(test)]
type StandIn = Box<dyn Fn(&str, &Value) -> Result<Value, String>>;

#[cfg(test)]
thread_local! {
    static STAND_IN: std::cell::RefCell<Option<StandIn>> = const { std::cell::RefCell::new(None) };
}

/// What the service says, without the key and without a ready signature (an agent must not get a way to call).
fn scrub(mut v: Value) -> Value {
    match &mut v {
        Value::Object(o) => {
            for k in ["secret", "secret_shown_once", "curl", "curl_valid_for_s"] {
                o.remove(k);
            }
            o.values_mut().for_each(|x| *x = scrub(x.take()));
        }
        Value::Array(a) => a.iter_mut().for_each(|x| *x = scrub(x.take())),
        _ => {}
    }
    v
}

// ------------------------------------------------------------------ as ferramentas

fn listar(c: &Ctx, _: &Value) -> Result<Out, String> {
    let v = service(c.peer, "internet.trigger_list", &json!({}))?;
    Ok(Out::read(json!({"gatilhos": scrub(v)}), tr("listed the internet triggers")))
}

fn mostrar(c: &Ctx, a: &Value) -> Result<Out, String> {
    let v = service(c.peer, "internet.trigger_show", &json!({"id": s(a, "id")?}))?;
    Ok(Out::read(scrub(v), tr("read an internet trigger")))
}

fn chamadas(c: &Ctx, a: &Value) -> Result<Out, String> {
    let v = service(c.peer, "internet.calls", &json!({"trigger_id": a.get("id"), "limit": a.get("limite")}))?;
    Ok(Out::read(json!({"chamadas": v}), tr("listed the latest calls")))
}

/// The service's names for the tool's arguments (the key is never asked for).
fn service_config(a: &Value) -> Value {
    let mut p = json!({"flow_id": a.get("fluxo")});
    let map = [("metodos", "methods"), ("corpo_max_bytes", "max_body_bytes"), ("por_minuto", "rate_per_minute"), ("tipos", "content_types"),
        ("nome", "name"), ("ips", "allow_ips"), ("sem_assinatura", "public_unsigned"), ("nivel", "max_level"), ("fila_max", "max_pending"), ("espera_s", "wait_timeout_s")];
    for (from, to) in map {
        if let Some(v) = a.get(from) {
            p[to] = v.clone();
        }
    }
    if let Some(r) = a.get("resposta").and_then(Value::as_str) {
        p["respond"] = json!(if r == "esperar" { "wait" } else { "accepted" });
    }
    p
}

/// The service's arguments plus who is asking: the service refuses a level above the creator's.
fn config_for(a: &Value, rule: &crate::policy::CallerRule) -> Value {
    let mut p = service_config(a);
    p["creator"] = json!({"id": rule.name, "max_level": rule.max_level.name()});
    p
}

fn criar(c: &Ctx, a: &Value) -> Result<Out, String> {
    asked_level(a, c.rule)?;
    let mut p = config_for(a, c.rule);
    if let Some(x) = take_expected(c, "gatilho_internet.criar", a) {
        p["expected"] = x;
    }
    let v = service(c.peer, "internet.trigger_create", &p)?;
    let id = v["id"].as_str().unwrap_or("").to_string();
    Ok(Out {
        value: scrub(v),
        summary: trf("internet trigger created (off): {id}", &[("id", id.clone())]),
        inverse: Some(json!({"ferramenta": "gatilho_internet.apagar", "argumentos": {"id": id}})),
    })
}

fn ativar(c: &Ctx, a: &Value) -> Result<Out, String> {
    let id = s(a, "id")?;
    let mut p = json!({"id": id, "on": true, "creator": {"id": c.rule.name, "max_level": c.rule.max_level.name()}});
    if let Some(x) = take_expected(c, "gatilho_internet.ativar", a) {
        p["expected"] = x;
    }
    let v = service(c.peer, "internet.trigger_set_enabled", &p)?;
    Ok(Out {
        value: scrub(v),
        summary: trf("internet trigger turned on: {id}", &[("id", id.to_string())]),
        inverse: Some(json!({"ferramenta": "gatilho_internet.desativar", "argumentos": {"id": id}})),
    })
}

fn editar(c: &Ctx, a: &Value) -> Result<Out, String> {
    let id = s(a, "id")?;
    let level = asked_level(&json!({"nivel": s(a, "nivel")?}), c.rule)?;
    let before = current_level(c.peer, id)?;
    // a rise is only the one the person approved, from the level they were shown; anything else is lowering or nothing
    let expected = if rises(&before, &level) {
        take_approved_rise(c.peer.uid, id, &level).ok_or_else(|| tr("raising the level needs the approval of exactly that change").to_string())?
    } else {
        before.clone()
    };
    let mut p = json!({"id": id, "max_level": level, "expected_level": expected});
    p["creator"] = json!({"id": c.rule.name, "max_level": c.rule.max_level.name()});
    service(c.peer, "internet.trigger_update", &p)?;
    Ok(Out {
        value: json!({"id": id, "nivel": level}),
        summary: trf("level of internet trigger {id}: {from} → {to}", &[("id", id.to_string()), ("from", expected.clone()), ("to", level)]),
        inverse: Some(json!({"ferramenta": "gatilho_internet.editar", "argumentos": {"id": id, "nivel": expected}})),
    })
}

/// The level a trigger has now, from the service.
fn current_level(peer: &Peer, id: &str) -> Result<String, String> {
    let list = service(peer, "internet.trigger_list", &json!({}))?;
    let t = list.as_array().and_then(|a| a.iter().find(|t| t["id"] == id)).ok_or_else(|| tr("no such internet trigger").to_string())?;
    t["level"].as_str().filter(|l| LEVELS.contains(l)).map(str::to_string).ok_or_else(|| tr("the service did not say the level of the trigger").to_string())
}

/// The approval itself is `needs_approval`'s; reaching here means the person said yes.
fn autorizar_nivel(_: &Ctx, a: &Value) -> Result<Out, String> {
    Ok(Out { value: json!({"autorizado": true}), summary: trf("the person allowed the level {to} for an internet trigger", &[("to", s(a, "para")?.to_string())]), inverse: None })
}

/// The approval itself is `needs_approval`'s; reaching here means the person said yes. Turning a trigger on (the service's
/// question, "enable") goes on only if the trigger is still what the card showed: its facts are read from the service
/// again now and compared with the card's, and they go back with the answer, as the precondition of the change.
fn autorizar_ampliacao(c: &Ctx, a: &Value) -> Result<Out, String> {
    let mut value = json!({"autorizado": true});
    if a["o_que"] == "enable" {
        let id = s(a, "id")?;
        let shown = take_expected(c, "gatilho_internet.autorizar_ampliacao", a).ok_or_else(|| tr("the facts shown to you are missing: nothing was turned on").to_string())?;
        let (t, flow_id) = trigger_now(c.peer, id)?;
        if enable_facts(&t, flow_id)? != shown {
            return Err(tr("the trigger changed after you answered: nothing was turned on").into());
        }
        value["esperado"] = shown;
    }
    Ok(Out { value, summary: tr("the person allowed an internet trigger to accept more callers").to_string(), inverse: None })
}

/// A trigger as the service describes it now (`internet.trigger_list`), and the flow it starts (`internet.trigger_show`).
fn trigger_now(peer: &Peer, id: &str) -> Result<(Value, Option<String>), String> {
    let list = service(peer, "internet.trigger_list", &json!({}))?;
    let t = list.as_array().and_then(|a| a.iter().find(|t| t["id"] == id)).cloned().ok_or_else(|| tr("no such internet trigger").to_string())?;
    let flow_id = service(peer, "internet.trigger_show", &json!({"id": id})).ok().and_then(|v| v["flow_id"].as_str().map(str::to_string));
    Ok((t, flow_id))
}

/// What the card of turning a trigger on shows and is based on — level, signature, flow, address list and address, read
/// strictly — and what the service refuses a trigger for when it is no longer that.
fn enable_facts(t: &Value, flow_id: Option<String>) -> Result<Value, String> {
    let (signed, level) = strict_facts(t)?;
    Ok(json!({"level": level, "signed": signed, "flow_id": flow_id, "allow_ips": t["allow_ips"], "path": t["path"]}))
}

/// Turning a trigger on, as the person sees it: the text and the card, from what the service says of the trigger now,
/// with its facts kept for the run (`_esperado`). The agent's `gatilho_internet.ativar` and the service's own question
/// (`gatilho_internet.autorizar_ampliacao` "enable", when a program of the person's turns it on) both come here, so the two
/// can never say different things of the same trigger (a public one is called public, in coral, with its level and
/// address, whoever asks).
fn enable_card(peer: &Peer, rule: &crate::policy::CallerRule, cfg: Option<&crate::policy::Config>, id: &str) -> Result<(String, Value), String> {
    let (t, flow_id) = trigger_now(peer, id)?;
    let facts = enable_facts(&t, flow_id)?;
    let signed = facts["signed"] == true;
    let level = facts["level"].as_str().unwrap_or("observar").to_string();
    let text = lines_text(
        trf("turn on the internet webhook trigger {id} (address {path}): its address starts answering when a tunnel is open", &[("id", one_line(&json!(id), 40)), ("path", one_line(&t["path"], 80))]),
        trigger_lines(&t)?,
    );
    let mut card = card(
        trf("Turn on the trigger {id}? Its address starts answering when a tunnel is open.", &[("id", one_line(&json!(id), 60))]),
        Some((
            tr("It can be fired from outside").to_string(),
            if signed { tr("While a tunnel is open, anyone who has the key can fire this flow.").to_string() } else { tr("While a tunnel is open, anyone with the address can fire this flow: it has no signature.").to_string() },
            if signed { "ouro" } else { "coral" },
        )),
        vec![
            (tr("Trigger").to_string(), one_line(&json!(id), 40)),
            (tr("Address").to_string(), one_line(&t["path"], 80)),
            (tr("Signature").to_string(), if signed { tr("required (HMAC)").to_string() } else { tr("NONE (public)").to_string() }),
            (tr("Level").to_string(), level_label(&level)),
            (tr("Asked by").to_string(), caller_label(peer, rule, cfg)),
        ],
        tr("Turning it on does not open a tunnel; that is asked separately.").to_string(),
        if signed { "ouro" } else { "coral" },
        tr("Turn on").to_string(),
        tr("Deny").to_string(),
    );
    card["_esperado"] = facts;
    Ok((text, card))
}

fn desativar(c: &Ctx, a: &Value) -> Result<Out, String> {
    let id = s(a, "id")?;
    let v = service(c.peer, "internet.trigger_set_enabled", &json!({"id": id, "on": false}))?;
    Ok(Out { value: scrub(v), summary: trf("internet trigger turned off: {id}", &[("id", id.to_string())]), inverse: None })
}

fn girar(c: &Ctx, a: &Value) -> Result<Out, String> {
    let id = s(a, "id")?;
    // (no reveal: the new key stays in the vault)
    service(c.peer, "internet.trigger_rotate_secret", &json!({"id": id}))?;
    Ok(Out { value: json!({"girado": true}), summary: trf("key of internet trigger {id} replaced", &[("id", id.to_string())]), inverse: None })
}

fn apagar(c: &Ctx, a: &Value) -> Result<Out, String> {
    let id = s(a, "id")?;
    service(c.peer, "internet.trigger_delete", &json!({"id": id}))?;
    Ok(Out { value: json!({"apagado": true}), summary: trf("internet trigger deleted: {id}", &[("id", id.to_string())]), inverse: None })
}

fn estado(c: &Ctx, _: &Value) -> Result<Out, String> {
    Ok(Out::read(service(c.peer, "internet.tunnel_status", &json!({}))?, tr("read the internet tunnel's state")))
}

fn abrir(c: &Ctx, a: &Value) -> Result<Out, String> {
    let mut p = json!({"mode": if s(a, "modo")? == "nomeado" { "named" } else { "quick" }});
    if let Some(d) = a.get("dominio") {
        p["hostname"] = d.clone();
    }
    if let Some(i) = a.get("ociosidade_s") {
        p["idle_close_s"] = i.clone();
    }
    // the service asks the person (tunel.autorizar) before it starts anything, saying who asked
    p["creator"] = json!({"id": c.rule.name, "max_level": c.rule.max_level.name()});
    let v = service(c.peer, "internet.tunnel_open", &p)?;
    Ok(Out {
        summary: trf("internet tunnel opened: {url}", &[("url", v["url"].as_str().unwrap_or("").to_string())]),
        value: v,
        inverse: Some(json!({"ferramenta": "tunel.fechar", "argumentos": {}})),
    })
}

fn fechar(c: &Ctx, _: &Value) -> Result<Out, String> {
    let v = service(c.peer, "internet.tunnel_close_all", &json!({}))?;
    Ok(Out { value: v, summary: tr("internet tunnel closed").into(), inverse: None })
}

/// The approval itself is `needs_approval`'s; reaching here means the person said yes.
fn autorizar(_: &Ctx, a: &Value) -> Result<Out, String> {
    Ok(Out { value: json!({"autorizado": true}), summary: trf("the person allowed opening the internet tunnel ({mode})", &[("mode", s(a, "modo")?.to_string())]), inverse: None })
}

// ------------------------------------------------------------------ a aprovação

/// The tools that always ask, with their own text (never an "always allow").
pub fn asks(name: &str) -> bool {
    matches!(name, "gatilho_internet.criar" | "gatilho_internet.ativar" | "gatilho_internet.girar_segredo" | "gatilho_internet.editar" | "gatilho_internet.autorizar_nivel" | "gatilho_internet.autorizar_ampliacao" | "tunel.autorizar")
}

/// One line, no control characters, at most `max` characters: a value the model or the service wrote can never start
/// a line of its own in the approval.
fn one_line(v: &Value, max: usize) -> String {
    let t = match v {
        Value::String(t) => t.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    };
    let t: String = t.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.chars().count() > max { format!("{}…", t.chars().take(max).collect::<String>()) } else { t }
}

fn list_of(v: &Value) -> String {
    v.as_array().map(|a| a.iter().map(|x| one_line(x, 60)).collect::<Vec<_>>().join(", ")).unwrap_or_default()
}

/// What a trigger is, one line per fact (from the service's description of it).
/// The fields of a trigger as the service describes it, strictly: `signed` must be a boolean and `level` one of the four
/// levels. Anything else is an error, never a guess (a missing field must not show as "signed").
fn strict_facts(t: &Value) -> Result<(bool, String), String> {
    let signed = t["signed"].as_bool().ok_or_else(|| tr("the service did not say whether the trigger is signed").to_string())?;
    let level = t["level"].as_str().filter(|l| LEVELS.contains(l)).ok_or_else(|| tr("the service did not say the level of the trigger").to_string())?;
    Ok((signed, level.to_string()))
}

const LEVELS: [&str; 4] = ["observar", "usuario", "sistema", "total"];

/// Whether going from `from` to `to` raises what the flow may do (only a rise asks).
fn rises(from: &str, to: &str) -> bool {
    rank(to).unwrap_or(0) > rank(from).unwrap_or(0)
}

fn rank(level: &str) -> Option<u8> {
    LEVELS.iter().position(|l| *l == level).map(|i| i as u8)
}

/// The level an agent asks for: the highest the flow may use when started from outside. Above what the agent itself
/// may do is refused (never lowered quietly): a trigger is a way to run things at that level.
fn asked_level(args: &Value, rule: &crate::policy::CallerRule) -> Result<String, String> {
    let level = args.get("nivel").and_then(Value::as_str).unwrap_or("observar").to_string();
    let asked = rank(&level).ok_or_else(|| trf("unknown level {level}", &[("level", level.clone())]))?;
    if asked > rule.max_level as u8 {
        return Err(trf("the level {level} is above the {max} that {who} may use: a trigger cannot give a flow more than its creator has",
            &[("level", level), ("max", rule.max_level.name().to_string()), ("who", rule.name.clone())],
        ));
    }
    Ok(level)
}

/// What the person will see must be what was asked: the preview's signature and level are the arguments' (a public
/// trigger always runs at the lowest level).
fn check_preview_matches(args: &Value, preview: &Value) -> Result<(), String> {
    let (signed, level) = strict_facts(preview)?;
    let wants_public = args.get("sem_assinatura").and_then(Value::as_bool).unwrap_or(false);
    if signed == wants_public {
        return Err(tr("the preview does not match what was asked about the signature").into());
    }
    let wants = if wants_public { "observar".to_string() } else { args.get("nivel").and_then(Value::as_str).unwrap_or("observar").to_string() };
    if level != wants {
        return Err(tr("the preview does not match what was asked about the level").into());
    }
    Ok(())
}

fn trigger_lines(t: &Value) -> Result<Vec<String>, String> {
    let (signed, _) = strict_facts(t)?;
    let mut v = vec![if signed {
        tr("each call must carry an HMAC-SHA256 signature of its date and body, with a key of its own kept in the system vault; a repeated call is refused").to_string()
    } else {
        tr("PUBLIC, WITHOUT SIGNATURE: anyone who knows the address can start it (only at the lowest level, and the result is never sent back)").to_string()
    }];
    v.push(trf("the flow runs at the level {level}{note}", &[
        ("level", one_line(&t["level"], 20)),
        ("note", if t["level"] == "observar" { String::new() } else { format!(" — {}", tr("higher than the lowest, chosen by you for this trigger")) }),
    ]));
    v.push(trf("methods {methods}; body up to {bytes} bytes; {rate} calls per minute; types {types}", &[
        ("methods", list_of(&t["methods"])), ("bytes", one_line(&t["max_body_bytes"], 10)), ("rate", one_line(&t["rate_per_minute"], 6)), ("types", list_of(&t["content_types"])),
    ]));
    v.push(match t["respond"]["mode"].as_str() {
        Some("wait") => trf("the caller waits up to {s} s for the result of the flow", &[("s", one_line(&t["respond"]["timeout_s"], 6))]),
        _ => tr("the caller gets an answer at once with the number of the run").to_string(),
    });
    let ips = list_of(&t["allow_ips"]);
    v.push(if ips.is_empty() { tr("anyone with the signature may call (no list of addresses)").to_string() } else { trf("only these addresses may call: {ips}", &[("ips", ips)]) });
    v.push(trf("at most {n} runs waiting; the content of the call is external data and never becomes an argument of an action that changes the system", &[("n", one_line(&t["max_pending"], 6))]));
    Ok(v)
}

fn lines_text(head: String, lines: Vec<String>) -> String {
    std::iter::once(head).chain(lines.into_iter().map(|l| format!("• {l}"))).collect::<Vec<_>>().join("\n")
}

fn describe_tunnel(a: &Value) -> Result<String, String> {
    let mode = s(a, "modo")?;
    if s(a, "provedor")? != "cloudflare" {
        return Err(tr("only Cloudflare is supported as the tunnel provider").into());
    }
    let port = a["porta"].as_u64().filter(|p| (1..=65535).contains(p)).ok_or_else(|| tr("give the receiver's port").to_string())?;
    let mut lines = vec![trf("through Cloudflare: all the traffic of these calls passes through it (privacy and availability are theirs); the program is the pinned cloudflared of Kalango Remote", &[])];
    match mode {
        "rapido" => lines.push(tr("a quick tunnel: a random public address (trycloudflare.com), no account and no guarantee from Cloudflare; the address changes each time").to_string()),
        "nomeado" => {
            let d = a.get("dominio").and_then(Value::as_str).unwrap_or("");
            let ok = d.len() <= 253 && d.contains('.') && d.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-') && !d.starts_with(['.', '-']);
            if !ok {
                return Err(tr("give a valid domain name for the named tunnel").into());
            }
            lines.push(trf("your own domain {domain}, through a named tunnel (its token comes from the system vault and is never shown; the routes of your provider's dashboard are ignored: this computer serves only the receiver below and answers 404 to the rest)", &[("domain", d.to_string())]));
        }
        _ => return Err(tr("the tunnel mode is rapido or nomeado").into()),
    }
    lines.push(trf("what becomes reachable from the internet: only the receiver of the webhook triggers (this computer, port {port}), and only the triggers listed here", &[("port", port.to_string())]));
    match a.get("gatilhos").and_then(Value::as_array).filter(|g| !g.is_empty()) {
        Some(g) => {
            for t in g.iter().take(20) {
                lines.push(trf("trigger {id}: flow “{flow}”, {kind}, level {level}", &[
                    ("id", one_line(&t["id"], 40)),
                    ("flow", one_line(&t["flow"], 60)),
                    // (missing or not a boolean: shown as PUBLIC)
                    ("kind", if t["signed"].as_bool() == Some(true) { tr("signed").to_string() } else { tr("PUBLIC, unsigned").to_string() }),
                    ("level", one_line(&t["level"], 20)),
                ]));
            }
        }
        None => lines.push(tr("no webhook trigger is on: the address would answer nothing").to_string()),
    }
    match a.get("ociosidade_s").and_then(Value::as_u64) {
        Some(n) => lines.push(trf("it closes by itself after {n} minutes without use; the Close all tunnels button closes it at once", &[("n", (n / 60).max(1).to_string())])),
        None => lines.push(tr("it stays open until you close it (the Close all tunnels button closes it at once)").to_string()),
    }
    Ok(lines_text(tr("open a tunnel to the internet").to_string(), lines))
}

/// The text of the approval (what the log and a plain screen show): what the tests read.
#[cfg(test)]
pub fn needs_approval(peer: &Peer, rule: &crate::policy::CallerRule, name: &str, args: &Value) -> Result<Option<String>, String> {
    needs_approval_card(peer, rule, None, name, args).map(|o| o.map(|(text, _)| text))
}

/// Who is asking, as the cards say it. “You · flow editor” only when the caller is verified as the person's own
/// interface (the very program listed, outside the agents' slice, not tampered with): any other process of the same user
/// is shown by its real program and its rule, never as the person.
fn caller_label(peer: &Peer, rule: &crate::policy::CallerRule, cfg: Option<&crate::policy::Config>) -> String {
    if cfg.is_some_and(|c| crate::allowlist::is_person_ui(peer, &c.interface_da_pessoa, &c.interface_ambiente_permitido)) {
        return tr("you · flow editor").to_string();
    }
    let program = std::path::Path::new(&peer.exe).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
    one_line(&json!(format!("{program} · {}", rule.name)), 60)
}

/// Rises of level the person approved and that have not run yet: (uid, trigger, level asked, level it was from, when).
/// The run takes the level the approval was based on and hands it to the service as a precondition, so a level changed
/// between the question and the change is refused there, never applied.
static APPROVED_RISES: std::sync::Mutex<Vec<(u32, String, String, String, std::time::Instant)>> = std::sync::Mutex::new(Vec::new());
const APPROVAL_VALID_FOR: std::time::Duration = std::time::Duration::from_secs(600);

fn remember_rise(uid: u32, id: &str, to: &str, from: &str) {
    let mut v = APPROVED_RISES.lock().unwrap_or_else(|e| e.into_inner());
    v.retain(|(_, _, _, _, at)| at.elapsed() < APPROVAL_VALID_FOR);
    v.retain(|(u, i, t, _, _)| !(*u == uid && i == id && t == to));
    v.push((uid, id.to_string(), to.to_string(), from.to_string(), std::time::Instant::now()));
}

/// The level an approved rise was based on (once: taking it uses it up).
fn take_approved_rise(uid: u32, id: &str, to: &str) -> Option<String> {
    let mut v = APPROVED_RISES.lock().unwrap_or_else(|e| e.into_inner());
    v.retain(|(_, _, _, _, at)| at.elapsed() < APPROVAL_VALID_FOR);
    let i = v.iter().position(|(u, t, l, _, _)| *u == uid && t == id && l == to)?;
    Some(v.remove(i).3)
}

/// The person said yes to this card: what must be remembered for the run that follows (a rise of level, with the level the
/// answer was based on). A denial never gets here.
pub fn after_approval(who: &Who, name: &str, args: &Value, card: &Value) {
    if card["_esperado"].is_object() {
        let mut v = EXPECTED.lock().unwrap_or_else(|e| e.into_inner());
        v.retain(|(_, _, at)| at.elapsed() < APPROVAL_VALID_FOR);
        v.push((preapproval_key(who, name, args), card["_esperado"].clone(), std::time::Instant::now()));
    }
    if name != "gatilho_internet.editar" {
        return;
    }
    if let Some(a) = card["_subida"].as_array() {
        if let [id, to, from] = a.as_slice() {
            remember_rise(who.uid, id.as_str().unwrap_or(""), to.as_str().unwrap_or(""), from.as_str().unwrap_or(""));
        }
    }
}

/// What the card showed (level, signed, flow, addresses) for requests the person approved and that have not run yet: the run
/// hands it to the service, which refuses a trigger that is no longer that.
static EXPECTED: std::sync::Mutex<Vec<(String, Value, std::time::Instant)>> = std::sync::Mutex::new(Vec::new());

fn take_expected(c: &Ctx, name: &str, args: &Value) -> Option<Value> {
    let who = Who::of(c.peer, c.rule);
    let mut v = EXPECTED.lock().unwrap_or_else(|e| e.into_inner());
    v.retain(|(_, _, at)| at.elapsed() < APPROVAL_VALID_FOR);
    let key = preapproval_key(&who, name, args);
    let i = v.iter().position(|(k, _, _)| *k == key)?;
    Some(v.remove(i).1)
}

/// The card's facts for the lowered request: the same, with the lowest level.
pub fn lowered_card(card: &Value) -> Value {
    let mut c = card.clone();
    if c["_esperado"].is_object() {
        c["_esperado"]["level"] = json!("observar");
    }
    c
}

/// The same request with the level rewritten to the lowest, for the “only at the lowest level” answer. Only a creation
/// that asked for more than the lowest has one.
pub fn lowered(name: &str, args: &Value) -> Option<Value> {
    if name != "gatilho_internet.criar" || args.get("nivel").and_then(Value::as_str).is_none_or(|l| l == "observar") {
        return None;
    }
    let mut a = args.clone();
    a["nivel"] = json!("observar");
    Some(a)
}

/// Requests the person approved in their lowered form and that have not run yet: the second pass does not ask again.
static PREAPPROVED: std::sync::Mutex<Vec<(String, std::time::Instant)>> = std::sync::Mutex::new(Vec::new());

/// Who asked, for the answers that are kept for one request: the user, the process, and the rule it was let in by. Another
/// process of the same user, or the same program under another rule, does not use what was approved for this one.
pub struct Who {
    uid: u32,
    pid: i32,
    rule: String,
}

impl Who {
    pub fn of(peer: &Peer, rule: &crate::policy::CallerRule) -> Who {
        Who { uid: peer.uid, pid: peer.pid, rule: rule.name.clone() }
    }
}

fn preapproval_key(who: &Who, name: &str, args: &Value) -> String {
    format!("{}|{}|{}|{name}|{args}", who.uid, who.pid, who.rule)
}

pub fn preapprove(who: &Who, name: &str, args: &Value) {
    let mut v = PREAPPROVED.lock().unwrap_or_else(|e| e.into_inner());
    v.retain(|(_, at)| at.elapsed() < APPROVAL_VALID_FOR);
    v.push((preapproval_key(who, name, args), std::time::Instant::now()));
}

/// Uses up the approval of exactly this request, if there is one.
pub fn take_preapproved(who: &Who, name: &str, args: &Value) -> bool {
    let mut v = PREAPPROVED.lock().unwrap_or_else(|e| e.into_inner());
    v.retain(|(_, at)| at.elapsed() < APPROVAL_VALID_FOR);
    let key = preapproval_key(who, name, args);
    match v.iter().position(|(k, _)| *k == key) {
        Some(i) => {
            v.remove(i);
            true
        }
        None => false,
    }
}

/// A short label of a level, as the cards say it.
fn level_label(level: &str) -> String {
    match level {
        "observar" => tr("lowest (observe)").to_string(),
        "usuario" => tr("User").to_string(),
        "sistema" => tr("Administrator").to_string(),
        other => one_line(&json!(other), 20),
    }
}

/// One card of the approval screen (design page "37 · Gatilho pela internet": "abrir túnel público", "agente cria um
/// gatilho", "subir o nível"): the sentence, a warning with its title, the facts as rows, the footnote and the buttons.
fn card(sentence: String, warning: Option<(String, String, &str)>, rows: Vec<(String, String)>, footnote: String, risk_tone: &str, approve: String, deny: String) -> Value {
    json!({
        "tipo": "internet", "frase": sentence,
        "aviso": warning.map(|(title, text, tone)| json!({"titulo": title, "texto": text, "tom": tone})),
        "linhas": rows.into_iter().map(|(k, v)| json!([k, v])).collect::<Vec<_>>(),
        "rodape": footnote, "risco": risk_tone, "aprovar": approve, "negar": deny,
    })
}

fn tunnel_card(a: &Value) -> Result<Value, String> {
    let mode = s(a, "modo")?;
    let triggers: Vec<&Value> = a.get("gatilhos").and_then(Value::as_array).into_iter().flatten().take(20).collect();
    let names: Vec<String> = triggers.iter().map(|t| format!("“{}”", one_line(&t["flow"], 60))).collect();
    let sentence = match names.len() {
        0 => tr("Open a public tunnel? No trigger is on: the address would answer nothing.").to_string(),
        1 => trf("Open a public tunnel for the trigger {names}? Anyone with the address can call this flow while the tunnel is open.", &[("names", names.join(", "))]),
        _ => trf("Open a public tunnel for the triggers {names}? Anyone with the address can call these flows while the tunnel is open.", &[("names", names.join(", "))]),
    };
    // (a trigger that does not say it is signed is shown as public: the alarming reading is the safe one)
    let public = triggers.iter().filter(|t| t["signed"].as_bool() != Some(true)).count();
    let level = triggers.iter().filter_map(|t| t["level"].as_str()).max_by_key(|l| rank(l).unwrap_or(0)).unwrap_or("observar");
    let address = if mode == "nomeado" { one_line(&a["dominio"], 80) } else { tr("random, shown when it opens").to_string() };
    let kind = if mode == "nomeado" { tr("my domain (fixed)").to_string() } else { tr("quick tunnel (changes at each opening)").to_string() };
    let signature = if public > 0 { trf("NONE in {n} public trigger(s)", &[("n", public.to_string())]) } else { tr("required (HMAC)").to_string() };
    // who asked: an agent kalangod named, or a program of the person's through the service: never “you”, which only the
    // person's own interface is called (and the service cannot say that)
    let asked_by = match a.get("pedido_por").and_then(Value::as_str).filter(|x| !x.is_empty()) {
        Some(agent) => trf("the agent {agent}, through the service", &[("agent", one_line(&json!(agent), 40))]),
        None => tr("a program of yours, through the service").to_string(),
    };
    let lowest = level == "observar";
    Ok(card(
        sentence,
        Some((
            tr("Data from outside comes into your computer").to_string(),
            match (public > 0, lowest) {
                (true, true) => tr("Some calls need no signature: anyone can fire those flows. The data runs at the lowest level. The tunnel closes with the bar's button or in Settings › Tunnels.").to_string(),
                (false, true) => tr("Calls need the HMAC signature; the data runs at the lowest level. The tunnel closes with the bar's button or in Settings › Tunnels.").to_string(),
                (true, false) => trf("Some calls need no signature: anyone can fire those flows. The data runs at the {level} level at most. The tunnel closes with the bar's button or in Settings › Tunnels.", &[("level", level_label(level))]),
                (false, false) => trf("Calls need the HMAC signature; the data runs at the {level} level at most. The tunnel closes with the bar's button or in Settings › Tunnels.", &[("level", level_label(level))]),
            },
            if public > 0 { "coral" } else { "ouro" },
        )),
        vec![
            (tr("Address").to_string(), address),
            (tr("Type").to_string(), kind),
            (tr("Level").to_string(), level_label(level)),
            (tr("Signature").to_string(), signature),
            (tr("Asked by").to_string(), asked_by),
        ],
        tr("Valid only for this opening. There is no “always allow” for tunnels.").to_string(),
        if public > 0 { "coral" } else { "ouro" },
        tr("Open once").to_string(),
        tr("Deny").to_string(),
    ))
}

pub fn needs_approval_card(peer: &Peer, rule: &crate::policy::CallerRule, cfg: Option<&crate::policy::Config>, name: &str, args: &Value) -> Result<Option<(String, Value)>, String> {
    match name {
        "tunel.autorizar" => Ok(Some((describe_tunnel(args)?, tunnel_card(args)?))),
        "gatilho_internet.criar" => {
            // above the creator's own level: refused before anyone is asked (the service enforces it too)
            let level = asked_level(args, rule)?;
            let p = service(peer, "internet.trigger_preview", &config_for(args, rule))?;
            check_preview_matches(args, &p)?;
            let flow = one_line(&p["flow"], 80);
            let mut lines = trigger_lines(&p)?;
            lines.insert(0, trf("asked by {who}, for the level {level} of the flow when it is started from the internet", &[("who", one_line(&json!(rule.name), 40)), ("level", level.clone())]));
            let text = lines_text(
                trf("create an internet webhook trigger for the flow “{flow}” (created OFF, with a random address and its own key; turning it on and opening a tunnel are asked separately)", &[("flow", flow.clone())]),
                lines,
            );
            let (signed, _) = strict_facts(&p)?;
            let warning = (level != "observar").then(|| {
                (
                    tr("Request for a level above the lowest").to_string(),
                    trf("The agent asked for the {level} level of the flow when it starts from outside. Raising it later asks you again.", &[("level", level_label(&level))]),
                    "ouro",
                )
            });
            let warning = if signed { warning } else { Some((tr("Anyone with the address could fire this flow").to_string(), tr("Without a signature there is no way to know who called: the level stays at the lowest and the limit is 5 calls per minute.").to_string(), "coral")) };
            let alternative = (level != "observar" && signed).then(|| tr("Only at the lowest").to_string());
            let facts = json!({"level": level, "signed": signed, "flow_id": args.get("fluxo"), "allow_ips": p["allow_ips"]});
            let card = card(
                trf("The agent {agent} wants to create the internet trigger of the flow {flow}.", &[("agent", one_line(&json!(rule.name), 40)), ("flow", flow.clone())]),
                warning,
                vec![
                    (tr("Agent").to_string(), caller_label(peer, rule, cfg)),
                    (tr("Flow").to_string(), flow.clone()),
                    (tr("Trigger").to_string(), one_line(args.get("nome").unwrap_or(&json!(flow)), 80)),
                    (tr("Level asked").to_string(), level_label(&level)),
                    (tr("Signature").to_string(), if signed { tr("HMAC · new key").to_string() } else { tr("NONE (public)").to_string() }),
                    (tr("Answer").to_string(), if p["respond"]["mode"] == "wait" { tr("waits and gives the result of the flow to whoever calls").to_string() } else { tr("at once, with the number of the run").to_string() }),
                    (tr("Tunnel").to_string(), tr("opens later, with another approval").to_string()),
                ],
                tr("It is created off. Turning it on and opening the tunnel are asked separately.").to_string(),
                "ouro",
                tr("Approve as asked").to_string(),
                tr("Deny").to_string(),
            );
            let mut card = card;
            if let Some(a) = alternative {
                card["alternativa"] = json!(a);
            }
            card["_esperado"] = facts;
            Ok(Some((text, card)))
        }
        "gatilho_internet.editar" => {
            let id = s(args, "id")?;
            let to = asked_level(args, rule)?;
            let from = current_level(peer, id)?;
            // lowering never asks
            if !rises(&from, &to) {
                return Ok(None);
            }
            let list = service(peer, "internet.trigger_list", &json!({}))?;
            let t = list.as_array().and_then(|a| a.iter().find(|t| t["id"] == id)).ok_or_else(|| tr("no such internet trigger").to_string())?;
            let (signed, _) = strict_facts(t)?;
            if !signed {
                return Err(tr("a public trigger never goes above the lowest level").into());
            }
            let flow = service(peer, "internet.trigger_show", &json!({"id": id})).ok().and_then(|v| v["flow_id"].as_str().map(str::to_string)).unwrap_or_default();
            let text = trf("raise the level of the internet trigger {id} from {from} to {to}: data that comes from outside will be able to act in the system at that level", &[("id", one_line(&json!(id), 40)), ("from", from.clone()), ("to", to.clone())]);
            let mut card = card(
                trf("Raise the level of the trigger {id} from {from} to {to}?", &[("id", one_line(&json!(flow.is_empty().then_some(id).unwrap_or(&flow)), 60)), ("from", level_label(&from)), ("to", level_label(&to))]),
                Some((
                    tr("Data from outside will be able to act on the system").to_string(),
                    trf("With the {level} level, a request that arrives can do what that level does in this flow without asking again. The change goes to the Timeline and can be undone.", &[("level", level_label(&to))]),
                    "coral",
                )),
                vec![
                    (tr("Trigger").to_string(), one_line(&json!(id), 40)),
                    (tr("From → to").to_string(), format!("{} → {}", level_label(&from), level_label(&to))),
                    (tr("Signature").to_string(), tr("required (HMAC)").to_string()),
                    (tr("Asked by").to_string(), caller_label(peer, rule, cfg)),
                ],
                tr("Valid until you lower it again. A public trigger without signature never goes up in level.").to_string(),
                "coral",
                trf("Raise to {level}", &[("level", level_label(&to))]),
                tr("Cancel").to_string(),
            );
            // what the approval was based on: remembered by `after_approval`, only if the person says yes
            card["_subida"] = json!([id, to, from]);
            Ok(Some((text, card)))
        }
        // the service asks when nobody else did (a level set on its own socket): the same card, the same refusal of what
        // cannot be raised
        "gatilho_internet.autorizar_nivel" => {
            let (id, from, to) = (s(args, "id")?, s(args, "de")?, s(args, "para")?);
            if rank(to).is_none() || rank(from).is_none() || !rises(from, to) {
                return Err(tr("only a rise of level is asked").into());
            }
            let text = trf("raise the level of the internet trigger {id} from {from} to {to}: data that comes from outside will be able to act in the system at that level", &[("id", one_line(&json!(id), 40)), ("from", from.to_string()), ("to", to.to_string())]);
            let card = card(
                trf("Raise the level of the trigger {id} from {from} to {to}?", &[("id", one_line(&json!(id), 60)), ("from", level_label(from)), ("to", level_label(to))]),
                Some((
                    tr("Data from outside will be able to act on the system").to_string(),
                    trf("With the {level} level, a request that arrives can do what that level does in this flow without asking again. The change goes to the Timeline and can be undone.", &[("level", level_label(to))]),
                    "coral",
                )),
                vec![
                    (tr("Trigger").to_string(), one_line(&json!(id), 40)),
                    (tr("From → to").to_string(), format!("{} → {}", level_label(from), level_label(to))),
                    (tr("Asked by").to_string(), caller_label(peer, rule, cfg)),
                ],
                tr("Valid until you lower it again. A public trigger without signature never goes up in level.").to_string(),
                "coral",
                trf("Raise to {level}", &[("level", level_label(to))]),
                tr("Cancel").to_string(),
            );
            Ok(Some((text, card)))
        }
        // (the same card as the service's own question to turn it on: `enable_card`)
        "gatilho_internet.ativar" => enable_card(peer, rule, cfg, s(args, "id")?).map(Some),
        "gatilho_internet.girar_segredo" => {
            let id = s(args, "id")?;
            let text = trf("replace the key of the internet trigger {id}: the current key stops working at once, and the new one stays in the vault, never shown to the agent (to see a key, replace it yourself with kalango-fluxosd gatilho girar-segredo)", &[("id", one_line(&json!(id), 40))]);
            let card = card(
                trf("Replace the key of the trigger {id}?", &[("id", one_line(&json!(id), 60))]),
                Some((
                    tr("The current key stops working at once").to_string(),
                    tr("Whoever calls with the old key gets a refusal until they have the new one, which stays in the vault and is never shown to the agent.").to_string(),
                    "ouro",
                )),
                vec![(tr("Trigger").to_string(), one_line(&json!(id), 40)), (tr("Asked by").to_string(), caller_label(peer, rule, cfg))],
                tr("To see a key, replace it yourself with kalango-fluxosd gatilho girar-segredo.").to_string(),
                "ouro",
                tr("Replace key").to_string(),
                tr("Deny").to_string(),
            );
            Ok(Some((text, card)))
        }
        // the service asks before a trigger accepts more callers (made public, or its address list emptied or widened)
        "gatilho_internet.autorizar_ampliacao" => {
            let (id, what) = (s(args, "id")?, s(args, "o_que")?);
            if !["public", "addresses", "enable"].contains(&what) {
                return Err(tr("only “public” or “addresses” can be widened").into());
            }
            // (turning it on, asked by the service: the very card of `gatilho_internet.ativar`, from what the trigger is now)
            if what == "enable" {
                return enable_card(peer, rule, cfg, id).map(Some);
            }
            let public = what == "public";
            let text = trf("let the internet trigger {id} accept more callers ({what})", &[("id", one_line(&json!(id), 40)), ("what", what.to_string())]);
            let card = card(
                trf("Let the trigger {id} accept more callers?", &[("id", one_line(&json!(id), 60))]),
                Some((
                    if public { tr("Anyone with the address could fire this flow").to_string() } else { tr("Callers from other addresses could fire this flow").to_string() },
                    if public {
                        tr("Without a signature there is no way to know who called: the level stays at the lowest and the limit is 5 calls per minute.").to_string()
                    } else {
                        tr("The list of allowed addresses grows or goes away: callers that were refused until now can fire this flow.").to_string()
                    },
                    "coral",
                )),
                vec![
                    (tr("Trigger").to_string(), one_line(&json!(id), 40)),
                    (tr("Change").to_string(), if public { tr("public, with no signature").to_string() } else { tr("any address, or more addresses").to_string() }),
                    (tr("Asked by").to_string(), caller_label(peer, rule, cfg)),
                ],
                tr("It cannot be done while a tunnel is open.").to_string(),
                "coral",
                tr("Allow").to_string(),
                tr("Cancel").to_string(),
            );
            Ok(Some((text, card)))
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_args() -> Value {
        json!({"modo": "rapido", "provedor": "cloudflare", "dominio": "", "porta": 40123, "ociosidade_s": 7200,
            "gatilhos": [{"id": "gi-1", "flow": "Receber pedidos", "signed": true, "level": "observar"}]})
    }

    #[test]
    fn the_tunnel_approval_says_what_is_reachable_through_whom_and_which_triggers() {
        let t = describe_tunnel(&open_args()).unwrap();
        for must in ["Cloudflare", "trycloudflare.com", "40123", "Receber pedidos", "gi-1", "observar", "120 minutes", "Close all tunnels"] {
            assert!(t.contains(must), "{must} missing in\n{t}");
        }
        let named = describe_tunnel(&json!({"modo": "nomeado", "provedor": "cloudflare", "dominio": "hooks.example.test", "porta": 5000, "gatilhos": []})).unwrap();
        assert!(named.contains("hooks.example.test") && named.contains("would answer nothing") && named.contains("until you close it"), "{named}");
    }

    #[test]
    fn a_public_trigger_is_called_out_in_the_approval() {
        let mut a = open_args();
        a["gatilhos"][0]["signed"] = json!(false);
        assert!(describe_tunnel(&a).unwrap().contains("PUBLIC, unsigned"));
    }

    #[test]
    fn odd_input_is_an_error_never_a_panic_and_never_a_line_of_its_own() {
        assert!(describe_tunnel(&json!({"modo": "rapido", "provedor": "tailscale", "porta": 1})).is_err());
        assert!(describe_tunnel(&json!({"modo": "rapido", "provedor": "cloudflare", "porta": 0})).is_err());
        assert!(describe_tunnel(&json!({"modo": "x", "provedor": "cloudflare", "porta": 80})).is_err());
        for d in ["", "semponto", "a b.test", "x.test\nAprovar tudo", "-x.test"] {
            assert!(describe_tunnel(&json!({"modo": "nomeado", "provedor": "cloudflare", "dominio": d, "porta": 80})).is_err(), "{d:?}");
        }
        let mut a = open_args();
        a["gatilhos"][0]["flow"] = json!("Bom\n• tudo aprovado\u{7}");
        let t = describe_tunnel(&a).unwrap();
        assert!(!t.contains("\n• tudo aprovado"), "{t}");
        assert_eq!(describe_tunnel(&json!([1, 2])).unwrap_err().is_empty(), false);
    }

    #[test]
    fn the_trigger_lines_show_the_signature_the_level_and_the_limits() {
        let t = json!({"signed": true, "level": "usuario", "methods": ["POST"], "max_body_bytes": 65536, "rate_per_minute": 30,
            "content_types": ["application/json"], "respond": {"mode": "wait", "timeout_s": 20}, "allow_ips": ["203.0.113.0/24"], "max_pending": 20});
        let text = trigger_lines(&t).unwrap().join("\n");
        for must in ["HMAC-SHA256", "usuario", "higher than the lowest", "POST", "65536", "30 calls per minute", "20 s", "203.0.113.0/24", "never becomes an argument"] {
            assert!(text.contains(must), "{must} missing in\n{text}");
        }
        let low = trigger_lines(&json!({"signed": true, "level": "observar", "respond": {"mode": "accepted"}})).unwrap().join("\n");
        assert!(!low.contains("higher than the lowest") && low.contains("at once"));
    }

    #[test]
    fn the_four_doors_always_ask_and_the_closing_never_does() {
        for n in ["gatilho_internet.criar", "gatilho_internet.ativar", "gatilho_internet.girar_segredo", "tunel.autorizar"] {
            assert!(asks(n), "{n}");
        }
        for n in ["tunel.fechar", "tunel.fechar_todos", "gatilho_internet.desativar", "gatilho_internet.listar", "tunel.estado"] {
            assert!(!asks(n), "{n}");
        }
        // and by their contract: the doors need the system level, the closings need none
        let all = tools();
        let level = |n: &str| all.iter().find(|t| t.name == n).unwrap().level;
        assert_eq!(level("gatilho_internet.criar"), Level::Sistema);
        assert_eq!(level("tunel.autorizar"), Level::Sistema);
        assert_eq!(level("tunel.fechar_todos"), Level::Observar);
    }

    fn rule(max: Level) -> crate::policy::CallerRule {
        crate::policy::CallerRule { name: "agente-de-teste".into(), uid: None, exe_prefix: None, cgroup_prefix: None, max_level: max, allowed_paths: vec![] }
    }

    #[test]
    fn an_agent_cannot_ask_for_a_level_above_its_own() {
        let a = |nivel: &str| json!({"fluxo": "f", "nivel": nivel});
        assert_eq!(asked_level(&json!({"fluxo": "f"}), &rule(Level::Sistema)).unwrap(), "observar");
        assert_eq!(asked_level(&a("sistema"), &rule(Level::Sistema)).unwrap(), "sistema");
        assert_eq!(asked_level(&a("usuario"), &rule(Level::Sistema)).unwrap(), "usuario");
        let e = asked_level(&a("total"), &rule(Level::Sistema)).unwrap_err();
        assert!(e.contains("above") && e.contains("agente-de-teste"), "{e}");
        assert!(asked_level(&a("sistema"), &rule(Level::Usuario)).is_err());
        assert!(asked_level(&a("banana"), &rule(Level::Total)).is_err());
        // refused before the service or the person is even reached (no service here: the answer is the refusal itself)
        let peer = crate::peer::Peer { pid: 1, uid: 1000, gid: 1000, exe: "/bin/x".into(), cgroup: String::new(), home: "/nonexistent".into(), passwd_home: "/nonexistent".into(), user: "t".into(), has_tty: false };
        let e = needs_approval(&peer, &rule(Level::Sistema), "gatilho_internet.criar", &a("total")).unwrap_err();
        assert!(e.contains("above"), "{e}");
    }

    #[test]
    fn a_preview_with_a_missing_field_is_an_error_and_never_shows_as_signed() {
        for p in [json!({"level": "observar"}), json!({"signed": "true", "level": "observar"}), json!({"signed": true}), json!({"signed": true, "level": "root"}), json!({})] {
            assert!(strict_facts(&p).is_err(), "{p}");
            assert!(trigger_lines(&p).is_err(), "{p}");
        }
        // in the tunnel's list, a missing field reads as public, never as signed
        let mut a = open_args();
        a["gatilhos"][0].as_object_mut().unwrap().remove("signed");
        let t = describe_tunnel(&a).unwrap();
        assert!(t.contains("PUBLIC, unsigned") && !t.contains(", signed,"), "{t}");
    }

    #[test]
    fn what_the_person_sees_is_what_was_asked() {
        let ok = json!({"signed": true, "level": "usuario"});
        assert!(check_preview_matches(&json!({"nivel": "usuario"}), &ok).is_ok());
        // asked public, shown signed (or the other way round): refused
        assert!(check_preview_matches(&json!({"sem_assinatura": true}), &ok).is_err());
        assert!(check_preview_matches(&json!({"nivel": "usuario"}), &json!({"signed": false, "level": "observar"})).is_err());
        // only the signature differs (the level agrees): still refused, both ways
        assert!(check_preview_matches(&json!({"sem_assinatura": true}), &json!({"signed": true, "level": "observar"})).is_err());
        assert!(check_preview_matches(&json!({}), &json!({"signed": false, "level": "observar"})).is_err());
        // asked a level, shown another
        assert!(check_preview_matches(&json!({"nivel": "sistema"}), &ok).is_err());
        assert!(check_preview_matches(&json!({}), &ok).is_err());
        // a public trigger is at the lowest level whatever was written
        assert!(check_preview_matches(&json!({"sem_assinatura": true, "nivel": "sistema"}), &json!({"signed": false, "level": "observar"})).is_ok());
    }

    #[test]
    fn the_tunnel_card_has_the_rows_of_the_design_and_calls_a_public_trigger_by_its_name() {
        let c = tunnel_card(&open_args()).unwrap();
        assert_eq!(c["tipo"], "internet");
        assert!(c["frase"].as_str().unwrap().contains("Receber pedidos"));
        let rows: Vec<String> = c["linhas"].as_array().unwrap().iter().map(|r| r[0].as_str().unwrap().to_string()).collect();
        assert_eq!(rows, vec!["Address", "Type", "Level", "Signature", "Asked by"]);
        assert_eq!(c["aprovar"], "Open once", "no “always” on this card");
        assert_eq!(c["aviso"]["tom"], "ouro");
        // a public trigger, or one that does not say, turns the card red and says so in the signature row
        for gatilho in [json!({"id": "g", "flow": "F", "signed": false, "level": "observar"}), json!({"id": "g", "flow": "F", "level": "observar"})] {
            let mut a = open_args();
            a["gatilhos"] = json!([gatilho]);
            let c = tunnel_card(&a).unwrap();
            assert_eq!(c["aviso"]["tom"], "coral");
            assert!(c["linhas"][3][1].as_str().unwrap().contains("NONE"), "{c}");
        }
        // the highest level of the triggers is the one shown
        let mut a = open_args();
        a["gatilhos"] = json!([{"id": "a", "flow": "A", "signed": true, "level": "observar"}, {"id": "b", "flow": "B", "signed": true, "level": "sistema"}]);
        assert!(tunnel_card(&a).unwrap()["linhas"][2][1].as_str().unwrap().contains("Administrator"));
    }

    #[test]
    fn a_card_never_carries_a_line_of_its_own_from_a_name() {
        let mut a = open_args();
        a["gatilhos"][0]["flow"] = json!("Bom\nSenha: 1234\u{7}");
        let c = tunnel_card(&a).unwrap().to_string();
        assert!(!c.contains("\\nSenha") || !c.contains("\n"), "{c}");
        assert!(!c.contains('\u{7}'));
    }

    #[test]
    fn an_approved_rise_is_used_once_for_exactly_what_was_approved() {
        remember_rise(1000, "gi-x", "usuario", "observar");
        assert_eq!(take_approved_rise(1000, "gi-x", "sistema"), None, "another level was not approved");
        assert_eq!(take_approved_rise(1001, "gi-x", "usuario"), None, "another user's approval is not this one's");
        assert_eq!(take_approved_rise(1000, "gi-y", "usuario"), None, "another trigger");
        assert_eq!(take_approved_rise(1000, "gi-x", "usuario").as_deref(), Some("observar"), "the level the person was shown");
        assert_eq!(take_approved_rise(1000, "gi-x", "usuario"), None, "once");
        remember_rise(1000, "gi-z", "usuario", "observar");
        remember_rise(1000, "gi-z", "usuario", "usuario");
        assert_eq!(take_approved_rise(1000, "gi-z", "usuario").as_deref(), Some("usuario"));
        assert_eq!(take_approved_rise(1000, "gi-z", "usuario"), None);
    }

    #[test]
    fn a_process_of_the_users_is_never_shown_as_the_person() {
        let peer = crate::peer::Peer { pid: 1, uid: 1000, gid: 1000, exe: "/tmp/qualquer-script".into(), cgroup: String::new(), home: "/x".into(), passwd_home: "/x".into(), user: "t".into(), has_tty: false };
        let who = caller_label(&peer, &rule(Level::Sistema), None);
        assert!(who.contains("qualquer-script") && who.contains("agente-de-teste"), "{who}");
        assert_ne!(who, tr("you · flow editor"));
    }

    #[test]
    fn the_lowered_request_is_the_same_one_with_the_lowest_level_and_is_approved_once() {
        let a = json!({"fluxo": "f", "nivel": "usuario", "ips": ["203.0.113.0/24"]});
        let l = lowered("gatilho_internet.criar", &a).unwrap();
        assert_eq!(l["nivel"], "observar");
        assert_eq!(l["ips"], a["ips"], "nothing else changes");
        // nothing to lower: the lowest already, or another tool
        assert!(lowered("gatilho_internet.criar", &json!({"fluxo": "f"})).is_none());
        assert!(lowered("gatilho_internet.criar", &json!({"fluxo": "f", "nivel": "observar"})).is_none());
        assert!(lowered("gatilho_internet.editar", &a).is_none());
        let who = |uid: u32, pid: i32, rule: &str| Who { uid, pid, rule: rule.into() };
        preapprove(&who(1000, 77, "agente"), "gatilho_internet.criar", &l);
        assert!(!take_preapproved(&who(1001, 77, "agente"), "gatilho_internet.criar", &l), "another user");
        assert!(!take_preapproved(&who(1000, 78, "agente"), "gatilho_internet.criar", &l), "another process of the same user");
        assert!(!take_preapproved(&who(1000, 77, "outra-regra"), "gatilho_internet.criar", &l), "the same program under another rule");
        assert!(!take_preapproved(&who(1000, 77, "agente"), "gatilho_internet.criar", &a), "the original request was not the one approved");
        assert!(take_preapproved(&who(1000, 77, "agente"), "gatilho_internet.criar", &l));
        assert!(!take_preapproved(&who(1000, 77, "agente"), "gatilho_internet.criar", &l), "once");
    }

    #[test]
    fn a_rise_is_remembered_only_after_the_person_said_yes() {
        let card = json!({"tipo": "internet", "_subida": ["gi-r1", "usuario", "observar"]});
        // asked (the card exists), not answered: nothing to take
        assert_eq!(take_approved_rise(4242, "gi-r1", "usuario"), None);
        // answered no: nothing is remembered (after_approval is never reached); answered yes: it is, once
        let me = Who { uid: 4242, pid: 1, rule: "r".into() };
        after_approval(&me, "gatilho_internet.editar", &json!({}), &card);
        assert_eq!(take_approved_rise(4242, "gi-r1", "usuario").as_deref(), Some("observar"));
        assert_eq!(take_approved_rise(4242, "gi-r1", "usuario"), None);
        // another tool's card carries no rise
        after_approval(&me, "gatilho_internet.criar", &json!({}), &card);
        assert_eq!(take_approved_rise(4242, "gi-r1", "usuario"), None);
    }

    #[test]
    fn the_tunnel_card_warns_with_the_highest_level_listed_and_says_who_asked() {
        let mut a = open_args();
        // the lowest: the old wording
        let card = tunnel_card(&a).unwrap();
        assert!(card["aviso"]["texto"].as_str().unwrap().contains("lowest level"), "{card}");
        // a trigger above the lowest: the warning says that level, not the lowest
        a["gatilhos"] = json!([{"id": "gi-1", "flow": "A", "signed": true, "level": "observar"}, {"id": "gi-2", "flow": "B", "signed": true, "level": "sistema"}]);
        let card = tunnel_card(&a).unwrap();
        let text = card["aviso"]["texto"].as_str().unwrap();
        assert!(text.contains("Administrator level at most") && !text.contains("lowest level"), "{text}");
        let rows = card["linhas"].as_array().unwrap();
        let asked = rows.iter().find(|r| r[0] == "Asked by").expect("a row for who asked");
        assert_eq!(asked[1], "a program of yours, through the service");
        assert!(!asked[1].as_str().unwrap().to_lowercase().starts_with("you"), "never “you”: {asked}");
        // an agent named by kalangod
        a["pedido_por"] = json!("assistente");
        let card = tunnel_card(&a).unwrap();
        let rows = card["linhas"].as_array().unwrap();
        assert!(rows.iter().any(|r| r[0] == "Asked by" && r[1].as_str().unwrap().contains("assistente")), "{card}");
    }

    #[test]
    fn the_service_is_spoken_to_over_its_own_socket_and_only_when_it_is_the_persons() {
        use std::os::unix::net::UnixListener;
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("fluxos.sock");
        let l = UnixListener::bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            use std::io::{BufRead, Write};
            for answer in [json!({"jsonrpc": "2.0", "id": 1, "result": {"ok": true}}), json!({"jsonrpc": "2.0", "id": 1, "error": {"code": -32000, "message": "no way"}})] {
                let (s, _) = l.accept().unwrap();
                let mut line = String::new();
                std::io::BufReader::new(s.try_clone().unwrap()).read_line(&mut line).unwrap();
                let req: Value = serde_json::from_str(&line).unwrap();
                assert_eq!(req["method"], "internet.trigger_list");
                let mut s = s;
                writeln!(s, "{answer}").unwrap();
            }
        });
        let me = nix::unistd::getuid().as_raw();
        let p = path.to_str().unwrap();
        assert_eq!(call_socket(p, Some(me), "internet.trigger_list", &json!({})).unwrap(), json!({"ok": true}));
        assert_eq!(call_socket(p, Some(me), "internet.trigger_list", &json!({})).unwrap_err(), "no way");
        server.join().unwrap();
        // the other end of what was connected must be this person (the path may have been swapped after it was looked at)
        let probe = UnixListener::bind(d.path().join("sonda.sock")).unwrap();
        let conn = std::os::unix::net::UnixStream::connect(d.path().join("sonda.sock")).unwrap();
        let _accepted = probe.accept().unwrap();
        assert!(peer_is(&conn, me), "this very program is the one on the other end");
        assert!(!peer_is(&conn, me + 1), "and nobody else");
        // a socket of someone else is never spoken to; nor a plain file, nor a link
        assert!(call_socket(p, Some(me + 1), "x", &json!({})).unwrap_err().contains("not the person's own"));
        std::fs::write(d.path().join("arquivo"), "x").unwrap();
        assert!(call_socket(d.path().join("arquivo").to_str().unwrap(), Some(me), "x", &json!({})).is_err());
        std::os::unix::fs::symlink(&path, d.path().join("link")).unwrap();
        assert!(call_socket(d.path().join("link").to_str().unwrap(), Some(me), "x", &json!({})).is_err());
    }

    #[test]
    fn the_widening_tool_always_asks_and_only_two_things_can_be_widened() {
        assert!(asks("gatilho_internet.autorizar_ampliacao"));
        let all = tools();
        let t = all.iter().find(|t| t.name == "gatilho_internet.autorizar_ampliacao").unwrap();
        assert_eq!(t.level, Level::Sistema);
    }

    fn short() -> Deadlines {
        Deadlines { connect: std::time::Duration::from_millis(300), whole: std::time::Duration::from_millis(600) }
    }

    #[test]
    fn what_is_connected_must_be_the_persons_even_when_the_file_looked_right() {
        use std::os::unix::net::UnixListener;
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("s.sock");
        let _l = UnixListener::bind(&path).unwrap();
        let me = nix::unistd::getuid().as_raw();
        // the file is mine (it passes the look) but the process on the other end is said to be someone else's: refused before
        // anything is sent
        let e = call_socket_with(path.to_str().unwrap(), Some(me), Some(me + 1), "x", &json!({}), short()).unwrap_err();
        assert!(e.contains("not the person's own"), "{e}");
    }

    #[test]
    fn a_service_that_never_accepts_does_not_hold_the_caller() {
        use std::os::unix::net::UnixListener;
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("cheio.sock");
        let _l = UnixListener::bind(&path).unwrap();
        // the queue fills up (nobody accepts), and a plain connect would then wait for as long as nobody does
        let p = path.to_str().unwrap();
        let mut held = Vec::new();
        while let Ok(c) = connect_by(p, std::time::Instant::now()) {
            held.push(c);
            assert!(held.len() < 100_000, "the queue never filled");
        }
        let t0 = std::time::Instant::now();
        let e = call_socket_with(p, None, None, "x", &json!({}), short()).unwrap_err();
        assert!(e.contains("busy") || e.contains("in time"), "{e}");
        assert!(t0.elapsed() < std::time::Duration::from_secs(3), "{:?}", t0.elapsed());
        drop(held);
    }

    #[test]
    fn a_reply_that_trickles_is_cut_at_one_deadline_for_the_whole_of_it() {
        use std::os::unix::net::UnixListener;
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("gota.sock");
        let l = UnixListener::bind(&path).unwrap();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            let (mut s, _) = l.accept().unwrap();
            let mut b = [0u8; 4096];
            let _ = s.read(&mut b);
            // a byte every 100 ms, never a line: each read comes in time, the whole never does
            for _ in 0..200 {
                if s.write_all(b"x").is_err() {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        });
        let t0 = std::time::Instant::now();
        let e = call_socket_with(path.to_str().unwrap(), None, None, "x", &json!({}), short()).unwrap_err();
        assert!(e.contains("in time"), "{e}");
        assert!(t0.elapsed() < std::time::Duration::from_secs(3), "{:?}", t0.elapsed());
    }

    #[test]
    fn a_reply_over_a_mebibyte_is_refused() {
        use std::os::unix::net::UnixListener;
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("grande.sock");
        let l = UnixListener::bind(&path).unwrap();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            let (mut s, _) = l.accept().unwrap();
            let mut b = [0u8; 4096];
            let _ = s.read(&mut b);
            let big = vec![b'a'; 2 << 20];
            let _ = s.write_all(&big);
            let _ = s.write_all(b"\n");
        });
        let long = Deadlines { connect: std::time::Duration::from_secs(2), whole: std::time::Duration::from_secs(20) };
        let e = call_socket_with(path.to_str().unwrap(), None, None, "x", &json!({}), long).unwrap_err();
        assert!(e.contains("too large"), "{e}");
    }

    #[test]
    fn a_lowered_card_keeps_its_facts_at_the_lowest_level() {
        let card = json!({"_esperado": {"level": "usuario", "signed": true, "flow_id": "f"}});
        let c = lowered_card(&card);
        assert_eq!(c["_esperado"]["level"], "observar");
        assert_eq!(c["_esperado"]["signed"], true);
        assert_eq!(card["_esperado"]["level"], "usuario", "the original is untouched");
        assert!(lowered_card(&json!({}))["_esperado"].is_null());
    }

    #[test]
    fn only_a_rise_of_level_asks() {
        assert!(rises("observar", "usuario") && rises("usuario", "sistema") && rises("observar", "sistema"));
        assert!(!rises("usuario", "observar") && !rises("sistema", "usuario") && !rises("usuario", "usuario"));
        assert!(asks("gatilho_internet.editar"));
        assert_eq!(tools().iter().find(|t| t.name == "gatilho_internet.editar").unwrap().level, Level::Sistema);
    }

    #[test]
    fn what_goes_back_to_an_agent_has_no_key_and_no_ready_signature() {
        let v = scrub(json!({"id": "gi-1", "secret": "abc", "secret_shown_once": true, "curl": "curl ...", "nested": [{"secret": "x", "ok": 1}]}));
        assert_eq!(v, json!({"id": "gi-1", "nested": [{"ok": 1}]}));
    }

    /// The service as a stand-in on this thread: the triggers it lists (the test changes them through `triggers`) and
    /// the calls it got.
    struct Service {
        triggers: std::rc::Rc<std::cell::RefCell<Value>>,
        calls: std::rc::Rc<std::cell::RefCell<Vec<(String, Value)>>>,
    }

    impl Service {
        fn start(triggers: Value) -> Service {
            let t = std::rc::Rc::new(std::cell::RefCell::new(triggers));
            let calls = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
            let (t2, c2) = (t.clone(), calls.clone());
            STAND_IN.with(|s| {
                *s.borrow_mut() = Some(Box::new(move |m: &str, p: &Value| {
                    c2.borrow_mut().push((m.to_string(), p.clone()));
                    match m {
                        "internet.trigger_list" => Ok(t2.borrow().clone()),
                        "internet.trigger_show" => Ok(json!({"id": p["id"], "flow_id": "receber-pedidos"})),
                        "internet.trigger_set_enabled" => Ok(json!({"id": p["id"], "enabled": true})),
                        _ => Err(format!("the stand-in has no {m}")),
                    }
                }))
            });
            Service { triggers: t, calls }
        }
    }

    impl Drop for Service {
        fn drop(&mut self) {
            STAND_IN.with(|s| *s.borrow_mut() = None);
        }
    }

    fn trigger(id: &str, signed: bool, level: &str) -> Value {
        json!({"id": id, "path": format!("/h/{id}-8f3a"), "signed": signed, "level": level, "methods": ["POST"], "max_body_bytes": 65536,
            "rate_per_minute": 5, "content_types": ["application/json"], "respond": {"mode": "accepted"}, "allow_ips": ["203.0.113.0/24"], "max_pending": 20})
    }

    fn the_service_program() -> Peer {
        Peer { pid: 4321, uid: 1000, gid: 1000, exe: "/usr/lib/kalango/kalango-fluxosd".into(), cgroup: String::new(), home: "/x".into(), passwd_home: "/x".into(), user: "t".into(), has_tty: false }
    }

    fn row(card: &Value, k: &str) -> String {
        card["linhas"].as_array().unwrap().iter().find(|r| r[0] == k).and_then(|r| r[1].as_str()).unwrap_or("").to_string()
    }

    #[test]
    fn turning_on_a_public_trigger_says_so_with_its_level_and_address_whoever_asks() {
        let _s = Service::start(json!([trigger("gi-pub", false, "observar")]));
        let peer = the_service_program();
        // the agent's tool and the service's own question (the person's program turned it on over the raw socket)
        for (name, args) in [("gatilho_internet.ativar", json!({"id": "gi-pub"})), ("gatilho_internet.autorizar_ampliacao", json!({"id": "gi-pub", "o_que": "enable"}))] {
            let (text, card) = needs_approval_card(&peer, &rule(Level::Sistema), None, name, &args).unwrap().unwrap();
            assert_eq!(card["aviso"]["tom"], "coral", "{name}: {card}");
            assert_eq!(card["risco"], "coral", "{name}");
            assert!(card["aviso"]["texto"].as_str().unwrap().contains("no signature"), "{name}: {card}");
            assert_eq!(row(&card, "Signature"), "NONE (public)", "{name}");
            assert_eq!(row(&card, "Level"), "lowest (observe)", "{name}");
            assert_eq!(row(&card, "Address"), "/h/gi-pub-8f3a", "{name}");
            assert!(text.contains("PUBLIC, WITHOUT SIGNATURE") && text.contains("/h/gi-pub-8f3a"), "{name}: {text}");
            assert_eq!(card["_esperado"]["signed"], false, "{name}");
        }
        // a signed one above the lowest: gold, the key, its level
        let _s = Service::start(json!([trigger("gi-s", true, "usuario")]));
        let (_, card) = needs_approval_card(&peer, &rule(Level::Sistema), None, "gatilho_internet.autorizar_ampliacao", &json!({"id": "gi-s", "o_que": "enable"})).unwrap().unwrap();
        assert_eq!(card["aviso"]["tom"], "ouro");
        assert_eq!(row(&card, "Signature"), "required (HMAC)");
        assert_eq!(row(&card, "Level"), "User");
        // a trigger the service does not describe strictly is an error, never a card that reads "signed"
        let _s = Service::start(json!([{"id": "gi-x", "level": "observar"}]));
        assert!(needs_approval_card(&peer, &rule(Level::Sistema), None, "gatilho_internet.autorizar_ampliacao", &json!({"id": "gi-x", "o_que": "enable"})).is_err());
        assert!(needs_approval_card(&peer, &rule(Level::Sistema), None, "gatilho_internet.autorizar_ampliacao", &json!({"id": "nenhum", "o_que": "enable"})).is_err());
    }

    #[test]
    fn both_ways_of_turning_a_trigger_on_show_the_same_card() {
        let peer = the_service_program();
        for t in [trigger("gi-1", false, "observar"), trigger("gi-1", true, "observar"), trigger("gi-1", true, "sistema")] {
            let _s = Service::start(json!([t]));
            let agent = needs_approval_card(&peer, &rule(Level::Sistema), None, "gatilho_internet.ativar", &json!({"id": "gi-1"})).unwrap().unwrap();
            let service = needs_approval_card(&peer, &rule(Level::Sistema), None, "gatilho_internet.autorizar_ampliacao", &json!({"id": "gi-1", "o_que": "enable"})).unwrap().unwrap();
            assert_eq!(agent, service, "{t}");
        }
    }

    #[test]
    fn turning_on_a_trigger_that_changed_after_the_answer_is_refused() {
        let svc = Service::start(json!([trigger("gi-s", true, "observar")]));
        let (peer, r) = (the_service_program(), rule(Level::Sistema));
        let (d, cfg) = (tempfile::tempdir().unwrap(), crate::policy::Config::system_default());
        let tl = crate::timeline::Timeline::open(d.path()).unwrap();
        let c = Ctx { cfg: &cfg, peer: &peer, rule: &r, timeline: &tl, lan: None, state: None, startup_ok: vec![] };
        let (name, args) = ("gatilho_internet.autorizar_ampliacao", json!({"id": "gi-s", "o_que": "enable"}));
        let approve = || {
            let (_, card) = needs_approval_card(&peer, &r, None, name, &args).unwrap().unwrap();
            after_approval(&Who::of(&peer, &r), name, &args, &card);
            card
        };
        // as the person saw it: yes, with the card's facts going back as the precondition
        let card = approve();
        let out = autorizar_ampliacao(&c, &args).unwrap();
        assert_eq!(out.value["esperado"], card["_esperado"]);
        assert_eq!(out.value["autorizado"], true);
        // made public, raised, its address list emptied or its address changed between the answer and the run: refused
        for (k, v) in [("signed", json!(false)), ("level", json!("sistema")), ("allow_ips", json!([])), ("path", json!("/h/outro"))] {
            *svc.triggers.borrow_mut() = json!([trigger("gi-s", true, "observar")]);
            approve();
            svc.triggers.borrow_mut()[0][k] = v;
            let e = autorizar_ampliacao(&c, &args).err().expect("refused");
            assert!(e.contains("changed after you answered"), "{k}: {e}");
        }
        // gone meanwhile: refused
        *svc.triggers.borrow_mut() = json!([trigger("gi-s", true, "observar")]);
        approve();
        *svc.triggers.borrow_mut() = json!([]);
        assert!(autorizar_ampliacao(&c, &args).is_err());
        // with no card behind it (nothing remembered, or used up already), it is never a yes
        *svc.triggers.borrow_mut() = json!([trigger("gi-s", true, "observar")]);
        assert!(autorizar_ampliacao(&c, &args).err().expect("refused").contains("missing"));
        approve();
        autorizar_ampliacao(&c, &args).unwrap();
        assert!(autorizar_ampliacao(&c, &args).is_err(), "once");
        // the agent's tool hands the very facts of its card to the service, which refuses a trigger that is no longer that
        let card = {
            let (_, card) = needs_approval_card(&peer, &r, None, "gatilho_internet.ativar", &json!({"id": "gi-s"})).unwrap().unwrap();
            after_approval(&Who::of(&peer, &r), "gatilho_internet.ativar", &json!({"id": "gi-s"}), &card);
            card
        };
        ativar(&c, &json!({"id": "gi-s"})).unwrap();
        let calls = svc.calls.borrow();
        let set = calls.iter().rev().find(|(m, _)| m == "internet.trigger_set_enabled").unwrap();
        assert_eq!(set.1["expected"], card["_esperado"]);
        assert_eq!(set.1["expected"]["flow_id"], "receber-pedidos");
    }

    #[test]
    fn the_arguments_become_the_services_and_never_ask_for_the_key() {
        let p = service_config(&json!({"fluxo": "f1", "resposta": "esperar", "espera_s": 30, "nivel": "usuario", "ips": ["203.0.113.0/24"], "sem_assinatura": false}));
        assert_eq!(p["flow_id"], "f1");
        assert_eq!(p["respond"], "wait");
        assert_eq!(p["max_level"], "usuario");
        assert!(p.get("reveal_secret").is_none());
        // the schema does not even offer it
        assert!(criar_schema()["properties"].get("reveal_secret").is_none() && criar_schema()["properties"].get("segredo").is_none());
    }
}
