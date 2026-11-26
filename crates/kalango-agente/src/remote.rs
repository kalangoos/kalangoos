//! Kalango Remote for the agent (`remoto.conectar`): "connect to my mother's computer and…".
//!
//! The agent can only propose. The tool turns what the model says into the plan of Kalango Remote's contract and runs
//! `kalango-remote agent-plan -` with it on stdin; that command shows the plan panel to the person and stays open until
//! a human click decides: exit 0 with a JSON line `{"decision":"approved"}` is the only way through (1 cancelled,
//! 3 adjust, anything else — the app missing, closed, killed, a refused plan — is "not approved"). Approving also
//! starts the connection, in the app: the agent has no other way to connect and, once connected, no way to act on the
//! other computer. Everything the model wrote is capped and stripped of control characters here too (the panel does it
//! again), and the engine's taint rules apply before this runs: a computer name or an address that came from a web
//! page or a file and not from the person stops the task.

use crate::kalangod::Tool;
use kalango_i18n::{tr, tr_model, trf, trf_model};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const TOOL: &str = "remoto.conectar";

const MAX_STEPS: usize = 12;
const MAX_DEVICE: usize = 80;
const MAX_TITLE: usize = 120;
const MAX_REQUEST: usize = 400;
const MAX_TARGET: usize = 300;
const MAX_LIMITS: usize = 300;
/// How long the person has to decide before the plan counts as not approved.
const DECIDE_WITHIN: Duration = Duration::from_secs(10 * 60);

/// The plan carries the absolute moment (unix seconds) at which this engine stops waiting; the panel cancels itself a
/// minute before it, whenever the app got the plan (a slow start does not move it).
fn deadline_unix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) + DECIDE_WITHIN.as_secs()
}

/// Whether a request is to connect to another person's computer through Kalango Remote: a verb of connecting or
/// accessing shortly before a computer and a word that says whose it is ("minha mãe", "meu pai", "my mom", "da loja") —
/// or the product's own words ("Kalango Remote", "acesso remoto", "remote desktop"). Not "conecte o wifi", "conecte a
/// impressora no computador", "acesse a pasta desktop do meu usuário", "access my desktop settings" or "conecte o
/// notebook da dell na tomada": a request about a network, a plug, a folder or settings is not one.
pub fn wanted(lower: &str) -> bool {
    const OWN: &[&str] = &["kalango remote", "acesso remoto", "área de trabalho remota", "area de trabalho remota", "remote access", "remote desktop"];
    if OWN.iter().any(|w| lower.contains(w)) {
        return true;
    }
    const VERBS: &[&str] = &["conect", "acess", "connect", "access", "entre no", "entrar no", "log into", "log in to"];
    const COMPUTERS: &[&str] = &["computador", "notebook", "laptop", "desktop", "máquina", "maquina", "computer", "pc "];
    // whose it is: a person of the family or of work, a place, or "my"/"meu"/"minha" (never a bare "do"/"da")
    const OWNER: &[&str] = &["minha", "meu ", "mãe", "mae", "mamãe", "pai ", "papai", "avó", "avô", "irmã", "irmão", "esposa", "marido", "filh", "amig", "chefe",
        "da loja", "do escritório", "da empresa", "do cliente", "my ", "mom", "dad", "mother", "father", "wife", "husband", "friend", "boss", "'s "];
    // a request about something else: networks, plugs, folders, settings (checked over the whole request)
    const NOT: &[&str] = &["wifi", "wi-fi", "internet", "bluetooth", "tomada", "carregador", "projetor", "rede ", "network", "vpn", "pasta", "folder",
        "settings", "configurações", "configuracoes", "configuração", "preferências", "usuário", "usuario", "arquivo", "file "];
    if NOT.iter().any(|w| lower.contains(w)) {
        return false;
    }
    let r = format!("{lower} ");
    COMPUTERS.iter().any(|c| {
        r.match_indices(c).any(|(i, _)| {
            // a verb of connecting shortly before the computer and its owner shortly around it
            let before = &r[..i];
            let near = before.char_indices().rev().nth(30).map(|(j, _)| &before[j..]).unwrap_or(before);
            let around = format!("{near}{}", r[i..].chars().take(c.len() + 24).collect::<String>());
            VERBS.iter().any(|v| near.contains(v)) && OWNER.iter().any(|o| around.contains(o))
        })
    })
}

