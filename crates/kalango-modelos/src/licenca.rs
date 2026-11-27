//! The licence of each model and the person's acceptance of it.
//!
//! The weights are other people's work, under licences that are not on the project's accepted list (the LFM Open
//! License for LFM3 and d1; the Llama and Gemma terms). KalangoOS does not ship them: it downloads them, and only after
//! the person accepted the licence. The acceptance is a record per model, licence and licence version, with the date,
//! kept in the person's config folder (`~/.config/kalango/modelos-licencas.json`); the installer writes the same file
//! for the new account. The download queue refuses a model whose licence needs acceptance and has none (a licence
//! version bump asks again), so no interface can skip it.
//!
//! Apache-2.0 models (Qwen3) only get a notice: [`Licenca::exige_aceite`] is false for them.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A licence a model is under.
#[derive(Debug, PartialEq, Eq)]
pub struct Licenca {
    /// stable id of the licence (what the acceptance records)
    pub id: &'static str,
    pub nome: &'static str,
    /// how the screens call the models under it ("LFM3")
    pub familia: &'static str,
    /// the version the person accepted; a new version asks again
    pub versao: &'static str,
    /// SPDX id, or the `LicenseRef-` of a custom licence
    pub spdx: &'static str,
    /// false: only a notice (a licence on the project's accepted list)
    pub exige_aceite: bool,
    /// the full text under `/usr/share/doc/kalango/modelos/` ("" = not shipped: only the link)
    pub arquivo: &'static str,
    /// the official source of the text
    pub url: &'static str,
}

pub const LFM: Licenca = Licenca {
    id: "lfm-open",
    nome: "LFM Open License v1.0",
    familia: "LFM3",
    versao: "1.0",
    spdx: "LicenseRef-LFM-Open-1.0",
    exige_aceite: true,
    arquivo: "LFM-Open-1.0.txt",
    url: "https://huggingface.co/LiquidAI/LFM3-1.7B-GGUF/blob/main/LICENSE",
};
pub const APACHE_QWEN: Licenca = Licenca {
    id: "apache-2.0",
    nome: "Apache License 2.0",
    familia: "Qwen3",
    versao: "2.0",
    spdx: "Apache-2.0",
    exige_aceite: false,
    arquivo: "Qwen-Apache-2.0.txt",
    url: "https://huggingface.co/Qwen/Qwen3-8B-GGUF/blob/main/LICENSE",
};
pub const LLAMA: Licenca = Licenca {
    id: "llama-3.1",
    nome: "Llama 3.1 Community License",
    familia: "Llama 3.1",
    versao: "3.1",
    spdx: "LicenseRef-Llama-3.1",
    exige_aceite: true,
    arquivo: "",
    url: "https://www.llama.com/llama3_1/license/",
};
pub const GEMMA: Licenca = Licenca {
    id: "gemma",
    nome: "Gemma Terms of Use",
    familia: "Gemma 3",
    versao: "2026-04-01",
    spdx: "LicenseRef-Gemma",
    exige_aceite: true,
    arquivo: "",
    url: "https://ai.google.dev/gemma/terms",
};

/// The licence of a catalog model; `None` for a model that is not one of the catalog's (a private server's own).
pub fn licenca_do_modelo(id: &str) -> Option<&'static Licenca> {
    if id.starts_with("lfm3-") || id.starts_with("d1-") {
        Some(&LFM)
    } else if id.starts_with("qwen3-") {
        Some(&APACHE_QWEN)
    } else if id.starts_with("llama-3.1") {
        Some(&LLAMA)
    } else if id.starts_with("gemma-3") {
        Some(&GEMMA)
    } else {
        None
    }
}

/// One acceptance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Aceite {
    pub modelo: String,
    pub licenca: String,
    pub versao: String,
    /// `AAAA-MM-DD`
    pub data: String,
}

#[derive(Default, Serialize, Deserialize)]
struct Arquivo {
    #[serde(default)]
    aceites: Vec<Aceite>,
}

