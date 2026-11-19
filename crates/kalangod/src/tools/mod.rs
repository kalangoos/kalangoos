//! Ferramentas expostas pelo MCP do sistema. Cada uma declara nível, efeito e se é reversível (o contrato).

pub mod admin;
pub mod arquivos;
pub mod importar;
pub mod observar;
pub mod sistema;
pub mod navegador;
pub mod live_browser;
pub mod web;

use crate::peer::Peer;
use crate::policy::{CallerRule, Config, Effect, Level};
use crate::timeline::Timeline;
pub use kalango_i18n::{n_, tr, trf};
use serde_json::{json, Value};
use std::process::Command;

pub struct Ctx<'a> {
    pub cfg: &'a Config,
    pub peer: &'a Peer,
    pub rule: &'a CallerRule,
    pub timeline: &'a Timeline,
    /// an address of the person's network they approved for this call (web tools only)
    pub lan: Option<(String, u16)>,
}

/// Resultado de uma ferramenta.
pub struct Out {
    pub value: Value,
    pub summary: String,
    /// Chamada que desfaz esta ação: `{"ferramenta": ..., "argumentos": {...}}`.
    pub inverse: Option<Value>,
}

impl Out {
    pub fn read(value: Value, summary: impl Into<String>) -> Out {
        Out { value, summary: summary.into(), inverse: None }
    }
}

pub type Run = fn(&Ctx, &Value) -> Result<Out, String>;

pub struct Tool {
    pub name: &'static str,
    pub description: &'static str,
    pub level: Level,
    pub effect: Effect,
    pub reversible: bool,
    pub schema: fn() -> Value,
    pub run: Run,
}

pub fn all() -> Vec<Tool> {
    let mut v = observar::tools();
    v.extend(arquivos::tools());
    v.extend(sistema::tools());
    v.extend(admin::tools());
    v.extend(importar::tools());
    v.extend(web::tools());
    v
}

pub fn find(name: &str) -> Option<Tool> {
    all().into_iter().find(|t| t.name == name)
}

/// Descrição MCP (`tools/list`), com o contrato nas anotações.
pub fn describe(t: &Tool) -> Value {
    json!({
        "name": t.name,
        // description in the system language, with the contract (level, effect, reversibility)
        "description": format!("{} [{}]", tr(t.description), [
            trf("level {level}", &[("level", t.level.name().to_string())]),
            match t.effect { Effect::Leitura => tr("read only"), Effect::Escreve => tr("writes"), Effect::Destrutiva => tr("destructive") }.to_string(),
        ].into_iter().chain(if t.reversible { Some(tr("reversible").to_string()) } else if t.effect == Effect::Leitura { None } else { Some(tr("irreversible").to_string()) }).collect::<Vec<_>>().join("; ")),
        "inputSchema": (t.schema)(),
        "annotations": {
            "readOnlyHint": t.effect == Effect::Leitura,
            "destructiveHint": t.effect == Effect::Destrutiva,
            "idempotentHint": t.effect == Effect::Leitura,
            "openWorldHint": false
        },
        "_meta": { "os.kalango/nivel": t.level.name(), "os.kalango/reversivel": t.reversible }
    })
}

// ------------------------------------------------------------------ ajudantes comuns
pub fn obj(props: Value, required: &[&str]) -> Value {
    json!({ "type": "object", "properties": props, "required": required, "additionalProperties": false })
}

pub fn none() -> Value {
    obj(json!({}), &[])
}

pub fn s<'v>(args: &'v Value, k: &str) -> Result<&'v str, String> {
    args.get(k).and_then(Value::as_str).ok_or(kalango_i18n::trf("argument '{k}' is required", &[("k", k.to_string())]))
}

/// Executa um programa sem shell (argumentos nunca são interpretados).
pub fn exec(prog: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(prog).args(args).env("LC_ALL", "C").output().map_err(|e| format!("{prog}: {e}"))?;
    if !out.status.success() {
        return Err(format!("{prog} terminou com {}: {}", out.status, String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Nome de pacote, serviço ou chave válido (impede injeção de opções como "--algo").
pub fn safe_name(v: &str) -> Result<&str, String> {
    let ok = !v.is_empty() && !v.starts_with('-') && v.len() < 200 && v.chars().all(|c| c.is_ascii_alphanumeric() || "._+-@:/".contains(c));
    if ok { Ok(v) } else { Err(kalango_i18n::trf("invalid name: {v}", &[("v", format!("{v:?}"))])) }
}

/// Executa (ou, em simulação, só descreve) um comando que muda o sistema.
pub fn act(c: &Ctx, prog: &str, args: &[&str]) -> Result<String, String> {
    if c.cfg.dry_run {
        return Ok(format!("[simulado] {prog} {}", args.join(" ")));
    }
    exec(prog, args)
}

/// Lista de strings de um argumento.
pub fn strings(a: &Value, k: &str) -> Vec<String> {
    a.get(k).and_then(Value::as_array).map(|v| v.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default()
}
