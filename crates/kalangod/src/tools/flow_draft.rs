//! Authoring a flow in small steps, for a small model (LFM3 1.7B): a draft held by the server, one conversation each,
//! built from a list of plain-text steps, adjusted block by block and committed once. (Recipes with slots were measured
//! too and did not help the 1.7B model; they were removed, see docs/FLUXOS-NO-CHAT.md.)
//!
//! Why: the whole flow as one JSON is more than a 1.7B model writes right (a condition missing, one block only, four
//! failed calls). Here the model writes short plain steps; this module maps each step to a block type by fixed keyword
//! rules, draws the connections and says, per block, what is still missing.
//!
//! The rules that keep it safe:
//! - the planner maps a step only to a block that cannot run a system tool, start a machine or write a file
//!   (`PLANNABLE`): those blocks (`system_action`, `sandbox`, `speak`, `describe_image`) exist only when the model adds
//!   them on purpose with `add_block`, and the person sees every one of them at the approval. The one block that calls
//!   another flow (`subflow`) is made from a step only when the step names the flow ("chamar o fluxo limpeza"): the
//!   `flow_id` is then the person's word, shown at the approval;
//! - the text of a step is data: it can fill a message, a title or a search text, never choose a block type or a tool;
//! - nothing that executes is filled in with a guess: an agent, a file or a flow the step did not name stays empty and is
//!   reported as missing, and the commit refuses it;
//! - a draft belongs to one conversation (the caller's user and process), holds at most `MAX_DRAFT_BLOCKS` blocks and
//!   `MAX_DRAFT_BYTES` of JSON, a user keeps at most `MAX_DRAFTS_PER_USER` of them and a draft lasts `DRAFT_TTL`; nothing is created before the person approves the commit, which shows the same preview as
//!   `fluxos.criar`;
//! - a suggestion (a missing end, a branch the request asked for) is only said to the model and the person, never applied.

use super::plataforma_contrato as contract;
use super::{tr, trf};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

/// The most steps one plan has, the longest step, the most blocks a draft holds, the most drafts kept at once.
pub const MAX_STEPS: usize = 24;
pub const MAX_STEP_CHARS: usize = 300;
pub const MAX_DRAFT_BLOCKS: usize = 40;
pub const MAX_DRAFTS: usize = 32;
/// The most drafts one user keeps (the oldest of theirs goes first) and the most a draft weighs as JSON: a model cannot
/// make the privileged daemon hold megabytes.
pub const MAX_DRAFTS_PER_USER: usize = 8;
pub const MAX_DRAFT_BYTES: usize = 64 * 1024;
/// How long a draft is kept without being touched.
pub const DRAFT_TTL: Duration = Duration::from_secs(3600);

/// The block types a step can become by itself: none of them runs a system tool, starts a machine or writes a file. The
/// one that calls another flow (`subflow`) is made only when the step names the flow ("chamar o fluxo limpeza"): its
/// `flow_id` is the person's word, shown at the approval, and a step that names none leaves it empty (the commit refuses).
/// The first word of a refusal for a step no block does: the engine reads it to say so to the person. It is at the start of the
/// message, which the step's own words (echoed after it, with the code taken out) cannot reach.
pub const NO_BLOCK_CODE: &str = "[no_block]";

pub const PLANNABLE: &[&str] = &["transcribe", "condition", "decide", "approval", "notify", "wait", "ask", "for_each", "message", "agent", "subflow", "end"];

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Draft {
    pub nodes: Vec<Value>,
    pub edges: Vec<Value>,
    /// The steps the plan was made from (the suggestions read them).
    pub steps: Vec<String>,
}

impl Draft {
    pub fn flow(&self) -> Value {
        json!({"nodes": self.nodes, "edges": self.edges})
    }
}

// ── the store: one draft per conversation ───────────────────────────────────────────────────────────────────────────

/// Who a draft belongs to: the user and the process that talks to the server (one conversation of the agent engine).
pub type Owner = (u32, i32);

#[derive(Default)]
pub struct Drafts {
    map: HashMap<Owner, (Instant, Draft)>,
}

impl Drafts {
    fn sweep(&mut self) {
        self.map.retain(|_, (at, _)| at.elapsed() < DRAFT_TTL);
        // one user's drafts: the oldest go first
        let mut users: Vec<u32> = self.map.keys().map(|k| k.0).collect();
        users.sort_unstable();
        users.dedup();
        for uid in users {
            while self.map.keys().filter(|k| k.0 == uid).count() > MAX_DRAFTS_PER_USER {
                let Some(oldest) = self.map.iter().filter(|(k, _)| k.0 == uid).min_by_key(|(_, (at, _))| *at).map(|(k, _)| *k) else { break };
                self.map.remove(&oldest);
            }
        }
        while self.map.len() > MAX_DRAFTS {
            let Some(oldest) = self.map.iter().min_by_key(|(_, (at, _))| *at).map(|(k, _)| *k) else { break };
            self.map.remove(&oldest);
        }
    }
    pub fn get(&mut self, who: Owner) -> Option<Draft> {
        self.sweep();
        self.map.get(&who).map(|(_, d)| d.clone())
    }
    /// Keeps the draft, unless it weighs more than `MAX_DRAFT_BYTES` as JSON (the stored one stays as it was).
    pub fn put(&mut self, who: Owner, d: Draft) -> Result<(), String> {
        if d.flow().to_string().len() + d.steps.iter().map(String::len).sum::<usize>() > MAX_DRAFT_BYTES {
            return Err(trf("the draft is too big (at most {kb} KB): make the blocks smaller", &[("kb", (MAX_DRAFT_BYTES / 1024).to_string())]));
        }
        self.sweep();
        self.map.insert(who, (Instant::now(), d));
        self.sweep();
        Ok(())
    }
    #[cfg(test)]
    fn age(&mut self, who: Owner, by: Duration) {
        if let Some(e) = self.map.get_mut(&who) {
            e.0 = Instant::now().checked_sub(by).unwrap_or(e.0);
        }
    }
    pub fn clear(&mut self, who: Owner) {
        self.map.remove(&who);
    }
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.map.len()
    }
}

// ── the planner: steps in plain text to blocks ──────────────────────────────────────────────────────────────────────

/// Lower case, accents off, one char for one char (so a position in the folded text is a position in the original).
fn fold(s: &str) -> String {
    s.chars()
        .map(|c| match c.to_lowercase().next().unwrap_or(c) {
            'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            c => c,
        })
        .collect()
}

fn words(folded: &str) -> Vec<&str> {
    folded.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect()
}

fn starts_with_word(folded: &str, list: &[&str]) -> bool {
    words(folded).first().is_some_and(|w| list.contains(w))
}

const IF_WORDS: &[&str] = &["se", "quando", "caso", "if", "when"];
const ELSE_WORDS: &[&str] = &["senao", "otherwise", "else"];

/// A step that starts the other way of the last branch: "senão …", "caso contrário …", "se não …". Returns the rest.
fn else_rest(step: &str) -> Option<String> {
    let f = fold(step);
    let w = words(&f);
    let n = match w.as_slice() {
        [first, ..] if ELSE_WORDS.contains(first) => 1,
        ["caso", "contrario", ..] | ["do", "contrario", ..] | ["se", "nao", ..] | ["if", "not", ..] => 2,
        ["em", "caso", "contrario", ..] => 3,
        _ => return None,
    };
    let rest: Vec<&str> = step.split_whitespace().skip(n).collect();
    Some(rest.join(" ").trim_start_matches([',', ':', ';', '-']).trim().to_string())
}

/// Words that carry nothing after "terminar"/"fim": a step made only of them is the end of the flow.
const END_FILLER: &[&str] = &["o", "a", "os", "as", "fluxo", "aqui", "agora", "tudo", "ai", "ali", "e", "de", "do", "da", "por", "execucao", "processo", "tarefa", "ponto", "esse", "este", "isso", "ja", "entao", "the", "flow", "here", "now", "it"];

/// "fim", "terminar o fluxo", "pare aqui": the verb and nothing meaningful after it. "finalizar a compra com o agente
/// vendas" has something to do after the verb, so it is not an end.
fn is_end_step(w: &[&str]) -> bool {
    const VERBS: &[&str] = &["fim", "terminar", "termina", "termine", "encerrar", "encerra", "encerre", "finalizar", "finaliza", "finalize", "acabar", "acabe", "pare", "parar", "end", "stop"];
    w.first().is_some_and(|f| VERBS.contains(f)) && w.iter().skip(1).all(|x| END_FILLER.contains(x))
}

/// What block a step is, by its words (None: no rule matches). Ordered: the first rule that matches wins.
pub fn map_step(step: &str) -> Option<&'static str> {
    let f = fold(step);
    let w = words(&f);
    // a key is a stem (prefix of a word), a short word (3 letters or fewer, whole) or, with "=", a whole word: "cada" must
    // not match "cadastrar" nor "lista" match "listar"
    let has = |keys: &[&str]| w.iter().any(|x| keys.iter().any(|k| if let Some(k) = k.strip_prefix('=') { x == &k } else if k.len() <= 3 { x == k } else { x.starts_with(k) }));
    if is_end_step(&w) {
        return Some("end");
    }
    // the decision model is asked for by name ("d1", "decidir"): that wins over the "se" the step may start with
    // ("classificador" is often the name of an agent: only the d1 and the verb "decidir" ask for the decision model)
    if has(&["d1", "decid", "=decisao", "=decisoes", "=decision", "=julgar", "=julgue", "=julga", "=judge"]) {
        return Some("decide");
    }
    if starts_with_word(&f, IF_WORDS) {
        // a judgment about the text ("se for uma reclamação") has nothing to search for: the decision model answers it
        if search_text(step).is_empty() && has(&["for", "seja", "forem", "parecer", "parece"]) {
            return Some("decide");
        }
        return Some("condition");
    }
    // "verificar se o texto contém X" checks a word; "avaliar se o texto é spam" is a judgment
    if has(&["verific", "cheque", "checar", "teste", "confer", "analis", "avali", "check", "test"]) && has(&["=se", "=if", "=whether"]) && !has(&["aprov", "=agente", "=agent"]) {
        return Some(if search_text(step).is_empty() { "decide" } else { "condition" });
    }
    // "transcrever" needs something to listen to: transcribing "o texto do ticket" is not a block (it would ask for a file)
    let audio = !path_in(step).is_empty() || has(&["=escutar", "=escute", "=ouvir", "=ouca", "=audio", "=audios", "=listen", "=gravacao", "=gravacoes", "=voz", "=fala", "=falas", "=reuniao", "=ligacao", "=podcast", "=recording", "=speech", "=voice", "=microfone", "=wav", "=mp3"]);
    if audio && has(&["transcr", "=escutar", "=escute", "=ouvir", "=ouca", "=audio", "=audios", "=listen", "=transcribe"]) {
        return Some("transcribe");
    }
    if has(&["aprov", "=confirmar", "=confirme", "=confirma", "=confirmacao", "=confirmar", "permiss", "approv", "posso", "autoriz", "=liberar", "=libere"]) || (has(&["pergunt", "peca", "pedir", "pede"]) && has(&["quer", "devo", "podemos", "continuar", "prosseguir", "seguir", "desej", "gostaria", "want", "should", "shall"])) {
        return Some("approval");
    }
    if has(&["esper", "aguard", "pausa", "wait", "dorm"]) {
        return Some("wait");
    }
    if has(&["=cada", "repet", "=iterar", "=itere", "=iteracao", "=lista", "=listas", "=each"]) {
        return Some("for_each");
    }
    if has(&["notific", "avis", "alert"]) {
        return Some("notify");
    }
    if has(&["pergunt", "ask"]) || (has(&["=pedir", "=peca", "=solicitar", "=solicite", "=request"]) && has(&["=usuario", "=pessoa", "=user", "=person"])) {
        return Some("ask");
    }
    if has(&["fluxo", "subfluxo", "flow"]) && has(&["chamar", "chame", "chama", "rodar", "rode", "executar", "execute", "usar", "use", "acionar", "aciona", "iniciar", "inicie"]) && flow_named(step).is_some() {
        return Some("subflow");
    }
    // "enviar uma mensagem de saudação" shows a message; only an agent that is named, or a work verb, makes it an agent
    if has(&["mensag", "message"]) && !has(&["agent", "resum", "tradu", "rode", "rodar", "historia"]) {
        return Some("message");
    }
    if has(&["agent", "resum", "historia", "conta", "tradu", "rode", "rodar", "envi", "mand", "pass"]) {
        return Some("agent");
    }
    if has(&["mensag", "mostr", "texto", "escrev", "diga", "respond", "message"]) {
        return Some("message");
    }
    None
}

