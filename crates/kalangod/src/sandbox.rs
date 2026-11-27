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
    // (a protected folder is compared as written and where it really is: with the home behind a link, /home → /var/home,
    // a link to it resolves to the real place)
    for prot in protected {
        let base = normalize(&expand(prot, &peer.home));
        if is_within(&p, &base) || is_within(&resolved, &base) || is_within(&resolved, &resolve_existing(&base)) {
            return Err(format!("pasta protegida: {} (nenhum agente acessa)", base.display()));
        }
    }
    let ok = allowed.iter().map(|a| normalize(&expand(a, &peer.home)))
        .any(|base| (is_within(&resolved, &base) || is_within(&resolved, &resolve_existing(&base))) && is_within(&p, &base));
    if !ok {
        return Err(kalango_i18n::trf("outside the folders allowed for this agent: {path}", &[("path", p.display().to_string())]));
    }
    Ok(p)
}

/// Where `p` really leads: every symbolic link on the way is followed, also one whose target does not exist yet
/// (`canonicalize` fails on a dangling link, and writing through it creates its target: a link in Documents to a
/// protected file not created yet must still count as that file). The parts that do not exist are kept as given.
pub fn resolve_existing(p: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::from("/");
    let mut pending: std::collections::VecDeque<std::ffi::OsString> = p.components().filter_map(|c| match c {
        Component::Normal(n) => Some(n.to_os_string()),
        Component::ParentDir => Some("..".into()),
        _ => None,
    }).collect();
    // the kernel gives up after 40 links (ELOOP); so does this, keeping the rest as given
    let mut hops = 0;
    while let Some(name) = pending.pop_front() {
        if name == ".." {
            out.pop();
            continue;
        }
        let cand = out.join(&name);
        match std::fs::symlink_metadata(&cand) {
            Ok(md) if md.file_type().is_symlink() && hops < 40 => {
                hops += 1;
                let target = std::fs::read_link(&cand).unwrap_or_default();
                if target.is_absolute() {
                    out = PathBuf::from("/");
                }
                for c in target.components().rev() {
                    match c {
                        Component::Normal(n) => pending.push_front(n.to_os_string()),
                        Component::ParentDir => pending.push_front("..".into()),
                        _ => {}
                    }
                }
            }
            _ => out = cand,
        }
    }
    out
}

/// As `check_path`, for a path that will be changed (written, moved, renamed, trashed, deleted): it also may not
/// contain a protected folder. Moving `~/.config` away (or to the trash) would carry `~/.config/kalango` to a place
/// that is not protected, where it could be changed and then moved back.
pub fn check_change_path(raw: &str, peer: &Peer, allowed: &[String], protected: &[String]) -> Result<PathBuf, String> {
    let p = check_path(raw, peer, allowed, protected)?;
    let resolved = resolve_existing(&p);
    for prot in protected {
        let base = normalize(&expand(prot, &peer.home));
        if is_within(&base, &p) || is_within(&base, &resolved) || is_within(&resolve_existing(&base), &resolved) {
            return Err(format!("pasta protegida: {} (nenhum agente acessa)", base.display()));
        }
    }
    Ok(p)
}

/// What a file tool's helper may do at one of this call's own paths (besides the cut of the allowed folders).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grant {
    /// read and write this existing file
    File,
    /// create entries in this folder (and write what it creates there)
    Create,
    /// remove entries from this folder (or move them out)
    Remove,
}

