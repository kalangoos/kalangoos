//! Aprovações pendentes e concessões do nível total.
//!
//! A aprovação é presencial: vem de `kalangod aprovar <id>` num terminal do usuário ou da tela de aprovação do
//! sistema (`kalango-aprovacao`, uma superfície do compositor que nenhum agente desenha). Um agente não consegue
//! aprovar a si mesmo: o aprovador precisa ser outro processo, do mesmo usuário ou root, fora do cgroup dos agentes,
//! e ter terminal de controle ou ser um aprovador gráfico confiável (executável listado na configuração, de dono root
//! e sem escrita para grupo ou outros — um agente não consegue trocá-lo nem se passar por ele, porque o caminho do
//! executável vem do kernel).

use kalango_i18n::{tr, trf};
use crate::peer::Peer;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize)]
pub struct Pending {
    pub id: u64,
    pub tool: String,
    pub level: String,
    pub args: Value,
    pub caller: String,
    pub uid: u32,
    pub pid: i32,
    pub reason: String,
    /// O que a ferramenta faz, efeito e se é reversível: o que a tela de aprovação mostra.
    pub descricao: String,
    pub efeito: String,
    pub reversivel: bool,
    pub criado: u64,
    pub prazo_s: u64,
    /// the caller's executable, as the kernel reports it ("always" answers are tied to it)
    pub exe: String,
    #[serde(skip)]
    pub decision: Option<bool>,
    /// A change to files that run at the person's next login (`startup.rs`): which ones and the exact diff, for the
    /// approval screen.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub startup: Option<Value>,
    /// "Always" is refused for this request (the screen shows no such button): each change to a startup file is
    /// seen before it happens.
    pub no_always: bool,
    /// What the approval screen shows for a request that is not a tool call as such: a folder an agent asks its
    /// commands to write in (level "pasta": path, summary, first entries) or a command that installs for the person
    /// (level "instalar": where it writes and why).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cartao: Option<Value>,
}

#[derive(Default)]
struct Inner {
    next: u64,
    pending: HashMap<u64, Pending>,
    /// muda a cada pedido novo ou decidido (para quem espera mudanças)
    generation: u64,
    /// uid → instante em que a concessão do nível total expira
    total_grants: HashMap<u32, Instant>,
    /// "always" answers: the same agent asking for the same system action with the same arguments is not asked again
    standing: Vec<Standing>,
    /// requests answered "always" whose asker keeps the answer itself (a project folder), not as a standing answer
    always: std::collections::HashSet<u64>,
}

/// An "always" answer from the person (system level only; the total level is always temporary). It covers one
/// exact request: the same executable (as the kernel reports it), tool and arguments.
#[derive(Debug, Clone, Serialize, serde::Deserialize, PartialEq)]
pub struct Standing {
    pub uid: u32,
    pub exe: String,
    pub tool: String,
    /// canonical JSON of the arguments
    pub args: String,
    pub criado: u64,
}

#[derive(Default)]
pub struct Approvals {
    inner: Mutex<Inner>,
    cv: Condvar,
    /// where the "always" answers are kept (none in memory-only use)
    store: Option<PathBuf>,
}

fn canonical(v: &Value) -> String {
    // serde_json keeps maps sorted (no preserve_order): the same arguments give the same text
    v.to_string()
}

pub enum Outcome {
    Approved,
    Denied,
    Timeout,
}

impl Approvals {
    /// With the "always" answers kept in `state_dir/aprovacoes-sempre.json`.
    pub fn open(state_dir: &std::path::Path) -> Approvals {
        let store = state_dir.join("aprovacoes-sempre.json");
        let standing: Vec<Standing> = std::fs::read_to_string(&store).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        Approvals { inner: Mutex::new(Inner { standing, ..Default::default() }), cv: Condvar::new(), store: Some(store) }
    }

    fn save(&self, standing: &[Standing]) {
        if let Some(p) = &self.store {
            let tmp = p.with_extension("json.tmp");
            if std::fs::write(&tmp, serde_json::to_string_pretty(standing).unwrap_or_default()).is_ok() {
                let _ = std::fs::rename(&tmp, p);
            }
        }
    }

    /// The person already answered "always" to exactly this request.
    pub fn is_standing(&self, uid: u32, exe: &str, tool: &str, args: &Value) -> bool {
        let a = canonical(args);
        self.inner.lock().unwrap().standing.iter().any(|s| s.uid == uid && s.exe == exe && s.tool == tool && s.args == a)
    }

    /// The person's "always" answers (all of them for root).
    pub fn standing_for(&self, uid: u32) -> Vec<Standing> {
        self.inner.lock().unwrap().standing.iter().filter(|s| uid == 0 || s.uid == uid).cloned().collect()
    }

    /// Takes back an "always" answer (the n-th of the person's list). Taking one back only removes permission.
    pub fn revoke(&self, uid: u32, n: usize) -> Result<Standing, String> {
        let mut g = self.inner.lock().unwrap();
        let idx: Vec<usize> = g.standing.iter().enumerate().filter(|(_, s)| uid == 0 || s.uid == uid).map(|(i, _)| i).collect();
        let i = *idx.get(n).ok_or(tr("there is no such “always” answer"))?;
        let s = g.standing.remove(i);
        let all = g.standing.clone();
        drop(g);
        self.save(&all);
        Ok(s)
    }