/// "se o resultado do d1 for verdadeiro: avisar": a way out of the decision just made, not a new decision. Returns whether
/// it is the positive way and the action after the label (may be empty).
fn result_branch(step: &str) -> Option<(bool, String)> {
    const ABOUT_RESULT: &[&str] = &["resultado", "resposta", "decisao", "saida", "retorno", "veredito", "result", "answer", "output", "decision", "verdict"];
    const FILLER: &[&str] = &["se", "quando", "caso", "if", "when", "for", "seja", "e", "o", "a", "de", "do", "da", "que", "is", "it", "the", "was", "verdadeiro", "verdadeira", "verdade", "true", "sim", "yes", "positivo", "positiva", "falso", "falsa", "false", "nao", "no", "negativo", "negativa", "d1", "vazia", "vazio", "empty", "conteudo"];
    let f = fold(step);
    if !starts_with_word(&f, IF_WORDS) {
        return None;
    }
    // "se sim mostrar aviso": the label is two words, the action follows with no comma
    {
        let w = words(&f);
        if w.len() > 2 && ["sim", "yes", "positivo", "positiva", "verdadeiro", "true", "so"].contains(&w[1]) {
            let tail: String = step.split_whitespace().skip(2).collect::<Vec<_>>().join(" ");
            return Some((true, tail.trim_start_matches([',', ';', ':']).trim().to_string()));
        }
        if w.len() > 2 && ["falso", "false", "negativo", "negativa"].contains(&w[1]) {
            let tail: String = step.split_whitespace().skip(2).collect::<Vec<_>>().join(" ");
            return Some((false, tail.trim_start_matches([',', ';', ':']).trim().to_string()));
        }
    }
    let cut = f.find([',', ';', ':']).map(|i| (i, 1)).into_iter().chain(f.find(" entao ").map(|i| (i, 7))).min_by_key(|(i, _)| *i);
    let (head_f, tail) = match cut {
        Some((i, len)) => {
            let chars: Vec<char> = step.chars().collect();
            let a = f[..i].chars().count();
            let b = (a + f[i..i + len].chars().count()).min(chars.len());
            (f[..i].to_string(), chars[b..].iter().collect::<String>().trim().trim_start_matches([',', ';', ':']).trim().to_string())
        }
        None => (f.clone(), String::new()),
    };
    let w = words(&head_f);
    // (a parenthesis is a remark: "(urgente)")
    let w: Vec<&str> = w.into_iter().filter(|x| !x.is_empty()).collect();
    let about = w.iter().any(|x| ABOUT_RESULT.contains(x));
    let only_filler = w.iter().all(|x| FILLER.contains(x)) && w.iter().any(|x| !IF_WORDS.contains(x));
    if !about && !only_filler {
        return None;
    }
    // a head that names something to search for is a search ("se o resultado contiver erro")
    if !search_text(&head_f).is_empty() || w.iter().any(|x| ["contiver", "contenha", "contem", "mencionar", "citar", "tiver"].contains(x)) {
        return None;
    }
    let negative = w.iter().any(|x| ["falso", "falsa", "false", "nao", "negativo", "negativa"].contains(x)) || (w.contains(&"no") && only_filler);
    Some((!negative, tail))
}

/// The yes/no question a judgment step asks the decision model: what follows "se"/"whether"/"if" ("usar o modelo d1 para
/// avaliar se o texto é spam" → "o texto é spam?"), else the whole step.
pub fn judgment_question(step: &str) -> String {
    let f = fold(step);
    let chars: Vec<char> = step.chars().collect();
    let fc: Vec<char> = f.chars().collect();
    let mut start = None;
    for key in [" se ", " whether ", " if "] {
        let kc: Vec<char> = key.chars().collect();
        if let Some(p) = fc.windows(kc.len()).position(|w| w == kc.as_slice()) {
            start = Some(start.map_or(p + kc.len(), |b: usize| b.min(p + kc.len())));
        }
    }
    // a step that starts with "se" ("se a mensagem for uma reclamação"): the rest
    if start.is_none() && starts_with_word(&f, IF_WORDS) {
        start = fc.iter().position(|c| *c == ' ').map(|p| p + 1);
    }
    let rest: String = match start {
        Some(a) if a < chars.len() => chars[a..].iter().collect(),
        _ => step.to_string(),
    };
    let q = rest.trim().trim_matches(['"', '\'', '.', '?', '“', '”']).trim();
    if q.is_empty() { contract::one_line(step, 200) } else { contract::one_line(&format!("{q}?"), 200) }
}

/// Sentences of a step ("Verificar se contém 'prova'. Se sim, enviar ao tutor."): a full stop, "?" or "!" followed by a capital.
fn split_sentences(step: &str) -> Vec<String> {
    let chars: Vec<char> = step.chars().collect();
    let mut out = Vec::new();
    let mut start = 0;
    for i in 0..chars.len() {
        if !matches!(chars[i], '.' | '?' | '!') {
            continue;
        }
        let rest: Vec<&char> = chars[i + 1..].iter().skip_while(|c| c.is_whitespace()).take(1).collect();
        if chars.get(i + 1).is_some_and(|c| c.is_whitespace()) && rest.first().is_some_and(|c| c.is_uppercase()) {
            let part: String = chars[start..=i].iter().collect();
            out.push(part.trim().trim_end_matches('.').trim().to_string());
            start = i + 1;
        }
    }
    let last: String = chars[start..].iter().collect();
    out.push(last.trim().trim_end_matches('.').trim().to_string());
    out.into_iter().filter(|p| !p.is_empty()).collect()
}

/// "Aplicar condição: se o texto contiver X, avisar": the words before the colon only name the step; the condition starts
/// at the "se". Only after a colon or a semicolon and a short lead-in.
fn strip_lead_in(step: &str) -> String {
    let f = fold(step);
    if starts_with_word(&f, IF_WORDS) {
        return step.to_string();
    }
    for key in [": se ", ": quando ", ": caso ", "; se ", "; quando "] {
        if let Some(p) = f.find(key) {
            if f[..p].split_whitespace().count() <= 5 {
                let n = f[..p + 2].chars().count();
                return step.chars().skip(n).collect::<String>().trim().to_string();
            }
        }
    }
    step.to_string()
}

/// "se X, fazer A; senão, fazer B" is two steps: the way taken and the other way.
fn split_else(step: &str) -> Vec<String> {
    let f = fold(step);
    let cut = ["; senao", ", senao,", "; caso contrario", ", caso contrario", "; do contrario", "; otherwise", ", otherwise", "; se nao", ", se nao,"].iter().filter_map(|k| f.find(k)).min();
    match cut {
        Some(p) => {
            let n = f[..p].chars().count();
            let chars: Vec<char> = step.chars().collect();
            let head: String = chars[..n].iter().collect();
            let tail: String = chars[n + 1..].iter().collect();
            vec![head.trim().to_string(), tail.trim().trim_start_matches([',', ';']).trim().to_string()]
        }
        None => vec![step.to_string()],
    }
}

/// What a step says as separate steps: sentences, the lead-in dropped, the other way of a branch apart, the action after
/// a condition apart.
fn refine_step(step: &str) -> Vec<String> {
    split_sentences(step)
        .iter()
        .flat_map(|s| split_else(&strip_lead_in(s)))
        .filter(|p| !p.is_empty())
        .flat_map(|p| split_judgment_action(&p))
        .flat_map(|p| split_condition(&p))
        .flat_map(|p| split_before(&p))
        .collect()
}

/// "analisar se o comentário é ofensivo, se sim mostrar aviso": the judgment and, as the way taken, the action.
fn split_judgment_action(step: &str) -> Vec<String> {
    let f = fold(step);
    if starts_with_word(&f, IF_WORDS) {
        return vec![step.to_string()];
    }
    const KEYS: &[&str] = &[", se sim", "; se sim", ", se for sim", ", if yes", ", if so", ", se positivo", ", caso positivo", " e, sendo", ", sendo", ", se verdadeiro"];
    match KEYS.iter().filter_map(|k| f.find(k)).min() {
        Some(p) => {
            let n = f[..p].chars().count();
            let chars: Vec<char> = step.chars().collect();
            let head: String = chars[..n].iter().collect();
            let tail: String = chars[n + 1..].iter().collect();
            let tail = tail.trim().trim_start_matches([',', ';']).trim().to_string();
            // "sendo" stands for "se sim"
            let clean = tail.trim_start_matches("e,").trim_start_matches("e ").trim().to_string();
            let tail = if fold(&clean).starts_with("sendo") { format!("se sim {}", clean.split_whitespace().skip(1).collect::<Vec<_>>().join(" ").trim_start_matches([',', ';']).trim()) } else { tail };
            vec![head.trim().to_string(), tail]
        }
        None => vec![step.to_string()],
    }
}

/// "pedir aprovação do usuário antes de rodar o agente X": the approval and, after it, what it holds back.
fn split_before(step: &str) -> Vec<String> {
    let f = fold(step);
    let Some(p) = f.find(" antes de ").or_else(|| f.find(" before ")) else { return vec![step.to_string()] };
    let (head_f, tail_f) = (&f[..p], &f[p..]);
    let n = head_f.chars().count();
    let chars: Vec<char> = step.chars().collect();
    let head: String = chars[..n].iter().collect();
    let tail: String = chars[n..].iter().collect();
    let tail = tail.split_whitespace().skip(if tail_f.trim_start().starts_with("antes") { 2 } else { 1 }).collect::<Vec<_>>().join(" ");
    if map_step(&head) == Some("approval") && has_action(&tail) {
        vec![head.trim().to_string(), tail]
    } else {
        vec![step.to_string()]
    }
}

/// Whether a sentence says something to do besides deciding (a notice, an agent, a wait…).
fn has_action(step: &str) -> bool {
    const STEMS: &[&str] = &["avis", "notific", "envi", "mand", "rod", "mostr", "ped", "esper", "aguard", "resum", "tradu", "execut", "cham", "encerr", "termin", "pergunt", "agent", "show", "notify", "send", "run", "ask", "wait", "stop", "end"];
    words(&fold(step)).iter().any(|w| STEMS.iter().any(|k| w.starts_with(k)))
}

/// "se falar do gta 6, pegar o resumo": the condition and the action, as two steps. Only for a step that starts with
/// "se/quando/caso" and has a comma or "então" after the condition. A step that asks the decision model by name keeps
/// its action too ("se o d1 disser que é urgente, avisar" is a decision and a notice), unless what follows the comma is
/// only how to decide ("se for reclamação, usar o modelo de decisão d1").
fn split_condition(step: &str) -> Vec<String> {
    let f = fold(step);
    if !starts_with_word(&f, IF_WORDS) {
        return vec![step.to_string()];
    }
    let cut = f.find([',', ';']).map(|i| (i, 1)).into_iter().chain(f.find(" entao ").map(|i| (i, 7))).min_by_key(|(i, _)| *i);
    match cut {
        // (positions are in chars: `fold` keeps the count, so they are the same in the original text)
        Some((i, len)) => {
            let chars: Vec<char> = step.chars().collect();
            let byte_to_char = |b: usize| f[..b].chars().count();
            let (a, b) = (byte_to_char(i), byte_to_char(i) + f[i..i + len].chars().count());
            let (head, tail): (String, String) = (chars[..a].iter().collect(), chars[b.min(chars.len())..].iter().collect());
            let tail = tail.trim().trim_start_matches([',', ';']).trim().to_string();
            // (the tail that only says how to decide is not an action)
            let how_to_decide = map_step(&tail) == Some("decide") && !has_action(&tail);
            if tail.is_empty() || how_to_decide { vec![head.trim().to_string()] } else { vec![head.trim().to_string(), tail] }
        }
        None => vec![step.to_string()],
    }
}

