//! KalangoOS · UI translations.
//!
//! Source strings are English and double as message ids: `tr("Search apps and actions…")`. Each locale has a
//! catalog (`locales/<locale>.json`, English → translation) embedded in the binary. The language is the one chosen
//! in the system (`LANGUAGE`, `LC_ALL`, `LC_MESSAGES`, `LANG`, in that order); a missing language or string falls
//! back to English.
//!
//! Placeholders are named: `trf("{n} requests", &[("n", n.to_string())])`. Every catalog has the same keys as
//! `pt_BR.json` (the tests say which are missing; `scripts/i18n-faltando.py` lists and applies them).

use std::collections::HashMap;
use std::sync::OnceLock;

/// (locale, catalog) pairs embedded at build time. Catalogs are per language (`es` serves `es_ES`, `es_MX`…)
/// except Portuguese and Chinese, whose written forms differ by country.
const CATALOGS: &[(&str, &str)] = &[
    ("pt_BR", include_str!("../locales/pt_BR.json")),
    ("es", include_str!("../locales/es.json")),
    ("fr", include_str!("../locales/fr.json")),
    ("it", include_str!("../locales/it.json")),
    ("de", include_str!("../locales/de.json")),
    ("ja", include_str!("../locales/ja.json")),
    ("ru", include_str!("../locales/ru.json")),
    ("zh_CN", include_str!("../locales/zh_CN.json")),
];

/// The languages KalangoOS offers (the same as the site's), as the locale the system is set to and the language's
/// name in itself — the order the installer and Settings list them in.
pub const LANGUAGES: &[(&str, &str)] = &[
    ("en_US.UTF-8", "English (US)"),
    ("pt_BR.UTF-8", "Português (Brasil)"),
    ("es_ES.UTF-8", "Español"),
    ("fr_FR.UTF-8", "Français"),
    ("it_IT.UTF-8", "Italiano"),
    ("de_DE.UTF-8", "Deutsch"),
    ("ja_JP.UTF-8", "日本語"),
    ("ru_RU.UTF-8", "Русский"),
    ("zh_CN.UTF-8", "简体中文"),
];

/// Catalogs whose text for the AI model (system prompt, tool descriptions) was tested with the real model; see
/// [`tr_model`].
const MODEL_LOCALES: &[&str] = &["pt_BR"];

/// Catalogs a native speaker has reviewed. The others were translated by an AI and nobody who speaks the language
/// has read them yet (docs/i18n/REVISAO.md); Settings › Language says so.
const REVIEWED: &[&str] = &["pt_BR"];

static LOCALE: OnceLock<Option<&'static str>> = OnceLock::new();
static ACTIVE: OnceLock<HashMap<String, String>> = OnceLock::new();

fn names() -> impl Iterator<Item = &'static str> {
    CATALOGS.iter().map(|(n, _)| *n)
}

/// Normalizes a POSIX locale (`pt_BR.UTF-8@euro`) to a catalog name: `pt` alone maps to `pt_BR`, `es_MX` to `es`,
/// `zh_CN`/`zh_SG`/`zh_Hans` to `zh_CN`. Traditional Chinese (`zh_TW`, `zh_HK`, `zh_MO`, `zh_Hant`) has no catalog
/// yet: it falls back to English rather than to the simplified characters.
pub fn normalize(locale: &str) -> Option<&'static str> {
    let base = locale.split(['.', '@']).next().unwrap_or("").replace('-', "_");
    if base.is_empty() || base == "C" || base == "POSIX" {
        return None;
    }
    let mut parts = base.split('_');
    let lang = parts.next().unwrap_or("");
    if lang == "zh" {
        let traditional = parts.any(|p| matches!(p, "TW" | "HK" | "MO" | "Hant"));
        return (!traditional).then_some("zh_CN");
    }
    names().find(|n| *n == base).or_else(|| names().find(|n| n.split('_').next() == Some(lang)))
}

/// A language the system can be set to that has no catalog, so the screens stay in English (Traditional Chinese,
/// Korean…). English itself and `C` are not "without translation".
pub fn without_translation(locale: &str) -> bool {
    let lang = locale.split(['.', '@', '_', '-']).next().unwrap_or("");
    !(lang.is_empty() || lang == "C" || lang == "POSIX" || lang == "en") && normalize(locale).is_none()
}

/// Whether the screens in this locale are an AI translation that no native speaker has reviewed (a catalog exists
/// and is not in [`REVIEWED`]; English and the languages without a catalog are not).
pub fn machine_translated(locale: &str) -> bool {
    normalize(locale).is_some_and(|l| !REVIEWED.contains(&l))
}

