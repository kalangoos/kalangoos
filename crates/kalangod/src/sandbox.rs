//! Operações de arquivo do nível usuário: rodam num processo filho com o UID do chamador e preso pelo Landlock
//! às pastas permitidas. Mesmo que o kalangod rode como root, o filho não enxerga nada fora dessas pastas.

use crate::peer::Peer;
use crate::policy::{expand, is_within, normalize};
use landlock::{Access, AccessFs, PathBeneath, PathFd, Ruleset, RulesetAttr, RulesetCreatedAttr, RulesetStatus, ABI};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Resolve um caminho pedido pelo agente e confere as regras antes de qualquer acesso.
pub fn check_path(raw: &str, peer: &Peer, allowed: &[String], protected: &[String]) -> Result<PathBuf, String> {
    let p = expand(raw, &peer.home);
    if !p.is_absolute() {
        return Err(format!("caminho precisa ser absoluto ou começar com ~: {raw}"));
    }
    let p = normalize(&p);
    // segue links simbólicos do que já existe, para que um link não leve a uma pasta protegida
    let resolved = resolve_existing(&p);
    for prot in protected {
        let base = normalize(&expand(prot, &peer.home));
        if is_within(&p, &base) || is_within(&resolved, &base) {
            return Err(format!("pasta protegida: {} (nenhum agente acessa)", base.display()));
        }
    }
    let ok = allowed.iter().map(|a| normalize(&expand(a, &peer.home))).any(|base| is_within(&resolved, &base) && is_within(&p, &base));
    if !ok {
        return Err(kalango_i18n::trf("outside the folders allowed for this agent: {path}", &[("path", p.display().to_string())]));
    }
    Ok(p)
}

fn resolve_existing(p: &Path) -> PathBuf {
    let mut cur = p.to_path_buf();
    let mut tail = Vec::new();
    loop {
        if let Ok(c) = std::fs::canonicalize(&cur) {
            let mut out = c;
            for t in tail.iter().rev() {
                out.push(t);
            }
            return out;
        }
        match (cur.parent(), cur.file_name()) {
            (Some(parent), Some(name)) => {
                tail.push(name.to_os_string());
                cur = parent.to_path_buf();
            }
            _ => return p.to_path_buf(),
        }
    }
}

/// Executa `op` num filho com o UID do chamador e Landlock nas pastas `roots`.
/// This program, to start its helpers. /proc/self/exe still runs it after its file was replaced (an update, a new
/// build): current_exe then names a path ending in " (deleted)" and no helper starts — every file tool failed with
/// "No such file or directory" until kalangod was restarted.
pub fn self_exe() -> Result<PathBuf, String> {
    let p = PathBuf::from("/proc/self/exe");
    if p.exists() {
        return Ok(p);
    }
    std::env::current_exe().map_err(|e| e.to_string())
}

/// This program's file, for another process to start it (systemd-run): when the file was replaced, the one now at
/// that place (current_exe adds " (deleted)" to the old one's path).
pub fn exe_on_disk() -> Result<PathBuf, String> {
    let p = std::env::current_exe().map_err(|e| e.to_string())?;
    let s = p.display().to_string();
    match s.strip_suffix(" (deleted)").map(PathBuf::from).filter(|n| n.is_file()) {
        Some(n) => Ok(n),
        None => Ok(p),
    }
}

pub fn run_as_user(peer: &Peer, roots: &[PathBuf], op: Value) -> Result<Value, String> {
    let exe = self_exe()?;
    let mut cmd = Command::new(exe);
    cmd.arg("__auxiliar").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    if nix::unistd::geteuid().is_root() && peer.uid != 0 {
        // a biblioteca padrão já remove os grupos suplementares do root antes de trocar de usuário
        cmd.uid(peer.uid).gid(peer.gid);
    }
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    let input = json!({ "roots": roots, "op": op });
    child.stdin.take().unwrap().write_all(input.to_string().as_bytes()).map_err(|e| e.to_string())?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_slice(&out.stdout).map_err(|_| format!("auxiliar falhou: {}", String::from_utf8_lossy(&out.stderr).trim()))?;
    match v.get("erro").and_then(Value::as_str) {
        Some(e) => Err(e.to_string()),
        None => Ok(v),
    }
}

/// Ponto de entrada do processo auxiliar (`kalangod __auxiliar`).
pub fn helper_main() -> ! {
    let mut s = String::new();
    let _ = std::io::stdin().read_to_string(&mut s);
    let res = (|| -> Result<Value, String> {
        let input: Value = serde_json::from_str(&s).map_err(|e| e.to_string())?;
        let roots: Vec<PathBuf> = serde_json::from_value(input["roots"].clone()).map_err(|e| e.to_string())?;
        restrict(&roots)?;
        // depois do Landlock (que liga no_new_privs): sem exec, rede, montagem, ptrace, bpf, módulos…
        crate::limits::seccomp_helper()?;
        do_op(&input["op"])
    })();
    let out = match res {
        Ok(v) => v,
        Err(e) => json!({ "erro": e }),
    };
    println!("{out}");
    std::process::exit(0)
}