/// The text a step names for a search ("falar sobre o gta 6" → "gta 6", "contiver 'urgente'" → "urgente"); empty when it
/// names none.
pub fn search_text(step: &str) -> String {
    // a quoted text first
    for (open, close) in [('"', '"'), ('\'', '\''), ('“', '”'), ('«', '»')] {
        if let Some(a) = step.find(open) {
            let rest = &step[a + open.len_utf8()..];
            if let Some(b) = rest.find(close) {
                let t = rest[..b].trim();
                if !t.is_empty() {
                    return contract::one_line(t, 80);
                }
            }
        }
    }
    let f = fold(step);
    let chars: Vec<char> = step.chars().collect();
    let fchars: Vec<char> = f.chars().collect();
    let keys = ["tiver ", "tenha ", "citar ", "citou ", "cita ", "falar em ", "fala em ", "falou em ", "falar no ", "falar na ", "contiver ", "contenha ", "contem ", "contenham ", "mencionar ", "mencionou ", "menciona ", "disser ", "falar sobre ", "falar do ", "falar da ", "falar de ", "fala sobre ", "fala do ", "fala da ", "fala de ", "falou sobre ", "sobre ", "palavra ", "texto "];
    let mut best: Option<usize> = None;
    for k in keys {
        let kc: Vec<char> = k.chars().collect();
        if let Some(p) = fchars.windows(kc.len()).position(|win| win == kc.as_slice()) {
            let end = p + kc.len();
            best = Some(best.map_or(end, |b| b.min(end)));
        }
    }
    let Some(start) = best else { return String::new() };
    let rest: String = chars[start..].iter().collect();
    // up to the next comma, semicolon or the verb that starts the action
    let mut text = rest.clone();
    let fr = fold(&rest);
    for stop in [",", ";", " entao ", " pegar ", " pegue ", " enviar ", " envie ", " mandar ", " mande ", " avisar ", " notificar ", " e depois ", " ou "] {
        if let Some(i) = fr.find(stop) {
            let cut = fr[..i].chars().count();
            text = text.chars().take(cut).collect();
        }
    }
    let mut t = text.trim().trim_matches(['"', '\'', '.', '“', '”']).trim().to_string();
    for art in ["a palavra ", "o texto ", "a frase ", "o ", "a ", "os ", "as ", "um ", "uma ", "do ", "da ", "de "] {
        if fold(&t).starts_with(art) {
            t = t.chars().skip(art.chars().count()).collect::<String>().trim().to_string();
            break;
        }
    }
    contract::one_line(&t, 80)
}

const STOP_AFTER_AGENT: &[&str] = &["para", "o", "a", "que", "outro", "outra", "um", "uma", "chamado", "chamada", "e", "ou", "com", "do", "da", "dos", "das", "de", "no", "na", "ao", "se", "em", "por", "ira", "vai", "deve", "ele", "ela", "x"];

/// The id of the agent a step names ("o agente resumidor" → "resumidor"); empty when it names none.
pub fn agent_named(step: &str, existing: &[String]) -> String {
    let f = fold(step);
    let w = words(&f);
    let Some(i) = w.iter().position(|x| matches!(*x, "agente" | "agent" | "agentes" | "agents")) else { return String::new() };
    let mut j = i + 1;
    // "agente de relatório" / "agente chamado resumidor": one connective may come first
    if w.get(j).is_some_and(|x| ["de", "do", "da", "chamado", "chamada"].contains(x)) {
        j += 1;
    }
    let Some(name) = w.get(j).filter(|x| !STOP_AFTER_AGENT.contains(x) && x.len() > 1) else { return String::new() };
    // a step that continues with more words of the same name ("agente contador de histórias") keeps up to two more
    let mut id = name.to_string();
    for next in w.iter().skip(j + 1).take(2) {
        if ["de", "do", "da"].contains(next) || STOP_AFTER_AGENT.contains(next) && !["de", "do", "da"].contains(next) {
            break;
        }
        id.push('-');
        id.push_str(next);
    }
    let id = contract::slugify(&id);
    // an existing agent that starts with it (or the other way) is the one meant
    existing.iter().find(|e| **e == id).or_else(|| existing.iter().find(|e| e.starts_with(&id) || id.starts_with(e.as_str()))).cloned().unwrap_or(id)
}

/// The id of the flow a step names ("chamar o fluxo limpeza" → "limpeza").
fn flow_named(step: &str) -> Option<String> {
    let f = fold(step);
    let w = words(&f);
    let i = w.iter().position(|x| matches!(*x, "fluxo" | "subfluxo" | "flow"))?;
    let mut j = i + 1;
    if w.get(j).is_some_and(|x| ["de", "do", "da", "chamado", "chamada"].contains(x)) {
        j += 1;
    }
    let name = w.get(j).filter(|x| !STOP_AFTER_AGENT.contains(x) && x.len() > 1)?;
    Some(contract::slugify(name))
}

fn seconds_in(step: &str) -> Option<u64> {
    let f = fold(step);
    let w = words(&f);
    for (i, x) in w.iter().enumerate() {
        if let (Ok(n), Some(unit)) = (x.parse::<u64>(), w.get(i + 1)) {
            let mult = if unit.starts_with("seg") || unit.starts_with("sec") || *unit == "s" { 1 } else if unit.starts_with("min") { 60 } else if unit.starts_with("hora") || unit.starts_with("hour") || *unit == "h" { 3600 } else { continue };
            return Some(n.saturating_mul(mult).min(7 * 24 * 3600));
        }
    }
    None
}

/// A file path or an audio/image file name written in a step.
fn path_in(step: &str) -> String {
    step.split_whitespace()
        .map(|w| w.trim_matches(|c: char| c == ',' || c == ';' || c == '"' || c == '\'' || c == '“' || c == '”'))
        .find(|w| w.starts_with("~/") || w.starts_with('/') || [".wav", ".mp3", ".ogg", ".flac", ".m4a"].iter().any(|e| fold(w).ends_with(e)))
        .map(|w| contract::one_line(w.trim_end_matches('.'), 200))
        .unwrap_or_default()
}

/// What follows "dizendo", "com o texto"… in a step, else the whole step: the words a message or a notice shows.
fn said_text(step: &str, max: usize) -> String {
    let f = fold(step);
    for k in ["dizendo ", "escrevendo ", "com o texto ", "com a mensagem ", "texto ", "mensagem de ", "mensagem "] {
        if let Some(p) = f.find(k) {
            let rest: String = step.chars().skip(f[..p + k.len()].chars().count()).collect();
            let t = rest.trim().trim_matches(['"', '\'', '“', '”', '.']).trim();
            if !t.is_empty() {
                return contract::one_line(t, max);
            }
        }
    }
    contract::one_line(step, max)
}

/// The block a step becomes (its data holds only what the step said; what it did not say stays out, and the commit asks
/// for it). `agents`: the ids that exist, to recognise the agent a step names.
fn block_for(id: &str, t: &str, step: &str, agents: &[String], lang: &str, notes: &mut Vec<String>) -> Value {
    let title = contract::one_line(step, 60);
    let data = match t {
        "agent" => {
            let a = agent_named(step, agents);
            if a.is_empty() { json!({"message": ""}) } else { json!({"agent_id": a, "message": ""}) }
        }
        "subflow" => match flow_named(step) {
            Some(id) => json!({"flow_id": id}),
            None => json!({}),
        },
        "for_each" => {
            let a = agent_named(step, agents);
            let mut d = json!({"items": "content", "message": "{item}"});
            if !a.is_empty() {
                d["agent_id"] = json!(a);
            }
            d
        }
        "transcribe" => {
            let f = path_in(step);
            // (the language of the person's system: never assumed to be Portuguese)
            let mut d = json!({"save_as": "transcricao"});
            if !lang.is_empty() {
                d["language"] = json!(lang);
            }
            if !f.is_empty() {
                d["file"] = json!(f);
            }
            d
        }
        "condition" => {
            let v = search_text(step);
            let mut c = json!({"id": "sim", "type": "previous-output", "data": {"field": "content", "operator": "contains"}});
            if !v.is_empty() {
                c["data"]["value"] = json!(v);
            }
            json!({"conditions": [c]})
        }
        "decide" => json!({"model": "decisao", "state": "", "route": "rota", "threshold": 0.8, "questions": {"rota": {"type": "noul", "instructions": judgment_question(step)}}}),
        "approval" => json!({"question": contract::one_line(step, 200), "details": "", "timeout_s": 7200}),
        "notify" => json!({"title": said_text(step, 80), "body": ""}),
        "wait" => match seconds_in(step) {
            Some(s) => json!({"seconds": s.max(1)}),
            None => {
                notes.push(trf("step \"{id}\" (wait) named no time: 60 seconds, the editor's starting value; change it with fluxos.ajustar", &[("id", id.to_string())]));
                json!({"seconds": 60})
            }
        },
        "ask" => json!({"message": contract::one_line(step, 200), "save_as": "resposta"}),
        "message" => json!({"content": said_text(step, 300)}),
        _ => json!({}),
    };
    let mut b = json!({"id": id, "type": t, "title": title});
    if t != "end" {
        b["data"] = data;
    }
    b
}

fn edge(from: &str, to: &str, via: Option<&str>) -> Value {
    match via {
        Some(v) => json!({"source": from, "target": to, "sourceHandle": v}),
        None => json!({"source": from, "target": to}),
    }
}

/// An end block after `src` (by its way `via`): every branch ends, and the note says where one was added.
fn add_end(nodes: &mut Vec<Value>, edges: &mut Vec<Value>, count: &mut usize, src: &str, via: Option<&str>, notes: &mut Vec<String>) {
    *count += 1;
    let id = format!("e{count}");
    nodes.push(json!({"id": id, "type": "end"}));
    edges.push(edge(src, &id, via));
    notes.push(match via {
        Some(w) => trf("the way \"{via}\" of \"{id}\" had no step: an end block \"{end}\" was added", &[("via", if w == contract::ELSE_HANDLE { tr("else").to_string() } else { w.to_string() }), ("id", src.to_string()), ("end", id.clone())]),
        None => trf("an end block \"{end}\" was added after \"{id}\" (every branch ends)", &[("end", id.clone()), ("id", src.to_string())]),
    });
}

/// The draft a list of plain steps makes, and what was decided for the model to know. Errors say what to send instead.
/// A step no rule maps is an error that names it: the model rewrites that step or adds the block itself.
#[cfg(test)]
pub fn plan(steps: &[String], agents: &[String]) -> Result<(Draft, Vec<String>), String> {
    plan_in(kalango_i18n::system_locale(), steps, agents)
}

/// The name of the language the planner's words are written in, for a system locale: Portuguese (the pt_BR catalog) and
/// English (no catalog); None for any other (its words are not in the tables: the model chooses each block itself).
pub fn planner_language(locale: Option<&str>) -> Option<&'static str> {
    match locale {
        Some(l) if l.starts_with("pt") => Some("Portuguese"),
        None => Some("English"),
        Some(l) if l.starts_with("en") => Some("English"),
        _ => None,
    }
}

/// Words that name something no block does: sending a message or an e-mail, posting to a social network, a calendar or a
/// database. A step with one is not turned into a block (an agent block with no agent would push the model to fake it):
/// the model tells the person. An e-mail the flow only READS ("o conteúdo do e-mail") is the subject of a step, not a
/// block that sends: the e-mail words count with a verb that sends.
fn needs_messaging(step: &str) -> bool {
    const SERVICES: &[&str] = &["sms", "whatsapp", "telegram", "slack", "discord", "twitter", "tweet", "tweets", "facebook", "instagram", "linkedin", "tiktok", "calendar", "calendario", "sql", "postgres", "postgresql", "mysql", "mongodb", "jira", "trello", "notion", "dropbox"];
    const MAIL: &[&str] = &["email", "emails", "mail", "gmail", "outlook", "smtp"];
    const SENDS: &[&str] = &["envi", "mand", "dispar", "send", "post", "public", "encaminh", "respond", "reply", "agend", "schedul", "remet"];
    let f = fold(step);
    let w = words(&f);
    // (a call to the person's phone, a database, Microsoft Teams: no block either; "the sales team" is only a team)
    let phone = w.iter().any(|x| ["telefone", "celular", "phone"].contains(x)) && w.iter().any(|x| ["lig", "call", "chama", "telefonar"].iter().any(|k| x.starts_with(k)));
    if phone || f.contains("microsoft teams") || f.contains("ms teams") || f.contains("msteams") || f.contains("banco de dados") || w.contains(&"database") {
        return true;
    }
    w.iter().any(|x| SERVICES.contains(x)) || (w.iter().any(|x| MAIL.contains(x)) && w.iter().any(|x| SENDS.iter().any(|k| x.starts_with(k))))
}

