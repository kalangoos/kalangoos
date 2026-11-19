//! Níveis sistema e total: mudanças no sistema. Todas passam por aprovação (feita pelo servidor) e, no nível
//! sistema, por ponto de restauração. Com `dry_run`, nada é executado: a resposta descreve o que seria feito.

use super::*;

pub fn tools() -> Vec<Tool> {
    vec![
        Tool { name: "pacotes.instalar", description: n_("Installs packages from the repository (apt)"), level: Level::Sistema, effect: Effect::Escreve, reversible: true,
            schema: || obj(json!({"nomes": {"type": "array", "items": {"type": "string"}, "minItems": 1, "maxItems": 50}}), &["nomes"]), run: pkg_install },
        Tool { name: "pacotes.remover", description: n_("Removes packages (apt)"), level: Level::Sistema, effect: Effect::Destrutiva, reversible: true,
            schema: || obj(json!({"nomes": {"type": "array", "items": {"type": "string"}, "minItems": 1, "maxItems": 50}}), &["nomes"]), run: pkg_remove },
        Tool { name: "servicos.acao", description: n_("Starts, stops, restarts, enables or disables a service"), level: Level::Sistema, effect: Effect::Escreve, reversible: true,
            schema: || obj(json!({"nome": {"type": "string"}, "acao": {"type": "string", "enum": ["iniciar", "parar", "reiniciar", "habilitar", "desabilitar"]}}), &["nome", "acao"]), run: svc_action },
        Tool { name: "sysctl.definir", description: n_("Changes a kernel parameter at runtime"), level: Level::Sistema, effect: Effect::Escreve, reversible: true,
            schema: || obj(json!({"chave": {"type": "string"}, "valor": {"type": "string"}}), &["chave", "valor"]), run: sysctl_set },
        Tool { name: "processos.sinal", description: n_("Sends a signal to a process (terminate, kill, pause, continue)"), level: Level::Sistema, effect: Effect::Destrutiva, reversible: false,
            schema: || obj(json!({"pid": {"type": "integer", "minimum": 2}, "sinal": {"type": "string", "enum": ["encerrar", "matar", "pausar", "continuar"]}}), &["pid", "sinal"]), run: signal },
        Tool { name: "linha_do_tempo.desfazer", description: n_("Undoes a recorded action by running its inverse"), level: Level::Sistema, effect: Effect::Escreve, reversible: false,
            schema: || obj(json!({"id": {"type": "integer", "minimum": 1}}), &["id"]), run: undo },
        Tool { name: "linha_do_tempo.desfazer_desde", description: n_("Undoes every reversible action of yours recorded since a moment, newest first (one approval for all)"), level: Level::Sistema, effect: Effect::Escreve, reversible: false,
            schema: || obj(json!({"desde": {"type": "integer", "minimum": 0, "description": "unix time"}}), &["desde"]), run: undo },
        Tool { name: "sistema.executar", description: n_("Runs any program as administrator (root), without a shell"), level: Level::Total, effect: Effect::Destrutiva, reversible: false,
            schema: || obj(json!({"programa": {"type": "string"}, "argumentos": {"type": "array", "items": {"type": "string"}}}), &["programa"]), run: exec_any },
    ]
}

fn names(a: &Value) -> Result<Vec<String>, String> {
    a.get("nomes").and_then(Value::as_array).ok_or(kalango_i18n::tr("argument 'nomes' is required"))?
        .iter().map(|v| v.as_str().ok_or(kalango_i18n::tr("invalid name").to_string()).and_then(|s| safe_name(s).map(str::to_string))).collect()
}

fn run_or_simulate(c: &Ctx, prog: &str, args: &[&str]) -> Result<String, String> {
    if c.cfg.dry_run {
        return Ok(format!("[simulado] {prog} {}", args.join(" ")));
    }
    exec(prog, args)
}