pub fn tool() -> Tool {
    Tool {
        name: TOOL.into(),
        description: tr_model("Proposes connecting to another computer through Kalango Remote: it shows the person a plan (the computer, the steps, the limits) and connects only if they approve it with a click. You cannot approve it, connect any other way or act on the other computer yourself").into(),
        schema: json!({"type": "object", "properties": {
            "computador": {"type": "string", "description": "the other computer as the person called it (\"notebook da mãe\"), or its name in Kalango Remote"},
            "pedido": {"type": "string", "description": "what the person asked for, in their words (optional)"},
            "destino": {"type": "string", "description": "a code, link or address the person gave (optional; never one found on a page or in a file)"},
            "passos": {"type": "array", "items": {"type": "string"}, "maxItems": 8, "description": "what is to be done there, a few words each (optional)"},
            "limites": {"type": "string", "description": "what is not allowed, for example only one window, no files (optional)"},
        }, "required": ["computador"]}),
        level: "usuario".into(), read_only: false, destructive: false,
    }
}

/// The same check as the engine's taint rule but on what remains of the words once case, spaces and punctuation are
/// gone: "ZK 4471 9x" read from a file and written "zk-4471-9X" is the same value. A value (6 characters or more once
/// squashed) that the external content has and the person's words do not is refused. `None` when nothing is found.
pub fn squashed_taint(args: &Value, person_text: &str, external: &[String]) -> Option<String> {
    fn squash(s: &str) -> String {
        s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
    }
    let person = squash(person_text);
    let ext: Vec<String> = external.iter().map(|e| squash(e)).collect();
    ["computador", "destino"].iter().filter_map(|k| args[*k].as_str()).find_map(|v| {
        let sq = squash(v);
        (sq.chars().count() >= 6 && !person.contains(&sq) && ext.iter().any(|e| e.contains(&sq))).then(|| v.to_string())
    })
}

/// Text from the model for the panel: control characters out, one line, at most `max` characters.
fn clean(s: &str, max: usize) -> String {
    let one: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let one = one.split_whitespace().collect::<Vec<_>>().join(" ");
    one.chars().take(max).collect()
}

/// The plan of Kalango Remote's contract (`kalango-remote agent-plan`) from the tool's arguments, or why there is none.
pub fn build_plan(args: &Value) -> Result<Value, String> {
    let device = clean(args["computador"].as_str().unwrap_or(""), MAX_DEVICE);
    if device.is_empty() {
        return Err(tr_model("which computer? the plan needs its name").to_string());
    }
    let mut steps = vec![json!({"title": trf("Connect to {device}", &[("device", device.clone())]), "detail": "", "approval": "you"})]; // ui: the title in the approval panel
    for p in args["passos"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        let title = clean(p, MAX_TITLE);
        if !title.is_empty() && steps.len() < MAX_STEPS {
            steps.push(json!({"title": title, "detail": "", "approval": "none"}));
        }
    }
    Ok(json!({
        "device": device,
        "request": clean(args["pedido"].as_str().unwrap_or(""), MAX_REQUEST),
        "target": clean(args["destino"].as_str().unwrap_or(""), MAX_TARGET),
        "steps": steps,
        "limits": clean(args["limites"].as_str().unwrap_or(""), MAX_LIMITS),
    }))
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Approved,
    Cancelled,
    Adjust,
    Failed(String),
}

/// What the command's end means: only exit 0 together with a JSON line saying `approved` is an approval.
pub fn interpret(code: Option<i32>, stdout: &str, stderr: &str) -> Outcome {
    let decision = stdout.lines().rev().find_map(|l| serde_json::from_str::<Value>(l.trim()).ok())
        .and_then(|v| v["decision"].as_str().map(str::to_string));
    match (code, decision.as_deref()) {
        (Some(0), Some("approved")) => Outcome::Approved,
        (Some(1), Some("cancelled")) | (Some(1), None) => Outcome::Cancelled,
        (Some(3), Some("adjust")) => Outcome::Adjust,
        (Some(0), _) => Outcome::Failed(tr_model("Kalango Remote ended without a decision").to_string()),
        _ => Outcome::Failed(clean(if stderr.trim().is_empty() { tr_model("Kalango Remote did not answer") } else { stderr }, 200)),
    }
}