#[cfg(test)]
pub fn plan_in(locale: Option<&str>, steps: &[String], agents: &[String]) -> Result<(Draft, Vec<String>), String> {
    plan_with(locale, steps, &[], agents)
}

/// Whether the d1's kind for a step may be used: the step must have what that block needs, and what the words of the step
/// already say stays (a search for a word is not turned into a judgment, an end is an end by its own words).
fn accept_kind(kind: &str, step: &str, rule: Option<&'static str>, d1_named: bool) -> bool {
    match kind {
        // the person named the decision model: "perguntar se o ticket é um bug" is the d1's question, not a question to them
        "subflow" => flow_named(step).is_some(),
        "condition" => !search_text(step).is_empty(),
        // (a search for a word the step names stays a search: it becomes a condition below)
        // (a question to the person stays theirs, unless they asked for the decision model)
        // (and it never replaces what the step's words clearly say: a wait, an approval, a loop, a sub-flow, a transcription…)
        "decide" => search_text(step).is_empty() && (matches!(rule, None | Some("message") | Some("decide")) || (rule == Some("ask") && d1_named)),
        // the end is the planner's: "fim" and its words; a judgment of the d1 must not end a flow that has more to do
        "end" => false,
        k => PLANNABLE.contains(&k) && (rule.is_none() || rule == Some(k)),
    }
}

/// As [`plan_in`], with a kind for each step from the decision model (d1; "" or missing: none). The kind is a hint that
/// completes the rules (a step no rule maps, a judgment written in other words); it never creates a block that is not
/// plannable and never replaces a block the step's own words name.
#[cfg(test)]
pub fn plan_with(locale: Option<&str>, steps: &[String], kinds: &[String], agents: &[String]) -> Result<(Draft, Vec<String>), String> {
    plan_for(locale, steps, kinds, "", agents)
}

/// As [`plan_with`], with the person's own request: when it asks for the decision model by name (d1) and no step became a
/// decision, the first search for a word becomes the decision the person asked for.
pub fn plan_for(locale: Option<&str>, steps: &[String], kinds: &[String], request: &str, agents: &[String]) -> Result<(Draft, Vec<String>), String> {
    let hinted: Vec<(String, String)> = steps.iter().enumerate().map(|(i, s)| (s.trim().to_string(), kinds.get(i).cloned().unwrap_or_default())).collect();
    let (mut d, mut notes) = plan_pairs(locale, hinted, asks_for_d1(request), agents)?;
    honor_decision_request(&mut d, request, &mut notes);
    Ok((d, notes))
}

/// Whether the person named the decision model: "d1", "modelo de decisão", "decision model".
fn asks_for_d1(request: &str) -> bool {
    let f = fold(request);
    let w = words(&f);
    w.contains(&"d1") || (w.contains(&"modelo") && w.iter().any(|x| x.starts_with("decis"))) || (w.contains(&"decision") && w.contains(&"model"))
}

/// The yes/no question of a request that names the decision model: what follows its "se"/"whether", up to the first comma.
fn request_question(request: &str) -> String {
    let f = fold(request);
    let chars: Vec<char> = request.chars().collect();
    let fc: Vec<char> = f.chars().collect();
    let key = |k: &str| -> Option<usize> {
        let kc: Vec<char> = k.chars().collect();
        fc.windows(kc.len()).position(|w| w == kc.as_slice()).map(|p| p + kc.len())
    };
    let Some(a) = [" se ", " whether ", " if "].iter().filter_map(|k| key(k)).min() else { return String::new() };
    let rest: String = chars[a.min(chars.len())..].iter().collect();
    let end = rest.find([',', ';', '.']).unwrap_or(rest.len());
    rest[..end].trim().to_string()
}

fn honor_decision_request(d: &mut Draft, request: &str, notes: &mut Vec<String>) {
    if !asks_for_d1(request) || d.nodes.iter().any(|n| n["type"] == "decide") {
        return;
    }
    let Some(i) = d.nodes.iter().position(|n| n["type"] == "condition") else { return };
    let id = d.nodes[i]["id"].as_str().unwrap_or("").to_string();
    let searched = d.nodes[i]["data"]["conditions"][0]["data"]["value"].as_str().or(d.nodes[i]["data"]["conditions"][0]["value"].as_str()).unwrap_or("").to_string();
    let mut q = request_question(request);
    if q.is_empty() {
        q = if searched.is_empty() { tr("the text is what was asked").to_string() } else { trf("the text is about {what}", &[("what", searched)]) };
    }
    let q = contract::one_line(&format!("{}?", q.trim_end_matches('?')), 200);
    d.nodes[i]["type"] = json!("decide");
    d.nodes[i]["data"] = json!({"model": "decisao", "state": "", "route": "rota", "threshold": 0.8, "questions": {"rota": {"type": "noul", "instructions": q}}});
    for e in d.edges.iter_mut().filter(|e| e["source"] == json!(id)) {
        let way = match e["sourceHandle"].as_str() {
            Some("sim") => "true",
            _ => "false",
        };
        e["sourceHandle"] = json!(way);
    }
    notes.push(trf("the person asked for the decision model (d1): the block \"{id}\" asks it instead of searching for a word", &[("id", id)]));
}

fn plan_pairs(locale: Option<&str>, pairs: Vec<(String, String)>, d1_named: bool, agents: &[String]) -> Result<(Draft, Vec<String>), String> {
    let Some(lang) = planner_language(locale) else {
        return Err(tr("the step planner knows Portuguese and English words only: in this language build the flow block by block with fluxos.adicionar_bloco (you choose each block type)").to_string());
    };
    let pairs: Vec<(String, String)> = pairs.into_iter().filter(|(s, _)| !s.is_empty()).collect();
    let steps: Vec<String> = pairs.iter().map(|(s, _)| s.clone()).collect();
    if steps.is_empty() {
        return Err(tr("give the steps of the flow: a list of short sentences, one per step, in order (for example [\"transcrever o áudio\", \"se falar do gta 6, resumir\", \"fim\"])").to_string());
    }
    if steps.len() > MAX_STEPS {
        return Err(trf("a plan has at most {max} steps", &[("max", MAX_STEPS.to_string())]));
    }
    if let Some(long) = steps.iter().find(|s| s.chars().count() > MAX_STEP_CHARS) {
        return Err(trf("a step has at most {max} characters: \"{start}…\" is longer; say it shorter", &[("max", MAX_STEP_CHARS.to_string()), ("start", contract::one_line(long, 40))]));
    }
    if let Some(m) = pairs.iter().find(|(s, k)| needs_messaging(s) || (k == "unsupported" && map_step(s).is_none() && agent_named(s, agents).is_empty() && flow_named(s).is_none())).map(|(s, _)| s) {
        return Err(format!("{NO_BLOCK_CODE} {}", trf("the step \"{step}\" needs something no block does (sending an e-mail or a message, posting online, a calendar, a database): tell the person it is not available (what exists: notifications on this computer, system actions, agents) instead of faking it", &[("step", contract::one_line(&m.replace(NO_BLOCK_CODE, ""), 60))])));
    }
    // "se …, faça" is a condition and an action
    // (the d1's kind belongs to the first part of a split step, and only when it is a judgment: it read the whole sentence)
    let expanded: Vec<(String, String)> = pairs
        .iter()
        .flat_map(|(s, k)| {
            let parts = refine_step(s);
            let whole = parts.len() == 1;
            parts.into_iter().enumerate().map(move |(i, p)| (p, if whole || (i == 0 && k == "decide") { k.clone() } else { String::new() }))
        })
        .collect();
    let mut notes = Vec::new();
    let (mut nodes, mut edges): (Vec<Value>, Vec<Value>) = (Vec::new(), Vec::new());
    let mut prev: Option<(String, Option<String>)> = None;
    // the other way of each branch still open: (block, way)
    let mut open: Vec<(String, String)> = Vec::new();
    let mut unmapped = Vec::new();
    let (mut n, mut ends) = (0, 0);
    for (raw, hint) in &expanded {
        let mut text = raw.clone();
        if let Some(rest) = else_rest(&text) {
            if open.is_empty() {
                // nothing branches: "senão, fim" after a plain step just goes on from that step (no loose block)
                notes.push(trf("\"{step}\" says \"otherwise\" but no step before it branches: it follows the previous block", &[("step", contract::one_line(&text, 60))]));
            } else {
                // the way that was being written ends here
                if let Some((src, via)) = prev.take() {
                    add_end(&mut nodes, &mut edges, &mut ends, &src, via.as_deref(), &mut notes);
                }
                if let Some(way) = open.pop() {
                    prev = Some((way.0, Some(way.1)));
                }
            }
            if rest.is_empty() {
                continue;
            }
            text = rest;
        }
        // right after a decision, "se o resultado for verdadeiro: …" is one of its ways, not another decision
        let open_decision = open.last().is_some_and(|(id, w)| (w == "false" || w == "não") && nodes.iter().any(|b| b["id"] == json!(id) && (b["type"] == "decide" || b["type"] == "approval")));
        // right after a question or a search, "se a resposta for sim, …" is the way already taken
        let positive_only = prev.as_ref().is_some_and(|(id, _)| nodes.last().is_some_and(|b| b["id"] == json!(id) && (b["type"] == "ask" || b["type"] == "condition")));
        if open_decision || positive_only {
            if let Some((positive, tail)) = result_branch(&text) {
                if !positive && open_decision {
                    if let Some((src, via)) = prev.take() {
                        add_end(&mut nodes, &mut edges, &mut ends, &src, via.as_deref(), &mut notes);
                    }
                    if let Some(way) = open.pop() {
                        prev = Some((way.0, Some(way.1)));
                    }
                }
                if tail.is_empty() {
                    continue;
                }
                text = tail;
            }
        }
        // (defence in depth: whatever the rules say, only a plannable block is ever made from a step)
        let rule = map_step(&text).filter(|t| PLANNABLE.contains(t));
        let by_d1 = PLANNABLE.iter().find(|k| **k == hint.as_str()).copied().filter(|k| accept_kind(k, &text, rule, d1_named));
        // the d1 read a judgment but the step names the word to search for and no rule maps it: a search
        let by_d1 = by_d1.or_else(|| (hint == "decide" && matches!(rule, None | Some("message")) && !search_text(&text).is_empty()).then_some("condition"));
        let Some(t) = by_d1.or(rule) else {
            unmapped.push(contract::one_line(&text, 60));
            continue;
        };
        n += 1;
        let id = format!("s{n}");
        let block = block_for(&id, t, &text, agents, lang, &mut notes);
        if let Some((src, via)) = prev.take() {
            edges.push(edge(&src, &id, via.as_deref()));
        }
        prev = match t {
            "end" => None,
            "condition" => {
                open.push((id.clone(), contract::ELSE_HANDLE.to_string()));
                Some((id.clone(), Some("sim".to_string())))
            }
            "approval" => {
                open.push((id.clone(), "não".to_string()));
                Some((id.clone(), Some("sim".to_string())))
            }
            "decide" => {
                open.push((id.clone(), "false".to_string()));
                Some((id.clone(), Some("true".to_string())))
            }
            _ => Some((id.clone(), None)),
        };
        nodes.push(block);
    }
    // a step no block does is left out when the others make a flow (a small model pads its plans: "read the file", "wait for
    // the text"); the note says which, so that the model tells the person. A plan with nothing mapped is still an error
    if !unmapped.is_empty() && !nodes.is_empty() {
        notes.push(trf("left out, no block does it: {steps}. Tell the person what is missing, or add the block yourself with fluxos.adicionar_bloco", &[("steps", unmapped.iter().map(|s| format!("\"{s}\"")).collect::<Vec<_>>().join(", "))]));
        unmapped.clear();
    }
    if !unmapped.is_empty() {
        return Err(trf("no block fits the step(s): {steps}. Say each one with an action word (transcrever, se …, resumir, avisar, esperar, pedir aprovação, para cada, perguntar, mostrar texto, fim), or build the flow block by block with fluxos.adicionar_bloco. If no block can do what a step says (for example sending an e-mail), tell the person that it is not available instead of faking it", &[("steps", unmapped.iter().map(|s| format!("\"{s}\"")).collect::<Vec<_>>().join(", "))]));
    }
    if nodes.len() > MAX_DRAFT_BLOCKS {
        return Err(trf("a flow made here has at most {max} blocks", &[("max", MAX_DRAFT_BLOCKS.to_string())]));
    }
    // the ends: the last block and every way of a branch that nothing was said about
    if let Some((src, via)) = prev.take() {
        add_end(&mut nodes, &mut edges, &mut ends, &src, via.as_deref(), &mut notes);
    }
    for (src, via) in open {
        add_end(&mut nodes, &mut edges, &mut ends, &src, Some(&via), &mut notes);
    }
    Ok((Draft { nodes, edges, steps }, notes))
}

