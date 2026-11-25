//! O catálogo dos Apps criativos, embutido no binário (o mesmo no shell e no kalangod): ninguém o troca por um arquivo
//! da pasta da pessoa. `KALANGO_CRAFT_CATALOGO` (testes) lê outro arquivo.

use serde::Deserialize;
use std::sync::OnceLock;

const CATALOGO: &str = include_str!("../catalogo/apps-criativos.json");
/// A chave pública do projeto (OpenPGP, binária), a mesma do `GPGKey` de getartcraft.com/artcraft-oci.flatpakrepo e
/// a fixada no código do próprio launcher; vale só se a impressão digital do catálogo bater (`verificar`).
pub const CHAVE: &[u8] = include_bytes!("../catalogo/artcraft-release.gpg");

#[derive(Clone, Debug, Deserialize)]
pub struct Fonte {
    pub indice: String,
    pub assinaturas: String,
    pub registro: String,
    pub impressao_digital: String,
    pub canal: String,
}

/// Uma versão que o KalangoOS bloqueia: pela versão, pelo digest da imagem ou pelos dois.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct Bloqueio {
    #[serde(default)]
    pub versao: String,
    #[serde(default)]
    pub digest: String,
    #[serde(default)]
    pub motivo: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct ComandosLivres {
    /// a versão do app em que a lista foi conferida
    #[serde(default)]
    pub versao: String,
    #[serde(default)]
    pub ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct App {
    pub id: String,
    pub repositorio: String,
    pub nome: String,
    pub categoria: String,
    pub descricao: String,
    #[serde(default)]
    pub sobre: String,
    pub tamanho_mb: u64,
    pub licenca: String,
    pub cli: String,
    /// o `mcp` aceita `--automation-read-root` / `--automation-write-root`
    #[serde(default)]
    pub raizes: bool,
    /// a opção do `mcp` que liga a uma janela aberta com `--control <porta>` (`--bridge`, `--connect`, `--attach`)
    #[serde(default)]
    pub janela: Option<String>,
    /// a variável com a ficha de controle que o app e o `mcp` leem
    #[serde(default)]
    pub token: Option<String>,
    /// as pastas que o app vê por padrão (nomes do Flatpak: xdg-pictures…); a pessoa muda por app
    #[serde(default)]
    pub pastas: Vec<String>,
    #[serde(default)]
    pub bloqueadas: Vec<Bloqueio>,
    /// os ids de comando que só editam o documento do servidor (sem arquivo, rede nem código), conferidos na lista de
    /// comandos do app na versão indicada; qualquer outro id pedido a um executor pede aprovação
    #[serde(default)]
    pub comandos_livres: ComandosLivres,
    /// toda ferramenta pede aprovação (um app que executa programas para investigá-los)
    #[serde(default)]
    pub somente_aprovar: bool,
}

impl App {
    /// O nome da imagem no registro ("filmcraft" de storytold/filmcraft).
    pub fn imagem(&self) -> &str {
        self.repositorio.rsplit('/').next().unwrap_or(&self.repositorio)
    }

    /// As iniciais do marcador neutro (os ícones definitivos chegam com o pacote): "FC", "CA".
    pub fn iniciais(&self) -> String {
        let mut s: String = self.nome.split_whitespace().filter_map(|w| w.chars().next()).collect();
        if s.chars().count() < 2 {
            // "FilmCraft" → F + C, "CADCraft" → C + C: a primeira letra e a do "Craft"
            s = self.nome.chars().take(1).chain(self.nome.strip_suffix("Craft").map(|_| 'C')).collect();
        }
        s.to_uppercase()
    }

    /// O bloqueio que pega esta versão (ou este digest), se algum.
    pub fn bloqueio(&self, versao: &str, digest: &str) -> Option<&Bloqueio> {
        self.bloqueadas.iter().find(|b| (!b.versao.is_empty() && b.versao == versao) || (!b.digest.is_empty() && b.digest == digest))
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Ferramentas {
    pub livre: Vec<String>,
    /// os executores de comandos (`command_run`, `batch`…): o nível vem de cada id de comando pedido
    pub executores: Vec<String>,
    pub edicao: Vec<String>,
    pub confirmar: Vec<String>,
    pub aprovar: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Catalogo {
    pub versao: u32,
    pub fonte: Fonte,
    /// (id, rótulo em inglês para o tr())
    pub categorias: Vec<(String, String)>,
    pub ferramentas: Ferramentas,
    pub apps: Vec<App>,
}

impl Catalogo {
    pub fn app(&self, id: &str) -> Option<&App> {
        self.apps.iter().find(|a| a.id == id || a.imagem() == id || a.nome.eq_ignore_ascii_case(id))
    }

    pub fn ler(texto: &str) -> Result<Catalogo, String> {
        serde_json::from_str(texto).map_err(|e| format!("catálogo dos Apps criativos: {e}"))
    }
}

/// O catálogo (lido uma vez).
pub fn catalogo() -> &'static Catalogo {
    static C: OnceLock<Catalogo> = OnceLock::new();
    C.get_or_init(|| {
        if let Some(p) = std::env::var_os("KALANGO_CRAFT_CATALOGO") {
            if let Ok(t) = std::fs::read_to_string(p) {
                if let Ok(c) = Catalogo::ler(&t) {
                    return c;
                }
            }
        }
        Catalogo::ler(CATALOGO).expect("catálogo embutido válido")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalog_has_the_seventeen_apps_with_their_categories() {
        let c = Catalogo::ler(CATALOGO).unwrap();
        assert_eq!(c.apps.len(), 17);
        let count = |k: &str| c.apps.iter().filter(|a| a.categoria == k).count();
        assert_eq!((count("imagem"), count("video"), count("audio"), count("documentos"), count("3d"), count("ferramentas")), (5, 3, 1, 4, 3, 1));
        for a in &c.apps {
            assert!(a.id.starts_with("ai.storyteller."), "{}", a.id);
            assert!(c.categorias.iter().any(|(k, _)| *k == a.categoria), "{}", a.categoria);
            assert!(!a.descricao.is_empty() && a.tamanho_mb > 0);
            assert_eq!(a.licenca, "MIT OR Apache-2.0");
        }
        // ids únicos
        let mut ids: Vec<_> = c.apps.iter().map(|a| a.id.clone()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 17);
    }

    #[test]
    fn the_key_fingerprint_is_pinned() {
        let c = Catalogo::ler(CATALOGO).unwrap();
        assert_eq!(c.fonte.impressao_digital, "2B76741B414AAD6CD33E631213A005C0B9D7C3ED");
        assert!(CHAVE.len() > 100);
    }

    #[test]
    fn initials_and_blocks() {
        let c = Catalogo::ler(CATALOGO).unwrap();
        assert_eq!(c.app("filmcraft").unwrap().iniciais(), "FC");
        assert_eq!(c.app("ai.storyteller.craft-anything").unwrap().iniciais(), "CA");
        assert_eq!(c.app("CADCraft").unwrap().iniciais(), "CC");
        let mut a = c.app("effectcraft").unwrap().clone();
        a.bloqueadas.push(Bloqueio { versao: "0.25.1".into(), digest: String::new(), motivo: "falha ao exportar".into() });
        assert!(a.bloqueio("0.25.1", "sha256:x").is_some());
        assert!(a.bloqueio("0.25.2", "sha256:x").is_none());
    }
}