/// The Landlock rules of a file tool's helper. A call that only reads (list, read) gets the allowed folders cut like a
/// command's (`startup::carve`), read-only, with nothing in the protected folders. A call that changes something gets
/// exactly its own paths, each with only what the operation needs there (`Grant`): the file it writes, the folder it
/// creates in, the folder it removes from. A startup path gets a grant only when the startup gate approved this very
/// call for it (`approved`). Landlock rules hold for everything below a folder, so a grant on the home's top folder
/// (a file created or removed there) also reaches into ~/.config — the helper's own operation is then fixed by name
/// and folder descriptor (`open_dir_nofollow`), which no link can redirect.
pub fn file_rules(peer: &Peer, cfg: &crate::policy::Config, allowed: &[String], targets: &[(PathBuf, Grant)], approved: &[PathBuf], extra: &[(PathBuf, Grant)]) -> Value {
    let roots: Vec<PathBuf> = allowed.iter().map(|a| expand(a, &peer.home)).collect();
    let protected: Vec<PathBuf> = cfg.protected_paths.iter().map(|p| expand(p, &peer.home)).collect();
    let ask = crate::startup::mode(cfg, peer) == crate::startup::Mode::Ask;
    // a call that changes something gets nothing but its own paths (it reads nothing else); one that reads (list, read)
    // gets the cut of the allowed folders, read-only
    let cut = if targets.is_empty() {
        let c = crate::startup::carve(&roots, &protected, &[]);
        crate::startup::Carved { read: c.write.into_iter().chain(c.read).collect(), list: c.list, write: vec![] }
    } else {
        crate::startup::Carved::default()
    };
    let approved: Vec<PathBuf> = approved.iter().map(|a| resolve_existing(a)).collect();
    let (mut file, mut create, mut remove) = (Vec::new(), Vec::new(), Vec::new());
    for (t, g) in targets.iter().chain(extra) {
        let r = resolve_existing(t);
        let is_extra = extra.iter().any(|(e, _)| e == t);
        // (a folder a call creates in or removes from is checked by the files it touches; the file itself here)
        let startup_target = ask && !is_extra && *g == Grant::File && crate::startup::startup_base(&r.display().to_string(), peer, cfg).is_some();
        if startup_target && !approved.iter().any(|a| is_within(&r, a) || is_within(a, &r)) {
            continue;
        }
        match g {
            Grant::File if r.exists() => file.push(r),
            Grant::File => {}
            Grant::Create => create.push(nearest_existing(&r)),
            Grant::Remove => remove.push(r),
        }
    }
    let real_roots: Vec<PathBuf> = roots.iter().map(|r| resolve_existing(r)).chain(extra.iter().map(|(e, _)| resolve_existing(e))).collect();
    json!({"escrita": cut.write.into_iter().chain(file).collect::<Vec<_>>(), "leitura": cut.read, "listar": cut.list,
        "criar": create, "remover": remove, "raizes": real_roots})
}

/// The folder itself, or the nearest one above it that exists (a write creates the rest).
fn nearest_existing(p: &Path) -> PathBuf {
    let mut d = p.to_path_buf();
    while !d.exists() {
        match d.parent() {
            Some(x) => d = x.to_path_buf(),
            None => break,
        }
    }
    d
}

/// As `run_as_user`, with `file_rules` instead of whole roots.
pub fn run_with_rules(peer: &Peer, rules: Value, op: Value) -> Result<Value, String> {
    run_helper(peer, json!({ "regras": rules, "op": op }))
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
    run_helper(peer, json!({ "roots": roots, "op": op }))
}

