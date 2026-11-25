//! O que a pessoa escolheu para cada app criativo, em `~/.config/kalango/apps-criativos.json`: só a atualização
//! automática (ligada por padrão, geral e por app). O que dá acesso — as pastas que o app vê, a rede e se os agentes
//! podem usá-lo — fica no estado do kalangod, que os programas da pessoa (os agentes inclusive) não escrevem.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct DoApp {
    /// None: segue a geral
    #[serde(default)]
    pub atualizar: Option<bool>,
    /// voltou a uma versão anterior: a atualização automática deste app fica parada até a pessoa ligar de novo
    #[serde(default)]
    pub voltou: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Ajustes {
    #[serde(default = "sim")]
    pub atualizar: bool,
    #[serde(default)]
    pub apps: BTreeMap<String, DoApp>,
}

fn sim() -> bool {
    true
}

impl Default for Ajustes {
    fn default() -> Self {
        Ajustes { atualizar: true, apps: BTreeMap::new() }
    }
}

impl Ajustes {
    pub fn do_app(&self, id: &str) -> DoApp {
        self.apps.get(id).cloned().unwrap_or_default()
    }

    /// Se a Central atualiza este app sozinha.
    pub fn atualiza(&self, id: &str) -> bool {
        let a = self.do_app(id);
        !a.voltou && a.atualizar.unwrap_or(self.atualizar)
    }
}

pub fn arquivo() -> PathBuf {
    if let Some(p) = std::env::var_os("KALANGO_CRAFT_AJUSTES") {
        return PathBuf::from(p);
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join("kalango/apps-criativos.json")
}

pub fn ler() -> Ajustes {
    std::fs::read_to_string(arquivo()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn gravar(a: &Ajustes) -> Result<(), String> {
    let p = arquivo();
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(a).unwrap_or_default()).map_err(|e| e.to_string())?;
    std::fs::rename(tmp, p).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_update_is_on_by_default_and_follows_the_app_switch() {
        let mut a: Ajustes = serde_json::from_str("{}").unwrap();
        assert!(a.atualizar);
        assert!(a.atualiza("ai.storyteller.filmcraft"));
        a.apps.insert("ai.storyteller.filmcraft".into(), DoApp { atualizar: Some(false), ..Default::default() });
        assert!(!a.atualiza("ai.storyteller.filmcraft"));
        a.atualizar = false;
        assert!(!a.atualiza("ai.storyteller.anicraft"));
        a.apps.insert("ai.storyteller.anicraft".into(), DoApp { atualizar: Some(true), voltou: true });
        assert!(!a.atualiza("ai.storyteller.anicraft"), "after going back, auto-update waits for the person");
    }
}
