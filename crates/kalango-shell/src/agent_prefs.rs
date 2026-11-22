//! The agents' general settings (Agents › Settings › General, Notifications, Advanced), kept in
//! ~/.config/kalango/agente-ajustes.json. The engine (kalango-agente) reads the answer length and the steps per request
//! from the same file; the shell reads the rest where each one acts.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentPrefs {
    /// "curtas", "equilibradas" or "detalhadas" (the engine's system prompt)
    #[serde(rename = "respostas")]
    pub answers: String,
    /// a turn's steps stay open after the answer (else they fold under "N passos")
    #[serde(rename = "passos_abertos")]
    pub steps_open: bool,
    /// the local model's suggestions in the Editor, the Terminal and the Monitor
    #[serde(rename = "sugestoes")]
    pub suggestions: bool,
    /// the desktop's day summary written by the agent
    #[serde(rename = "resumo_do_dia")]
    pub day_summary: bool,
    /// the agents center starts on a new conversation ("nova") or the last one ("ultima")
    #[serde(rename = "comecar_em")]
    pub start_on: String,
    /// a task the agent finished out of sight: "tela" (a banner), "lista" (only in the list) or "nada"
    #[serde(rename = "aviso_tarefa")]
    pub task_notice: String,
    /// a routine's run: "sempre" or "falha" (only when it fails)
    #[serde(rename = "aviso_rotina")]
    pub routine_notice: String,
    /// a sound when the agent asks something or waits for an approval
    #[serde(rename = "som")]
    pub sound: bool,
    /// the agents' notices wait for Do not disturb (approvals always come)
    #[serde(rename = "nao_perturbe")]
    pub respect_dnd: bool,
    /// steps (tool calls) per request, at most (the engine)
    #[serde(rename = "passos_por_pedido")]
    pub steps_per_request: u32,
    /// conversations are kept this many days (0: always)
    #[serde(rename = "guardar_dias")]
    pub keep_days: u32,
    /// each step shows what the tool got and gave back
    #[serde(rename = "detalhes_tecnicos")]
    pub technical_details: bool,
    /// the conversation's memory (the engine's context.rs, docs/CONTEXTO.md): the last exchanges always sent whole
    #[serde(rename = "trocas_mantidas")]
    pub keep_exchanges: u32,
    /// past this many exchanges since the last summary, the older ones are summarized (0: only when the window fills)
    #[serde(rename = "resumir_a_cada")]
    pub compact_every: u32,
    /// the prompt's quality budget, in tokens (no control: kept as written in the file)
    #[serde(rename = "contexto_max_tokens")]
    pub context_max_tokens: u32,
    /// keys this version does not know (a newer engine's) stay in the file when the shell saves it
    #[serde(flatten)]
    pub other: serde_json::Map<String, serde_json::Value>,
}

impl Default for AgentPrefs {
    fn default() -> Self {
        AgentPrefs {
            answers: "equilibradas".into(),
            steps_open: false,
            suggestions: true,
            day_summary: true,
            start_on: "nova".into(),
            task_notice: "tela".into(),
            routine_notice: "falha".into(),
            sound: true,
            respect_dnd: true,
            steps_per_request: 24,
            keep_days: 0,
            technical_details: false,
            keep_exchanges: 6,
            compact_every: 40,
            context_max_tokens: 12_288,
            other: serde_json::Map::new(),
        }
    }
}

pub const MIN_STEPS: u32 = 4;
/// "Lembrar as últimas trocas inteiras" and "Resumir conversas longas a cada" (the engine's ranges).
pub const KEEP_RANGE: (u32, u32) = (1, 30);
pub const COMPACT_RANGE: (u32, u32) = (0, 60);
pub const MAX_STEPS: u32 = 48;

pub fn path() -> PathBuf {
    gtk::glib::user_config_dir().join("kalango/agente-ajustes.json")
}

pub fn load() -> AgentPrefs {
    std::fs::read_to_string(path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn save(p: &AgentPrefs) -> Result<(), String> {
    let f = path();
    if let Some(d) = f.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    std::fs::write(&f, serde_json::to_string_pretty(p).unwrap_or_default()).map_err(|e| e.to_string())
}

/// Changes one setting and keeps it (logged for the session test).
pub fn update(f: impl FnOnce(&mut AgentPrefs)) {
    let mut p = load();
    f(&mut p);
    p.steps_per_request = p.steps_per_request.clamp(MIN_STEPS, MAX_STEPS);
    p.keep_exchanges = p.keep_exchanges.clamp(KEEP_RANGE.0, KEEP_RANGE.1);
    p.compact_every = p.compact_every.clamp(COMPACT_RANGE.0, COMPACT_RANGE.1);
    if let Err(e) = save(&p) {
        eprintln!("kalango-shell: {}: {e}", path().display());
    }
    if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
        eprintln!("debug agente-ajustes: {}", serde_json::to_string(&p).unwrap_or_default());
    }
}

/// Conversations older than `keep_days` (their last change) go away; returns how many. Runs at start and daily.
pub fn prune_conversations(keep_days: u32) -> usize {
    if keep_days == 0 {
        return 0;
    }
    let limit = std::time::SystemTime::now() - std::time::Duration::from_secs(u64::from(keep_days) * 86_400);
    let dir = crate::conversa::conversas_dir();
    let Ok(rd) = std::fs::read_dir(&dir) else { return 0 };
    let mut n = 0;
    for e in rd.flatten() {
        let p = e.path();
        if !p.is_dir() {
            continue;
        }
        // the newest file inside says when it was last used
        let newest = std::fs::read_dir(&p).ok().into_iter().flatten().flatten()
            .filter_map(|f| f.metadata().ok()?.modified().ok()).max()
            .or_else(|| e.metadata().ok()?.modified().ok());
        // (a pinned conversation stays)
        let pinned = crate::conversa::meta(&e.file_name().to_string_lossy())["fixada"].as_bool().unwrap_or(false);
        if newest.is_some_and(|t| t < limit) && !pinned && std::fs::remove_dir_all(&p).is_ok() {
            n += 1;
        }
    }
    if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
        eprintln!("debug agente-ajustes: {n} conversas com mais de {keep_days} dias apagadas");
    }
    n
}

