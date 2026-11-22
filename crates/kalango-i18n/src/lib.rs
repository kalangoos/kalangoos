//! KalangoOS · UI translations.
//!
//! Source strings are English and double as message ids: `tr("Search apps and actions…")`. Each locale has a
//! catalog (`locales/<locale>.json`, English → translation) embedded in the binary. The language is chosen by
//! `KALANGO_UI_LANG` (override for tests and agents) or the system (`LANGUAGE`, `LC_ALL`, `LC_MESSAGES`, `LANG`,
//! in that order). A missing language or string falls back to English (the msgid itself).
//!
//! Placeholders are named: `trf("{n} requests", &[("n", n.to_string())])`.
//!
//! See `docs/I18N.md` for the global language strategy (catalogs, Weblate, image fonts/IME).

use std::collections::HashMap;
use std::sync::OnceLock;

/// (locale, catalog) pairs embedded at build time. English is never listed: the msgid is the English string.
const CATALOGS: &[(&str, &str)] = &[("pt_BR", include_str!("../locales/pt_BR.json"))];

static ACTIVE: OnceLock<HashMap<String, String>> = OnceLock::new();
static ACTIVE_NAME: OnceLock<Option<&'static str>> = OnceLock::new();

/// Catalog names shipped in this binary (never includes English — English is the msgid).
pub fn available_catalogs() -> &'static [&'static str] {
    // Keep order stable for UI / docs; today only pt_BR.
    static NAMES: OnceLock<Vec<&'static str>> = OnceLock::new();
    NAMES.get_or_init(|| CATALOGS.iter().map(|(n, _)| *n).collect()).as_slice()
}

/// Whether this binary embeds a UI catalog for `locale` (after [`normalize`]).
pub fn has_ui_catalog(locale: &str) -> bool {
    normalize(locale).is_some()
}

/// Normalizes a POSIX locale (`pt_BR.UTF-8@euro`) to a catalog name; `pt` alone maps to `pt_BR`.
pub fn normalize(locale: &str) -> Option<&'static str> {
    let base = locale.split(['.', '@']).next().unwrap_or("");
    if base.is_empty() || base == "C" || base == "POSIX" {
        return None;
    }
    // Explicit English → no catalog (msgid is already English).
    let lang = base.split('_').next().unwrap_or("");
    if lang.eq_ignore_ascii_case("en") {
        return None;
    }
    CATALOGS.iter().map(|(n, _)| *n).find(|n| *n == base).or_else(|| {
        CATALOGS.iter().map(|(n, _)| *n).find(|n| n.split('_').next() == Some(lang))
    })
}

/// Override from the environment (`KALANGO_UI_LANG`), if set and non-empty.
///
/// Must be read before the first [`tr`] / [`trf`] call: the active catalog is locked for the process lifetime
/// (language changes in Settings apply after the next sign-in).
fn override_locale() -> Option<&'static str> {
    match std::env::var("KALANGO_UI_LANG") {
        Ok(v) if !v.trim().is_empty() => normalize(v.trim()),
        _ => None,
    }
}

/// The catalog chosen by override or the system environment (or None for English).
pub fn system_locale() -> Option<&'static str> {
    if let Some(l) = override_locale() {
        return Some(l);
    }
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

/// Catalog name currently active for [`tr`] / [`trf`], or `None` when falling back to English msgids.
pub fn active_locale() -> Option<&'static str> {
    *ACTIVE_NAME.get_or_init(system_locale)
}

pub fn catalog(locale: Option<&str>) -> HashMap<String, String> {
    locale
        .and_then(|l| CATALOGS.iter().find(|(n, _)| *n == l))
        .and_then(|(_, json)| serde_json::from_str(json).ok())
        .unwrap_or_default()
}

fn active() -> &'static HashMap<String, String> {
    ACTIVE.get_or_init(|| {
        let name = system_locale();
        let _ = ACTIVE_NAME.set(name);
        catalog(name)
    })
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
        assert_eq!(normalize("en"), None);
        assert_eq!(normalize("C.UTF-8"), None);
        assert_eq!(normalize("es_ES.UTF-8"), None, "no Spanish catalog yet");
    }

    #[test]
    fn catalogs_list_ships_pt_br_only_today() {
        let names = available_catalogs();
        assert!(names.contains(&"pt_BR"));
        assert!(!names.iter().any(|n| n.starts_with("en")), "English is the msgid, not a catalog");
        assert!(has_ui_catalog("pt_BR.UTF-8"));
        assert!(!has_ui_catalog("en_US.UTF-8"));
        assert!(!has_ui_catalog("ja_JP.UTF-8"));
    }

    #[test]
    fn english_fallback_returns_msgid() {
        // Empty catalog (English path): tr identity.
        let empty = catalog(None);
        assert!(empty.is_empty());
        assert_eq!(empty.get("Search"), None);
    }

    /// Every string marked for translation in the UI crates has a pt_BR entry, and no entry is left unused.
    #[test]
    fn catalog_is_complete_and_has_no_leftovers() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut used = BTreeSet::new();
        let mut files = Vec::new();
        let mut dirs: Vec<_> = ["kalango-shell", "kalango-aprovacao", "kalangod", "kalango-agente", "kalango-entrada", "kalango-instalador", "kalango-d1"].iter().map(|k| root.join(k).join("src")).collect();
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
