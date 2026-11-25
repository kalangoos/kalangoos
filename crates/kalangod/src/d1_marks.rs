//! "Aprovar sozinho" (Permissões › Aprovação pelo d1, design page 31): the agents' engine lets a narrow kind of action
//! go ahead without asking when the d1 calls it safe, and tells kalangod just before the call so that the timeline
//! says "aprovado pelo d1 · 94 %". The engine is not trusted with that claim: kalangod keeps the mark only if the
//! person's own setting (the protected permissions file, never the request) is on, the action is one of the narrow
//! list of `kalango_d1::auto_eligible` (judged here from the tool's own contract, not from what the engine says it
//! is), no hard rule applies, the confidence reaches the person's minimum and the reason is one of the d1's safe ones.
//! A mark is for one call of one process, for a minute.

use crate::peer::Peer;
use crate::policy::{expand, Effect};
use crate::server::State;
use kalango_d1::{ActionInfo, Reason, Setting, Side};
use serde_json::{json, Value};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const LIFE: Duration = Duration::from_secs(60);
const MOST: usize = 32;

struct Mark {
    pid: i32,
    tool: String,
    args: String,
    mark: Value,
    at: Instant,
}

#[derive(Default)]
pub struct Marks {
    marks: Mutex<Vec<Mark>>,
    /// per person: the denials in a row (on the approval screen, or undoing an action the d1 approved)
    streak: Mutex<std::collections::HashMap<u32, u32>>,
}

/// Only the agents' engine announces: the confidence is its own report, so nothing else is believed. By the full path
/// of the installed engine (the configuration lists a development session's own): a file merely named
/// `kalango-agente` anywhere else is not it.
pub fn is_engine(exe: &str, engines: &[std::path::PathBuf]) -> bool {
    !exe.is_empty() && engines.iter().any(|e| e == std::path::Path::new(exe))
}

impl Marks {
    /// The person denied (a request on the approval screen) or undid an action the d1 had approved.
    pub fn person_denied(&self, uid: u32) {
        *self.streak.lock().unwrap().entry(uid).or_insert(0) += 1;
    }

    /// The person approved something: the count starts again.
    pub fn person_approved(&self, uid: u32) {
        self.streak.lock().unwrap().remove(&uid);
    }

    /// Three denials in a row: no mark is kept until the person approves something.
    pub fn refusing(&self, uid: u32) -> bool {
        kalango_d1::asks_again(self.streak.lock().unwrap().get(&uid).copied().unwrap_or(0))
    }

    fn put(&self, pid: i32, tool: &str, args: &Value, mark: Value) {
        let mut g = self.marks.lock().unwrap();
        g.retain(|m| m.at.elapsed() < LIFE);
        if g.len() >= MOST {
            g.remove(0);
        }
        g.push(Mark { pid, tool: tool.into(), args: args.to_string(), mark, at: Instant::now() });
    }

    /// The mark for this call, once (taken, so it covers one call only).
    pub fn take(&self, pid: i32, tool: &str, args: &Value) -> Option<Value> {
        let a = args.to_string();
        let mut g = self.marks.lock().unwrap();
        g.retain(|m| m.at.elapsed() < LIFE);
        let i = g.iter().position(|m| m.pid == pid && m.tool == tool && m.args == a)?;
        Some(g.remove(i).mark)
    }
}

/// The person's setting, read from their protected permissions file (not through a link).
fn setting(st: &State, peer: &Peer) -> Setting {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let p = expand(&st.cfg.startup_setting, &peer.home);
    let Ok(f) = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW).open(p) else { return Setting::default() };
    let mut text = String::new();
    if f.take(1 << 20).read_to_string(&mut text).is_err() {
        return Setting::default();
    }
    Setting::parse(&serde_json::from_str(&text).unwrap_or_default())
}

/// The mark to keep for an announcement, or None. Pure: the setting, the tool's contract and the file's state come in.
pub fn validate(setting: &Setting, a: &ActionInfo, home: &str, confidence: u64, reason: &str) -> Option<Value> {
    let reason = Reason::from_key(reason).filter(|r| r.side() == Side::Safe)?;
    if !setting.enabled || confidence > 100 || confidence < setting.min_confidence as u64 {
        return None;
    }
    if kalango_d1::hard_rule(a).is_some() || !kalango_d1::auto_eligible(a, home) {
        return None;
    }
    Some(json!({"confianca": confidence, "minimo": setting.min_confidence, "motivo": reason.key()}))
}