fn restrict(roots: &[PathBuf]) -> Result<(), String> {
    let abi = ABI::V3;
    let mut rs = Ruleset::default().handle_access(AccessFs::from_all(abi)).map_err(|e| e.to_string())?.create().map_err(|e| e.to_string())?;
    for r in roots {
        if let Ok(fd) = PathFd::new(r) {
            rs = rs.add_rule(PathBeneath::new(fd, AccessFs::from_all(abi))).map_err(|e| e.to_string())?;
        }
    }
    let st = rs.restrict_self().map_err(|e| e.to_string())?;
    if st.ruleset == RulesetStatus::NotEnforced {
        return Err(kalango_i18n::tr("Landlock unavailable on this kernel: operation refused").into());
    }
    Ok(())
}

/// What a file is when it is not text: by its first bytes (the usual pictures, sounds, videos, archives and
/// programs), else "binário" when it has NUL bytes or is not UTF-8 where text would be. None: text.
pub fn binary_kind(d: &[u8]) -> Option<&'static str> {
    let magia: [(&[u8], &str); 16] = [
        (b"\x89PNG", "imagem PNG"), (b"\xff\xd8\xff", "imagem JPEG"), (b"GIF8", "imagem GIF"), (b"RIFF", "RIFF (áudio WAV ou imagem WebP)"),
        (b"ID3", "áudio MP3"), (b"fLaC", "áudio FLAC"), (b"OggS", "áudio/vídeo Ogg"), (b"\x1aE\xdf\xa3", "vídeo Matroska/WebM"),
        (b"PK\x03\x04", "arquivo ZIP (ou documento do Office)"), (b"\x1f\x8b", "arquivo gzip"), (b"\xfd7zXZ", "arquivo xz"), (b"7z\xbc\xaf", "arquivo 7z"),
        (b"\x7fELF", "programa (ELF)"), (b"!<arch>", "pacote Debian"), (b"\x28\xb5\x2f\xfd", "arquivo zstd"), (b"SQLite format 3", "banco SQLite"),
    ];
    if let Some((_, k)) = magia.iter().find(|(m, _)| d.starts_with(m)) {
        return Some(k);
    }
    if d.len() >= 12 && &d[4..8] == b"ftyp" {
        return Some("vídeo/áudio MP4");
    }
    let amostra = &d[..d.len().min(8192)];
    let texto = match std::str::from_utf8(amostra) {
        Ok(_) => true,
        // (a multibyte character cut at the sample's end is still text)
        Err(e) => e.error_len().is_none() && e.valid_up_to() + 4 > amostra.len(),
    };
    (amostra.contains(&0) || !texto).then_some("binário")
}

