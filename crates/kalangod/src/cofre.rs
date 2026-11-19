//! Cofre de segredos do usuário (tokens de conectores, chaves de API), fora dos arquivos de configuração.
//!
//! Guarda no chaveiro do sistema (Secret Service, o mesmo do GNOME, via `secret-tool`), que fica criptografado e
//! destrancado com a sessão. Sem chaveiro (servidor, VM, teste), usa um arquivo 0600 em
//! `~/.local/share/kalango/cofre.json`, que é pasta protegida: nenhum agente nem conector o alcança.
//! `KALANGO_COFRE=arquivo|chaveiro` força o modo.
//!
//! Os manifestos de conector citam segredos como `cofre:NOME`; o valor só é lido na hora de subir o conector e só
//! vai para o ambiente dele.

use kalango_i18n::{tr, trf};
use serde_json::{Map, Value};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn valid(nome: &str) -> Result<&str, String> {
    let ok = !nome.is_empty() && nome.len() <= 128 && !nome.starts_with('-') && nome.chars().all(|c| c.is_ascii_alphanumeric() || "._:-".contains(c));
    if ok { Ok(nome) } else { Err(trf("invalid secret name: {name}", &[("name", format!("{nome:?}"))])) }
}

enum Modo {
    Chaveiro,
    Arquivo,
}

fn modo() -> Modo {
    match std::env::var("KALANGO_COFRE").as_deref() {
        Ok("arquivo") => Modo::Arquivo,
        Ok("chaveiro") => Modo::Chaveiro,
        _ if crate::restore::which("secret-tool") && std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some() => Modo::Chaveiro,
        _ => Modo::Arquivo,
    }
}

pub fn file() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|p| p.is_absolute())
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share"));
    base.join("kalango/cofre.json")
}

fn read_file() -> Map<String, Value> {
    std::fs::read_to_string(file()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn write_file(m: &Map<String, Value>) -> Result<(), String> {
    let f = file();
    let dir = f.parent().unwrap();
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    let tmp = f.with_extension("tmp");
    let mut h = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp).map_err(|e| e.to_string())?;
    h.write_all(serde_json::to_string_pretty(m).unwrap().as_bytes()).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &f).map_err(|e| e.to_string())
}

fn secret_tool(args: &[&str], input: Option<&str>) -> Result<String, String> {
    let mut c = Command::new("secret-tool");
    c.args(args).stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() }).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut ch = c.spawn().map_err(|e| format!("secret-tool: {e}"))?;
    if let Some(i) = input {
        ch.stdin.take().unwrap().write_all(i.as_bytes()).map_err(|e| e.to_string())?;
    }
    let o = ch.wait_with_output().map_err(|e| e.to_string())?;
    if !o.status.success() {
        return Err(String::from_utf8_lossy(&o.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&o.stdout).into_owned())
}

/// Guarda (ou troca) um segredo. Devolve onde ficou.
pub fn put(nome: &str, valor: &str) -> Result<&'static str, String> {
    let nome = valid(nome)?;
    match modo() {
        Modo::Chaveiro => {
            secret_tool(&["store", &format!("--label=KalangoOS: {nome}"), "app", "kalango", "nome", nome], Some(valor))?;
            Ok(tr("system keyring"))
        }
        Modo::Arquivo => {
            let mut m = read_file();
            m.insert(nome.into(), Value::String(valor.into()));
            write_file(&m)?;
            Ok(tr("protected file"))
        }
    }
}

pub fn get(nome: &str) -> Result<Option<String>, String> {
    let nome = valid(nome)?;
    Ok(match modo() {
        Modo::Chaveiro => secret_tool(&["lookup", "app", "kalango", "nome", nome], None).ok().filter(|v| !v.is_empty()),
        Modo::Arquivo => read_file().get(nome).and_then(Value::as_str).map(str::to_string),
    })
}

pub fn delete(nome: &str) -> Result<bool, String> {
    let nome = valid(nome)?;
    let existed = get(nome)?.is_some();
    match modo() {
        Modo::Chaveiro => {
            let _ = secret_tool(&["clear", "app", "kalango", "nome", nome], None);
        }
        Modo::Arquivo => {
            let mut m = read_file();
            m.remove(nome);
            write_file(&m)?;
        }
    }
    Ok(existed)
}

/// Nomes dos segredos guardados (nunca os valores).
pub fn list() -> Vec<String> {
    let mut v: Vec<String> = match modo() {
        // o secret-tool põe os atributos na saída de erro (e os valores na padrão, que são descartados)
        Modo::Chaveiro => Command::new("secret-tool").args(["search", "--all", "app", "kalango"]).stdin(Stdio::null()).output()
            .map(|o| String::from_utf8_lossy(&o.stderr).lines().filter_map(|l| l.strip_prefix("attribute.nome = ").map(str::to_string)).collect())
            .unwrap_or_default(),
        Modo::Arquivo => read_file().keys().cloned().collect(),
    };
    v.sort();
    v.dedup();
    v
}

/// Troca `cofre:NOME` pelo valor guardado; outros valores passam como estão.
pub fn resolve(v: &str) -> Result<String, String> {
    match v.strip_prefix("cofre:") {
        Some(nome) => get(nome)?.ok_or(trf("the secret '{name}' is not in the vault (kalangod cofre guardar {name})", &[("name", nome.to_string())])),
        None => Ok(v.to_string()),
    }
}

/// Lê um valor do terminal sem mostrar o que é digitado (ou da entrada padrão, se não for terminal).
pub fn read_secret_stdin() -> Result<String, String> {
    let fd = 0;
    let tty = unsafe { libc::isatty(fd) } == 1;
    let mut old: libc::termios = unsafe { std::mem::zeroed() };
    if tty {
        eprint!("{}", tr("value (not shown on screen): "));
        unsafe {
            libc::tcgetattr(fd, &mut old);
            let mut t = old;
            t.c_lflag &= !libc::ECHO;
            libc::tcsetattr(fd, libc::TCSANOW, &t);
        }
    }
    let mut s = String::new();
    let r = std::io::stdin().read_line(&mut s);
    if tty {
        unsafe { libc::tcsetattr(fd, libc::TCSANOW, &old) };
        eprintln!();
    }
    r.map_err(|e| e.to_string())?;
    let s = s.trim_end_matches(['\n', '\r']).to_string();
    if s.is_empty() {
        return Err(tr("empty value").into());
    }
    Ok(s)
}