    /// Registra o pedido e espera a decisão (ou o tempo esgotar).
    pub fn request(&self, p: Pending, timeout: Duration) -> Outcome {
        self.request_with_always(p, timeout).0
    }

    /// As `request`, also saying whether the person answered "always" (a folder: kept among the project folders).
    pub fn request_with_always(&self, mut p: Pending, timeout: Duration) -> (Outcome, bool) {
        let id = {
            let mut g = self.inner.lock().unwrap();
            g.next += 1;
            p.id = g.next;
            p.criado = crate::timeline::now();
            p.prazo_s = timeout.as_secs();
            g.pending.insert(p.id, p.clone());
            g.generation += 1;
            self.cv.notify_all();
            p.id
        };
        eprintln!("kalangod: {}", trf("approval #{id} pending: {caller} asks for {tool} ({reason}) — `kalangod aprovar {id}` or `kalangod negar {id}`", &[("id", id.to_string()), ("caller", p.caller.clone()), ("tool", p.tool.clone()), ("reason", p.reason.clone())]));
        let deadline = Instant::now() + timeout;
        let mut g = self.inner.lock().unwrap();
        loop {
            if let Some(d) = g.pending.get(&id).and_then(|p| p.decision) {
                g.pending.remove(&id);
                let always = g.always.remove(&id);
                g.generation += 1;
                self.cv.notify_all();
                return (if d { Outcome::Approved } else { Outcome::Denied }, d && always);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                g.pending.remove(&id);
                g.generation += 1;
                self.cv.notify_all();
                return (Outcome::Timeout, false);
            }
            g = self.cv.wait_timeout(g, left).unwrap().0;
        }
    }

    pub fn list(&self) -> Vec<Pending> {
        let mut v: Vec<Pending> = self.inner.lock().unwrap().pending.values().cloned().collect();
        v.sort_by_key(|p| p.id);
        v
    }

    /// Espera a lista mudar (ou `max` passar). Devolve a geração atual e os pedidos que ainda esperam decisão.
    pub fn wait_change(&self, since: u64, max: Duration) -> (u64, Vec<Pending>) {
        let deadline = Instant::now() + max;
        let mut g = self.inner.lock().unwrap();
        while g.generation == since {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            g = self.cv.wait_timeout(g, left).unwrap().0;
        }
        let mut v: Vec<Pending> = g.pending.values().filter(|p| p.decision.is_none()).cloned().collect();
        v.sort_by_key(|p| p.id);
        (g.generation, v)
    }

    /// Decide um pedido. Devolve erro se o aprovador não puder decidir.
    #[allow(clippy::too_many_arguments)]
    pub fn decide(&self, id: u64, approve: bool, always: bool, approver: &Peer, requires_tty: bool, trusted: &[std::path::PathBuf], grant_total: Duration) -> Result<String, String> {
        let mut g = self.inner.lock().unwrap();
        let p = g.pending.get(&id).cloned().ok_or(trf("approval #{id} does not exist or was already decided", &[("id", id.to_string())]))?;
        // one decision per request: an answer that is already there is never overwritten (a second one, however it comes,
        // could turn a refusal into an approval, or make "only at the lowest" an approval of the level asked)
        if p.decision.is_some() {
            return Err(trf("approval #{id} does not exist or was already decided", &[("id", id.to_string())]).into());
        }
        if approver.pid == p.pid {
            return Err(tr("a process cannot approve its own request").into());
        }
        if approver.uid != 0 && approver.uid != p.uid {
            return Err(tr("only the person themselves or the administrator approves").into());
        }
        if approver.cgroup.contains("/kalango-agentes.slice") {
            return Err(tr("agents cannot approve requests").into());
        }
        if requires_tty && !approver.has_tty && !trusted_approver(&approver.exe, trusted) {
            return Err(tr("the approval has to be made by you: in a terminal or on the system approval screen (in person)").into());
        }
        // a card that offers an alternative way to approve (the internet trigger's “only at the lowest level”): that button
        // answers "always" in the protocol, but it is not the standing answer: it is kept for this request only
        let alt = p.cartao.as_ref().is_some_and(|c| c["alternativa"].is_string());
        if always && p.no_always && !alt {
            return Err(tr("this request cannot be answered with “always”: each change is approved on its own").into());
        }
        if always && alt && !approve {
            return Err(tr("the alternative answer approves; to refuse, deny").into());
        }
        if always && !alt && (!approve || !matches!(p.level.as_str(), "sistema" | "pasta")) {
            return Err(tr("“always” is only for approving system changes; full access is always temporary").into());
        }
        g.pending.get_mut(&id).unwrap().decision = Some(approve);
        if approve && p.level == "total" {
            g.total_grants.insert(p.uid, Instant::now() + grant_total);
        }
        let mut saved = None;
        if always && (p.level == "pasta" || alt) {
            g.always.insert(id);
        } else if always {
            let s = Standing { uid: p.uid, exe: p.exe.clone(), tool: p.tool.clone(), args: canonical(&p.args), criado: crate::timeline::now() };
            if !g.standing.contains(&s) {
                g.standing.push(s);
            }
            saved = Some(g.standing.clone());
        }
        self.cv.notify_all();
        drop(g);
        if let Some(all) = saved {
            self.save(&all);
            return Ok(trf("#{id} approved, and from now on this same request is not asked again", &[("id", id.to_string())]));
        }
        Ok(if approve { trf("#{id} approved", &[("id", id.to_string())]) } else { trf("#{id} denied", &[("id", id.to_string())]) })
    }