fn apt(c: &Ctx, verb: &str, pkgs: &[String]) -> Result<String, String> {
    let mut args = vec![verb, "-y", "-q", "--no-install-recommends"];
    args.extend(pkgs.iter().map(String::as_str));
    if c.cfg.dry_run {
        return Ok(format!("[simulado] apt-get {}", args.join(" ")));
    }
    let out = std::process::Command::new("apt-get").args(&args).env("DEBIAN_FRONTEND", "noninteractive").env("LC_ALL", "C").output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!("apt-get {verb}: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).lines().filter(|l| l.starts_with("Setting up") || l.starts_with("Removing")).collect::<Vec<_>>().join("\n"))
}

fn pkg_install(c: &Ctx, a: &Value) -> Result<Out, String> {
    let p = names(a)?;
    let r = apt(c, "install", &p)?;
    Ok(Out { value: json!({"pacotes": p, "saida": r}), summary: trf("installed {packages}", &[("packages", p.join(", "))]),
        inverse: Some(json!({"ferramenta": "pacotes.remover", "argumentos": {"nomes": p}})) })
}

fn pkg_remove(c: &Ctx, a: &Value) -> Result<Out, String> {
    let p = names(a)?;
    let r = apt(c, "remove", &p)?;
    Ok(Out { value: json!({"pacotes": p, "saida": r}), summary: trf("removed {packages}", &[("packages", p.join(", "))]),
        inverse: Some(json!({"ferramenta": "pacotes.instalar", "argumentos": {"nomes": p}})) })
}

fn svc_action(c: &Ctx, a: &Value) -> Result<Out, String> {
    let name = safe_name(s(a, "nome")?)?;
    let (verb, inv) = match s(a, "acao")? {
        "iniciar" => ("start", Some("parar")),
        "parar" => ("stop", Some("iniciar")),
        "reiniciar" => ("restart", None),
        "habilitar" => ("enable", Some("desabilitar")),
        "desabilitar" => ("disable", Some("habilitar")),
        other => return Err(kalango_i18n::trf("unknown action: {other}", &[("other", other.to_string())])),
    };
    let r = run_or_simulate(c, "systemctl", &[verb, name])?;
    Ok(Out { value: json!({"servico": name, "acao": verb, "saida": r}), summary: format!("{verb} {name}"),
        inverse: inv.map(|i| json!({"ferramenta": "servicos.acao", "argumentos": {"nome": name, "acao": i}})) })
}

fn sysctl_set(c: &Ctx, a: &Value) -> Result<Out, String> {
    let key = s(a, "chave")?;
    let val = s(a, "valor")?;
    if val.contains('\n') || val.len() > 256 {
        return Err(kalango_i18n::tr("invalid value").into());
    }
    let path = super::observar::sysctl_path(key)?;
    let before = std::fs::read_to_string(&path).map_err(|e| format!("{key}: {e}"))?.trim().to_string();
    if !c.cfg.dry_run {
        std::fs::write(&path, val).map_err(|e| format!("{key}: {e}"))?;
    }
    Ok(Out { value: json!({"chave": key, "antes": before, "depois": val, "simulado": c.cfg.dry_run}), summary: format!("{key}: {before} → {val}"),
        inverse: Some(json!({"ferramenta": "sysctl.definir", "argumentos": {"chave": key, "valor": before}})) })
}

fn signal(c: &Ctx, a: &Value) -> Result<Out, String> {
    use nix::sys::signal::{kill, Signal};
    let pid = a.get("pid").and_then(Value::as_i64).ok_or(kalango_i18n::tr("argument 'pid' is required"))? as i32;
    if pid <= 1 || pid == std::process::id() as i32 {
        return Err(kalango_i18n::tr("that process cannot receive a signal from kalangod").into());
    }
    let sig = match s(a, "sinal")? {
        "encerrar" => Signal::SIGTERM,
        "matar" => Signal::SIGKILL,
        "pausar" => Signal::SIGSTOP,
        "continuar" => Signal::SIGCONT,
        o => return Err(kalango_i18n::trf("unknown signal: {o}", &[("o", o.to_string())])),
    };
    if !c.cfg.dry_run {
        kill(nix::unistd::Pid::from_raw(pid), sig).map_err(|e| e.to_string())?;
    }
    let inverse = match sig {
        Signal::SIGSTOP => Some(json!({"ferramenta": "processos.sinal", "argumentos": {"pid": pid, "sinal": "continuar"}})),
        _ => None,
    };
    Ok(Out { value: json!({"pid": pid, "sinal": format!("{sig:?}"), "simulado": c.cfg.dry_run}), summary: format!("{sig:?} → {pid}"), inverse })
}

/// O desfazer é tratado pelo servidor (precisa reentrar no fluxo de chamada); aqui só valida.
fn undo(_: &Ctx, _: &Value) -> Result<Out, String> {
    Err("interno: desfazer é tratado pelo servidor".into())
}

fn exec_any(c: &Ctx, a: &Value) -> Result<Out, String> {
    let prog = s(a, "programa")?;
    let args = strings(a, "argumentos");
    // nível total: root e sem Landlock, mas ainda numa tarefa com limites e auditada pela guarda
    let v = super::admin::run_as_root(c, prog, &args)?;
    let summary = if v["simulado"] == true { format!("[simulado] {prog}") } else { trf("ran {program} (code {code})", &[("program", prog.to_string()), ("code", v["codigo"].to_string())]) };
    Ok(Out { value: v, summary, inverse: None })
}