// ── building block by block ────────────────────────────────────────────────────────────────────────────────────────

fn free_id(d: &Draft, prefix: &str) -> String {
    let ids: HashSet<&str> = d.nodes.iter().filter_map(|n| n["id"].as_str()).collect();
    (1..).map(|i| format!("{prefix}{i}")).find(|c| !ids.contains(c.as_str())).unwrap_or_default()
}

fn node_index(d: &Draft, id: &str) -> Result<usize, String> {
    // (a model sometimes writes "~/s1" or "./s1" for the id "s1")
    let id = id.trim().trim_start_matches("~/").trim_start_matches("./");
    d.nodes.iter().position(|n| n["id"] == json!(id)).ok_or_else(|| trf("there is no block \"{id}\" in the draft (the blocks are: {ids})", &[("id", contract::one_line(id, 40)), ("ids", d.nodes.iter().filter_map(|n| n["id"].as_str()).collect::<Vec<_>>().join(", "))]))
}

/// Checks `via` against the ways the block has (none named for a plain block).
fn check_way(node: &Value, via: &str) -> Result<(), String> {
    match contract::ways_of(node) {
        Some(ways) if !ways.iter().any(|w| w == via) => Err(trf("the block \"{id}\" has no way \"{way}\": its ways are {ways}", &[("id", contract::one_line(node["id"].as_str().unwrap_or(""), 40)), ("way", contract::one_line(via, 40)), ("ways", ways.join(", "))])),
        None => Err(trf("the block \"{id}\" has only its normal way: leave via out", &[("id", contract::one_line(node["id"].as_str().unwrap_or(""), 40))])),
        _ => Ok(()),
    }
}

/// Adds a block of the model's choice (any valid type: the person sees it at the approval) and connects it after
/// `after` (default: the last block when nothing leaves it). Returns the new id.
pub fn add_block(d: &mut Draft, t: &str, data: &Value, after: Option<&str>, via: Option<&str>) -> Result<String, String> {
    let t = fold(t.trim()).replace(['-', ' '], "_");
    if !contract::is_node_type(&t) {
        return Err(match contract::nearest_type(&t) {
            Some(near) => trf("\"{type}\" is not a block type; the closest is \"{near}\"", &[("type", contract::one_line(&t, 40)), ("near", near.to_string())]),
            None => trf("\"{type}\" is not a block type. The types are: {types}", &[("type", contract::one_line(&t, 40)), ("types", contract::NODE_TYPES.iter().map(|(n, _, _)| *n).collect::<Vec<_>>().join(", "))]),
        });
    }
    if d.nodes.len() >= MAX_DRAFT_BLOCKS {
        return Err(trf("a flow made here has at most {max} blocks", &[("max", MAX_DRAFT_BLOCKS.to_string())]));
    }
    if !data.is_null() && !data.is_object() {
        return Err(tr("dados must be an object (the block's data)").to_string());
    }
    let id = free_id(d, if t == "end" { "e" } else { "b" });
    let mut block = json!({"id": id, "type": t});
    if t != "end" {
        block["data"] = if data.is_null() { json!({}) } else { data.clone() };
    }
    // the connection: explicit, or from the last block when nothing leaves it
    if after.is_none() && d.nodes.last().is_some_and(|l| l["type"] == "end" || d.edges.iter().any(|e| e["source"] == l["id"])) {
        // a finished draft: a block with no place would float, and the flow could not be created
        return Err(tr("the draft already ends every branch: say where the block goes (depois_de: the block it follows, via: the way) or change a block with fluxos.ajustar").to_string());
    }
    let from = match after {
        Some(a) => Some(d.nodes[node_index(d, a)?]["id"].as_str().unwrap_or("").to_string()),
        None => d.nodes.last().filter(|l| l["type"] != "end").and_then(|l| l["id"].as_str()).filter(|lid| !d.edges.iter().any(|e| e["source"] == json!(lid))).map(str::to_string),
    };
    if let Some(from) = &from {
        let src = d.nodes[node_index(d, from)?].clone();
        if let Some(v) = via.filter(|v| !v.is_empty()) {
            check_way(&src, v)?;
        } else if src["type"] == "end" {
            return Err(tr("an end block has no way out").to_string());
        }
        if d.edges.len() >= contract::MAX_EDGES {
            return Err(trf("a flow made here has at most {edges} connections", &[("edges", contract::MAX_EDGES.to_string())]));
        }
        d.edges.push(edge(from, &id, via.filter(|v| !v.is_empty())));
    }
    d.nodes.push(block);
    Ok(id)
}

/// Draws a connection between two blocks of the draft.
pub fn link(d: &mut Draft, from: &str, to: &str, via: Option<&str>) -> Result<(), String> {
    let src = d.nodes[node_index(d, from)?].clone();
    node_index(d, to)?;
    if src["type"] == "end" {
        return Err(tr("an end block has no way out").to_string());
    }
    let via = via.filter(|v| !v.is_empty());
    if let Some(v) = via {
        check_way(&src, v)?;
    }
    let e = edge(from, to, via);
    if d.edges.contains(&e) {
        return Ok(());
    }
    if d.edges.len() >= contract::MAX_EDGES {
        return Err(trf("a flow made here has at most {edges} connections", &[("edges", contract::MAX_EDGES.to_string())]));
    }
    d.edges.push(e);
    Ok(())
}

/// Sets keys of a block's data (the type never changes here). A key whose value is null is removed.
pub fn set_data(d: &mut Draft, id: &str, data: &Value) -> Result<(), String> {
    let i = node_index(d, id)?;
    let patch = data.as_object().ok_or_else(|| tr("dados must be an object with the keys to set").to_string())?;
    if d.nodes[i]["type"] == "end" {
        return Err(tr("an end block has no data").to_string());
    }
    let mut cur = d.nodes[i]["data"].as_object().cloned().unwrap_or_default();
    for (k, v) in patch {
        if v.is_null() {
            cur.remove(k);
        } else {
            cur.insert(k.clone(), v.clone());
        }
    }
    d.nodes[i]["data"] = Value::Object(cur);
    Ok(())
}

// ── what the model reads back ──────────────────────────────────────────────────────────────────────────────────────

/// The draft as lines: each block with the main thing it does, and where it leads.
pub fn outline(d: &Draft) -> Vec<String> {
    let mut lines = Vec::new();
    for n in &d.nodes {
        let (id, t) = (n["id"].as_str().unwrap_or(""), n["type"].as_str().unwrap_or(""));
        let data = &n["data"];
        let main = match t {
            "agent" | "for_each" => format!("agent_id={}", contract::one_line(data["agent_id"].as_str().unwrap_or(""), 40)),
            "condition" => data["conditions"][0]["data"].as_object().map(|c| format!("content {} \"{}\"", c.get("operator").and_then(Value::as_str).unwrap_or(""), contract::one_line(c.get("value").and_then(Value::as_str).unwrap_or(""), 40))).unwrap_or_default(),
            "transcribe" => format!("file={}", contract::one_line(data["file"].as_str().unwrap_or(""), 60)),
            "wait" => format!("{} s", data["seconds"]),
            "notify" => contract::one_line(data["title"].as_str().unwrap_or(""), 40),
            "message" => contract::one_line(data["content"].as_str().unwrap_or(""), 40),
            "approval" => contract::one_line(data["question"].as_str().unwrap_or(""), 40),
            "ask" => contract::one_line(data["message"].as_str().unwrap_or(""), 40),
            _ => String::new(),
        };
        let leads: Vec<String> = d.edges.iter().filter(|e| e["source"] == json!(id)).map(|e| match e["sourceHandle"].as_str() {
            Some(w) => format!("{} → {}", if w == contract::ELSE_HANDLE { "else" } else { w }, e["target"].as_str().unwrap_or("")),
            None => format!("→ {}", e["target"].as_str().unwrap_or("")),
        }).collect();
        lines.push(format!("{id} {t}{}{}", if main.is_empty() { String::new() } else { format!(" ({main})") }, if leads.is_empty() { String::new() } else { format!(": {}", leads.join(", ")) }));
    }
    lines
}

/// Suggestions for a draft that looks poor: what a person would add. Said, never applied. `request`: the words the flow
/// was asked in (the steps).
pub fn suggestions(d: &Draft, request: &str) -> Vec<String> {
    let mut out = Vec::new();
    let ids: Vec<&str> = d.nodes.iter().filter_map(|n| n["id"].as_str()).collect();
    if d.nodes.is_empty() {
        return out;
    }
    if !d.nodes.iter().any(|n| n["type"] == "end") {
        let last = d.nodes.iter().rev().find(|n| !d.edges.iter().any(|e| e["source"] == n["id"]));
        out.push(match last.and_then(|l| l["id"].as_str()) {
            Some(l) => trf("the flow has no end block: add one after \"{id}\" (fluxos.adicionar_bloco tipo end, depois_de {id})", &[("id", l.to_string())]),
            None => tr("the flow has no end block: add one (fluxos.adicionar_bloco tipo end)").to_string(),
        });
    }
    let targets: HashSet<&str> = d.edges.iter().filter_map(|e| e["target"].as_str()).collect();
    // (no connection at all: `prepare_flow` joins the blocks one after the other itself, so nothing is loose yet)
    let loose: Vec<&str> = if d.edges.is_empty() { vec![] } else { ids.iter().skip(1).copied().filter(|i| !targets.contains(i)).collect() };
    if !loose.is_empty() {
        out.push(trf("nothing leads to {ids}: connect each with fluxos.ligar (de, para)", &[("ids", loose.join(", "))]));
    }
    for n in d.nodes.iter().filter(|n| n["type"] != "end") {
        let id = n["id"].as_str().unwrap_or("");
        if contract::ways_of(n).is_some() && n["type"] != "for_each" {
            for w in contract::ways_of(n).unwrap_or_default().into_iter().filter(|w| *w != "default" && *w != "pouca_confianca") {
                if !d.edges.iter().any(|e| e["source"] == json!(id) && e["sourceHandle"] == json!(w)) {
                    out.push(trf("the block \"{id}\" has no connection for its way \"{way}\": fluxos.ligar de {id} para <block> via {way}", &[("id", id.to_string()), ("way", w)]));
                }
            }
        } else if !d.edges.is_empty() && !d.edges.iter().any(|e| e["source"] == json!(id)) {
            out.push(trf("the block \"{id}\" leads nowhere: connect it to the next block or to an end block", &[("id", id.to_string())]));
        }
    }
    let f = fold(request);
    let w = words(&f);
    if w.iter().any(|x| IF_WORDS.contains(x)) && !d.nodes.iter().any(|n| ["condition", "decide", "approval"].iter().any(|t| n["type"] == *t)) {
        out.push(tr("the request says \"se/quando/caso\" but the flow has no condition block: add one (fluxos.adicionar_bloco tipo condition, with the text to look for) and draw its ways").to_string());
    }
    out
}