fn run_helper(peer: &Peer, input: Value) -> Result<Value, String> {
    let exe = self_exe()?;
    let mut cmd = Command::new(exe);
    cmd.arg("__auxiliar").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    if nix::unistd::geteuid().is_root() && peer.uid != 0 {
        // a biblioteca padrão já remove os grupos suplementares do root antes de trocar de usuário
        cmd.uid(peer.uid).gid(peer.gid);
    }
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
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
        // the rules cut by kalangod (`file_rules`: the protected folders out, the startup files read-only unless the
        // person approved this change), or plain roots (the trash's first creation)
        match input.get("regras") {
            Some(r) => {
                let list = |k: &str| -> Vec<PathBuf> { serde_json::from_value(r[k].clone()).unwrap_or_default() };
                let _ = ROOTS.set(list("raizes"));
                restrict_rules(&list("escrita"), &list("leitura"), &list("listar"), &list("criar"), &list("remover"))?;
            }
            None => {
                let roots: Vec<PathBuf> = serde_json::from_value(input["roots"].clone()).map_err(|e| e.to_string())?;
                restrict(&roots)?;
            }
        }
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

/// As `restrict`, with kalangod's cut of the allowed folders: full access, reading, or only listing a folder's names.
fn restrict_rules(write: &[PathBuf], read: &[PathBuf], list: &[PathBuf], create: &[PathBuf], remove: &[PathBuf]) -> Result<(), String> {
    use landlock::BitFlags;
    let abi = ABI::V3;
    let mut rs = Ruleset::default().handle_access(AccessFs::from_all(abi)).map_err(|e| e.to_string())?.create().map_err(|e| e.to_string())?;
    // a folder a call creates in or removes from: new files and folders written as they are made, entries taken out,
    // and moved between the two (Refer). Both get the same rights: Landlock refuses (EXDEV) a move that would give the
    // file more rights where it lands than where it was.
    let make: BitFlags<AccessFs> = AccessFs::MakeReg | AccessFs::MakeDir | AccessFs::WriteFile | AccessFs::Truncate | AccessFs::ReadFile | AccessFs::ReadDir
        | AccessFs::Refer | AccessFs::RemoveFile | AccessFs::RemoveDir;
    let take = make;
    for (paths, access) in [(list, AccessFs::ReadDir.into()), (read, AccessFs::from_read(abi)), (write, AccessFs::from_all(abi)), (create, make), (remove, take)] {
        for p in paths {
            if let Ok(fd) = PathFd::new(p) {
                // (a rule on a file takes only a file's rights)
                let a = if p.is_dir() { access } else { access & AccessFs::from_file(abi) };
                rs = rs.add_rule(PathBeneath::new(fd, a)).map_err(|e| e.to_string())?;
            }
        }
    }
    let st = rs.restrict_self().map_err(|e| e.to_string())?;
    if st.ruleset == RulesetStatus::NotEnforced {
        return Err(kalango_i18n::tr("Landlock unavailable on this kernel: operation refused").into());
    }
    Ok(())
}

/// The real allowed folders this helper works beneath (kalangod gives them).
static ROOTS: std::sync::OnceLock<Vec<PathBuf>> = std::sync::OnceLock::new();

/// A folder opened without following any link on the way, beneath the allowed folder that holds it (`openat2`
/// RESOLVE_NO_SYMLINKS | RESOLVE_BENEATH from that folder's own descriptor), created when asked: what is then done
/// in it happens in that very folder, even if a link is put on the path after kalangod checked it.
fn open_dir_nofollow(dir: &Path, create: bool) -> Result<std::os::fd::OwnedFd, String> {
    use nix::fcntl::{openat2, OFlag, OpenHow, ResolveFlag};
    use std::os::fd::{AsRawFd, FromRawFd};
    let flags = OFlag::O_PATH | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC;
    let root = ROOTS.get().and_then(|rs| rs.iter().filter(|r| dir.starts_with(r)).max_by_key(|r| r.as_os_str().len()).cloned());
    let opened = match &root {
        Some(r) => {
            let rfd = openat2(libc::AT_FDCWD, r.as_path(), OpenHow::new().flags(flags).resolve(ResolveFlag::RESOLVE_NO_SYMLINKS))
                .map(|fd| unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) });
            match (rfd, dir.strip_prefix(r)) {
                (Ok(rfd), Ok(rel)) if rel.as_os_str().is_empty() => Ok(std::os::fd::IntoRawFd::into_raw_fd(rfd)),
                (Ok(rfd), Ok(rel)) => openat2(rfd.as_raw_fd(), rel, OpenHow::new().flags(flags).resolve(ResolveFlag::RESOLVE_NO_SYMLINKS | ResolveFlag::RESOLVE_BENEATH)),
                (Err(e), _) => Err(e),
                (_, Err(_)) => Err(nix::errno::Errno::EXDEV),
            }
        }
        None => openat2(libc::AT_FDCWD, dir, OpenHow::new().flags(flags).resolve(ResolveFlag::RESOLVE_NO_SYMLINKS)),
    };
    match opened {
        Ok(fd) => Ok(unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) }),
        Err(nix::errno::Errno::ENOENT) if create => {
            let (parent, name) = (dir.parent().ok_or_else(|| dir.display().to_string())?, dir.file_name().ok_or_else(|| dir.display().to_string())?);
            let pfd = open_dir_nofollow(parent, true)?;
            match nix::sys::stat::mkdirat(Some(pfd.as_raw_fd()), name, nix::sys::stat::Mode::from_bits_truncate(0o755)) {
                Ok(()) | Err(nix::errno::Errno::EEXIST) => {}
                Err(e) => return Err(format!("{}: {e}", dir.display())),
            }
            open_dir_nofollow(dir, false)
        }
        Err(nix::errno::Errno::ELOOP) => Err(kalango_i18n::trf("{path} changed into a link after it was checked: nothing was done", &[("path", dir.display().to_string())])),
        Err(e) => Err(format!("{}: {e}", dir.display())),
    }
}