    pub fn has_total_grant(&self, uid: u32) -> bool {
        let mut g = self.inner.lock().unwrap();
        match g.total_grants.get(&uid) {
            Some(t) if *t > Instant::now() => true,
            Some(_) => {
                g.total_grants.remove(&uid);
                false
            }
            None => false,
        }
    }

    pub fn total_grant_left(&self, uid: u32) -> Option<u64> {
        let g = self.inner.lock().unwrap();
        g.total_grants.get(&uid).and_then(|t| t.checked_duration_since(Instant::now())).map(|d| d.as_secs())
    }
}

/// O executável do aprovador está na lista e ninguém além do dono (root, ou o próprio kalangod em desenvolvimento)
/// consegue alterá-lo.
pub fn trusted_approver(exe: &str, trusted: &[std::path::PathBuf]) -> bool {
    use std::os::unix::fs::MetadataExt;
    let exe = std::path::Path::new(exe);
    if !trusted.iter().any(|t| std::fs::canonicalize(t).map(|c| c == exe).unwrap_or(false)) {
        return false;
    }
    let owner_ok = |m: &std::fs::Metadata| m.uid() == 0 || m.uid() == nix::unistd::geteuid().as_raw();
    let Ok(m) = std::fs::metadata(exe) else { return false };
    // o arquivo e a pasta onde ele está: nenhum dos dois pode ser trocado por outro usuário
    let dir_ok = exe.parent().and_then(|d| std::fs::metadata(d).ok()).map(|d| owner_ok(&d) && not_writable_by_others(&d)).unwrap_or(false);
    owner_ok(&m) && not_writable_by_others(&m) && dir_ok
}

/// Nobody but its owner can change it: no write for others, and no write for a group other than root's (members of
/// root's group can change root's files anyway — a system image with /usr at 775 root:root is no weaker).
fn not_writable_by_others(m: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    m.mode() & 0o002 == 0 && (m.mode() & 0o020 == 0 || m.gid() == 0)
}

#[cfg(test)]
mod decision_tests {
    use super::*;

    fn peer(pid: i32, uid: u32) -> Peer {
        Peer { pid, uid, gid: uid, exe: "/usr/bin/kalango-aprovacao".into(), cgroup: "/user.slice".into(), home: "/tmp".into(), passwd_home: "/tmp".into(), user: "teste".into(), has_tty: true }
    }

    fn pending(a: &Approvals, id: u64) {
        let p = Pending {
            id, tool: "gatilho_internet.criar".into(), level: "sistema".into(), args: json_null(), caller: "agente".into(), uid: 1000, pid: 4000,
            reason: "x".into(), descricao: String::new(), efeito: String::new(), reversivel: true, criado: 0, prazo_s: 0, exe: "/usr/bin/agente".into(),
            decision: None, startup: None, no_always: true, cartao: Some(serde_json::json!({"alternativa": "Only at the lowest"})),
        };
        a.inner.lock().unwrap().pending.insert(id, p);
    }

    fn json_null() -> Value {
        Value::Null
    }

    /// A request has one decision: the second answer (a refusal after the alternative approval, or the other way round) is
    /// refused and does not overwrite what the person chose first.
    #[test]
    fn a_request_that_was_decided_cannot_be_decided_again() {
        let a = Approvals::default();
        pending(&a, 1);
        let me = peer(5000, 1000);
        let none: &[PathBuf] = &[];
        // the alternative approval (the "always" of the protocol on a card that offers one)
        assert!(a.decide(1, true, true, &me, false, none, Duration::from_secs(1)).is_ok());
        assert_eq!(a.inner.lock().unwrap().pending[&1].decision, Some(true));
        // a refusal afterwards: refused, and the approval stays
        let e = a.decide(1, false, false, &me, false, none, Duration::from_secs(1)).unwrap_err();
        assert!(e.contains("already decided"), "{e}");
        assert_eq!(a.inner.lock().unwrap().pending[&1].decision, Some(true));
        // and a denial first stays a denial
        pending(&a, 2);
        assert!(a.decide(2, false, false, &me, false, none, Duration::from_secs(1)).is_ok());
        assert!(a.decide(2, true, false, &me, false, none, Duration::from_secs(1)).is_err());
        assert_eq!(a.inner.lock().unwrap().pending[&2].decision, Some(false));
    }
}
