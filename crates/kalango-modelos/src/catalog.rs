//! The model catalog: what each model is and its files (url, name, size, SHA-256).

use serde::Deserialize;

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct ModelFile {
    pub url: String,
    pub nome: String,
    pub bytes: u64,
    /// lower-case hex SHA-256 of the file; empty when the catalog does not say
    #[serde(default)]
    pub sha256: String,
}

/// A model of the catalog: its files (the weights and, for voice and vision, the projector).
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct ModelEntry {
    pub id: String,
    pub nome: String,
    pub descricao: String,
    /// "texto", "voz", "visao", "omni" or "decisao"
    #[serde(default)]
    pub tipo: String,
    pub arquivos: Vec<ModelFile>,
}

impl ModelEntry {
    pub fn bytes(&self) -> u64 {
        self.arquivos.iter().map(|f| f.bytes).sum()
    }
    /// The weights' file (not the projector).
    pub fn main_file(&self) -> Option<&ModelFile> {
        self.arquivos.iter().find(|f| !f.nome.contains("mmproj")).or(self.arquivos.first())
    }
    /// Every file has a SHA-256: the only models a private server or a folder may provide.
    pub fn verifiable(&self) -> bool {
        !self.arquivos.is_empty() && self.arquivos.iter().all(|f| is_sha256(&f.sha256))
    }
}

/// The five kinds the installer offers, in order: (kind, the catalog id the installer proposes for it).
pub const SLOTS: &[(&str, &str)] = &[("texto", "lfm3-1.7b"), ("voz", "lfm3-audio-2b"), ("visao", "lfm3-vl-2b"), ("omni", "lfm3-omni-2b"), ("decisao", "d1-2b")];
/// What is ticked at first: text, voice and vision. Text is what the agents need.
pub const DEFAULT_CHOSEN: &[&str] = &["lfm3-1.7b", "lfm3-audio-2b", "lfm3-vl-2b"];
/// The model the agents cannot work without.
pub const TEXT_ID: &str = "lfm3-1.7b";

pub fn is_sha256(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// A file name that stays inside the models folder: letters, digits and `._+-`, no leading dot.
pub fn safe_file_name(n: &str) -> bool {
    !n.is_empty() && n.len() <= 200 && !n.starts_with('.') && n.bytes().all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b))
}

/// Reads the catalog. A name that could leave the models folder, or a SHA-256 that is not 64 hex digits, refuses the
/// whole catalog (it is what protects the disk from a hostile mirror's names and the files from a wrong check).
pub fn parse_catalog(text: &str) -> Result<Vec<ModelEntry>, String> {
    let mut v: Vec<ModelEntry> = serde_json::from_str(text).map_err(|e| e.to_string())?;
    for m in &mut v {
        for f in &mut m.arquivos {
            if !safe_file_name(&f.nome) {
                return Err(format!("{}: unsafe file name {:?}", m.id, f.nome));
            }
            f.sha256 = f.sha256.to_ascii_lowercase();
            if !f.sha256.is_empty() && !is_sha256(&f.sha256) {
                return Err(format!("{}: {} has an invalid sha256", m.id, f.nome));
            }
        }
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn good() -> String {
        format!(r#"[{{"id":"m","nome":"M","descricao":"d","tipo":"texto","arquivos":[
      {{"url":"https://x/m.gguf","nome":"m.gguf","bytes":10,"sha256":"3793E1E6{}"}}]}}]"#, "ab".repeat(28))
    }

    #[test]
    fn reads_sha256_and_lowercases_it() {
        let c = parse_catalog(&good()).unwrap();
        assert!(c[0].verifiable());
        assert!(c[0].arquivos[0].sha256.starts_with("3793e1e6"));
    }

    #[test]
    fn an_entry_without_sha256_is_not_verifiable() {
        let c = parse_catalog(r#"[{"id":"m","nome":"M","descricao":"d","arquivos":[{"url":"u","nome":"m.gguf","bytes":1}]}]"#).unwrap();
        assert!(!c[0].verifiable());
    }

    #[test]
    fn refuses_a_bad_sha256_and_unsafe_names() {
        assert!(parse_catalog(&good().replace("3793E1E6", "zzzzzzzz")).is_err());
        for bad in ["../x.gguf", "a/b.gguf", ".hidden", "", "a b.gguf"] {
            let t = good().replace("\"m.gguf\"", &format!("{bad:?}"));
            assert!(parse_catalog(&t).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_shipped_catalog_is_complete_and_has_the_five_kinds() {
        let t = include_str!("../../../sessao/dist/share/kalango/loja/modelos.json");
        let c = parse_catalog(t).unwrap();
        assert!(c.iter().all(|m| m.verifiable()), "every file of the shipped catalog has a sha256");
        for (_, id) in SLOTS {
            assert!(c.iter().any(|m| m.id == *id), "{id}");
        }
        assert!(c.iter().all(|m| m.arquivos.iter().all(|f| f.bytes > 0 && f.url.starts_with("https://"))));
    }
}