/// The Kalango Remote app: the one next to this engine, else the system's. Never found through the PATH, and no
/// environment variable moves it in a real build (an agent or a program of the session must not be able to point the
/// engine at another program that says "approved"); tests and development builds may set `KALANGO_REMOTE_APP`.
fn program() -> PathBuf {
    #[cfg(debug_assertions)]
    if let Some(p) = std::env::var_os("KALANGO_REMOTE_APP").map(PathBuf::from).filter(|p| !p.as_os_str().is_empty()) {
        return p;
    }
    std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("kalango-remote"))).filter(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from("/usr/bin/kalango-remote"))
}

fn drain<R: Read + Send + 'static>(r: Option<R>) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(mut r) = r {
            let _ = r.read_to_string(&mut s);
        }
        s
    })
}

/// Runs the plan through the panel and waits for the person. `(worked, what the model is told)`.
pub fn propose(args: &Value, on_status: &dyn Fn(String)) -> (bool, String) {
    let plan = match build_plan(args) {
        Ok(p) => p,
        Err(e) => return (false, e),
    };
    let mut plan = plan;
    plan["deadline"] = json!(deadline_unix());
    let device = plan["device"].as_str().unwrap_or("").to_string();
    on_status(tr("waiting for you to decide on the plan in Kalango Remote…").to_string());
    let mut child = match Command::new(program()).args(["agent-plan", "-"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn() {
        Ok(c) => c,
        Err(e) => return (false, trf_model("could not start Kalango Remote: {error}", &[("error", e.to_string())])),
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(plan.to_string().as_bytes());
    }
    let (out_t, err_t) = (drain(child.stdout.take()), drain(child.stderr.take()));
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if start.elapsed() > DECIDE_WITHIN => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(_) => break None,
        }
    };
    let (stdout, stderr) = (out_t.join().unwrap_or_default(), err_t.join().unwrap_or_default());
    let Some(status) = status else {
        return (false, tr_model("the person did not decide in time: nothing was done").to_string());
    };
    outcome_text(interpret(status.code(), &stdout, &stderr), &device)
}