/// The acceptances, read from a file (a missing or broken file counts as none: nothing is accepted by accident).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Aceites {
    pub lista: Vec<Aceite>,
}

/// The person's file: `$XDG_CONFIG_HOME/kalango/modelos-licencas.json`.
pub fn aceites_path() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).filter(|p| p.is_absolute())
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config")).join("kalango/modelos-licencas.json")
}

impl Aceites {
    pub fn carregar(path: &Path) -> Aceites {
        let lista = std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str::<Arquivo>(&t).ok()).map(|a| a.aceites).unwrap_or_default();
        Aceites { lista }
    }

    /// Is `modelo` accepted under the licence version in force?
    pub fn aceito(&self, modelo: &str) -> bool {
        let Some(l) = licenca_do_modelo(modelo) else { return true };
        !l.exige_aceite || self.registro(modelo).is_some()
    }

    /// The acceptance of `modelo` under the licence and version in force.
    pub fn registro(&self, modelo: &str) -> Option<&Aceite> {
        let l = licenca_do_modelo(modelo)?;
        self.lista.iter().find(|a| a.modelo == modelo && a.licenca == l.id && a.versao == l.versao)
    }

    /// Records the acceptance of `modelo` (replacing an older one of the same licence). Nothing is recorded for a
    /// model that needs none.
    pub fn aceitar(&mut self, modelo: &str, data: &str) {
        let Some(l) = licenca_do_modelo(modelo) else { return };
        if !l.exige_aceite {
            return;
        }
        self.lista.retain(|a| !(a.modelo == modelo && a.licenca == l.id));
        self.lista.push(Aceite { modelo: modelo.to_string(), licenca: l.id.to_string(), versao: l.versao.to_string(), data: data.to_string() });
    }

    /// Revokes the acceptance of every model under the same licence as `modelo` (the licence is one: LFM3's models share it).
    pub fn revogar(&mut self, modelo: &str) {
        let Some(l) = licenca_do_modelo(modelo) else { return };
        self.lista.retain(|a| a.licenca != l.id);
    }

    pub fn salvar(&self, path: &Path) -> Result<(), String> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        let t = serde_json::to_string_pretty(&Arquivo { aceites: self.lista.clone() }).map_err(|e| e.to_string())?;
        // (written aside and renamed: a reader never sees half a file)
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, t).map_err(|e| format!("{}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, path).map_err(|e| e.to_string())
    }
}

/// Accepts `modelo` on disk (reads, adds, writes).
pub fn aceitar_em(path: &Path, modelo: &str, data: &str) -> Result<(), String> {
    let mut a = Aceites::carregar(path);
    a.aceitar(modelo, data);
    a.salvar(path)
}

/// Revokes on disk the acceptance of the licence `modelo` is under.
pub fn revogar_em(path: &Path, modelo: &str) -> Result<(), String> {
    let mut a = Aceites::carregar(path);
    a.revogar(modelo);
    a.salvar(path)
}

/// Does the download of `modelo` need an acceptance the person has not given?
pub fn falta_aceite(path: &Path, modelo: &str) -> bool {
    !Aceites::carregar(path).aceito(modelo)
}

/// Today as `AAAA-MM-DD` (UTC).
pub fn hoje() -> String {
    let dias = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| (d.as_secs() / 86_400) as i64).unwrap_or(0);
    data_de_dias(dias)
}

/// The civil date of a day count since 1970-01-01 (Howard Hinnant's algorithm).
pub fn data_de_dias(dias: i64) -> String {
    let z = dias + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{:04}-{:02}-{:02}", if m <= 2 { y + 1 } else { y }, m, d)
}

/// The full text of a licence, when it is shipped: `/usr/share/doc/kalango/modelos/`, or the session's own tree.
pub fn texto_completo(l: &Licenca) -> Option<String> {
    if l.arquivo.is_empty() {
        return None;
    }
    ["/usr/share/doc/kalango/modelos", "sessao/dist/share/doc/kalango/modelos"].iter().find_map(|d| std::fs::read_to_string(Path::new(d).join(l.arquivo)).ok())
}