/// Puts what is in `from` (a staging folder no agent reaches) into `to`, from folder descriptors opened without
/// following links (`open_dir_nofollow`) and with `renameat2(RENAME_NOREPLACE)`: whatever an agent turned into a link
/// on the way since the check, nothing is written through it. What already exists at the destination is kept (the
/// import's "ignore existing"); folders on both sides are merged. Same filesystem (both in the home). Returns how many
/// entries were put in place.
pub fn merge_tree(from: &Path, to: &Path) -> Result<usize, String> {
    use std::os::fd::AsRawFd;
    let df = open_dir_nofollow(from, false)?;
    let dt = open_dir_nofollow(to, true)?;
    let mut n = 0;
    for e in std::fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))?.flatten() {
        let name = e.file_name();
        match nix::fcntl::renameat2(Some(df.as_raw_fd()), name.as_os_str(), Some(dt.as_raw_fd()), name.as_os_str(), nix::fcntl::RenameFlags::RENAME_NOREPLACE) {
            Ok(()) => n += 1,
            Err(nix::errno::Errno::EEXIST) => {
                let both_dirs = e.file_type().map(|t| t.is_dir()).unwrap_or(false)
                    && nix::sys::stat::fstatat(Some(dt.as_raw_fd()), name.as_os_str(), nix::fcntl::AtFlags::AT_SYMLINK_NOFOLLOW)
                        .map(|st| st.st_mode & libc::S_IFMT == libc::S_IFDIR).unwrap_or(false);
                if both_dirs {
                    n += merge_tree(&from.join(&name), &to.join(&name))?;
                }
            }
            Err(err) => return Err(format!("{} → {}: {err}", from.join(&name).display(), to.join(&name).display())),
        }
    }
    Ok(n)
}

/// A file's folder (opened as `open_dir_nofollow`) and its name.
fn split_nofollow(f: &Path, create: bool) -> Result<(std::os::fd::OwnedFd, std::ffi::OsString), String> {
    let name = f.file_name().ok_or_else(|| kalango_i18n::tr("path without a name").to_string())?.to_os_string();
    let dir = open_dir_nofollow(f.parent().unwrap_or(Path::new("/")), create)?;
    Ok((dir, name))
}

