//! Nível usuário: arquivos do chamador, nas pastas permitidas, com Landlock e com desfazer.

use super::*;
use crate::sandbox::{check_path, run_as_user};
use std::path::PathBuf;

pub fn tools() -> Vec<Tool> {
    vec![
        Tool { name: "arquivos.listar", description: n_("Lists a folder of the user"), level: Level::Usuario, effect: Effect::Leitura, reversible: false,
            schema: || obj(json!({"caminho": {"type": "string", "description": "absoluto ou começando com ~"}}), &["caminho"]), run: list },
        Tool { name: "arquivos.ler", description: n_("Reads a file of the user: text, or the text of a PDF with [p. N] page markers"), level: Level::Usuario, effect: Effect::Leitura, reversible: false,
            schema: || obj(json!({"caminho": {"type": "string"}, "max_bytes": {"type": "integer", "minimum": 1, "maximum": 4194304}}), &["caminho"]), run: read },
        Tool { name: "arquivos.escrever", description: n_("Creates or replaces a text file (the previous content is kept for undo)"), level: Level::Usuario, effect: Effect::Escreve, reversible: true,
            schema: || obj(json!({"caminho": {"type": "string"}, "conteudo": {"type": "string"}, "conteudo_base64": {"type": "string", "description": "binary content (audio, image) for a new file only"}}), &["caminho"]), run: write },
        Tool { name: "arquivos.mover", description: n_("Moves or renames a file or folder"), level: Level::Usuario, effect: Effect::Escreve, reversible: true,
            schema: || obj(json!({"de": {"type": "string"}, "para": {"type": "string"}}), &["de", "para"]), run: mv },
        Tool { name: "arquivos.lixeira", description: n_("Moves to the trash (nothing is really deleted)"), level: Level::Usuario, effect: Effect::Escreve, reversible: true,
            schema: || obj(json!({"caminho": {"type": "string"}}), &["caminho"]), run: trash },
        Tool { name: "arquivos.restaurar", description: n_("Restores a file from the copy kept by the timeline (used by undo)"), level: Level::Usuario, effect: Effect::Escreve, reversible: false,
            schema: || obj(json!({"caminho": {"type": "string"}, "copia": {"type": "string"}}), &["caminho", "copia"]), run: restore },
        Tool { name: "arquivos.apagar_criado", description: n_("Deletes a file created by an earlier action (used by undo)"), level: Level::Usuario, effect: Effect::Escreve, reversible: false,
            schema: || obj(json!({"caminho": {"type": "string"}}), &["caminho"]), run: delete_created },
    ]
}

fn path(c: &Ctx, raw: &str) -> Result<PathBuf, String> {
    check_path(raw, c.peer, &c.rule.allowed_paths, &c.cfg.protected_paths)
}

fn roots(c: &Ctx) -> Vec<PathBuf> {
    c.rule.allowed_paths.iter().map(|a| crate::policy::expand(a, &c.peer.home)).collect()
}

fn list(c: &Ctx, a: &Value) -> Result<Out, String> {
    let p = path(c, s(a, "caminho")?)?;
    let mut v = run_as_user(c.peer, &roots(c), json!({"tipo": "listar", "caminho": p}))?;
    // nunca mostra o que está dentro de pastas protegidas
    if let Some(items) = v.get_mut("itens").and_then(Value::as_array_mut) {
        items.retain(|i| check_path(&p.join(i["nome"].as_str().unwrap_or("")).display().to_string(), c.peer, &c.rule.allowed_paths, &c.cfg.protected_paths).is_ok());
    }
    let n = v["itens"].as_array().map(|x| x.len()).unwrap_or(0);
    Ok(Out::read(v, trf("{n} items in {path}", &[("n", n.to_string()), ("path", p.display().to_string())])))
}

fn read(c: &Ctx, a: &Value) -> Result<Out, String> {
    let p = path(c, s(a, "caminho")?)?;
    let v = run_as_user(c.peer, &roots(c), json!({"tipo": "ler", "caminho": p, "max_bytes": a.get("max_bytes")}))?;
    Ok(Out::read(v, trf("read {path}", &[("path", p.display().to_string())])))
}

fn backup_dir(c: &Ctx) -> PathBuf {
    c.cfg.state_dir.join("copias").join(c.peer.uid.to_string())
}