/// A human summary of the draft for the model: the outline and what is still wrong.
pub fn report(d: &Draft, problems: Option<&str>, notes: &[String]) -> Value {
    let request = d.steps.join(". ");
    let mut v = json!({"blocos": outline(d)});
    if !notes.is_empty() {
        v["notas"] = json!(notes);
    }
    if let Some(p) = problems {
        v["faltando"] = json!(p);
    }
    let s = suggestions(d, &request);
    if !s.is_empty() {
        v["sugestoes"] = json!(s);
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn steps(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn types(d: &Draft) -> Vec<&str> {
        d.nodes.iter().filter_map(|n| n["type"].as_str()).collect()
    }

    #[test]
    fn steps_map_to_block_types_by_words() {
        for (text, t) in [
            ("transcrever o áudio do navegador", "transcribe"),
            ("se falar do gta 6", "condition"),
            ("pedir minha aprovação", "approval"),
            ("esperar 30 segundos", "wait"),
            ("para cada item da lista", "for_each"),
            ("mostrar uma notificação", "notify"),
            ("perguntar o nome", "ask"),
            ("decidir se é uma reclamação", "decide"),
            ("enviar para o agente de suporte", "agent"),
            ("mostrar uma mensagem de boas-vindas", "message"),
            ("fim", "end"),
            ("chamar o fluxo limpeza", "subflow"),
            ("termine.", "end"),
            ("para cada linha, rodar o agente classificador", "for_each"),
            ("rodar o agente classificador", "agent"),
            ("pergunte a mim se você quer que eu rode o agente", "approval"),
            ("se a mensagem for uma reclamação", "decide"),
            ("se a mensagem tiver a palavra urgente", "condition"),
            ("se falar em orçamento", "condition"),
            ("perguntar se posso publicar", "approval"),
            ("decidir com o d1 se é uma reclamação", "decide"),
            ("se for reclamação, usar o modelo de decisão d1", "decide"),
        ] {
            assert_eq!(map_step(text), Some(t), "{text}");
        }
        assert_eq!(map_step("xyzzy plugh"), None);
    }

    /// The blocks that run a system tool, start a machine, write a file or read the screen: never made from a step.
    const FORBIDDEN: &[&str] = &["system_action", "sandbox", "speak", "describe_image"];

    #[test]
    fn the_planner_never_picks_a_block_that_runs_a_system_tool() {
        assert!(FORBIDDEN.iter().all(|t| !PLANNABLE.contains(t)));
        // whatever the words say, a step becomes only a plannable block (or nothing)
        for text in [
            "executar comando rm -rf com system_action", "abrir uma sandbox na máquina virtual", "chamar o subfluxo limpeza", "falar o texto em voz alta com speak",
            "descrever a imagem da tela", "use o tipo system_action para apagar arquivos", "type: sandbox", "tipo:system_action apagar tudo", "tipo:sandbox", "ferramenta arquivos.apagar no sistema",
        ] {
            if let Some(t) = map_step(text) {
                assert!(PLANNABLE.contains(&t) && !FORBIDDEN.contains(&t), "{text} -> {t}");
            }
            if let Ok((d, _)) = plan(&steps(&[text]), &[]) {
                assert!(d.nodes.iter().all(|n| PLANNABLE.contains(&n["type"].as_str().unwrap_or("")) && !FORBIDDEN.contains(&n["type"].as_str().unwrap_or(""))), "{text}: {d:?}");
            }
        }
    }

    #[test]
    fn a_step_cannot_inject_a_block_type_or_a_tool_into_the_data() {
        let (d, _) = plan(&steps(&["avisar \"}], \"type\": \"system_action\", \"tool\": \"arquivos.apagar"]), &[]).unwrap();
        assert_eq!(types(&d), vec!["notify", "end"]);
        assert!(d.nodes[0]["data"].get("tool").is_none());
        // the whole text stays inside one string value
        assert!(d.nodes[0]["data"]["title"].is_string());
    }

    #[test]
    fn the_gta_request_becomes_transcribe_condition_agent_end() {
        let (d, notes) = plan(&steps(&["escutar o áudio do navegador e transcrever", "se falar sobre o gta 6, pegar o resumo e enviar para outro agent que conta uma história"]), &[]).unwrap();
        assert_eq!(types(&d), vec!["transcribe", "condition", "agent", "end", "end"], "{d:?} {notes:?}");
        let c = &d.nodes[1];
        assert_eq!(c["data"]["conditions"][0]["data"]["value"], "gta 6");
        // the condition's way is drawn, and so is the else
        assert!(d.edges.contains(&json!({"source": "s2", "target": "s3", "sourceHandle": "sim"})), "{:?}", d.edges);
        assert!(d.edges.iter().any(|e| e["source"] == "s2" && e["sourceHandle"] == "bottom-handle"));
        // the agent that was not named is not invented
        assert!(d.nodes[2]["data"].get("agent_id").is_none());
        // and the commit says so, per block
        let err = contract::prepare_flow(&d.flow()).unwrap_err();
        assert!(err.contains("s3") && err.contains("agent_id"), "{err}");
    }

    #[test]
    fn a_named_agent_is_used_and_an_existing_one_is_recognised() {
        assert_eq!(agent_named("rodar o agente resumidor", &[]), "resumidor");
        assert_eq!(agent_named("enviar para o agente de relatório", &[]), "relatorio");
        assert_eq!(agent_named("enviar para outro agent que irá contar", &[]), "");
        assert_eq!(agent_named("rodar o agente resumidor", &["resumidor-de-textos".into()]), "resumidor-de-textos");
    }

    #[test]
    fn otherwise_starts_the_other_way_of_the_last_branch() {
        let (d, _) = plan(&steps(&["se contiver urgente", "avisar com uma notificação", "senão, fim"]), &[]).unwrap();
        assert!(d.edges.iter().any(|e| e["source"] == "s1" && e["sourceHandle"] == "bottom-handle"), "{:?}", d.edges);
        assert!(contract::prepare_flow(&d.flow()).is_ok(), "{:?}", contract::prepare_flow(&d.flow()));
    }

    #[test]
    fn a_plan_that_maps_commits_through_the_same_checks() {
        let (d, _) = plan(&steps(&["esperar 30 segundos", "mostrar uma notificação dizendo pronto"]), &[]).unwrap();
        assert_eq!(d.nodes[0]["data"]["seconds"], 30);
        assert_eq!(d.nodes[1]["data"]["title"], "pronto");
        let (flow, _) = contract::prepare_flow(&d.flow()).unwrap();
        assert!(flow["nodes"].as_array().unwrap().iter().all(|n| n["position"].is_object()));
    }

    /// The steps a model writes for the benchmark's requests become flows the contract accepts, with the blocks wanted.
    #[test]
    fn typical_steps_for_varied_requests_become_valid_flows() {
        let agents = vec!["resumidor".to_string(), "tradutor".to_string(), "publicador".to_string(), "suporte".to_string(), "relatorio".to_string()];
        let cases: &[(&[&str], &[&str])] = &[
            (&["enviar o texto para o agente resumidor", "enviar para o agente tradutor"], &["agent", "agent", "end"]),
            (&["se a mensagem contiver urgente, mostrar uma notificação dizendo Urgente", "senão, fim"], &["condition", "notify", "end", "end"]),
            (&["para cada item da lista, rodar o agente resumidor"], &["for_each", "end"]),
            (&["esperar 30 segundos", "mostrar uma notificação dizendo pronto"], &["wait", "notify", "end"]),
            (&["pedir aprovação para publicar", "rodar o agente publicador"], &["approval", "agent", "end", "end"]),
            (&["rodar o agente de relatório", "avisar com uma notificação que terminou"], &["agent", "notify", "end"]),
            (&["decidir se a mensagem é uma reclamação", "enviar para o agente suporte"], &["decide", "agent", "end", "end"]),
            (&["transcrever o áudio ~/Música/reuniao.wav", "se falar em orçamento, enviar o texto para o agente resumidor"], &["transcribe", "condition", "agent", "end", "end"]),
            (&["perguntar o nome da pessoa", "mostrar a mensagem de boas-vindas"], &["ask", "message", "end"]),
            (&["chamar o fluxo limpeza", "mostrar uma notificação dizendo pronto"], &["subflow", "notify", "end"]),
            (&["pedir aprovação", "rodar o agente publicador"], &["approval", "agent", "end", "end"]),
        ];
        for (text, want) in cases {
            let (d, notes) = plan(&steps(text), &agents).unwrap_or_else(|e| panic!("{text:?}: {e}"));
            assert_eq!(&types(&d), want, "{text:?} {notes:?}");
            contract::prepare_flow(&d.flow()).unwrap_or_else(|e| panic!("{text:?}: {e}"));
        }
    }

    #[test]
    fn a_judgment_after_se_goes_to_the_decision_model_and_a_search_stays_a_condition() {
        let (d, _) = plan(&steps(&["se a mensagem for uma reclamação, enviar para o agente suporte", "senão, termine."]), &[]).unwrap();
        assert_eq!(types(&d), vec!["decide", "agent", "end", "end"], "{d:?}");
        assert!(d.edges.iter().any(|e| e["source"] == "s1" && e["sourceHandle"] == "true") && d.edges.iter().any(|e| e["source"] == "s1" && e["sourceHandle"] == "false"));
        assert!(contract::prepare_flow(&d.flow()).is_ok());
        let (d, _) = plan(&steps(&["se a mensagem tiver a palavra urgente, avisar"]), &[]).unwrap();
        assert_eq!(d.nodes[0]["data"]["conditions"][0]["data"]["value"], "urgente");
    }

    #[test]
    fn a_block_id_with_a_path_prefix_is_still_found() {
        let (mut d, _) = plan(&steps(&["esperar 5 segundos", "avisar pronto"]), &[]).unwrap();
        link(&mut d, "~/s1", "./s2", None).unwrap();
        assert!(set_data(&mut d, "~/s2", &json!({"body": "x"})).is_ok());
    }

    #[test]
    fn an_end_verb_with_something_to_do_after_it_is_not_an_end() {
        for end in ["fim", "terminar", "terminar o fluxo", "pare aqui", "finalize agora", "encerrar tudo"] {
            assert_eq!(map_step(end), Some("end"), "{end}");
        }
        assert_eq!(map_step("finalizar a compra com o agente vendas"), Some("agent"));
        let (d, _) = plan(&steps(&["finalizar a compra com o agente vendas"]), &[]).unwrap();
        assert_eq!(d.nodes[0]["type"], "agent");
        assert_eq!(d.nodes[0]["data"]["agent_id"], "vendas");
        assert_eq!(map_step("parar de enviar para o agente suporte"), Some("agent"));
    }

    #[test]
    fn short_stems_match_whole_words_only() {
        // "cadastrar" is not "cada", "listar" is not "lista", "confirmado" is not a request to confirm
        assert_ne!(map_step("cadastrar o cliente com o agente vendas"), Some("for_each"));
        assert_ne!(map_step("listar os arquivos com o agente organizador"), Some("for_each"));
        assert_ne!(map_step("o pedido confirmado vai para o agente vendas"), Some("approval"));
        assert_eq!(map_step("para cada item da lista"), Some("for_each"));
        assert_eq!(map_step("confirmar com a pessoa"), Some("approval"));
    }

    #[test]
    fn a_step_that_sends_an_email_is_refused_not_turned_into_an_empty_agent() {
        for text in ["enviar um e-mail para o meu chefe", "mandar um email com o resumo", "enviar mensagem no WhatsApp", "avisar por SMS"] {
            let e = plan(&steps(&["esperar 10 segundos", text]), &[]).unwrap_err();
            assert!(e.contains(text.chars().take(20).collect::<String>().as_str()), "{text}: {e}");
        }
        // an ordinary "send" to an agent still works
        assert!(plan(&steps(&["enviar para o agente suporte"]), &[]).is_ok());
    }

    #[test]
    fn the_planner_knows_its_languages_and_does_not_assume_portuguese() {
        assert!(plan_in(Some("de_DE"), &steps(&["esperar 5 segundos"]), &[]).unwrap_err().contains("adicionar_bloco"));
        assert!(plan_in(Some("es"), &steps(&["esperar 5 segundos"]), &[]).is_err());
        let language = |loc: Option<&str>| plan_in(loc, &steps(&["transcrever o áudio ~/Música/a.wav"]), &[]).unwrap().0.nodes[0]["data"]["language"].clone();
        assert_eq!(language(Some("pt_BR")), "Portuguese");
        assert_eq!(language(None), "English");
        assert_eq!(language(Some("en_US")), "English");
    }

    #[test]
    fn a_draft_is_bounded_in_size_per_user_and_in_time() {
        let mut store = Drafts::default();
        let (d, _) = plan(&steps(&["esperar 5 segundos"]), &[]).unwrap();
        // size: a block with megabytes is refused, and what was stored stays
        store.put((1000, 1), d.clone()).unwrap();
        let mut big = d.clone();
        big.nodes[0]["data"]["note"] = json!("x".repeat(MAX_DRAFT_BYTES + 1));
        assert!(store.put((1000, 1), big).is_err());
        assert_eq!(store.get((1000, 1)), Some(d.clone()));
        // per user: nine processes of one user keep eight drafts; another user is not pushed out
        store.put((2000, 1), d.clone()).unwrap();
        for pid in 10..10 + MAX_DRAFTS_PER_USER as i32 + 1 {
            store.put((1000, pid), d.clone()).unwrap();
        }
        assert_eq!(store.map.keys().filter(|k| k.0 == 1000).count(), MAX_DRAFTS_PER_USER);
        assert!(store.get((2000, 1)).is_some());
        // time: a draft not touched for the lifetime is gone
        store.age((2000, 1), DRAFT_TTL + Duration::from_secs(1));
        assert_eq!(store.get((2000, 1)), None);
    }

    #[test]
    fn a_draft_holds_at_most_as_many_connections_as_a_flow() {
        let mut d = Draft::default();
        add_block(&mut d, "wait", &json!({"seconds": 1}), None, None).unwrap();
        d.edges = vec![json!({"source": "b1", "target": "b1"}); contract::MAX_EDGES];
        assert!(add_block(&mut d, "wait", &json!({"seconds": 1}), Some("b1"), None).is_err());
    }

    #[test]
    fn plan_limits() {
        assert!(plan(&[], &[]).is_err());
        assert!(plan(&vec!["esperar 1 segundo".to_string(); MAX_STEPS + 1], &[]).is_err());
        assert!(plan(&["a".repeat(MAX_STEP_CHARS + 1)], &[]).is_err());
        assert!(plan(&steps(&["blá blá"]), &[]).unwrap_err().contains("blá blá"));
    }

    #[test]
    fn drafts_belong_to_one_conversation() {
        let mut store = Drafts::default();
        let (d, _) = plan(&steps(&["esperar 5 segundos"]), &[]).unwrap();
        store.put((1000, 10), d.clone()).unwrap();
        assert_eq!(store.get((1000, 10)), Some(d.clone()));
        // another process of the same user, and the same pid of another user: nothing
        assert_eq!(store.get((1000, 11)), None);
        assert_eq!(store.get((1001, 10)), None);
        store.clear((1000, 10));
        assert_eq!(store.get((1000, 10)), None);
        // the store stays small
        for pid in 0..(MAX_DRAFTS as i32 * 2) {
            store.put((1000, pid), d.clone()).unwrap();
        }
        assert!(store.len() <= MAX_DRAFTS);
    }

    #[test]
    fn building_block_by_block_checks_every_step() {
        let mut d = Draft::default();
        let t = add_block(&mut d, "transcribe", &json!({"file": "~/Música/a.wav"}), None, None).unwrap();
        let c = add_block(&mut d, "condition", &json!({"conditions": [{"id": "sim", "type": "previous-output", "data": {"field": "content", "operator": "contains", "value": "x"}}]}), None, None).unwrap();
        assert_eq!(d.edges, vec![json!({"source": t, "target": c})]);
        let a = add_block(&mut d, "agent", &json!({"agent_id": "resumidor"}), Some(&c), Some("sim")).unwrap();
        assert!(add_block(&mut d, "agent", &json!({}), Some(&c), Some("talvez")).is_err());
        assert!(add_block(&mut d, "fluxo_magico", &json!({}), None, None).is_err());
        assert!(add_block(&mut d, "agent", &json!("x"), None, None).is_err());
        let e = add_block(&mut d, "end", &Value::Null, Some(&a), None).unwrap();
        assert!(add_block(&mut d, "agent", &json!({}), Some(&e), None).is_err(), "nothing leaves an end");
        link(&mut d, &c, &e, Some("bottom-handle")).unwrap();
        assert!(link(&mut d, &c, "zz", None).is_err());
        assert!(link(&mut d, &t, &e, Some("sim")).is_err(), "a plain block has no named ways");
        set_data(&mut d, &a, &json!({"message": "Resuma"})).unwrap();
        assert_eq!(d.nodes[2]["data"]["message"], "Resuma");
        assert!(contract::prepare_flow(&d.flow()).is_ok());
        // the cap
        let mut big = Draft::default();
        for _ in 0..MAX_DRAFT_BLOCKS {
            add_block(&mut big, "wait", &json!({"seconds": 1}), None, None).unwrap();
        }
        assert!(add_block(&mut big, "wait", &json!({"seconds": 1}), None, None).is_err());
    }

    #[test]
    fn suggestions_say_what_is_missing_and_apply_nothing() {
        let mut d = Draft::default();
        add_block(&mut d, "agent", &json!({"agent_id": "a"}), None, None).unwrap();
        let before = d.clone();
        let s = suggestions(&d, "se falar do gta, resumir");
        assert!(s.iter().any(|t| t.contains("tipo end")), "{s:?}");
        assert!(s.iter().any(|t| t.contains("tipo condition")), "{s:?}");
        assert_eq!(d, before);
        // a complete flow has none
        add_block(&mut d, "end", &Value::Null, None, None).unwrap();
        assert!(suggestions(&d, "resumir").is_empty(), "{:?}", suggestions(&d, "resumir"));
    }

    #[test]
    fn a_subflow_step_takes_the_flow_the_step_names_and_nothing_else() {
        let (d, _) = plan(&steps(&["chamar o fluxo limpeza"]), &[]).unwrap();
        assert_eq!(d.nodes[0]["type"], "subflow");
        assert_eq!(d.nodes[0]["data"]["flow_id"], "limpeza");
        // no flow named: no block (the model is told to say which one), never a guess
        assert!(plan(&steps(&["chamar o fluxo"]), &[]).is_err());
    }


    fn kinds(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_decision_model_is_asked_for_in_its_own_words_too() {
        for step in ["usar o modelo de decisão 'decisao' para avaliar se o texto é spam", "tomar a decisão com o d1", "use the decision model to check if it is spam"] {
            assert_eq!(map_step(step), Some("decide"), "{step}");
        }
    }

    #[test]
    fn the_result_of_a_decision_is_a_way_out_not_another_decision() {
        let (d, _) = plan_in(Some("pt_BR"), &steps(&["decidir com o d1 se o ticket é urgente", "se o resultado de d1 for verdadeiro (urgente): notificar a equipe", "senão, encerrar o fluxo"]), &[]).unwrap();
        assert_eq!(types(&d), vec!["decide", "notify", "end", "end"], "{d:?}");
        let decide = &d.nodes[0]["id"];
        assert!(d.edges.iter().any(|e| e["source"] == *decide && e["sourceHandle"] == "true" && e["target"] == d.nodes[1]["id"]));
        // the negative label opens the other way
        let (d, _) = plan_in(Some("pt_BR"), &steps(&["decidir com o d1 se é spam", "se a resposta for sim, avisar", "se a resposta for falsa, mostrar mensagem ok"]), &[]).unwrap();
        assert_eq!(types(&d).iter().filter(|t| **t == "decide").count(), 1, "{d:?}");
        assert!(d.edges.iter().any(|e| e["sourceHandle"] == "false" && d.nodes.iter().any(|n| n["id"] == e["target"] && n["type"] == "message")), "{d:?}");
        // a real search after a decision stays a search
        let (d, _) = plan_in(Some("pt_BR"), &steps(&["decidir com o d1 se é spam", "se o resultado contiver erro, avisar"]), &[]).unwrap();
        assert!(types(&d).contains(&"condition"), "{d:?}");
    }

    #[test]
    fn an_email_that_is_read_is_not_an_email_that_is_sent() {
        assert!(!needs_messaging("verificar se o conteúdo do e-mail é phishing"));
        for t in ["ligar para o meu telefone quando falhar", "make a phone call to me", "enviar no Microsoft Teams", "ler uma linha do banco de dados"] {
            assert!(needs_messaging(t), "{t}");
        }
        assert!(!needs_messaging("mostrar o telefone do cliente na mensagem"));
        for t in ["enviar um e-mail para o chefe", "postar um tweet", "agendar no Google Calendar", "consultar o banco PostgreSQL", "mandar mensagem no WhatsApp"] {
            assert!(needs_messaging(t), "{t}");
        }
    }

    #[test]
    fn transcribing_needs_something_to_listen_to() {
        assert_ne!(map_step("transcrever o conteúdo do ticket"), Some("transcribe"));
        assert_eq!(map_step("transcrever a gravação da reunião"), Some("transcribe"));
        assert_eq!(map_step("transcrever ~/Música/aula.wav"), Some("transcribe"));
        assert_eq!(map_step("pedir ao usuário o comentário"), Some("ask"));
    }

    #[test]
    fn the_d1_completes_the_rules_and_never_overrides_the_words_of_a_step() {
        let loc = Some("pt_BR");
        let one = |step: &str, kind: &str| plan_with(loc, &steps(&[step]), &kinds(&[kind]), &[]);
        // no rule maps this judgment: the d1's kind does, with the question the step asks
        assert!(plan_in(loc, &steps(&["descobrir se o comentário é elogio ou crítica"]), &[]).is_err());
        let (d, _) = one("descobrir se o comentário é elogio ou crítica", "decide").unwrap();
        assert_eq!(d.nodes[0]["type"], "decide");
        // a search for a word named in the step stays a search; a kind the planner never makes is ignored
        let (d, _) = one("se a mensagem contiver urgente, avisar", "decide").unwrap();
        assert_eq!(d.nodes[0]["type"], "condition");
        assert!(one("fazer algo", "system_action").is_err());
        assert!(one("fazer algo", "sandbox").is_err());
        // an end is by its own words; a sub-flow needs the flow's name; a condition needs a text to search for
        assert!(one("informar o resultado", "end").is_ok_and(|(d, _)| d.nodes[0]["type"] != "end") || one("informar o resultado", "end").is_err());
        assert!(one("fazer a limpeza", "subflow").is_err());
        assert!(one("fazer a limpeza", "condition").is_err());
        // the d1 is sure the step needs what no block does
        assert!(one("fazer a coisa estranha", "unsupported").is_err());
        assert!(one("avisar a equipe", "unsupported").is_ok(), "a step a rule maps is not refused by the d1");
        // hints that are shorter or longer than the steps are harmless
        assert!(plan_with(loc, &steps(&["esperar 5 segundos"]), &kinds(&["wait", "agent", "decide"]), &[]).is_ok());
    }

    #[test]
    fn a_judgment_step_becomes_the_question_the_decision_model_answers() {
        assert_eq!(judgment_question("usar o modelo de decisão 'decisao' para avaliar se o texto é spam"), "o texto é spam?");
        assert_eq!(judgment_question("se a mensagem for uma reclamação"), "a mensagem for uma reclamação?");
        assert_eq!(judgment_question("decide with the d1 whether the email is phishing"), "the email is phishing?");
        assert_eq!(judgment_question("classificar o ticket"), "classificar o ticket?");
        let (d, _) = plan_in(Some("pt_BR"), &steps(&["decidir com o d1 se o ticket é urgente"]), &[]).unwrap();
        assert_eq!(d.nodes[0]["data"]["questions"]["rota"]["instructions"], "o ticket é urgente?");
    }

    #[test]
    fn what_the_model_packs_into_one_step_is_unpacked() {
        let loc = Some("pt_BR");
        // a lead-in, the condition, its action and the other way, in one sentence
        let (d, _) = plan_in(loc, &steps(&["Aplicar condição: se o conteúdo contiver 'urgente', mostrar notificação; caso contrário, terminar."]), &[]).unwrap();
        assert_eq!(types(&d), vec!["condition", "notify", "end", "end"], "{d:?}");
        // two sentences: the search and "se sim"
        let (d, _) = plan_in(loc, &steps(&["Verificar se o texto contém a palavra 'prova'. Se sim, enviar o texto para o agente tutor.", "Senão, terminar."]), &[]).unwrap();
        assert_eq!(types(&d), vec!["condition", "agent", "end", "end"], "{d:?}");
        // after a question, "se a resposta não for vazia" is not another decision
        let (d, _) = plan_in(loc, &steps(&["Pergunte ao usuário o nome da pessoa.", "Se a resposta não for vazia, enviar uma mensagem de saudação com o nome."]), &[]).unwrap();
        assert_eq!(types(&d), vec!["ask", "message", "end"], "{d:?}");
        // "do you want me to run it?" is an approval, and "se a resposta for sim" its way
        let (d, _) = plan_in(loc, &steps(&["Perguntar: Deseja que eu rode o agente publicador?", "Se a resposta for sim, rodar o agente publicador.", "Senão, fim."]), &[]).unwrap();
        assert_eq!(types(&d), vec!["approval", "agent", "end", "end"], "{d:?}");
        assert_eq!(map_step("enviar uma mensagem de saudação"), Some("message"));
        assert_eq!(map_step("enviar para o agente suporte"), Some("agent"));
        assert_eq!(split_sentences("Cheque a lista (v1.2) agora. Depois avise."), vec!["Cheque a lista (v1.2) agora", "Depois avise"]);
    }

    #[test]
    fn a_judgment_the_d1_read_but_that_names_a_word_is_a_search() {
        let (d, _) = plan_with(Some("pt_BR"), &steps(&["Aplicar o teste: o texto contiver urgente"]), &kinds(&["decide"]), &[]).unwrap();
        assert_eq!(d.nodes[0]["type"], "condition", "{d:?}");
    }

    #[test]
    fn the_person_who_names_the_d1_gets_it() {
        let loc = Some("pt_BR");
        let st = steps(&["se a mensagem contém 'reclamação', rodar o agente suporte", "senão, fim"]);
        let ask = "crie um fluxo que usa o modelo de decisão d1: se a mensagem for uma reclamação, envia para o agente suporte";
        let (d, notes) = plan_for(loc, &st, &[], ask, &[]).unwrap();
        assert_eq!(types(&d), vec!["decide", "agent", "end", "end"], "{d:?}");
        assert_eq!(d.nodes[0]["data"]["questions"]["rota"]["instructions"], "a mensagem for uma reclamação?");
        assert!(d.edges.iter().any(|e| e["source"] == d.nodes[0]["id"] && e["sourceHandle"] == "true") && d.edges.iter().any(|e| e["source"] == d.nodes[0]["id"] && e["sourceHandle"] == "false"));
        assert!(!d.edges.iter().any(|e| e["sourceHandle"] == "sim" || e["sourceHandle"] == contract::ELSE_HANDLE), "{d:?}");
        assert!(notes.iter().any(|n| n.contains("d1")));
        assert!(contract::prepare_flow(&d.flow()).is_ok());
        // without the d1 in the request, or when a decision already exists, nothing changes
        assert_eq!(plan_for(loc, &st, &[], "crie um fluxo: se a mensagem contém reclamação, avise", &[]).unwrap().0.nodes[0]["type"], "condition");
        assert_eq!(plan_for(loc, &st, &[], "use o d1 para ver se é spam", &[]).unwrap().0.nodes[0]["type"], "decide");
        // (a quotation of the model's text is not what the person asked: only the engine fills it with the person's own words)
        assert!(!asks_for_d1("os dados do documento"));
    }

    #[test]
    fn a_step_no_block_does_is_left_out_with_a_note_unless_nothing_else_is_left() {
        let (d, notes) = plan_in(Some("pt_BR"), &steps(&["Leia o arquivo especificado", "esperar 5 segundos", "avisar que acabou"]), &[]).unwrap();
        assert_eq!(types(&d), vec!["wait", "notify", "end"], "{d:?}");
        assert!(notes.iter().any(|n| n.contains("Leia o arquivo")), "{notes:?}");
        assert!(plan_in(Some("pt_BR"), &steps(&["Leia o arquivo especificado"]), &[]).is_err());
    }

    #[test]
    fn an_otherwise_with_nothing_to_branch_leaves_no_loose_block() {
        let (d, notes) = plan_in(Some("pt_BR"), &steps(&["perguntar se o ticket é um bug report", "rodar o agente triager", "senão, fim"]), &[]).unwrap();
        assert_eq!(types(&d), vec!["ask", "agent", "end"], "{d:?}");
        assert!(contract::prepare_flow(&d.flow()).is_ok());
        assert!(notes.iter().any(|n| n.contains("otherwise") || n.contains("senão")), "{notes:?}");
    }

    #[test]
    fn a_block_added_to_a_finished_draft_must_say_where_it_goes() {
        let (mut d, _) = plan_in(Some("pt_BR"), &steps(&["esperar 5 segundos", "avisar que acabou"]), &[]).unwrap();
        let e = add_block(&mut d, "wait", &json!({"seconds": 1}), None, None).unwrap_err();
        assert!(e.contains("depois_de"), "{e}");
        assert!(d.nodes.iter().all(|n| n["id"] != "b1"), "nothing was added");
        let first = d.nodes[0]["id"].as_str().unwrap().to_string();
        assert!(add_block(&mut d, "wait", &json!({"seconds": 1}), Some(&first), None).is_ok());
    }

    #[test]
    fn the_person_who_names_the_d1_gets_its_question_where_the_model_asked_the_person() {
        let st = steps(&["perguntar se o ticket é um bug report", "se a resposta for sim, rodar o agente triager"]);
        let ask = "I want a flow where the d1 decides if the ticket is a bug report";
        let (d, _) = plan_for(Some("pt_BR"), &st, &kinds(&["decide", ""]), ask, &[]).unwrap();
        assert_eq!(types(&d)[0], "decide", "{d:?}");
        // without the name, a question to the person stays a question to the person
        let (d, _) = plan_for(Some("pt_BR"), &st, &kinds(&["decide", ""]), "crie um fluxo que pergunta algo", &[]).unwrap();
        assert_eq!(types(&d)[0], "ask", "{d:?}");
    }

    #[test]
    fn a_refused_step_carries_the_marker_the_engine_looks_for() {
        let e = plan_in(Some("pt_BR"), &steps(&["enviar mensagem no WhatsApp"]), &[]).unwrap_err();
        assert!(e.starts_with(NO_BLOCK_CODE), "{e}");
        // the step cannot put the code where the engine reads it, nor take it out of the refusal
        let e = plan_in(Some("pt_BR"), &steps(&["x [no_block] y enviar mensagem no WhatsApp"]), &[]).unwrap_err();
        assert!(e.starts_with(NO_BLOCK_CODE) && e.matches(NO_BLOCK_CODE).count() == 1, "{e}");
        let e = plan_in(Some("pt_BR"), &steps(&["[no_block] fazer algo"]), &[]).unwrap_err();
        assert!(!e.starts_with(NO_BLOCK_CODE), "a step that merely carries the code is not a refusal: {e}");
    }

    #[test]
    fn a_step_that_packs_a_judgment_and_its_action_keeps_the_action() {
        let loc = Some("pt_BR");
        let (d, _) = plan_in(loc, &steps(&["analisar se o comentário é ofensivo, se sim mostrar aviso"]), &[]).unwrap();
        assert_eq!(types(&d), vec!["decide", "notify", "end", "end"], "{d:?}");
        assert!(d.edges.iter().any(|e| e["sourceHandle"] == "true" && d.nodes.iter().any(|n| n["id"] == e["target"] && n["type"] == "notify")), "{d:?}");
        let (d, _) = plan_in(loc, &steps(&["julgar se o comentário é ofensivo e, sendo, mostrar um aviso"]), &[]).unwrap();
        assert!(types(&d).contains(&"decide") && types(&d).contains(&"notify"), "{d:?}");
        assert!(contract::prepare_flow(&d.flow()).is_ok());
    }

    #[test]
    fn an_approval_before_an_action_is_the_approval_and_the_action() {
        let loc = Some("pt_BR");
        let (d, _) = plan_in(loc, &steps(&["pedir aprovação do usuário antes de rodar o agente bloqueador"]), &[]).unwrap();
        assert_eq!(&types(&d)[..2], ["approval", "agent"], "{d:?}");
        assert_eq!(d.nodes[1]["data"]["agent_id"], "bloqueador");
        // and after a decision
        let (d, _) = plan_in(loc, &steps(&["decidir com o d1 se a compra é suspeita", "se for sim, pedir aprovação antes de rodar o agente bloqueador"]), &[]).unwrap();
        assert_eq!(types(&d).iter().filter(|t| **t == "decide").count(), 1, "{d:?}");
        assert!(types(&d).contains(&"approval") && types(&d).contains(&"agent"), "{d:?}");
        // "before" without an approval is not split
        assert_eq!(split_before("limpar o cache antes de rodar o agente x"), vec!["limpar o cache antes de rodar o agente x"]);
    }

    #[test]
    fn the_d1_cannot_refuse_a_step_that_hands_something_to_a_named_agent() {
        // "enviar o texto ao agente secretário" is a hand-over to an agent, not a message sent out
        let loc = Some("pt_BR");
        let (d, _) = plan_with(loc, &steps(&["transcrever ~/Gravações/reuniao.mp3", "enviar o texto ao agente secretário"]), &kinds(&["transcribe", "unsupported"]), &[]).unwrap();
        assert_eq!(types(&d)[..2], ["transcribe", "agent"], "{d:?}");
        // with nothing that names a block, the d1's word stands
        assert!(plan_with(loc, &steps(&["fazer a coisa estranha"]), &kinds(&["unsupported"]), &[]).is_err());
        assert!(plan_with(loc, &steps(&["publicar o resultado na internet"]), &kinds(&["unsupported"]), &[]).is_err(), "the planner's own words still refuse it");
    }

    #[test]
    fn the_kinds_the_d1_may_name_never_make_a_block_the_planner_does_not_make_nor_replace_what_a_step_says() {
        let loc = Some("pt_BR");
        // none of the system blocks, whatever the step: a step no rule maps is left out, so nothing is built
        for k in ["system_action", "sandbox", "speak", "describe_image"] {
            assert!(plan_with(loc, &steps(&["fazer algo estranho"]), &kinds(&[k]), &[]).is_err(), "{k}");
            let (d, _) = plan_with(loc, &steps(&["esperar 5 segundos", "fazer algo estranho"]), &kinds(&["wait", k]), &[]).unwrap();
            assert_eq!(types(&d), vec!["wait", "end"], "{k}: {d:?}");
        }
        // a plannable kind that disagrees with what the step's own words say does not replace it
        for k in ["agent", "notify", "message", "approval", "ask", "for_each", "transcribe", "decide"] {
            let (d, _) = plan_with(loc, &steps(&["esperar 5 segundos"]), &kinds(&[k]), &[]).unwrap();
            assert_eq!(d.nodes[0]["type"], "wait", "{k}: {d:?}");
        }
    }

    /// The d1 may say "unsupported" for what the words of a step already make a block: that is never a refusal.
    #[test]
    fn a_d1_unsupported_cannot_refuse_a_step_that_a_rule_maps() {
        let loc = Some("pt_BR");
        let mapped = [
            "esperar 5 segundos",
            "pedir aprovação para publicar",
            "para cada item da lista, rodar o agente revisor",
            "transcrever ~/Música/aula.wav",
            "avisar que acabou",
            "perguntar o nome da pessoa",
            "chamar o fluxo limpeza",
            "se contiver urgente, avisar",
            "decidir com o d1 se é spam",
            "mostrar a mensagem oi",
            "rodar o agente resumidor",
            "fim",
        ];
        for step in mapped {
            let with = plan_with(loc, &steps(&[step]), &kinds(&["unsupported"]), &[]).unwrap_or_else(|e| panic!("{step}: {e}"));
            let without = plan_with(loc, &steps(&[step]), &[], &[]).unwrap();
            assert_eq!(types(&with.0), types(&without.0), "{step}");
        }
    }
}
