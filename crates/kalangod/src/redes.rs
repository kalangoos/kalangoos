//! The networks where the person's agents may not use the internet (Configurações › Rede › "Permitir agentes
//! usarem esta rede"): kept for each person in kalangod's state, and checked by `web.ler` against the connections
//! active now (NetworkManager's). A metered phone hotspot, a café's network: the person decides, the agents follow.

use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn arquivo(state_dir: &Path) -> PathBuf {
    state_dir.join("redes-sem-agentes.json")
}

fn tudo(state_dir: &Path) -> Value {
    std::fs::read(arquivo(state_dir)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_else(|| json!({}))
}

/// The networks this person keeps the agents off.
pub fn lista(state_dir: &Path, uid: u32) -> Vec<String> {
    tudo(state_dir)[uid.to_string()].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

/// Allows or keeps the agents off a network (by its connection's name); the list as it is now.
pub fn definir(state_dir: &Path, uid: u32, rede: &str, permitir: bool) -> Result<Vec<String>, String> {
    if rede.is_empty() || rede.len() > 256 || rede.contains('\n') {
        return Err(kalango_i18n::tr("invalid network name").into());
    }
    let mut v = tudo(state_dir);
    let mut l = lista(state_dir, uid);
    l.retain(|x| x != rede);
    if !permitir {
        l.push(rede.to_string());
    }
    v[uid.to_string()] = json!(l);
    std::fs::create_dir_all(state_dir).map_err(|e| e.to_string())?;
    let tmp = arquivo(state_dir).with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(&v).unwrap_or_default()).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, arquivo(state_dir)).map_err(|e| e.to_string())?;
    Ok(l)
}

/// NetworkManager's command (a stand-in in the tests).
pub fn nmcli() -> String {
    std::env::var("KALANGO_NMCLI").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "nmcli".into())
}

/// The connections active now (their names).
pub fn ativas() -> Vec<String> {
    std::process::Command::new(nmcli()).args(["-t", "-f", "NAME", "connection", "show", "--active"]).env("LC_ALL", "C").output().ok()
        .filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).lines().map(|l| l.replace("\\:", ":")).filter(|l| !l.is_empty()).collect())
        .unwrap_or_default()
}

/// The active network this person keeps the agents off, if they are on one.
pub fn bloqueada(state_dir: &Path, uid: u32) -> Option<String> {
    let l = lista(state_dir, uid);
    if l.is_empty() {
        return None;
    }
    ativas().into_iter().find(|a| l.contains(a))
}

#[cfg(test)]
mod tests {
    #[test]
    fn each_person_keeps_their_own_list() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(super::definir(d.path(), 1000, "Celular", false).unwrap(), vec!["Celular"]);
        super::definir(d.path(), 1001, "Café", false).unwrap();
        assert_eq!(super::lista(d.path(), 1000), vec!["Celular"]);
        assert_eq!(super::definir(d.path(), 1000, "Celular", true).unwrap(), Vec::<String>::new());
        assert_eq!(super::lista(d.path(), 1001), vec!["Café"]);
        assert!(super::definir(d.path(), 1000, "", false).is_err());
    }
}
