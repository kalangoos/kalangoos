//! The day summary for the desktop widget ("Resumo do dia · gerado pelo agente").
//!
//! Facts come from the system (kalangod: timeline of today, requests waiting, failed services, disks); the
//! local model only turns each fact into a short natural sentence, with the output constrained to JSON. A
//! sentence that loses or changes a number of its fact is discarded and the plain fact is used: the model
//! never invents data. Without the model, the facts are shown as they are (`gerado: false`).

use crate::kalangod::Client;
use crate::modelo::Model;
use kalango_i18n::{tr, trf};
use serde_json::{json, Value};

/// (kind, text): "feito" (done, teal check), "aviso" (needs attention, gold), "uso" (AI use, muted bolt).
type Fact = (&'static str, String);

fn local_midnight_unix() -> i64 {
    extern "C" {
        fn time(t: *mut i64) -> i64;
        fn localtime_r(t: *const i64, tm: *mut [i32; 16]) -> *mut [i32; 16];
    }
    unsafe {
        let now = time(std::ptr::null_mut());
        let mut tm = [0i32; 16];
        localtime_r(&now, &mut tm);
        // tm_sec, tm_min, tm_hour are the first three fields
        now - i64::from(tm[0]) - 60 * i64::from(tm[1]) - 3600 * i64::from(tm[2])
    }
}

fn call(k: &mut Client, tool: &str, args: Value) -> Option<Value> {
    k.call(tool, &args).ok().and_then(|(text, _)| serde_json::from_str(&text).ok())
}

pub fn facts(k: &mut Client, model_name: &str) -> Vec<Fact> {
    let mut out: Vec<Fact> = Vec::new();
    // what was done today (people's and agents' actions recorded by kalangod)
    // only what changed something (kalangod declares which tools only read)
    let read_only: Vec<String> = k.tools().map(|t| t.into_iter().filter(|t| t.read_only).map(|t| t.name).collect()).unwrap_or_default();
    if let Some(v) = call(k, "linha_do_tempo.listar", json!({"limite": 200})) {
        let since = local_midnight_unix();
        let today: Vec<&Value> = v["registros"].as_array().map(|a| a.iter().filter(|r| {
            r["time"].as_i64().unwrap_or(0) >= since && matches!(r["status"].as_str(), Some("ok" | "simulado" | "desfeito"))
                && !read_only.iter().any(|n| Some(n.as_str()) == r["tool"].as_str())
        }).collect()).unwrap_or_default();
        if !today.is_empty() {
            let examples: Vec<&str> = today.iter().filter_map(|r| r["summary"].as_str()).take(2).collect();
            out.push(("feito", if today.len() == 1 {
                trf("1 change today: {what}.", &[("what", examples.join(""))])
            } else {
                trf("{n} changes today, such as: {what}.", &[("n", today.len().to_string()), ("what", examples.join("; "))])
            }));
        }
    }
    // what waits for the person
    if let Ok(p) = k.pending() {
        if p > 0 {
            out.push(("aviso", if p == 1 { tr("1 request waits for your approval.").to_string() } else { trf("{n} requests wait for your approval.", &[("n", p.to_string())]) }));
        }
    }
    if let Some(v) = call(k, "servicos.listar", json!({})) {
        let failed: Vec<&str> = v["servicos"].as_array().map(|a| a.iter().filter(|s| s["ativo"] == "failed").filter_map(|s| s["nome"].as_str()).collect()).unwrap_or_default();
        if !failed.is_empty() {
            out.push(("aviso", trf("{n} service(s) failed: {names}.", &[("n", failed.len().to_string()), ("names", failed.iter().take(3).map(|s| s.trim_end_matches(".service")).collect::<Vec<_>>().join(", "))])));
        }
    }
    if let Some(v) = call(k, "discos.uso", json!({})) {
        for d in v["discos"].as_array().into_iter().flatten() {
            let pct = d["uso_pct"].as_u64().unwrap_or(0);
            if pct >= 90 {
                out.push(("aviso", trf("The disk {mount} is {pct}% full ({free} GB free).", &[
                    ("mount", d["montado_em"].as_str().unwrap_or("").to_string()), ("pct", pct.to_string()), ("free", d["livre_gb"].to_string())])));
            }
        }
    }
    out.push(("uso", trf("The AI ran on this computer ({model}); no data was sent out.", &[("model", model_name.to_string())])));
    out.truncate(4);
    out
}

/// Every number of the fact must still be in the sentence.
fn keeps_numbers(fact: &str, sentence: &str) -> bool {
    let nums = |s: &str| s.split(|c: char| !c.is_ascii_digit()).filter(|x| !x.is_empty()).map(str::to_string).collect::<Vec<_>>();
    let have = nums(sentence);
    nums(fact).iter().all(|n| have.contains(n))
}

/// The summary as JSON: {"gerado": bool, "itens": [{"tipo", "texto"}]}.
pub fn summary(k: &mut Client, model: Option<&Model>, model_label: &str, language: &str) -> Value {
    let facts = facts(k, model_label);
    let mut items: Vec<Value> = facts.iter().map(|(t, s)| json!({"tipo": t, "texto": s})).collect();
    let mut generated = false;
    if let Some(m) = model {
        let list: Vec<&str> = facts.iter().map(|(_, s)| s.as_str()).collect();
        let schema = json!({"type": "object", "properties": {"frases": {"type": "array", "items": {"type": "string"}, "minItems": list.len(), "maxItems": list.len()}}, "required": ["frases"]});
        let prompt = format!("Below are facts for a person's day summary on their desktop, already in {language}. For each, write one short sentence in {language} with exactly the same meaning, only more natural. Keep every number and name exactly. Add nothing, remove nothing, do not change who did what. Facts:\n{}",
            list.iter().enumerate().map(|(i, f)| format!("{}. {f}", i + 1)).collect::<Vec<_>>().join("\n"));
        if let Ok(v) = m.complete_json(&prompt, &schema) {
            if let Some(frases) = v["frases"].as_array() {
                for (i, f) in frases.iter().enumerate().take(items.len()) {
                    if let Some(s) = f.as_str().map(str::trim).filter(|s| !s.is_empty() && s.chars().count() < 160) {
                        if keeps_numbers(&facts[i].1, s) {
                            items[i]["texto"] = json!(s);
                            generated = true;
                        }
                    }
                }
            }
        }
    }
    json!({"gerado": generated, "itens": items})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewritten_sentence_must_keep_the_numbers() {
        assert!(keeps_numbers("3 requests wait.", "Você tem 3 pedidos esperando."));
        assert!(!keeps_numbers("3 requests wait.", "Você tem pedidos esperando."));
        assert!(!keeps_numbers("The disk / is 91% full (92 GB free).", "O disco está 90% cheio, com 92 GB livres."));
    }
}