/// The text of a PDF, page by page, with a marker before each page ("[p. 2]") so answers can cite it. Parsed here,
/// inside the confined helper (the caller's UID, Landlock, seccomp): a hostile PDF reaches nothing else. A parser
/// panic becomes an error.
fn read_pdf(data: &[u8], max: usize) -> Result<Value, String> {
    const MAX_PDF: usize = 64 << 20;
    if data.len() > MAX_PDF {
        return Err(kalango_i18n::trf("PDF too large to read ({mb} MB; the limit is 64 MB)", &[("mb", (data.len() >> 20).to_string())]));
    }
    let pages = std::panic::catch_unwind(|| pdf_extract::extract_text_from_mem_by_pages(data))
        .map_err(|_| kalango_i18n::tr("could not read this PDF").to_string())?
        .map_err(|e| format!("{}: {e}", kalango_i18n::tr("could not read this PDF")))?;
    let mut text = String::new();
    for (i, page) in pages.iter().enumerate() {
        let page = page.trim();
        if !page.is_empty() {
            text.push_str(&format!("[p. {}]\n{page}\n\n", i + 1));
        }
    }
    let cut = text.len() > max;
    if cut {
        let mut end = max;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    Ok(json!({ "conteudo": text, "bytes": data.len(), "cortado": cut, "pdf_paginas": pages.len() }))
}

fn do_op(op: &Value) -> Result<Value, String> {
    let kind = op["tipo"].as_str().unwrap_or("");
    let p = |k: &str| -> Result<PathBuf, String> { op[k].as_str().map(PathBuf::from).ok_or(format!("falta '{k}'")) };
    match kind {
        "listar" => {
            let dir = p("caminho")?;
            let mut items = Vec::new();
            for e in std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?.flatten() {
                let md = e.metadata().ok();
                items.push(json!({
                    "nome": e.file_name().to_string_lossy(),
                    "pasta": md.as_ref().map(|m| m.is_dir()).unwrap_or(false),
                    "bytes": md.as_ref().map(|m| m.len()).unwrap_or(0),
                }));
            }
            items.sort_by(|a, b| a["nome"].as_str().cmp(&b["nome"].as_str()));
            Ok(json!({ "itens": items }))
        }
        "ler" => {
            let f = p("caminho")?;
            let max = op["max_bytes"].as_u64().unwrap_or(256 * 1024) as usize;
            let data = std::fs::read(&f).map_err(|e| format!("{}: {e}", f.display()))?;
            if data.starts_with(b"%PDF-") {
                return read_pdf(&data, max);
            }
            // a picture, a sound, a program: what it is, not its bytes as text (a model fed 64 KB of a WAV's bytes
            // never answered)
            if let Some(tipo) = binary_kind(&data) {
                return Ok(json!({ "binario": true, "tipo": tipo, "bytes": data.len(), "conteudo": "",
                    "nota": kalango_i18n::trf("{kind} file ({size} bytes): its content is not text, so it is not shown here", &[("kind", tipo.to_string()), ("size", data.len().to_string())]) }));
            }
            let cut = data.len() > max;
            Ok(json!({ "conteudo": String::from_utf8_lossy(&data[..data.len().min(max)]), "bytes": data.len(), "cortado": cut }))
        }
        "escrever" => {
            let f = p("caminho")?;
            let before = std::fs::read(&f).ok();
            if let Some(parent) = f.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            std::fs::write(&f, op["conteudo"].as_str().unwrap_or("")).map_err(|e| format!("{}: {e}", f.display()))?;
            Ok(json!({ "existia": before.is_some(), "anterior": before.map(|b| String::from_utf8_lossy(&b).into_owned()) }))
        }
        "preparar_lixeira" => {
            let trash = p("lixeira")?;
            for sub in ["files", "info"] {
                std::fs::create_dir_all(trash.join(sub)).map_err(|e| format!("{}: {e}", trash.join(sub).display()))?;
            }
            Ok(json!({}))
        }
                "apagar_arquivo" => {
            let f = p("caminho")?;
            std::fs::remove_file(&f).map_err(|e| format!("{}: {e}", f.display()))?;
            Ok(json!({}))
        }
        "mover" => {
            let (a, b) = (p("de")?, p("para")?);
            if b.exists() {
                return Err(kalango_i18n::trf("already exists: {path}", &[("path", b.display().to_string())]));
            }
            if let Some(parent) = b.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            std::fs::rename(&a, &b).map_err(|e| format!("{} → {}: {e}", a.display(), b.display()))?;
            Ok(json!({}))
        }
        "lixeira" => {
            // padrão freedesktop: ~/.local/share/Trash/{files,info}
            let f = p("caminho")?;
            let trash = p("lixeira")?;
            let name = f.file_name().ok_or(kalango_i18n::tr("path without a name"))?.to_string_lossy().into_owned();
            std::fs::create_dir_all(trash.join("files")).map_err(|e| e.to_string())?;
            std::fs::create_dir_all(trash.join("info")).map_err(|e| e.to_string())?;
            let mut target = name.clone();
            let mut n = 1;
            while trash.join("files").join(&target).exists() {
                n += 1;
                target = format!("{name}.{n}");
            }
            std::fs::rename(&f, trash.join("files").join(&target)).map_err(|e| format!("{}: {e}", f.display()))?;
            let info = format!("[Trash Info]\nPath={}\nDeletionDate={}\n", f.display(), op["data"].as_str().unwrap_or(""));
            std::fs::write(trash.join("info").join(format!("{target}.trashinfo")), info).map_err(|e| e.to_string())?;
            Ok(json!({ "na_lixeira": trash.join("files").join(&target) }))
        }
        _ => Err(kalango_i18n::trf("unknown operation: {kind}", &[("kind", kind.to_string())])),
    }
}

#[cfg(test)]
mod binary_tests {
    use super::binary_kind;

    #[test]
    fn a_binary_file_is_said_not_shown() {
        assert_eq!(binary_kind(b"\x89PNG\r\n\x1a\n...."), Some("imagem PNG"));
        assert_eq!(binary_kind(b"RIFF$\x00\x00\x00WAVEfmt "), Some("RIFF (áudio WAV ou imagem WebP)"));
        assert_eq!(binary_kind(b"\x00\x01\x02 dados"), Some("binário"));
        assert_eq!(binary_kind("linha um\nação e reunião\n".as_bytes()), None);
        // (a UTF-8 character cut by the 8 KB sample is still text)
        let mut t = "a".repeat(8191).into_bytes();
        t.extend("ã".as_bytes());
        assert_eq!(binary_kind(&t), None);
    }
}