/// Opens a file in a folder without following a link at the end either (O_NOFOLLOW).
fn open_in(dir: &std::os::fd::OwnedFd, name: &std::ffi::OsStr, flags: nix::fcntl::OFlag) -> nix::Result<std::fs::File> {
    use std::os::fd::{AsRawFd, FromRawFd};
    let fd = nix::fcntl::openat(Some(dir.as_raw_fd()), name, flags | nix::fcntl::OFlag::O_NOFOLLOW | nix::fcntl::OFlag::O_CLOEXEC, nix::sys::stat::Mode::from_bits_truncate(0o644))?;
    Ok(unsafe { std::fs::File::from_raw_fd(fd) })
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
            // (kalangod gives the path with its links already followed: the folders on the way and the file itself are
            // opened without following any, so a link put there after the check makes the write fail)
            use nix::fcntl::OFlag;
            use std::io::Write as _;
            let f = p("caminho")?;
            let (dir, name) = split_nofollow(&f, true)?;
            let before = open_in(&dir, &name, OFlag::O_RDONLY).ok().and_then(|mut h| {
                let mut b = Vec::new();
                h.read_to_end(&mut b).ok().map(|_| b)
            });
            let bytes = match op.get("conteudo_base64").and_then(|v| v.as_str()) {
                Some(b) => {
                    use base64::Engine as _;
                    base64::engine::general_purpose::STANDARD.decode(b).map_err(|e| format!("base64: {e}"))?
                }
                None => op["conteudo"].as_str().unwrap_or("").as_bytes().to_vec(),
            };
            // (a file that did not exist when kalangod checked is created, never opened: O_EXCL)
            let flags = if op["novo"] == true { OFlag::O_WRONLY | OFlag::O_CREAT | OFlag::O_EXCL } else { OFlag::O_WRONLY | OFlag::O_CREAT | OFlag::O_TRUNC };
            let mut h = open_in(&dir, &name, flags).map_err(|e| format!("{}: {e}", f.display()))?;
            h.write_all(&bytes).map_err(|e| format!("{}: {e}", f.display()))?;
            Ok(json!({ "existia": before.is_some(), "anterior": before.map(|b| String::from_utf8_lossy(&b).into_owned()) }))
        }
        // the startup files there are (policy::STARTUP_PATHS) with their text: what a command changed in them is seen
        // by comparing two of these (links are not followed; at most 200 files of 1 MiB)
        "instantaneo" => {
            let mut files = serde_json::Map::new();
            let mut pending: Vec<(PathBuf, usize)> = op["caminhos"].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(|s| (PathBuf::from(s), 0))).collect();
            while let Some((f, depth)) = pending.pop() {
                let Ok(md) = std::fs::symlink_metadata(&f) else { continue };
                if md.is_dir() && depth < 4 {
                    for e in std::fs::read_dir(&f).into_iter().flatten().flatten() {
                        pending.push((e.path(), depth + 1));
                    }
                } else if md.is_file() && files.len() < 200 {
                    let text = std::fs::File::open(&f).ok().and_then(|h| {
                        let mut b = Vec::new();
                        h.take(1 << 20).read_to_end(&mut b).ok().map(|_| String::from_utf8_lossy(&b).into_owned())
                    });
                    files.insert(f.display().to_string(), json!(text));
                } else if md.file_type().is_symlink() {
                    // (a link is recorded by where it points: a new link is a change too)
                    files.insert(f.display().to_string(), json!(format!("→ {}", std::fs::read_link(&f).unwrap_or_default().display())));
                }
            }
            Ok(json!({ "arquivos": files }))
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
            let (dir, name) = split_nofollow(&f, false)?;
            use std::os::fd::AsRawFd;
            nix::unistd::unlinkat(Some(dir.as_raw_fd()), name.as_os_str(), nix::unistd::UnlinkatFlags::NoRemoveDir).map_err(|e| format!("{}: {e}", f.display()))?;
            Ok(json!({}))
        }
        "mover" => {
            let (a, b) = (p("de")?, p("para")?);
            if std::fs::symlink_metadata(&b).is_ok() {
                return Err(kalango_i18n::trf("already exists: {path}", &[("path", b.display().to_string())]));
            }
            let (da, na) = split_nofollow(&a, false)?;
            let (db, nb) = split_nofollow(&b, true)?;
            use std::os::fd::AsRawFd;
            // (never over something that appeared there meanwhile)
            nix::fcntl::renameat2(Some(da.as_raw_fd()), na.as_os_str(), Some(db.as_raw_fd()), nb.as_os_str(), nix::fcntl::RenameFlags::RENAME_NOREPLACE)
                .map_err(|e| format!("{} → {}: {e}", a.display(), b.display()))?;
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
            while std::fs::symlink_metadata(trash.join("files").join(&target)).is_ok() {
                n += 1;
                target = format!("{name}.{n}");
            }
            let (da, na) = split_nofollow(&f, false)?;
            let dt = open_dir_nofollow(&trash.join("files"), false)?;
            use std::os::fd::AsRawFd;
            nix::fcntl::renameat2(Some(da.as_raw_fd()), na.as_os_str(), Some(dt.as_raw_fd()), target.as_str(), nix::fcntl::RenameFlags::RENAME_NOREPLACE).map_err(|e| format!("{}: {e}", f.display()))?;
            let info = format!("[Trash Info]\nPath={}\nDeletionDate={}\n", f.display(), op["data"].as_str().unwrap_or(""));
            let di = open_dir_nofollow(&trash.join("info"), false)?;
            use std::io::Write as _;
            open_in(&di, std::ffi::OsStr::new(&format!("{target}.trashinfo")), nix::fcntl::OFlag::O_WRONLY | nix::fcntl::OFlag::O_CREAT | nix::fcntl::OFlag::O_TRUNC)
                .map_err(|e| e.to_string())?.write_all(info.as_bytes()).map_err(|e| e.to_string())?;
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

#[cfg(test)]
mod path_tests {
    use super::*;
    use std::os::unix::fs::symlink;

    /// A home of the test's own (never the real one) with the default protected list.
    fn setup() -> (tempfile::TempDir, Peer, Vec<String>, Vec<String>) {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("casa");
        for p in ["Documentos", ".config/kalango/conectores", ".local/share/kalango/fluxos", ".local/share/kalango/virtualdesk/maquinas/ana",
            ".local/share/kalango/conectores/relogio"] {
            std::fs::create_dir_all(home.join(p)).unwrap();
        }
        let peer = Peer { pid: 1, uid: 1000, gid: 1000, exe: "/usr/bin/teste".into(), cgroup: String::new(), passwd_home: home.clone(), home, user: "teste".into(), has_tty: false };
        (d, peer, vec!["~".into()], crate::policy::Config::system_default().protected_paths)
    }

    fn refused(raw: &str, peer: &Peer, allowed: &[String], protected: &[String]) -> bool {
        check_path(raw, peer, allowed, protected).is_err() && check_change_path(raw, peer, allowed, protected).is_err()
    }

    #[test]
    fn kalango_settings_and_data_are_out_of_reach() {
        let (_d, peer, allowed, protected) = setup();
        for raw in ["~/.config/kalango/agente-permissoes.json", "~/.config/kalango/agente-ajustes.json", "~/.config/kalango/conectores/x.toml",
            "~/.config/kalango/conectores.json", "~/.config/kalango/remote/hosts.json", "~/.config/kalango/rotinas.json", "~/.config/kalango/orcamento.json",
            "~/.config/kalango/atalhos.kdl", "~/.config/kalango-remote/kalango-remote.toml", "~/.local/share/kalango/fluxos/fluxos.db",
            "~/.local/share/kalango/virtualdesk/maquinas/ana/maquina.json", "~/.local/share/kalango/conversas/1/eventos.jsonl",
            "~/.local/share/kalango/modelos/LFM3-1.7B-Q4_K_M.gguf", "~/.local/state/kalango/uso-tokens.jsonl", "~/.config/kalango"] {
            assert!(refused(raw, &peer, &allowed, &protected), "{raw} should be refused");
        }
        // through .. from an allowed folder
        assert!(refused("~/Documentos/../.config/kalango/agente-permissoes.json", &peer, &allowed, &protected));
        assert!(refused("~/Documentos/../.local/share/kalango/fluxos/fluxos.db", &peer, &allowed, &protected));
        // the person's own folders still work, and a connector's own data folder too
        assert!(check_change_path("~/Documentos/notas.md", &peer, &allowed, &protected).is_ok());
        assert!(check_change_path("~/Documentos", &peer, &allowed, &protected).is_ok());
        assert!(check_change_path("~/.local/share/kalango/conectores/relogio/estado.json", &peer, &allowed, &protected).is_ok());
    }

    /// The Mind's confirmation codes (`%t/kalango/mind-confirm`) are what the person's session reads to confirm what an
    /// agent asked to remember. Every file tool (read, write, list, change, move) is refused there — even for a caller
    /// allowed to all of `/run`, and by a `..`, by a link (also dangling) or for a file that does not exist yet — and the
    /// folders around it still work.
    #[test]
    fn the_mind_confirmation_codes_are_out_of_reach() {
        let (_d, peer, _, protected) = setup();
        let allowed: Vec<String> = vec!["~".into(), "/run".into()];
        let run = crate::policy::runtime_dir(nix::unistd::getuid().as_raw());
        let dir = run.join("kalango/mind-confirm");
        for raw in [dir.display().to_string(), dir.join("0f3c.token").display().to_string(), dir.join("sub/outro").display().to_string(),
            run.join("kalango/../kalango/mind-confirm/0f3c.token").display().to_string(), "%t/kalango/mind-confirm/0f3c.token".replace("%t", &run.display().to_string())] {
            assert!(refused(&raw, &peer, &allowed, &protected), "{raw} should be refused");
        }
        // moving or deleting the folder that holds it would carry it somewhere unprotected
        assert!(check_change_path(&run.join("kalango").display().to_string(), &peer, &allowed, &protected).is_err());
        // a link in the home to it, and one to a file there that does not exist yet
        let h = peer.home.clone();
        symlink(&dir, h.join("Documentos/codigos")).unwrap();
        symlink(dir.join("novo.token"), h.join("Documentos/novo")).unwrap();
        assert!(refused("~/Documentos/codigos/0f3c.token", &peer, &allowed, &protected));
        assert!(refused("~/Documentos/novo", &peer, &allowed, &protected));
        // the rest of the runtime folder is not the Mind's
        assert!(check_path(&run.join("kalango/know.sock").display().to_string(), &peer, &allowed, &protected).is_ok());
        assert!(check_path(&run.join("outra-coisa").display().to_string(), &peer, &allowed, &protected).is_ok());
    }

    #[test]
    fn a_link_into_the_settings_is_refused_even_dangling() {
        let (_d, peer, allowed, protected) = setup();
        let h = peer.home.clone();
        symlink(h.join(".config/kalango"), h.join("Documentos/atalho")).unwrap();
        assert!(refused("~/Documentos/atalho/agente-permissoes.json", &peer, &allowed, &protected));
        // a link to a file that does not exist yet: writing through it would create the protected file
        symlink(h.join(".config/kalango/agente-permissoes.json"), h.join("Documentos/permissoes.json")).unwrap();
        assert!(!h.join(".config/kalango/agente-permissoes.json").exists());
        assert!(refused("~/Documentos/permissoes.json", &peer, &allowed, &protected));
        // relative, and through a chain of links
        symlink("../.local/share/kalango/virtualdesk", h.join("Documentos/vd")).unwrap();
        symlink(h.join("Documentos/vd"), h.join("Documentos/vd2")).unwrap();
        assert!(refused("~/Documentos/vd/maquinas/ana/maquina.json", &peer, &allowed, &protected));
        assert!(refused("~/Documentos/vd2/maquinas/ana/maquina.json", &peer, &allowed, &protected));
        // a link to a missing folder whose parent is protected
        symlink(h.join(".local/share/kalango/fluxos/novo"), h.join("Documentos/fl")).unwrap();
        assert!(refused("~/Documentos/fl/fluxos.db", &peer, &allowed, &protected));
        // a link that stays among the person's folders is fine
        symlink(h.join("Documentos"), h.join("docs")).unwrap();
        assert!(check_change_path("~/docs/notas.md", &peer, &allowed, &protected).is_ok());
    }

    /// The helper writes, moves and deletes without following links on the way (kalangod gives it the real path): a
    /// folder swapped for a link after the check (to ~/.config/autostart, say) makes the operation fail, nothing is
    /// written there.
    #[test]
    fn the_helper_never_follows_a_link_put_on_the_path() {
        let (_d, peer, _allowed, _protected) = setup();
        let h = peer.home.clone();
        std::fs::create_dir_all(h.join(".config/autostart")).unwrap();
        symlink(h.join(".config/autostart"), h.join("Documentos/d")).unwrap();
        let r = do_op(&json!({"tipo": "escrever", "caminho": h.join("Documentos/d/x.desktop"), "conteudo": "evil"}));
        assert!(r.is_err(), "{r:?}");
        assert!(!h.join(".config/autostart/x.desktop").exists());
        // the last part too: a link where the file was
        symlink(h.join(".config/autostart/y.desktop"), h.join("Documentos/y")).unwrap();
        assert!(do_op(&json!({"tipo": "escrever", "caminho": h.join("Documentos/y"), "conteudo": "evil"})).is_err());
        assert!(!h.join(".config/autostart/y.desktop").exists());
        // moving into a link, and deleting through one
        std::fs::write(h.join("Documentos/a.txt"), "a").unwrap();
        assert!(do_op(&json!({"tipo": "mover", "de": h.join("Documentos/a.txt"), "para": h.join("Documentos/d/a.desktop")})).is_err());
        std::fs::write(h.join(".config/autostart/z.desktop"), "z").unwrap();
        assert!(do_op(&json!({"tipo": "apagar_arquivo", "caminho": h.join("Documentos/d/z.desktop")})).is_err());
        assert!(h.join(".config/autostart/z.desktop").exists());
        // an ordinary write, with the folders made
        assert!(do_op(&json!({"tipo": "escrever", "caminho": h.join("Documentos/novo/n.txt"), "conteudo": "ok"})).is_ok());
        assert_eq!(std::fs::read_to_string(h.join("Documentos/novo/n.txt")).unwrap(), "ok");
    }

    /// The import's merge from its staging folder: what is new goes in, what exists stays, folders merge, and a
    /// destination folder turned into a link (to autostart) is never written through.
    #[test]
    fn the_merge_never_writes_through_a_link() {
        let (_d, peer, _a, _p) = setup();
        let h = peer.home.clone();
        let st = h.join(".local/state/kalango/importar/1");
        std::fs::create_dir_all(st.join("Fotos")).unwrap();
        std::fs::write(st.join("novo.txt"), "novo").unwrap();
        std::fs::write(st.join("velho.txt"), "do outro sistema").unwrap();
        std::fs::write(st.join("Fotos/a.png"), "a").unwrap();
        std::fs::create_dir_all(st.join("Links")).unwrap();
        std::fs::write(st.join("Links/x.desktop"), "[Desktop Entry]").unwrap();
        let dest = h.join("Documentos/Antigos");
        std::fs::create_dir_all(dest.join("Fotos")).unwrap();
        std::fs::write(dest.join("velho.txt"), "da pessoa").unwrap();
        std::fs::create_dir_all(h.join(".config/autostart")).unwrap();
        symlink(h.join(".config/autostart"), dest.join("Links")).unwrap();
        let r = merge_tree(&st, &dest);
        assert_eq!(std::fs::read_to_string(dest.join("novo.txt")).unwrap(), "novo");
        assert_eq!(std::fs::read_to_string(dest.join("velho.txt")).unwrap(), "da pessoa", "what exists stays");
        assert_eq!(std::fs::read_to_string(dest.join("Fotos/a.png")).unwrap(), "a");
        assert!(!h.join(".config/autostart/x.desktop").exists(), "{r:?}");
    }

    #[test]
    fn a_folder_holding_a_protected_one_cannot_be_changed() {
        let (_d, peer, allowed, protected) = setup();
        // moved or trashed, ~/.config would carry ~/.config/kalango somewhere unprotected
        for raw in ["~/.config", "~/.local", "~/.local/share", "~/.local/share/kalango", "~"] {
            assert!(check_change_path(raw, &peer, &allowed, &protected).is_err(), "{raw} should not be changed");
            assert!(check_path(raw, &peer, &allowed, &protected).is_ok(), "{raw} can still be listed");
        }
        // nor through a link to it
        symlink(peer.home.join(".config"), peer.home.join("Documentos/cfg")).unwrap();
        assert!(check_change_path("~/Documentos/cfg", &peer, &allowed, &protected).is_err());
        // a destination that does not exist yet but would become one (a move that recreates ~/.config)
        assert!(check_change_path("~/.local/state", &peer, &allowed, &protected).is_err());
    }
}