/// The catalog chosen by an environment (`var` reads one variable), or None for English.
pub fn locale_from(var: impl Fn(&str) -> Option<String>) -> Option<&'static str> {
    if let Some(list) = var("LANGUAGE") {
        if let Some(l) = list.split(':').find_map(normalize) {
            return Some(l);
        }
    }
    for name in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Some(v) = var(name) {
            if !v.is_empty() {
                return normalize(&v);
            }
        }
    }
    None
}

/// The catalog chosen by the system environment (or None for English).
pub fn system_locale() -> Option<&'static str> {
    locale_from(|v| std::env::var(v).ok())
}

/// The catalog this process shows (read once, like the strings).
pub fn active_locale() -> Option<&'static str> {
    *LOCALE.get_or_init(system_locale)
}

pub fn catalog(locale: Option<&str>) -> HashMap<String, String> {
    locale
        .and_then(|l| CATALOGS.iter().find(|(n, _)| *n == l))
        .and_then(|(_, json)| serde_json::from_str(json).ok())
        .unwrap_or_default()
}

fn active() -> &'static HashMap<String, String> {
    ACTIVE.get_or_init(|| catalog(active_locale()))
}

/// A locale's name in its own language ("pt_BR.UTF-8" → "Português (Brasil)", "zh_TW" → "繁體中文（台灣）").
pub fn language_name(locale: &str) -> String {
    let code = locale.split(['.', '@']).next().unwrap_or(locale);
    if let Some((_, n)) = LANGUAGES.iter().find(|(l, _)| l.split('.').next() == Some(code)) {
        return n.to_string();
    }
    let (lang, region) = code.split_once('_').unwrap_or((code, ""));
    let name = match (lang, region) {
        ("zh", "TW") => return "繁體中文（台灣）".into(),
        ("zh", "HK") => return "繁體中文（香港）".into(),
        ("zh", "MO") => return "繁體中文（澳門）".into(),
        ("zh", "SG") => return "简体中文（新加坡）".into(),
        ("zh", _) => return "中文".into(),
        ("pt", _) => "Português",
        ("en", _) => "English",
        ("es", _) => "Español",
        ("fr", _) => "Français",
        ("de", _) => "Deutsch",
        ("it", _) => "Italiano",
        ("ja", _) => "日本語",
        ("ru", _) => "Русский",
        _ => lang,
    };
    let region_name = match (lang, region) {
        (_, "") => return name.to_string(),
        ("pt", "PT") => "Portugal",
        ("en", "GB") => "UK",
        ("en", "US") => "US",
        ("es", "ES") => "España",
        ("es", "MX") => "México",
        ("es", "AR") => "Argentina",
        ("fr", "FR") => "France",
        ("fr", "CA") => "Canada",
        ("fr", "BE") => "Belgique",
        ("fr", "CH") => "Suisse",
        ("de", "DE") => "Deutschland",
        ("de", "AT") => "Österreich",
        ("de", "CH") => "Schweiz",
        ("it", "IT") => "Italia",
        ("it", "CH") => "Svizzera",
        (_, r) => r,
    };
    format!("{name} ({region_name})")
}

/// The language list of the installer and of Settings: the nine KalangoOS languages in their order, then the other
/// UTF-8 locales the system has (`locale -a`, C excluded).
pub fn offered_languages(installed: &[String]) -> Vec<String> {
    let mut v: Vec<String> = LANGUAGES.iter().map(|(l, _)| l.to_string()).collect();
    let mut rest: Vec<String> = installed.iter().map(|l| l.replace(".utf8", ".UTF-8"))
        .filter(|l| !l.starts_with("C.") && !l.starts_with("POSIX") && !v.contains(l)).collect();
    rest.sort();
    rest.dedup();
    v.extend(rest);
    v
}