fn write(c: &Ctx, a: &Value) -> Result<Out, String> {
    let p = path(c, s(a, "caminho")?)?;
    // binary content (a voice recording, a picture) only creates new files: the undo of a replaced binary would need
    // a binary backup, and "undo" of a new file is deleting it
    let op = match a.get("conteudo_base64").and_then(Value::as_str) {
        Some(b) => {
            if p.exists() {
                return Err(kalango_i18n::trf("{path} already exists: binary content only creates new files", &[("path", p.display().to_string())]));
            }
            json!({"tipo": "escrever", "caminho": p, "conteudo_base64": b})
        }
        None => json!({"tipo": "escrever", "caminho": p, "conteudo": s(a, "conteudo")?}),
    };
    let v = run_as_user(c.peer, &roots(c), op)?;
    let inverse = if v["existia"].as_bool().unwrap_or(false) {
        // guarda a versão anterior fora do alcance do agente
        let dir = backup_dir(c);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let copy = dir.join(format!("{}-{}", crate::timeline::now(), std::process::id() as u64 ^ rand_tag()));
        std::fs::write(&copy, v["anterior"].as_str().unwrap_or("")).map_err(|e| e.to_string())?;
        json!({"ferramenta": "arquivos.restaurar", "argumentos": {"caminho": p, "copia": copy}})
    } else {
        json!({"ferramenta": "arquivos.apagar_criado", "argumentos": {"caminho": p}})
    };
    Ok(Out { value: json!({"caminho": p, "substituiu": v["existia"]}), summary: trf("wrote {path}", &[("path", p.display().to_string())]), inverse: Some(inverse) })
}

fn rand_tag() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos() as u64).unwrap_or(0)
}

fn mv(c: &Ctx, a: &Value) -> Result<Out, String> {
    let (from, to) = (path(c, s(a, "de")?)?, path(c, s(a, "para")?)?);
    // as `mv` does: an existing folder as the destination means into it, with the same name
    let to = match from.file_name() {
        Some(nome) if to.is_dir() && !to.starts_with(&from) => path(c, &to.join(nome).to_string_lossy())?,
        _ => to,
    };
    run_as_user(c.peer, &roots(c), json!({"tipo": "mover", "de": from, "para": to}))?;
    Ok(Out { value: json!({"de": from, "para": to}), summary: trf("moved {from} → {to}", &[("from", from.display().to_string()), ("to", to.display().to_string())]),
        inverse: Some(json!({"ferramenta": "arquivos.mover", "argumentos": {"de": to, "para": from}})) })
}

fn trash(c: &Ctx, a: &Value) -> Result<Out, String> {
    let p = path(c, s(a, "caminho")?)?;
    let lixeira = c.cfg.trash_dir.clone().unwrap_or_else(|| c.peer.home.join(".local/share/Trash"));
    // a person who never used the trash has no folder yet: Landlock only grants existing paths, so it is created
    // first, as the person, with access limited to its nearest existing parent and to nothing but that mkdir
    if !lixeira.join("files").is_dir() || !lixeira.join("info").is_dir() {
        let mut base = lixeira.clone();
        while !base.exists() {
            match base.parent() {
                Some(p) => base = p.to_path_buf(),
                None => break,
            }
        }
        run_as_user(c.peer, &[base], json!({"tipo": "preparar_lixeira", "lixeira": lixeira}))?;
    }
    let mut r = roots(c);
    r.push(lixeira.clone());
    let v = run_as_user(c.peer, &r, json!({"tipo": "lixeira", "caminho": p, "lixeira": lixeira, "data": iso_now()}))?;
    let at = v["na_lixeira"].as_str().unwrap_or("").to_string();
    Ok(Out { value: json!({"caminho": p, "na_lixeira": at}), summary: trf("{path} went to the trash", &[("path", p.display().to_string())]),
        inverse: Some(json!({"ferramenta": "arquivos.mover", "argumentos": {"de": at, "para": p}})) })
}

fn iso_now() -> String {
    exec("date", &["+%Y-%m-%dT%H:%M:%S"]).map(|s| s.trim().to_string()).unwrap_or_default()
}

fn restore(c: &Ctx, a: &Value) -> Result<Out, String> {
    let p = path(c, s(a, "caminho")?)?;
    let copy = PathBuf::from(s(a, "copia")?);
    if !crate::policy::is_within(&copy, &backup_dir(c)) {
        return Err(kalango_i18n::tr("copy outside this user's copies folder").into());
    }
    let content = std::fs::read_to_string(&copy).map_err(|e| e.to_string())?;
    run_as_user(c.peer, &roots(c), json!({"tipo": "escrever", "caminho": p, "conteudo": content}))?;
    Ok(Out::read(json!({"caminho": p}), trf("restored {path}", &[("path", p.display().to_string())])))
}

fn delete_created(c: &Ctx, a: &Value) -> Result<Out, String> {
    let p = path(c, s(a, "caminho")?)?;
    run_as_user(c.peer, &roots(c), json!({"tipo": "apagar_arquivo", "caminho": p}))?;
    Ok(Out::read(json!({"caminho": p}), trf("deleted {path}", &[("path", p.display().to_string())])))
}
