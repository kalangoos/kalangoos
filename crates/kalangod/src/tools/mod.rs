//! Ferramentas expostas pelo MCP do sistema. Cada uma declara nível, efeito e se é reversível (o contrato).

pub mod admin;
pub mod arquivos;
pub mod importar;
pub mod observar;
pub mod sistema;
pub mod navegador;
pub mod live_browser;
pub mod gpu;
pub mod plataforma;
pub mod plataforma_contrato;
pub mod maquinas;
pub mod web;
pub mod tela;

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
    /// the server's state (the screen tools' sessions, approvals and grants); None when undoing
    pub state: Option<&'a crate::server::State>,
    /// the startup files (`startup.rs`) this very call was approved for: only these are writable by the file helper
    pub startup_ok: Vec<std::path::PathBuf>,
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
    v.extend(maquinas::tools());
    v.extend(gpu::tools());
    v.extend(tela::tools());
    v.extend(plataforma::tools());
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

/// Whether `args` fit a tool's JSON schema — the subset the tools use: `type` (one or a list), `properties`,
/// `required`, `additionalProperties` (false or a schema), `enum`, `minimum`/`maximum`, `items`. Checked by the server
/// before anything else, so a tool never sees an argument it did not declare (a second name for the same thing, a
/// key its approval rules do not look at). The error names the first thing wrong.
pub fn check_args(schema: &Value, args: &Value) -> Result<(), String> {
    check_at(schema, args, "")
}

fn check_at(schema: &Value, v: &Value, at: &str) -> Result<(), String> {
    let place = || if at.is_empty() { kalango_i18n::tr("the arguments").to_string() } else { format!("'{at}'") };
    if let Some(ty) = schema.get("type") {
        let types: Vec<&str> = match ty {
            Value::String(s) => vec![s.as_str()],
            Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
            _ => vec![],
        };
        let fits = |t: &str| match t {
            "object" => v.is_object(),
            "array" => v.is_array(),
            "string" => v.is_string(),
            "boolean" => v.is_boolean(),
            "integer" => v.is_i64() || v.is_u64(),
            "number" => v.is_number(),
            "null" => v.is_null(),
            _ => true,
        };
        if !types.is_empty() && !types.iter().any(|t| fits(t)) {
            return Err(kalango_i18n::trf("{p} must be {t}", &[("p", place()), ("t", types.join(" or "))]));
        }
    }
    if let Some(options) = schema.get("enum").and_then(Value::as_array) {
        if !options.contains(v) {
            return Err(kalango_i18n::trf("{p} must be one of {o}", &[("p", place()), ("o", Value::Array(options.clone()).to_string())]));
        }
    }
    if let Some(n) = v.as_f64() {
        if schema.get("minimum").and_then(Value::as_f64).is_some_and(|m| n < m) || schema.get("maximum").and_then(Value::as_f64).is_some_and(|m| n > m) {
            return Err(kalango_i18n::trf("{p} is out of range", &[("p", place())]));
        }
    }
    if let (Some(items), Some(list)) = (schema.get("items"), v.as_array()) {
        for (i, x) in list.iter().enumerate() {
            check_at(items, x, &format!("{at}[{i}]"))?;
        }
    }
    if let Some(o) = v.as_object() {
        let props = schema.get("properties").and_then(Value::as_object);
        for k in schema.get("required").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
            if !o.contains_key(k) {
                return Err(kalango_i18n::trf("argument '{k}' is required", &[("k", k.to_string())]));
            }
        }
        for (k, x) in o {
            let name = if at.is_empty() { k.clone() } else { format!("{at}.{k}") };
            match props.and_then(|p| p.get(k)) {
                Some(s) => check_at(s, x, &name)?,
                None => match schema.get("additionalProperties") {
                    Some(Value::Bool(false)) => return Err(kalango_i18n::trf("unknown argument '{k}'", &[("k", name)])),
                    Some(s) if s.is_object() => check_at(s, x, &name)?,
                    _ => {}
                },
            }
        }
    }
    Ok(())
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

/// Refusals known before the person is asked (None of them needs an answer): a GPU that may never be given to a
/// virtual machine (the one that shows the screen, one without IOMMU, one whose group holds other devices).
pub fn precheck(state_dir: &std::path::Path, peer: &Peer, name: &str, args: &Value) -> Result<(), String> {
    match name {
        "gpu.vfio_reservar" => gpu::precheck(args),
        n if n.starts_with("vm.") => maquinas::precheck(state_dir, peer, n, args),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod check_args_tests {
    use super::*;

    #[test]
    fn arguments_follow_the_schema() {
        let s = obj(json!({"id": {"type": "string"}, "n": {"type": "integer", "minimum": 1, "maximum": 9}, "modo": {"type": "string", "enum": ["a", "b"]},
            "lista": {"type": "array", "items": {"type": "string"}}, "sub": obj(json!({"x": {"type": "boolean"}}), &["x"])}), &["id"]);
        assert!(check_args(&s, &json!({"id": "x"})).is_ok());
        assert!(check_args(&s, &json!({"id": "x", "n": 3, "modo": "b", "lista": ["a"], "sub": {"x": true}})).is_ok());
        assert!(check_args(&s, &json!({})).unwrap_err().contains("id"), "required");
        assert!(check_args(&s, &json!({"maquina": "x"})).is_err(), "another name for the machine is not the declared one");
        assert!(check_args(&s, &json!({"id": "x", "microfone": true})).unwrap_err().contains("microfone"), "undeclared");
        assert!(check_args(&s, &json!({"id": 3})).is_err(), "type");
        assert!(check_args(&s, &json!({"id": "x", "n": 10})).is_err(), "maximum");
        assert!(check_args(&s, &json!({"id": "x", "n": 1.5})).is_err(), "an integer");
        assert!(check_args(&s, &json!({"id": "x", "modo": "c"})).is_err(), "enum");
        assert!(check_args(&s, &json!({"id": "x", "lista": [1]})).is_err(), "items");
        assert!(check_args(&s, &json!({"id": "x", "sub": {}})).is_err(), "nested required");
        assert!(check_args(&s, &json!("x")).is_err(), "not an object");
    }

    #[test]
    fn every_tool_s_schema_can_be_checked() {
        // every schema is an object schema the checker reads, and an empty call is judged by it (no panic)
        for t in all() {
            let s = (t.schema)();
            assert_eq!(s["type"], "object", "{}", t.name);
            let _ = check_args(&s, &json!({}));
        }
    }
}