/// The language the agent answers in, named for the model ("Japanese (日本語)"); Portuguese in Portuguese, as the
/// Portuguese prompt says it.
pub fn answer_language(locale: &str) -> String {
    let code = locale.split(['.', '@']).next().unwrap_or(locale).replace('-', "_");
    let lang = code.split('_').next().unwrap_or("");
    match lang {
        "pt" => "português do Brasil".into(),
        "en" | "" | "C" => "English".into(),
        "es" => "Spanish (español)".into(),
        "fr" => "French (français)".into(),
        "it" => "Italian (italiano)".into(),
        "de" => "German (Deutsch)".into(),
        "ja" => "Japanese (日本語)".into(),
        "ru" => "Russian (русский)".into(),
        "zh" if normalize(&code).is_none() => "Traditional Chinese (繁體中文)".into(),
        "zh" => "Simplified Chinese (简体中文)".into(),
        _ => code,
    }
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

/// Text written for the AI model (the system prompt, tool descriptions): translated only in the languages whose
/// prompt was tested with the real model ([`MODEL_LOCALES`]); elsewhere the English source, which small models
/// follow best. The answer language is a rule of its own ([`answer_language`]).
pub fn tr_model(msgid: &'static str) -> &'static str {
    if active_locale().is_some_and(|l| MODEL_LOCALES.contains(&l)) {
        tr(msgid)
    } else {
        msgid
    }
}

/// A text already translated for the person (a tool description from kalangod, "Lê um arquivo [nível usuario; só
/// leitura]") as the model should get it: unchanged where [`tr_model`] translates, else back in the English source
/// — whole, or piece by piece for the "[contract; parts]" at the end. Pieces with no catalog entry stay as they are.
pub fn for_model(text: &str) -> String {
    let locale = active_locale();
    if locale.is_none() || locale.is_some_and(|l| MODEL_LOCALES.contains(&l)) {
        return text.to_string();
    }
    untranslate(active(), text)
}

fn untranslate(catalog: &HashMap<String, String>, text: &str) -> String {
    let back = |piece: &str| -> Option<String> {
        if let Some((k, _)) = catalog.iter().find(|(_, v)| v.as_str() == piece) {
            return Some(k.clone());
        }
        // a filled template: "レベル usuario" ← "level {level}"
        catalog.iter().filter(|(k, _)| k.contains('{')).find_map(|(k, v)| match_template(v, piece).map(|args| fill(k, &args.iter().map(|(n, x)| (n.as_str(), x.clone())).collect::<Vec<_>>())))
    };
    if let Some(t) = back(text) {
        return t;
    }
    match text.strip_suffix(']').and_then(|t| t.rsplit_once(" [")) {
        Some((desc, contract)) => {
            let desc = back(desc).unwrap_or_else(|| desc.to_string());
            let parts: Vec<String> = contract.split("; ").map(|p| back(p).unwrap_or_else(|| p.to_string())).collect();
            format!("{desc} [{}]", parts.join("; "))
        }
        None => text.to_string(),
    }
}

/// The placeholder values that make `template` ("Nível {level}") read `text`, if it does.
fn match_template(template: &str, text: &str) -> Option<Vec<(String, String)>> {
    let mut names = Vec::new();
    let mut literals = Vec::new();
    let mut rest = template;
    while let Some(i) = rest.find('{') {
        let j = rest[i..].find('}')? + i;
        literals.push(&rest[..i]);
        names.push(&rest[i + 1..j]);
        rest = &rest[j + 1..];
    }
    literals.push(rest);
    if names.is_empty() || literals.iter().all(|l| l.trim().is_empty()) {
        return None;
    }
    let mut s = text.strip_prefix(literals[0])?;
    let mut args = Vec::new();
    for (n, lit) in names.iter().zip(&literals[1..]) {
        let end = if lit.is_empty() { if args.len() + 1 == names.len() { s.len() } else { return None } } else { s.find(lit)? };
        let value = &s[..end];
        if value.is_empty() {
            return None;
        }
        args.push((n.to_string(), value.to_string()));
        s = &s[end + lit.len()..];
    }
    (s.is_empty()).then_some(args)
}

/// [`tr_model`] with named placeholders.
pub fn trf_model(msgid: &'static str, args: &[(&str, String)]) -> String {
    fill(tr_model(msgid), args)
}

/// Translates and fills named placeholders (`{name}`).
pub fn trf(msgid: &'static str, args: &[(&str, String)]) -> String {
    fill(tr(msgid), args)
}

fn fill(text: &str, args: &[(&str, String)]) -> String {
    let mut s = text.to_string();
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
        assert_eq!(normalize("pt"), Some("pt_BR"));
        assert_eq!(normalize("en_US.UTF-8"), None);
        assert_eq!(normalize("C.UTF-8"), None);
        for (l, c) in [("es_ES.UTF-8", "es"), ("es_MX.UTF-8", "es"), ("es", "es"), ("fr_FR.UTF-8", "fr"), ("fr_CA.UTF-8", "fr"),
            ("de_DE.UTF-8", "de"), ("de_AT.UTF-8@euro", "de"), ("de_CH", "de"), ("it_IT.UTF-8", "it"), ("it_CH", "it"),
            ("ja_JP.UTF-8", "ja"), ("ja", "ja"), ("ru_RU.UTF-8", "ru"), ("ru_UA.UTF-8", "ru"),
            ("zh_CN.UTF-8", "zh_CN"), ("zh_SG.UTF-8", "zh_CN"), ("zh_Hans", "zh_CN"), ("zh-Hans-CN", "zh_CN"), ("zh", "zh_CN")] {
            assert_eq!(normalize(l), Some(c), "{l}");
        }
        // Traditional Chinese has no catalog: English, not the simplified characters
        for l in ["zh_TW.UTF-8", "zh_HK.UTF-8", "zh_MO", "zh_Hant", "zh-Hant-TW"] {
            assert_eq!(normalize(l), None, "{l}");
            assert!(without_translation(l), "{l}");
        }
        assert!(without_translation("ko_KR.UTF-8"));
        assert!(!without_translation("en_GB.UTF-8") && !without_translation("C.UTF-8") && !without_translation("de_DE.UTF-8"));
    }

    #[test]
    fn only_portuguese_is_reviewed_by_native_speakers() {
        assert!(!machine_translated("pt_BR.UTF-8") && !machine_translated("pt_PT"));
        for l in ["es_ES.UTF-8", "fr_FR.UTF-8", "it_IT.UTF-8", "de_AT.UTF-8", "ja_JP.UTF-8", "ru_RU.UTF-8", "zh_CN.UTF-8"] {
            assert!(machine_translated(l), "{l}");
        }
        for l in ["en_US.UTF-8", "C.UTF-8", "zh_TW.UTF-8", "ko_KR.UTF-8"] {
            assert!(!machine_translated(l), "{l}: no catalog, nothing to warn about");
        }
    }

    #[test]
    fn the_environment_decides_in_gettext_order() {
        let env = |pairs: &'static [(&'static str, &'static str)]| move |v: &str| pairs.iter().find(|(k, _)| *k == v).map(|(_, x)| x.to_string());
        assert_eq!(locale_from(env(&[("LANG", "ja_JP.UTF-8")])), Some("ja"));
        assert_eq!(locale_from(env(&[("LANG", "pt_BR.UTF-8"), ("LC_MESSAGES", "de_DE.UTF-8")])), Some("de"));
        assert_eq!(locale_from(env(&[("LANG", "pt_BR.UTF-8"), ("LC_ALL", "C")])), None);
        // LANGUAGE is a list: the first language with a catalog wins (Traditional Chinese, then Simplified)
        assert_eq!(locale_from(env(&[("LANGUAGE", "zh_TW:zh_CN"), ("LANG", "zh_TW.UTF-8")])), Some("zh_CN"));
        assert_eq!(locale_from(env(&[("LANG", "zh_TW.UTF-8")])), None);
        assert_eq!(locale_from(env(&[("LANGUAGE", "ko:en"), ("LANG", "fr_FR.UTF-8")])), Some("fr"));
    }

    #[test]
    fn languages_have_their_own_names() {
        assert_eq!(LANGUAGES.len(), 9);
        for (l, _) in LANGUAGES {
            assert!(l.starts_with("en_") || normalize(l).is_some(), "{l} has no catalog");
        }
        assert_eq!(language_name("pt_BR.UTF-8"), "Português (Brasil)");
        assert_eq!(language_name("en_US.UTF-8"), "English (US)");
        assert_eq!(language_name("de_DE.UTF-8"), "Deutsch");
        assert_eq!(language_name("de_AT.UTF-8"), "Deutsch (Österreich)");
        assert_eq!(language_name("zh_CN.UTF-8"), "简体中文");
        assert_eq!(language_name("zh_TW.UTF-8"), "繁體中文（台灣）");
        assert_eq!(language_name("ja_JP"), "日本語");
        let offered = offered_languages(&["C.utf8".into(), "de_DE.utf8".into(), "zh_TW.UTF-8".into(), "ko_KR.utf8".into()]);
        assert_eq!(offered[..9].iter().map(|l| &l[..5]).collect::<Vec<_>>(), ["en_US", "pt_BR", "es_ES", "fr_FR", "it_IT", "de_DE", "ja_JP", "ru_RU", "zh_CN"]);
        assert_eq!(offered[9..], ["ko_KR.UTF-8".to_string(), "zh_TW.UTF-8".to_string()]);
        assert_eq!(answer_language("ja_JP.UTF-8"), "Japanese (日本語)");
        assert_eq!(answer_language("zh_TW"), "Traditional Chinese (繁體中文)");
        assert_eq!(answer_language("pt_BR"), "português do Brasil");
    }

    #[test]
    fn model_text_goes_back_to_english() {
        let c: HashMap<String, String> = [("Reads a file", "ファイルを読み取ります"), ("level {level}", "レベル {level}"), ("read only", "読み取りのみ"), ("{n} of {max}", "{max}件中{n}件")]
            .into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        assert_eq!(untranslate(&c, "ファイルを読み取ります [レベル usuario; 読み取りのみ]"), "Reads a file [level usuario; read only]");
        assert_eq!(untranslate(&c, "3件中1件"), "1 of 3");
        assert_eq!(untranslate(&c, "知らない文 [謎]"), "知らない文 [謎]");
        assert_eq!(match_template("{a}x{b}", "1x2"), Some(vec![("a".into(), "1".into()), ("b".into(), "2".into())]));
        assert_eq!(match_template("{a}{b}", "12"), None, "two placeholders with nothing between cannot be split");
    }

    /// The placeholders, markup, `<<<`/`>>>` markers and line breaks of a text, which a translation must keep.
    fn shape(s: &str) -> (Vec<String>, Vec<String>, usize, usize, (String, String)) {
        let pieces = |open: char, close: char| {
            let mut v: Vec<String> = s.split(open).skip(1).filter_map(|p| p.split_once(close).map(|(x, _)| x.to_string())).collect();
            v.sort();
            v
        };
        // Pango markup only: "<request>" in a usage line is a word to translate
        let markup = |t: &String| {
            let name = t.trim_start_matches('/').split([' ', '/']).next().unwrap_or("");
            ["b", "i", "u", "s", "tt", "big", "small", "sub", "sup", "span", "a", "br"].contains(&name)
        };
        let tags: Vec<String> = pieces('<', '>').into_iter().filter(markup).collect();
        let lead = s[..s.len() - s.trim_start().len()].to_string();
        let trail = s[s.trim_end().len()..].to_string();
        (pieces('{', '}'), tags, s.matches("<<<").count() + s.matches(">>>").count(), s.matches('\n').count(), (lead, trail))
    }

    /// Every catalog has exactly pt_BR's keys, keeps each text's placeholders, markup and outer spaces, and has no
    /// empty text. New UI strings need every catalog: `scripts/i18n-faltando.py` lists what is missing.
    #[test]
    fn every_catalog_has_the_same_keys_and_shapes() {
        let pt = catalog(Some("pt_BR"));
        assert!(pt.len() > 1000);
        let mut problems = Vec::new();
        for (name, _) in CATALOGS {
            let c = catalog(Some(name));
            assert!(!c.is_empty(), "{name}.json does not parse");
            let missing: BTreeSet<_> = pt.keys().filter(|k| !c.contains_key(*k)).collect();
            let extra: BTreeSet<_> = c.keys().filter(|k| !pt.contains_key(*k)).collect();
            if !missing.is_empty() {
                problems.push(format!("{name}.json: {} keys missing (python3 scripts/i18n-faltando.py lists them): {missing:#?}", missing.len()));
            }
            if !extra.is_empty() {
                problems.push(format!("{name}.json: keys not in pt_BR.json: {extra:#?}"));
            }
            for (k, v) in &c {
                if v.trim().is_empty() {
                    problems.push(format!("{name}.json: '{k}' is empty"));
                } else if shape(k) != shape(v) {
                    problems.push(format!("{name}.json: '{k}' → '{v}' changes placeholders, markup, line breaks or outer spaces"));
                }
            }
        }
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }

    /// The catalogs are written like pt_BR.json — one entry per line, one space of indentation, UTF-8 without
    /// escapes — and in its order, so a diff of one string is one line in every file.
    #[test]
    fn catalogs_keep_pt_br_layout_and_order() {
        let keys = |name: &str, json: &str| -> Vec<String> {
            let lines: Vec<&str> = json.lines().collect();
            assert_eq!(lines.first(), Some(&"{"), "{name}.json starts with a line of its own '{{'");
            assert_eq!(lines.last(), Some(&"}"), "{name}.json ends with a line of its own '}}'");
            assert!(json.ends_with("}\n"), "{name}.json ends with a newline");
            lines[1..lines.len() - 1].iter().enumerate().map(|(i, l)| {
                assert!(l.starts_with(" \"") && !l.starts_with("  "), "{name}.json line {}: one space of indentation", i + 2);
                let last = i == lines.len() - 3;
                assert_eq!(l.ends_with(','), !last, "{name}.json line {}: commas", i + 2);
                let entry: HashMap<String, String> = serde_json::from_str(&format!("{{{}}}", l.trim_end_matches(','))).unwrap_or_else(|e| panic!("{name}.json line {}: {e}", i + 2));
                let (k, v) = entry.into_iter().next().unwrap();
                assert_eq!(*l, format!(" {}: {}{}", serde_json::to_string(&k).unwrap(), serde_json::to_string(&v).unwrap(), if last { "" } else { "," }), "{name}.json line {}: written as serde_json/json.dumps(ensure_ascii=False) writes it", i + 2);
                k
            }).collect()
        };
        let pt = keys("pt_BR", CATALOGS[0].1);
        for (name, json) in &CATALOGS[1..] {
            let k = keys(name, json);
            if let Some(i) = (0..pt.len().min(k.len())).find(|&i| pt[i] != k[i]) {
                panic!("{name}.json line {} is '{}', pt_BR.json has '{}' there (python3 scripts/i18n-faltando.py --ordenar)", i + 2, k[i], pt[i]);
            }
        }
    }

    /// Each language loads and shows its own words for a few well-known strings.
    #[test]
    fn each_language_shows_its_own_text() {
        let expect = [
            ("pt_BR", "Settings", "Configurações"),
            ("es", "Settings", "Configuración"),
            ("fr", "Settings", "Paramètres"),
            ("it", "Settings", "Impostazioni"),
            ("de", "Settings", "Einstellungen"),
            ("ja", "Settings", "設定"),
            ("ru", "Settings", "Настройки"),
            ("zh_CN", "Settings", "设置"),
        ];
        for (l, k, v) in expect {
            assert_eq!(catalog(Some(l)).get(k).map(String::as_str), Some(v), "{l}");
        }
        for (name, _) in CATALOGS {
            let c = catalog(Some(name));
            for k in ["Timeline", "Undo", "Files", "Trash", "Language"] {
                let t = c.get(k).unwrap_or_else(|| panic!("{name}: no '{k}'"));
                assert!(name.starts_with("pt") || t != k, "{name}: '{k}' left in English");
            }
            // placeholders are filled the same way in every language
            let t = c.get("+ {n} networks nearby").map(|t| fill(t, &[("n", "3".into())])).unwrap_or_default();
            assert!(t.contains('3') && !t.contains("{n}"), "{name}: {t}");
        }
    }

    /// The permission dropdown of the conversation (free / ask / never) has keys of its own: "Free" is also the disk's
    /// free space, and a language that says "free of charge" or "available" there must not say it of a permission.
    #[test]
    fn permission_levels_have_their_own_keys() {
        let shell = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../kalango-shell/src/conversa.rs")).unwrap();
        for old in ["tr(\"Free\")", "tr(\"Ask\")", "tr(\"Never\")"] {
            assert!(!shell.contains(old), "conversa.rs: the permission dropdown must not use {old}");
        }
        for key in ["Permission: free", "Permission: ask", "Permission: never"] {
            assert!(shell.contains(&format!("tr(\"{key}\")")), "conversa.rs: no {key}");
            for (name, _) in CATALOGS {
                assert!(catalog(Some(name)).contains_key(key), "{name}: no '{key}'");
            }
        }
    }

    fn locale_names() -> impl Iterator<Item = &'static str> {
        CATALOGS.iter().map(|(n, _)| *n)
    }

    /// No value hides characters: controls other than the line break, zero-width and bidirectional marks (a "Deny"
    /// that renders as "Allow" with an override, an invisible character that makes two buttons look alike).
    fn hidden_char(c: char) -> bool {
        (c.is_control() && c != '\n') || ('\u{200B}'..='\u{200F}').contains(&c) || ('\u{202A}'..='\u{202E}').contains(&c)
            || ('\u{2066}'..='\u{2069}').contains(&c) || matches!(c, '\u{FEFF}' | '\u{2028}' | '\u{2029}' | '\u{00AD}')
    }

    #[test]
    fn no_value_has_hidden_or_bidirectional_characters() {
        assert!(hidden_char('\u{202E}') && hidden_char('\u{200B}') && hidden_char('\u{2066}') && hidden_char('\u{7}') && !hidden_char('\n') && !hidden_char('é'));
        let mut bad = Vec::new();
        for name in locale_names() {
            for (k, v) in catalog(Some(name)) {
                if let Some(c) = v.chars().chain(k.chars()).find(|c| hidden_char(*c)) {
                    bad.push(format!("{name}.json: '{k}' has U+{:04X}", c as u32));
                }
            }
        }
        assert!(bad.is_empty(), "{}", bad.join("\n"));
    }

    /// The words of an approval, reviewed by hand: (key, [pt_BR, es, fr, it, de, ja, ru, zh_CN]). A swapped Allow and
    /// Deny, or a Reject that says "accept", fails here. There is no bare "Approve" string; "approved" stands for it.
    /// "Never" and "Ask" are the permission levels' own keys (the bare ones are also used by other screens).
    const CONSENT_WORDS: &[(&str, [&str; 8])] = &[
        ("Allow", ["Permitir", "Permitir", "Autoriser", "Consenti", "Erlauben", "許可", "Разрешить", "允许"]),
        ("Allow once", ["Permitir uma vez", "Permitir una vez", "Autoriser une fois", "Consenti una volta", "Einmal erlauben", "1回だけ許可", "Разрешить один раз", "允许一次"]),
        ("Deny", ["Negar", "Denegar", "Refuser", "Nega", "Verweigern", "拒否", "Запретить", "拒绝"]),
        ("Reject", ["Rejeitar", "Rechazar", "Rejeter", "Rifiuta", "Verwerfen", "却下", "Отклонить", "拒绝"]),
        ("Always", ["Sempre", "Siempre", "Toujours", "Sempre", "Immer", "常に", "Всегда", "始终"]),
        ("Permission: free", ["Livre", "Libre", "Libre", "Libero", "Erlaubt", "自由", "Разрешено", "自由"]),
        ("Permission: ask", ["Perguntar", "Preguntar", "Demander", "Chiedere", "Fragen", "確認", "Спрашивать", "询问"]),
        ("Permission: never", ["Nunca", "Nunca", "Jamais", "Mai", "Nie", "許可しない", "Запрещено", "禁止"]),
        ("approved", ["aprovado", "aprobado", "approuvé", "approvato", "genehmigt", "承認済み", "одобрено", "已批准"]),
        ("Accept", ["Aceitar", "Aceptar", "Accepter", "Accetta", "Annehmen", "承諾", "Принять", "接受"]),
        ("Cancel", ["Cancelar", "Cancelar", "Annuler", "Annulla", "Abbrechen", "キャンセル", "Отмена", "取消"]),
    ];

    #[test]
    fn consent_words_say_what_the_reviewed_table_says() {
        let order: Vec<&str> = locale_names().collect();
        assert_eq!(order, ["pt_BR", "es", "fr", "it", "de", "ja", "ru", "zh_CN"], "the table's columns follow CATALOGS");
        for (key, expected) in CONSENT_WORDS {
            for (name, want) in order.iter().zip(expected) {
                assert_eq!(catalog(Some(name)).get(*key).map(String::as_str), Some(*want), "{name}: '{key}'");
            }
        }
        // whatever the words, in no language do two opposite answers read the same
        for name in &order {
            let c = catalog(Some(name));
            for (a, b) in [("Allow", "Deny"), ("Allow", "Reject"), ("Always", "Permission: never"), ("Permission: free", "Permission: never"), ("Allow once", "Allow")] {
                assert_ne!(c[a], c[b], "{name}: '{a}' and '{b}' read the same");
            }
        }
    }

    /// Values equal to their English key in a language that does not share the word: brands, protocols, key names
    /// and a few format strings. Anything else left in English is an untranslated string.
    const SAME_IN_EVERY_LANGUAGE: &[&str] = &["Alpha", "Bluetooth", "Caps Lock", "Esc", "Ethernet", "HTTP (streamable)", "OAuth", "Omni", "OpenAPI", "Shell",
        "Web", "dd/mm/yyyy", "kalangod", "omni", "{n} tok/s", "{n} tokens/s"];

    fn has_words(key: &str) -> bool {
        let mut plain = String::new();
        let mut depth = 0;
        for c in key.chars() {
            match c {
                '{' => depth += 1,
                '}' => depth -= 1,
                c if depth == 0 => plain.push(c),
                _ => {}
            }
        }
        plain.split(|c: char| !c.is_alphabetic()).any(|w| w.chars().count() >= 3 && w.chars().skip(1).any(char::is_lowercase))
    }

    #[test]
    fn catalogs_do_not_leave_strings_in_english() {
        assert!(has_words("Add connector") && !has_words("{n}") && !has_words("CPU") && !has_words("1 / 2"));
        for name in locale_names().filter(|n| *n != "pt_BR") {
            let c = catalog(Some(name));
            let same: Vec<&String> = c.iter().filter(|(k, v)| k == v && has_words(k)).map(|(k, _)| k).collect();
            if ["ja", "ru", "zh_CN"].contains(&name) {
                // another script: only the brands and tokens stay as they are
                let stray: Vec<_> = same.iter().filter(|k| !SAME_IN_EVERY_LANGUAGE.contains(&k.as_str())).collect();
                assert!(stray.is_empty(), "{name}: left in English: {stray:#?}");
            } else {
                // Latin script shares words (Terminal, Normal, "{n} messages"); a sentence never stays the same
                let ratio = same.len() as f64 / c.len() as f64;
                assert!(ratio < 0.035, "{name}: {:.1}% of the strings are the English text", ratio * 100.0);
                let sentences: Vec<_> = same.iter().filter(|k| k.split_whitespace().filter(|w| !w.starts_with('{')).count() >= 4).collect();
                assert!(sentences.is_empty(), "{name}: sentences left in English: {sentences:#?}");
            }
        }
    }

    /// Shortcuts, variables and paths a person types or presses stay as they are: `Ctrl+K`, `KALANGO_FOO`, `~/Pictures`,
    /// `--flag`. Only those that pt_BR (the reviewed catalog) keeps count: a few paths are examples that a language
    /// writes in its own words.
    fn literal_tokens(s: &str) -> Vec<String> {
        let mut out = Vec::new();
        for piece in s.split(|c: char| c.is_whitespace() || "(),;:\"'“”«»<>[]{}*=".contains(c)) {
            let t = piece.trim_end_matches(['.', '…', '!', '?']);
            let combo = ["Ctrl+", "Alt+", "Shift+", "Super+", "Cmd+", "Win+", "Meta+"].iter().any(|p| t.starts_with(p) && t.len() > p.len());
            let env = t.len() >= 4 && t.contains('_') && t.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') && t.starts_with(|c: char| c.is_ascii_uppercase());
            let path = (t.starts_with("~/") || t.starts_with('/')) && t.chars().skip(1).any(|c| c.is_ascii_alphanumeric()) && !s.contains(&format!("}}{t}"));
            let flag = t.starts_with("--") && t.len() > 3;
            if combo || env || path || flag {
                out.push(t.to_string());
            }
        }
        out
    }

    #[test]
    fn shortcuts_variables_and_paths_survive_translation() {
        assert_eq!(literal_tokens("Press Ctrl+Alt+T, set KALANGO_HOME or open ~/Pictures and /etc/hosts (--force)."),
            ["Ctrl+Alt+T", "KALANGO_HOME", "~/Pictures", "/etc/hosts", "--force"]);
        assert!(literal_tokens("Open {app} on 1/2 of the screen").is_empty());
        let pt = catalog(Some("pt_BR"));
        let mut bad = Vec::new();
        let mut checked = 0;
        for name in locale_names().filter(|n| *n != "pt_BR") {
            for (k, v) in catalog(Some(name)) {
                for t in literal_tokens(&k).into_iter().filter(|t| pt[&k].contains(t.as_str())) {
                    checked += 1;
                    if !v.contains(&t) {
                        bad.push(format!("{name}.json: '{k}' → '{v}' lost {t}"));
                    }
                }
            }
        }
        assert!(checked > 100, "only {checked} tokens were looked at");
        assert!(bad.is_empty(), "{}", bad.join("\n"));
    }

    /// Every string marked for translation in the UI crates has a pt_BR entry, and no entry is left unused.
    #[test]
    fn catalog_is_complete_and_has_no_leftovers() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut used = BTreeSet::new();
        let mut files = Vec::new();
        let mut dirs: Vec<_> = ["kalango-shell", "kalango-aprovacao", "kalangod", "kalango-agente", "kalango-entrada", "kalango-instalador", "kalango-d1", "kalango-craft"].iter().map(|k| root.join(k).join("src")).collect();
        while let Some(d) = dirs.pop() {
            for e in std::fs::read_dir(d).unwrap().flatten() {
                if e.path().is_dir() { dirs.push(e.path()) } else if e.path().extension().is_some_and(|x| x == "rs") { files.push(e.path()) }
            }
        }
        {
            for path in files {
                let text = std::fs::read_to_string(&path).unwrap();
                for marker in ["tr(\"", "trf(\"", "n_(\"", "tr_model(\"", "trf_model(\""] {
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