/// Everything that makes up the agents' configuration, in one file (Advanced › Export): these settings, the
/// permissions, the routines and the connectors' manifests.
pub fn export_bundle() -> serde_json::Value {
    let cfg = gtk::glib::user_config_dir().join("kalango");
    let read = |n: &str| std::fs::read_to_string(cfg.join(n)).ok().and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok());
    let mut connectors = serde_json::Map::new();
    if let Ok(rd) = std::fs::read_dir(crate::central_ajustes::pasta()) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "json") {
                if let Some(v) = std::fs::read_to_string(&p).ok().and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok()) {
                    connectors.insert(e.file_name().to_string_lossy().into_owned(), v);
                }
            }
        }
    }
    serde_json::json!({
        "kalangoos": "configuracao-dos-agentes", "versao": 1,
        "ajustes": serde_json::to_value(load()).unwrap_or_default(),
        "permissoes": read("agente-permissoes.json"),
        "rotinas": read("rotinas.json"),
        "conectores": connectors,
    })
}

/// Puts an exported configuration back (each part that is in it); returns what came in.
pub fn import_bundle(v: &serde_json::Value) -> Result<Vec<&'static str>, String> {
    if v["kalangoos"] != "configuracao-dos-agentes" {
        return Err(kalango_i18n::tr("This file is not an agents' configuration exported by KalangoOS.").into());
    }
    let cfg = gtk::glib::user_config_dir().join("kalango");
    std::fs::create_dir_all(&cfg).map_err(|e| e.to_string())?;
    let mut done = Vec::new();
    if let Ok(p) = serde_json::from_value::<AgentPrefs>(v["ajustes"].clone()) {
        save(&p)?;
        done.push("ajustes");
    }
    for (key, file) in [("permissoes", "agente-permissoes.json"), ("rotinas", "rotinas.json")] {
        if v[key].is_object() || v[key].is_array() {
            std::fs::write(cfg.join(file), serde_json::to_string_pretty(&v[key]).unwrap_or_default()).map_err(|e| e.to_string())?;
            done.push(key);
        }
    }
    if let Some(m) = v["conectores"].as_object().filter(|m| !m.is_empty()) {
        let dir = crate::central_ajustes::pasta();
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        for (name, manifest) in m {
            // (a plain file name: nothing outside the connectors' folder)
            let name = std::path::Path::new(name).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if name.ends_with(".json") && !name.starts_with('.') {
                std::fs::write(dir.join(&name), serde_json::to_string_pretty(manifest).unwrap_or_default()).map_err(|e| e.to_string())?;
            }
        }
        done.push("conectores");
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::AgentPrefs;

    #[test]
    fn missing_keys_take_the_defaults() {
        let p: AgentPrefs = serde_json::from_str(r#"{"respostas": "curtas", "guardar_dias": 30}"#).unwrap();
        assert_eq!(p.answers, "curtas");
        assert_eq!(p.keep_days, 30);
        assert_eq!(p.steps_per_request, 24);
        assert!(p.respect_dnd && p.suggestions && !p.steps_open);
        let back = serde_json::to_value(&p).unwrap();
        assert_eq!(back["passos_por_pedido"], 24);
        assert_eq!((back["trocas_mantidas"].as_u64(), back["resumir_a_cada"].as_u64(), back["contexto_max_tokens"].as_u64()), (Some(6), Some(40), Some(12_288)));
    }

    #[test]
    fn the_conversations_memory_survives_a_save_of_another_setting() {
        // (the engine reads these keys from the same file: a save of the answers' length must not drop them)
        let p: AgentPrefs = serde_json::from_str(r#"{"respostas": "curtas", "trocas_mantidas": 9, "resumir_a_cada": 0, "contexto_max_tokens": 20000, "chave_futura": [1]}"#).unwrap();
        let mut q = p.clone();
        q.answers = "detalhadas".into();
        let back = serde_json::to_value(&q).unwrap();
        assert_eq!((back["trocas_mantidas"].as_u64(), back["resumir_a_cada"].as_u64(), back["contexto_max_tokens"].as_u64()), (Some(9), Some(0), Some(20_000)));
        assert_eq!(back["chave_futura"], serde_json::json!([1]));
    }
}