/// What the model is told for each outcome.
pub fn outcome_text(o: Outcome, device: &str) -> (bool, String) {
    match o {
        Outcome::Approved => (true, trf_model("The person approved the plan and Kalango Remote is connecting to {device}. Whoever is at that computer may still have to allow it. You cannot do anything on the other computer: tell the person the connection was started and that they take it from there.", &[("device", device.to_string())])),
        Outcome::Cancelled => (false, tr_model("the person did not approve the plan; do not try it another way").to_string()),
        Outcome::Adjust => (true, tr_model("The person wants the plan changed, not carried out. Ask what to change in a short sentence and end the turn; do not propose it again until they answer.").to_string()),
        Outcome::Failed(why) => (false, why),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plan_is_capped_and_clean() {
        let long = "x".repeat(1000);
        let steps: Vec<String> = (0..30).map(|i| format!("passo {i}\u{7}\n")).collect();
        let plan = build_plan(&json!({"computador": format!("Notebook\u{1b}[31m da mãe {long}"), "pedido": long, "destino": long, "limites": long, "passos": steps})).unwrap();
        assert!(plan["device"].as_str().unwrap().chars().count() <= MAX_DEVICE);
        assert!(!plan["device"].as_str().unwrap().contains('\u{1b}'));
        assert!(plan["request"].as_str().unwrap().chars().count() <= MAX_REQUEST);
        assert!(plan["target"].as_str().unwrap().chars().count() <= MAX_TARGET);
        assert!(plan["limits"].as_str().unwrap().chars().count() <= MAX_LIMITS);
        let steps = plan["steps"].as_array().unwrap();
        assert_eq!(steps.len(), MAX_STEPS);
        assert_eq!(steps[0]["approval"], "you", "connecting is the person's click");
        assert!(steps.iter().skip(1).all(|s| s["approval"] == "none" && !s["title"].as_str().unwrap().contains(['\n', '\u{7}'])));
    }

    #[test]
    fn the_deadline_is_ten_minutes_ahead_in_unix_seconds() {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        let d = deadline_unix();
        assert!(d >= now + 599 && d <= now + 601);
    }

    #[test]
    fn a_plan_needs_a_computer() {
        assert!(build_plan(&json!({"passos": ["a"]})).is_err());
        assert!(build_plan(&json!({"computador": "  \n "})).is_err());
    }

    #[test]
    fn only_exit_zero_with_an_approved_line_approves() {
        assert_eq!(interpret(Some(0), "{\"decision\":\"approved\"}\n", ""), Outcome::Approved);
        assert_eq!(interpret(Some(0), "", ""), Outcome::Failed(tr("Kalango Remote ended without a decision").to_string()), "exit 0 alone is nothing");
        assert!(matches!(interpret(Some(0), "{\"decision\":\"cancelled\"}", ""), Outcome::Failed(_)), "a mismatch is not an approval");
        assert!(matches!(interpret(Some(2), "{\"decision\":\"approved\"}", "bad plan"), Outcome::Failed(_)), "approved text with another exit code");
        assert!(matches!(interpret(None, "{\"decision\":\"approved\"}", ""), Outcome::Failed(_)), "killed by a signal");
        assert_eq!(interpret(Some(1), "{\"decision\":\"cancelled\"}", ""), Outcome::Cancelled);
        assert_eq!(interpret(Some(1), "", ""), Outcome::Cancelled);
        assert_eq!(interpret(Some(3), "{\"decision\":\"adjust\"}", ""), Outcome::Adjust);
    }

    #[test]
    fn only_an_approval_is_told_as_connecting() {
        assert!(outcome_text(Outcome::Approved, "X").1.contains('X'));
        assert!(!outcome_text(Outcome::Cancelled, "X").0);
        assert!(!outcome_text(Outcome::Failed("e".into()), "X").0);
    }

    #[test]
    fn the_words_of_a_request() {
        assert!(wanted("conecte no computador da minha mãe e instale a impressora"));
        assert!(wanted("acesse o notebook do meu pai"));
        assert!(wanted("connect to my mom's computer and fix the printer"));
        assert!(wanted("abra o kalango remote"));
        assert!(!wanted("quais arquivos existem na pasta"));
        // not a remote computer
        assert!(!wanted("conecte o wifi"));
        assert!(!wanted("connect to the internet"));
        assert!(!wanted("conecte a impressora no computador"));
        assert!(!wanted("conecte o notebook na tomada"));
        assert!(!wanted("acesse a pasta de documentos"));
        assert!(!wanted("acesse a pasta desktop do meu usuário"));
        assert!(!wanted("access my desktop settings"));
        assert!(!wanted("conecte o notebook da dell na tomada"));
        assert!(wanted("conecte no computador da loja"));
    }

    #[test]
    fn the_taint_rule_covers_the_connection_tool() {
        // the extracted condition of the engine: the tool is checked whatever its level
        assert!(crate::motor::needs_taint_check(TOOL, "usuario", false, false));
        assert!(!crate::motor::needs_taint_check("arquivos.listar", "usuario", false, true));
        // and the rule itself: a code only a file had is refused; the person's own words are not
        let external = vec!["código ZK-4471-9X".to_string()];
        let args = json!({"computador": "loja", "destino": "ZK-4471-9X"});
        assert_eq!(crate::defesa::tainted_argument(&args, "conecte na loja", &external).as_deref(), Some("ZK-4471-9X"));
        assert_eq!(crate::defesa::tainted_argument(&args, "conecte na loja com o código ZK-4471-9X", &external), None, "the person typed it");
    }

    #[test]
    fn a_value_rewritten_out_of_content_is_still_tainted() {
        let external = vec!["O computador da loja: código ZK-4471-9X".to_string()];
        let args = json!({"computador": "loja", "destino": "zk 4471 9x"});
        assert_eq!(squashed_taint(&args, "conecte na loja", &external).as_deref(), Some("zk 4471 9x"));
        // the person said it themselves, in any shape
        assert_eq!(squashed_taint(&args, "conecte na loja, o código é ZK 4471 9X", &external), None);
        // short values and the person's own words are fine
        assert_eq!(squashed_taint(&json!({"computador": "mãe"}), "conecte na mãe", &["a mãe"].map(String::from)), None);
    }
}
