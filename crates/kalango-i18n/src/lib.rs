//! KalangoOS · UI translations.
//!
//! Source strings are English and double as message ids: `tr("Search apps and actions…")`. Each locale has a
//! catalog (`locales/<locale>.json`, English → translation) embedded in the binary. The language is the one chosen
//! in the system (`LANGUAGE`, `LC_ALL`, `LC_MESSAGES`, `LANG`, in that order); a missing language or string falls
//! back to English.
//!
//! Placeholders are named: `trf("{n} requests", &[("n", n.to_string())])`.

use std::collections::HashMap;
use std::sync::OnceLock;

/// (locale, catalog) pairs embedded at build time.
const CATALOGS: &[(&str, &str)] = &[("pt_BR", include_str!("../locales/pt_BR.json"))];

static ACTIVE: OnceLock<HashMap<String, String>> = OnceLock::new();

/// Normalizes a POSIX locale (`pt_BR.UTF-8@euro`) to a catalog name; `pt` alone maps to `pt_BR`.
pub fn normalize(locale: &str) -> Option<&'static str> {
    let base = locale.split(['.', '@']).next().unwrap_or("");
    if base.is_empty() || base == "C" || base == "POSIX" {
        return None;
    }
    CATALOGS.iter().map(|(n, _)| *n).find(|n| *n == base).or_else(|| {
        let lang = base.split('_').next().unwrap_or("");
        CATALOGS.iter().map(|(n, _)| *n).find(|n| n.split('_').next() == Some(lang))
    })
}

/// The catalog chosen by the system environment (or None for English).
pub fn system_locale() -> Option<&'static str> {
    if let Ok(list) = std::env::var("LANGUAGE") {
        if let Some(l) = list.split(':').find_map(normalize) {
            return Some(l);
        }
    }
    for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(v) = std::env::var(var) {
            if !v.is_empty() {
                return normalize(&v);
            }
        }
    }
    None
}

pub fn catalog(locale: Option<&str>) -> HashMap<String, String> {
    locale
        .and_then(|l| CATALOGS.iter().find(|(n, _)| *n == l))
        .and_then(|(_, json)| serde_json::from_str(json).ok())
        .unwrap_or_default()
}

fn active() -> &'static HashMap<String, String> {
    ACTIVE.get_or_init(|| catalog(system_locale()))
}

/// Marks a string for translation without translating it yet (for `const` tables); translate with [`tr`] when
/// shown. Same role as gettext's `N_()`.
pub const fn n_(msgid: &'static str) -> &'static str {
    msgid
}

/// Translates a UI string (English in the source).
pub fn tr(msgid: &'static str) -> &'static str {
    match active().get(msgid) {
        Some(t) => t.as_str(),
        None => msgid,
    }
}

/// Translates and fills named placeholders (`{name}`).
pub fn trf(msgid: &'static str, args: &[(&str, String)]) -> String {
    let mut s = tr(msgid).to_string();
    for (k, v) in args {
        s = s.replace(&format!("{{{k}}}"), v);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::path::Path;

    #[test]
    fn locale_detection() {
        assert_eq!(normalize("pt_BR.UTF-8"), Some("pt_BR"));
        assert_eq!(normalize("pt_PT.UTF-8"), Some("pt_BR"), "same language falls to the closest catalog");
        assert_eq!(normalize("en_US.UTF-8"), None);
        assert_eq!(normalize("C.UTF-8"), None);
    }

    /// Every string marked for translation in the UI crates has a pt_BR entry, and no entry is left unused.
    #[test]
    fn catalog_is_complete_and_has_no_leftovers() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut used = BTreeSet::new();
        let mut files = Vec::new();
        let mut dirs: Vec<_> = ["kalango-shell", "kalango-aprovacao", "kalangod", "kalango-agente", "kalango-entrada", "kalango-instalador"].iter().map(|k| root.join(k).join("src")).collect();
        while let Some(d) = dirs.pop() {
            for e in std::fs::read_dir(d).unwrap().flatten() {
                if e.path().is_dir() { dirs.push(e.path()) } else if e.path().extension().is_some_and(|x| x == "rs") { files.push(e.path()) }
            }
        }
        {
            for path in files {
                let text = std::fs::read_to_string(&path).unwrap();
                for marker in ["tr(\"", "trf(\"", "n_(\""] {
                    let mut rest = text.as_str();
                    while let Some(i) = rest.find(marker) {
                        // (only the function itself: "push_str(\"" or "toml_str(\"" also end in tr(")
                        let call = i == 0 || !rest.as_bytes()[i - 1].is_ascii_alphanumeric() && rest.as_bytes()[i - 1] != b'_';
                        rest = &rest[i + marker.len()..];
                        if !call {
                            continue;
                        }
                        // the msgid ends at the first unescaped quote
                        let mut end = 0;
                        let bytes = rest.as_bytes();
                        while end < bytes.len() && !(bytes[end] == b'"' && (end == 0 || bytes[end - 1] != b'\\')) {
                            end += 1;
                        }
                        used.insert(rest[..end].replace("\\\"", "\""));
                    }
                }
            }
        }
        let pt = catalog(Some("pt_BR"));
        let missing: Vec<_> = used.iter().filter(|m| !pt.contains_key(*m)).collect();
        let unused: Vec<_> = pt.keys().filter(|k| !used.contains(*k)).collect();
        assert!(missing.is_empty(), "sem tradução pt_BR: {missing:#?}");
        assert!(unused.is_empty(), "traduções sem uso: {unused:#?}");
        // placeholders survive translation
        for (k, v) in &pt {
            for part in k.split('{').skip(1) {
                let name = part.split('}').next().unwrap();
                assert!(v.contains(&format!("{{{name}}}")), "'{k}' → '{v}' perdeu {{{name}}}");
            }
        }
    }
}
