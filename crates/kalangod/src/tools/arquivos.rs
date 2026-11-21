//! Nível usuário: arquivos do chamador, nas pastas permitidas, com Landlock e com desfazer.

use super::*;
use crate::sandbox::{check_change_path, check_path, run_as_user, Grant};
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

/// A path the tool changes: also not a folder that holds a protected one (see `check_change_path`).
fn change_path(c: &Ctx, raw: &str) -> Result<PathBuf, String> {
    check_change_path(raw, c.peer, &c.rule.allowed_paths, &c.cfg.protected_paths)
}

/// Runs a file operation in the confined helper with kalangod's cut of the allowed folders (`sandbox::file_rules`):
/// `targets` are the paths it changes, each with what it does there.
fn run(c: &Ctx, targets: &[(PathBuf, Grant)], op: Value) -> Result<Value, String> {
    run_extra(c, targets, &[], op)
}

fn run_extra(c: &Ctx, targets: &[(PathBuf, Grant)], extra: &[(PathBuf, Grant)], op: Value) -> Result<Value, String> {
    let rules = crate::sandbox::file_rules(c.peer, c.cfg, &c.rule.allowed_paths, targets, &c.startup_ok, extra);
    crate::sandbox::run_with_rules(c.peer, rules, op)
}

/// The real paths this call is about to change, checked again now, just before the helper: a folder on the way may
/// have become a link to a startup folder since the gate looked (a race, round 2 of the review). A startup path it was
/// not approved for, or a protected one, stops it. The helper then works from folder descriptors opened without
/// following links, so the paths cannot change again under it.
fn recheck(c: &Ctx, paths: &[&std::path::Path]) -> Result<(), String> {
    for p in paths {
        let raw = p.display().to_string();
        check_change_path(&raw, c.peer, &c.rule.allowed_paths, &c.cfg.protected_paths)?;
        let ask = crate::startup::mode(c.cfg, c.peer) == crate::startup::Mode::Ask;
        if ask && crate::startup::startup_base(&raw, c.peer, c.cfg).is_some() {
            let r = real(p);
            let approved = c.startup_ok.iter().map(|a| real(a)).any(|a| crate::policy::is_within(&r, &a) || crate::policy::is_within(&a, &r));
            if !approved {
                return Err(trf("{path} became a file that runs when you log in since it was checked: nothing was done", &[("path", raw)]));
            }
        }
    }
    Ok(())
}

/// The path with every link on the way followed (what is written is the file the check saw, and the helper opens
/// that without following links again).
fn real(p: &std::path::Path) -> PathBuf {
    crate::sandbox::resolve_existing(p)
}

/// The path with the links of its folders followed, the last part kept (moving or deleting a link moves or deletes
/// the link).
fn real_parent(p: &std::path::Path) -> PathBuf {
    match (p.parent(), p.file_name()) {
        (Some(d), Some(n)) => real(d).join(n),
        _ => p.to_path_buf(),
    }
}

fn list(c: &Ctx, a: &Value) -> Result<Out, String> {
    let p = path(c, s(a, "caminho")?)?;
    let mut v = run(c, &[], json!({"tipo": "listar", "caminho": p}))?;
    // nunca mostra o que está dentro de pastas protegidas
    if let Some(items) = v.get_mut("itens").and_then(Value::as_array_mut) {
        items.retain(|i| check_path(&p.join(i["nome"].as_str().unwrap_or("")).display().to_string(), c.peer, &c.rule.allowed_paths, &c.cfg.protected_paths).is_ok());
    }
    let n = v["itens"].as_array().map(|x| x.len()).unwrap_or(0);
    Ok(Out::read(v, trf("{n} items in {path}", &[("n", n.to_string()), ("path", p.display().to_string())])))
}

fn read(c: &Ctx, a: &Value) -> Result<Out, String> {
    let p = path(c, s(a, "caminho")?)?;
    let v = run(c, &[], json!({"tipo": "ler", "caminho": p, "max_bytes": a.get("max_bytes")}))?;
    Ok(Out::read(v, trf("read {path}", &[("path", p.display().to_string())])))
}

fn backup_dir(c: &Ctx) -> PathBuf {
    c.cfg.state_dir.join("copias").join(c.peer.uid.to_string())
}