/// Keeps the mark when it is valid; says whether it did.
pub fn announce(st: &State, peer: &Peer, name: &str, args: &Value, confidence: u64, reason: &str) -> bool {
    if !is_engine(&peer.exe, &st.cfg.motor_do_agente) || st.d1_marks.refusing(peer.uid) {
        return false;
    }
    let Some(tool) = crate::tools::find(name) else { return false };
    let home = peer.home.display().to_string();
    let new_file = name == "arquivos.escrever" && args["caminho"].as_str().is_some_and(|p| {
        std::fs::symlink_metadata(crate::policy::normalize(&expand(p, &peer.home))).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
    });
    let info = ActionInfo {
        tool: name.into(), args: args.clone(), agent: String::new(), level: tool.level.name().into(),
        effect: match tool.effect { Effect::Leitura => "leitura", Effect::Escreve => "escreve", Effect::Destrutiva => "destrutiva" }.into(),
        reversible: tool.reversible, description: String::new(), startup: false, new_file, home: home.clone(),
    };
    match validate(&setting(st, peer), &info, &home, confidence, reason) {
        Some(mark) => {
            st.d1_marks.put(peer.pid, name, args, mark);
            true
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(tool: &str, args: Value) -> ActionInfo {
        ActionInfo { tool: tool.into(), args, agent: String::new(), level: "usuario".into(), effect: "escreve".into(), reversible: true, description: String::new(), startup: false, new_file: false, home: String::new() }
    }

    #[test]
    fn a_mark_needs_the_setting_the_list_and_the_confidence() {
        let on = Setting { enabled: true, min_confidence: 90, denials: 0 };
        let a = info("arquivos.lixeira", json!({"caminho": "~/Downloads/a.pdf"}));
        assert_eq!(validate(&on, &a, "/home/ana", 94, "trash"), Some(json!({"confianca": 94, "minimo": 90, "motivo": "trash"})));
        assert_eq!(validate(&Setting::default(), &a, "/home/ana", 94, "trash"), None, "off");
        assert_eq!(validate(&on, &a, "/home/ana", 89, "trash"), None, "below the minimum");
        assert_eq!(validate(&on, &a, "/home/ana", 101, "trash"), None);
        assert_eq!(validate(&on, &a, "/home/ana", 94, "deletes"), None, "a reason of the dangerous side");
        assert_eq!(validate(&on, &a, "/home/ana", 94, "<b>x</b>"), None);
        for never in [info("pacotes.instalar", json!({"nomes": ["htop"]})), info("arquivos.lixeira", json!({"caminho": "/etc/hosts"})), info("arquivos.escrever", json!({"caminho": "~/.bashrc", "conteudo": "x"}))] {
            assert_eq!(validate(&on, &never, "/home/ana", 99, "trash"), None, "{}", never.tool);
        }
    }

    #[test]
    fn only_the_engine_announces() {
        let installed = vec![std::path::PathBuf::from("/usr/libexec/kalango/kalango-agente")];
        assert!(is_engine("/usr/libexec/kalango/kalango-agente", &installed));
        // a program of the user's merely named like the engine is not it
        assert!(!is_engine("/home/x/kalango-agente", &installed));
        assert!(!is_engine("/tmp/kalango-agente", &installed));
        assert!(!is_engine("/usr/bin/python3", &installed));
        assert!(!is_engine("", &installed));
        // a development session lists its own
        let dev = vec![std::path::PathBuf::from("/home/x/target/debug/kalango-agente")];
        assert!(is_engine("/home/x/target/debug/kalango-agente", &dev) && !is_engine("/usr/libexec/kalango/kalango-agente", &dev));
    }

    #[test]
    fn three_denials_in_a_row_refuse_marks_until_an_approval() {
        let m = Marks::default();
        assert!(!m.refusing(1000));
        m.person_denied(1000);
        m.person_denied(1000);
        assert!(!m.refusing(1000));
        m.person_denied(1000);
        assert!(m.refusing(1000) && !m.refusing(1001), "per person");
        m.person_approved(1000);
        assert!(!m.refusing(1000));
    }

    #[test]
    fn a_mark_covers_one_call_of_one_process() {
        let m = Marks::default();
        let args = json!({"caminho": "~/a"});
        m.put(10, "arquivos.lixeira", &args, json!({"confianca": 95}));
        assert_eq!(m.take(11, "arquivos.lixeira", &args), None, "another process");
        assert_eq!(m.take(10, "arquivos.lixeira", &json!({"caminho": "~/b"})), None, "other arguments");
        assert_eq!(m.take(10, "arquivos.lixeira", &args), Some(json!({"confianca": 95})));
        assert_eq!(m.take(10, "arquivos.lixeira", &args), None, "once");
        for i in 0..100 {
            m.put(i, "t", &json!(i), json!(i));
        }
        assert!(m.marks.lock().unwrap().len() <= MOST);
    }
}