/// The words of the acceptance screens, shared by the installer and the shell (English source, pt-BR in the catalog).
/// Design: kalangoos/design, page "35 · Licença do modelo (LFM3)".
pub mod textos {
    use super::*;
    use kalango_i18n::{tr, trf};

    /// What a line of the summary is: a freedom, a limit, or a note.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Marca {
        Ok,
        Aviso,
        Info,
    }

    /// The plain-language summary, line by line (the full text prevails: the last line says so).
    pub fn topicos(l: &Licenca) -> Vec<(Marca, &'static str)> {
        let fim = (Marca::Info, tr("The full text of the licence is what counts; this summary only helps you read it."));
        match l.id {
            "lfm-open" => vec![
                (Marca::Ok, tr("You can use and share the model, including redistributing it.")),
                (Marca::Ok, tr("Personal use, study and non-commercial projects: allowed.")),
                (Marca::Aviso, tr("Commercial use only for companies with annual revenue below US$ 10 million.")),
                fim,
            ],
            "llama-3.1" => vec![
                (Marca::Ok, tr("You can use and share the model, keeping the \"Built with Llama\" notice.")),
                (Marca::Aviso, tr("Organizations with more than 700 million monthly users need a licence from Meta.")),
                (Marca::Aviso, tr("The use must follow Meta's acceptable use policy.")),
                fim,
            ],
            "gemma" => vec![
                (Marca::Ok, tr("You can use and share the model, following Google's prohibited use policy.")),
                (Marca::Aviso, tr("Google may restrict a use that breaks that policy.")),
                fim,
            ],
            _ => vec![(Marca::Ok, tr("You can use, change and share the model, keeping its notices. Nothing to accept.")), fim],
        }
    }

    pub fn titulo(l: &Licenca) -> String {
        trf("{family} licence · {licence}", &[("family", l.familia.to_string()), ("licence", l.nome.to_string())])
    }
    pub fn subtitulo() -> &'static str {
        tr("Summary in plain language")
    }
    pub fn ler() -> &'static str {
        tr("Read the full licence")
    }
    pub fn original(l: &Licenca) -> String {
        trf("original text (English) · v{version}", &[("version", l.versao.to_string())])
    }
    pub fn aceito(l: &Licenca) -> String {
        trf("I have read and accept the {family} licence", &[("family", l.familia.to_string())])
    }
    pub fn aceito_detalhe(l: &Licenca) -> String {
        trf("Required to download the {family} models · recorded only on this computer", &[("family", l.familia.to_string())])
    }
    pub fn pular() -> &'static str {
        tr("Skip: download later")
    }
    pub fn pular_detalhe() -> &'static str {
        tr("The system works without a model; the agents stay off until you download one in Settings › Models.")
    }
    pub fn folha_subtitulo(l: &Licenca) -> String {
        trf("Original text in English, shown as it is · version v{version} · source: the model's LICENSE file in the catalog", &[("version", l.versao.to_string())])
    }
    pub fn so_o_link(l: &Licenca) -> String {
        trf("The full text is not shipped with the system. Official source: {url}", &[("url", l.url.to_string())])
    }
    pub fn copiar() -> &'static str {
        tr("Copy text")
    }
    pub fn aceita_em(data: &str) -> String {
        trf("Licence accepted on {date}", &[("date", data.to_string())])
    }
    pub fn aceita_detalhe(l: &Licenca) -> String {
        trf("{licence} · accepted on this computer", &[("licence", l.nome.to_string())])
    }
    pub fn ver() -> &'static str {
        tr("view")
    }
    pub fn ver_licenca() -> &'static str {
        tr("View licence")
    }
    pub fn revogar_titulo() -> &'static str {
        tr("Revoke the acceptance")
    }
    pub fn revogar_texto(l: &Licenca) -> String {
        trf("To stop accepting the licence, the {family} models are removed from this computer. The agents that use them stay off until you pick another model (for example Qwen3, Apache-2.0) or accept it again.", &[("family", l.familia.to_string())])
    }
    pub fn revogar_botao(l: &Licenca) -> String {
        trf("Remove the {family} models and revoke", &[("family", l.familia.to_string())])
    }
    pub fn rodape_sem_modelo() -> &'static str {
        tr("Without a model the agents stay off; the rest of the system works normally.")
    }
    pub fn recusada() -> &'static str {
        tr("licence not accepted")
    }
    pub fn recusada_detalhe() -> &'static str {
        tr("The model's licence has not been accepted: accept it to download.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalogs_licences() {
        for id in ["lfm3-1.7b", "lfm3-audio-2b", "lfm3-vl-2b", "lfm3-omni-2b", "d1-2b", "d1-omni-2b"] {
            assert_eq!(licenca_do_modelo(id), Some(&LFM), "{id}");
        }
        assert!(!licenca_do_modelo("qwen3-8b").unwrap().exige_aceite);
        assert!(licenca_do_modelo("llama-3.1-8b").unwrap().exige_aceite);
        assert!(licenca_do_modelo("gemma-3-27b").unwrap().exige_aceite);
        assert_eq!(licenca_do_modelo("a-private-servers-own"), None);
    }

    #[test]
    fn the_dates() {
        assert_eq!(data_de_dias(0), "1970-01-01");
        assert_eq!(data_de_dias(20_453), "2025-12-31");
        assert_eq!(data_de_dias(20_454), "2026-01-01");
        assert_eq!(data_de_dias(19_782), "2024-02-29");
        assert_eq!(hoje().len(), 10);
    }

    #[test]
    fn an_acceptance_is_per_model_licence_and_version() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("kalango/modelos-licencas.json");
        assert!(falta_aceite(&f, "lfm3-1.7b"), "nothing accepted yet");
        aceitar_em(&f, "lfm3-1.7b", "2026-11-24").unwrap();
        assert!(!falta_aceite(&f, "lfm3-1.7b"));
        assert!(falta_aceite(&f, "lfm3-audio-2b"), "another model of the same licence is asked separately");
        let a = Aceites::carregar(&f);
        assert_eq!(a.registro("lfm3-1.7b").unwrap().data, "2026-11-24");
        // a licence version bump: the old record no longer counts
        let mut velho = a.clone();
        velho.lista[0].versao = "0.9".into();
        velho.salvar(&f).unwrap();
        assert!(falta_aceite(&f, "lfm3-1.7b"));
        // the models that need nothing and the ones that are not the catalog's never ask
        assert!(!falta_aceite(&f, "qwen3-8b") && !falta_aceite(&f, "x"));
        let mut b = Aceites::default();
        b.aceitar("qwen3-8b", "2026-11-24");
        assert!(b.lista.is_empty(), "an Apache-2.0 model records nothing");
        // a broken file accepts nothing
        std::fs::write(&f, "{ not json").unwrap();
        assert!(falta_aceite(&f, "lfm3-1.7b"));
    }

    #[test]
    fn revoking_asks_again_for_every_model_of_the_licence() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("l.json");
        aceitar_em(&f, "lfm3-1.7b", "2026-11-24").unwrap();
        aceitar_em(&f, "lfm3-vl-2b", "2026-11-24").unwrap();
        revogar_em(&f, "lfm3-1.7b").unwrap();
        assert!(falta_aceite(&f, "lfm3-1.7b") && falta_aceite(&f, "lfm3-vl-2b"));
        assert!(Aceites::carregar(&f).lista.is_empty());
    }

    #[test]
    fn the_shipped_texts_are_the_ones_the_licences_name() {
        let raiz = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sessao/dist/share/doc/kalango/modelos");
        let lfm = std::fs::read_to_string(raiz.join(LFM.arquivo)).unwrap();
        assert!(lfm.starts_with("LFM Open License v1.0"), "{}", &lfm[..40]);
        let ap = std::fs::read_to_string(raiz.join(APACHE_QWEN.arquivo)).unwrap();
        assert!(ap.contains("Apache License") && ap.contains("Version 2.0"));
    }
}