fn write(c: &Ctx, a: &Value) -> Result<Out, String> {
    let p = change_path(c, s(a, "caminho")?)?;
    // binary content (a voice recording, a picture) only creates new files: the undo of a replaced binary would need
    // a binary backup, and "undo" of a new file is deleting it
    let op = match a.get("conteudo_base64").and_then(Value::as_str) {
        Some(b) => {
            if p.exists() {
                return Err(kalango_i18n::trf("{path} already exists: binary content only creates new files", &[("path", p.display().to_string())]));
            }
            json!({"tipo": "escrever", "caminho": real(&p), "conteudo_base64": b, "novo": true})
        }
        None => json!({"tipo": "escrever", "caminho": real(&p), "conteudo": s(a, "conteudo")?, "novo": !real(&p).exists()}),
    };
    let rp = real(&p);
    recheck(c, &[&rp])?;
    let grant = if rp.exists() { (rp.clone(), Grant::File) } else { (rp.parent().unwrap_or(&rp).to_path_buf(), Grant::Create) };
    let v = run(c, &[grant], op)?;
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
    let (from, to) = (change_path(c, s(a, "de")?)?, change_path(c, s(a, "para")?)?);
    // as `mv` does: an existing folder as the destination means into it, with the same name
    let to = match from.file_name() {
        Some(nome) if to.is_dir() && !to.starts_with(&from) => change_path(c, &to.join(nome).to_string_lossy())?,
        _ => to,
    };
    let (rf, rt) = (real_parent(&from), real_parent(&to));
    recheck(c, &[&rf, &rt])?;
    let dir = |p: &PathBuf| p.parent().unwrap_or(p).to_path_buf();
    run(c, &[(dir(&rf), Grant::Remove), (dir(&rt), Grant::Create)], json!({"tipo": "mover", "de": rf, "para": rt}))?;
    Ok(Out { value: json!({"de": from, "para": to}), summary: trf("moved {from} → {to}", &[("from", from.display().to_string()), ("to", to.display().to_string())]),
        inverse: Some(json!({"ferramenta": "arquivos.mover", "argumentos": {"de": to, "para": from}})) })
}

fn trash(c: &Ctx, a: &Value) -> Result<Out, String> {
    let p = change_path(c, s(a, "caminho")?)?;
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
    let rp = real_parent(&p);
    recheck(c, &[&rp])?;
    let v = run_extra(c, &[(rp.parent().unwrap_or(&rp).to_path_buf(), Grant::Remove)], &[(lixeira.join("files"), Grant::Create), (lixeira.join("info"), Grant::Create)],
        json!({"tipo": "lixeira", "caminho": rp, "lixeira": real(&lixeira), "data": iso_now()}))?;
    let at = v["na_lixeira"].as_str().unwrap_or("").to_string();
    Ok(Out { value: json!({"caminho": p, "na_lixeira": at}), summary: trf("{path} went to the trash", &[("path", p.display().to_string())]),
        inverse: Some(json!({"ferramenta": "arquivos.mover", "argumentos": {"de": at, "para": p}})) })
}

/// Puts a file another program wrote in a staging folder no agent reaches (`staged`) at `dest`, the way the file tools
/// move: the destination checked again now, folder descriptors opened without following links, never over a file
/// that exists. For programs that write a path themselves (VirtualDesk copying out of a machine): they would follow
/// a link an agent put on the destination's path after the check.
pub fn put_in_place(c: &Ctx, staged: &std::path::Path, dest: &std::path::Path) -> Result<PathBuf, String> {
    let rt = real_parent(dest);
    recheck(c, &[&rt])?;
    let dir = |p: &std::path::Path| p.parent().unwrap_or(p).to_path_buf();
    run_extra(c, &[(dir(&rt), Grant::Create)], &[(dir(staged), Grant::Remove)], json!({"tipo": "mover", "de": staged, "para": rt}))?;
    Ok(rt)
}

fn iso_now() -> String {
    exec("date", &["+%Y-%m-%dT%H:%M:%S"]).map(|s| s.trim().to_string()).unwrap_or_default()
}

fn restore(c: &Ctx, a: &Value) -> Result<Out, String> {
    let p = change_path(c, s(a, "caminho")?)?;
    let copy = PathBuf::from(s(a, "copia")?);
    if !crate::policy::is_within(&copy, &backup_dir(c)) {
        return Err(kalango_i18n::tr("copy outside this user's copies folder").into());
    }
    let content = std::fs::read_to_string(&copy).map_err(|e| e.to_string())?;
    let rp = real(&p);
    recheck(c, &[&rp])?;
    let grant = if rp.exists() { (rp.clone(), Grant::File) } else { (rp.parent().unwrap_or(&rp).to_path_buf(), Grant::Create) };
    run(c, &[grant], json!({"tipo": "escrever", "caminho": rp, "conteudo": content, "novo": !p.exists()}))?;
    Ok(Out::read(json!({"caminho": p}), trf("restored {path}", &[("path", p.display().to_string())])))
}

fn delete_created(c: &Ctx, a: &Value) -> Result<Out, String> {
    let p = change_path(c, s(a, "caminho")?)?;
    let rp = real_parent(&p);
    recheck(c, &[&rp])?;
    run(c, &[(rp.parent().unwrap_or(&rp).to_path_buf(), Grant::Remove)], json!({"tipo": "apagar_arquivo", "caminho": rp}))?;
    Ok(Out::read(json!({"caminho": p}), trf("deleted {path}", &[("path", p.display().to_string())])))
}
