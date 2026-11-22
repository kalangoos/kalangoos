//! The private server's access token. It lives in the user's KalangoOS vault (`kalangod cofre`: the system
//! keyring, or the protected file where there is none), under [`VAULT_NAME`]; never in `modelos.toml`, a URL, a
//! process argument or a log.
//!
//! The installer runs as root in the live session and cannot write to the new user's vault, so it leaves a
//! hand-off file in the new user's own folder (mode 0600, the folder 0700, both owned by that user). The first
//! session's shell calls [`adopt_staged`], which moves it into the vault and overwrites and deletes the file.

use crate::Secret;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const VAULT_NAME: &str = "kalango.modelos.token";
/// Relative to the user's data folder (`~/.local/share`).
pub const STAGING_REL: &str = "kalango/modelos-token";

fn kalangod() -> PathBuf {
    if let Some(p) = std::env::var_os("KALANGOD") {
        return PathBuf::from(p);
    }
    std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("kalangod"))).filter(|p| p.is_file()).unwrap_or_else(|| "/usr/bin/kalangod".into())
}

fn data_home() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share"))
}

pub fn staging_path() -> PathBuf {
    data_home().join(STAGING_REL)
}

/// What the vault keeps: the token and the server it was kept for. A token never follows an address that changed.
#[derive(serde::Serialize, serde::Deserialize)]
struct Bound {
    origin: String,
    token: String,
}

fn parse_bound(raw: &str) -> Option<Bound> {
    serde_json::from_str::<Bound>(raw.trim()).ok().filter(|b| !b.token.is_empty() && !b.origin.is_empty())
}

/// The installer's side: leaves the token (bound to the server's origin) for the first session (`home` is the new
/// user's home, e.g. `/target/home/ana`). The caller changes the owner of the home afterwards.
pub fn write_staging(home: &Path, origin: &str, token: &Secret) -> std::io::Result<PathBuf> {
    let dir = home.join(".local/share/kalango");
    std::fs::create_dir_all(&dir)?;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    let path = home.join(".local/share").join(STAGING_REL);
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&path)?;
    f.write_all(serde_json::json!({"origin": origin, "token": token.expose()}).to_string().as_bytes())?;
    Ok(path)
}

fn store_raw(value: &str) -> Result<(), String> {
    let mut ch = Command::new(kalangod()).args(["cofre", "guardar", VAULT_NAME]).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped()).spawn().map_err(|e| format!("kalangod: {e}"))?;
    ch.stdin.take().ok_or("no stdin")?.write_all(format!("{value}\n").as_bytes()).map_err(|e| e.to_string())?;
    let o = ch.wait_with_output().map_err(|e| e.to_string())?;
    if o.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&o.stderr).trim().to_string()) }
}

/// Keeps the token in the vault, bound to the server's origin.
pub fn store(origin: &str, token: &Secret) -> Result<(), String> {
    store_raw(&serde_json::json!({"origin": origin, "token": token.expose()}).to_string())
}

fn read_raw() -> Option<Bound> {
    let o = Command::new(kalangod()).args(["cofre", "ler", VAULT_NAME]).stdin(Stdio::null()).stderr(Stdio::null()).output().ok().filter(|o| o.status.success());
    if let Some(b) = o.and_then(|o| parse_bound(&String::from_utf8_lossy(&o.stdout))) {
        return Some(b);
    }
    std::fs::read_to_string(staging_path()).ok().and_then(|t| parse_bound(&t))
}

/// The token, only when it was kept for this very origin (from the vault; failing that, from a hand-off file still
/// waiting to be adopted). Any other origin gets nothing.
pub fn read_for(origin: &str) -> Option<Secret> {
    read_raw().filter(|b| b.origin == origin).map(|b| Secret::new(b.token))
}

/// The origin the kept token belongs to, if there is one.
pub fn bound_origin() -> Option<String> {
    read_raw().map(|b| b.origin)
}

/// Forgets the token (vault and hand-off file).
pub fn forget() {
    let _ = Command::new(kalangod()).args(["cofre", "apagar", VAULT_NAME]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
    shred(&staging_path());
}

fn shred(p: &Path) {
    if let Ok(md) = std::fs::metadata(p) {
        if let Ok(mut f) = std::fs::OpenOptions::new().write(true).open(p) {
            let _ = f.write_all(&vec![0u8; md.len() as usize]);
            let _ = f.sync_all();
        }
        let _ = std::fs::remove_file(p);
    }
}

/// The first session: moves the installer's hand-off file into the vault, then overwrites and deletes it. Returns
/// whether there was one. If the vault refuses, the file stays (it is private to the user) and the error says why.
pub fn adopt_staged() -> Result<bool, String> {
    let p = staging_path();
    let Ok(t) = std::fs::read_to_string(&p) else { return Ok(false) };
    let Some(b) = parse_bound(&t) else {
        shred(&p);
        return Ok(false);
    };
    store(&b.origin, &Secret::new(b.token))?;
    shred(&p);
    Ok(true)
}
