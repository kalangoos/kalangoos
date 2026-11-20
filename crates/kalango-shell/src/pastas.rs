//! The person's folders, in one place: in a session test (`KALANGO_AREA_TESTE`) they are the test area's, never the
//! real home — the test's HOME is the person's, only the XDG folders are its own, so `glib::home_dir()` and a missing
//! `user-dirs.dirs` would lead an app straight into the person's Documents.

use gtk::glib;
use std::path::PathBuf;

/// A personal folder (Documents, Downloads…): the system's when it says, else `nome` in the home.
pub fn especial(d: glib::UserDirectory, nome: &str) -> PathBuf {
    if let Some(a) = std::env::var_os("KALANGO_AREA_TESTE") {
        return PathBuf::from(a).join(nome);
    }
    let home = glib::home_dir();
    glib::user_special_dir(d).filter(|p| *p != home).unwrap_or_else(|| home.join(nome))
}

pub fn documentos() -> PathBuf {
    especial(glib::UserDirectory::Documents, "Documentos")
}

/// The Desktop folder: what is in it shows on the desktop (`desktop_files`).
pub fn desktop() -> PathBuf {
    especial(glib::UserDirectory::Desktop, "Área de trabalho")
}

pub fn downloads() -> PathBuf {
    especial(glib::UserDirectory::Downloads, "Downloads")
}

/// The person's home — the test area in a session test.
pub fn pessoal() -> PathBuf {
    std::env::var_os("KALANGO_AREA_TESTE").map(PathBuf::from).unwrap_or_else(glib::home_dir)
}

/// A folder as the agent says it ("~/Downloads", "~/Imagens"…): the personal folders by their place in the system,
/// anything else under the home (the test area in a test).
pub fn resolver(caminho: &str) -> PathBuf {
    let c = caminho.trim();
    let Some(resto) = c.strip_prefix("~/").or_else(|| (c == "~").then_some("")) else { return PathBuf::from(c) };
    let (primeira, depois) = resto.split_once('/').unwrap_or((resto, ""));
    let base = match primeira.to_lowercase().as_str() {
        "downloads" | "transferências" | "transferencias" => downloads(),
        "documentos" | "documents" => documentos(),
        "imagens" | "pictures" | "fotos" => especial(glib::UserDirectory::Pictures, "Imagens"),
        "músicas" | "musicas" | "música" | "musica" | "music" => especial(glib::UserDirectory::Music, "Músicas"),
        "vídeos" | "videos" => especial(glib::UserDirectory::Videos, "Vídeos"),
        "área de trabalho" | "area de trabalho" | "desktop" => desktop(),
        "" => return pessoal(),
        _ => pessoal().join(primeira),
    };
    if depois.is_empty() { base } else { base.join(depois) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_test_never_gets_the_real_home() {
        std::env::set_var("KALANGO_AREA_TESTE", "/tmp/area-de-teste");
        assert_eq!(documentos(), PathBuf::from("/tmp/area-de-teste/Documentos"));
        assert_eq!(downloads(), PathBuf::from("/tmp/area-de-teste/Downloads"));
        assert_eq!(desktop(), PathBuf::from("/tmp/area-de-teste/Área de trabalho"));
        assert_eq!(resolver("~/Downloads"), PathBuf::from("/tmp/area-de-teste/Downloads"));
        assert_eq!(resolver("~/Downloads/instaladores"), PathBuf::from("/tmp/area-de-teste/Downloads/instaladores"));
        assert_eq!(resolver("~/projetos"), PathBuf::from("/tmp/area-de-teste/projetos"));
        assert_eq!(resolver("~"), PathBuf::from("/tmp/area-de-teste"));
        assert_eq!(resolver("/srv/x"), PathBuf::from("/srv/x"));
        std::env::remove_var("KALANGO_AREA_TESTE");
    }
}
