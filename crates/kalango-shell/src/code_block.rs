//! A code block in an answer (design "Chat · blocos de mensagem" › Código): a header with its language and file,
//! "Copiar" and — in the agent's conversation, when the block names its file — "Aplicar no arquivo", which asks the
//! agent to apply it (the change then shows as a diff to accept before it is written); under it the lines, numbered,
//! with the theme's syntax colours (keywords accent, strings teal, numbers gold, functions blue, types coral).

use crate::conversa::ui_icon;
use crate::superficies::{caixa, rotulo};
use gtk::prelude::*;
use gtk::{glib, Align, Orientation};
use kalango_i18n::tr;
use std::time::Duration;

/// A fence's info line: "rust", "rust src/ui/mod.rs", "rust:src/ui/mod.rs", "src/ui/mod.rs" → (language, file).
pub fn parse_info(info: &str) -> (String, Option<String>) {
    let info = info.trim();
    let (a, b) = match info.split_once(|c: char| c == ':' || c.is_whitespace()) {
        Some((a, b)) => (a.trim(), Some(b.trim().trim_matches(|c| c == '"' || c == '\'')).filter(|b| !b.is_empty())),
        None => (info, None),
    };
    // (a lone path: its extension says the language)
    if b.is_none() && (a.contains('/') || a.contains('.')) {
        let ext = a.rsplit('.').next().unwrap_or("");
        return (ext.to_lowercase(), Some(a.to_string()));
    }
    let b = b.map(|b| b.strip_prefix("title=").unwrap_or(b).trim_matches('"').to_string());
    (a.to_lowercase(), b)
}

/// The language as its header names it.
pub fn language_name(lang: &str) -> String {
    match lang {
        "rs" | "rust" => "Rust",
        "py" | "python" => "Python",
        "js" | "javascript" | "mjs" => "JavaScript",
        "ts" | "typescript" => "TypeScript",
        "sh" | "bash" | "shell" | "zsh" | "console" => "Shell",
        "toml" => "TOML",
        "json" => "JSON",
        "yaml" | "yml" => "YAML",
        "c" | "h" => "C",
        "cpp" | "c++" | "cc" | "hpp" => "C++",
        "go" => "Go",
        "html" => "HTML",
        "css" => "CSS",
        "md" | "markdown" => "Markdown",
        "sql" => "SQL",
        "diff" | "patch" => "Diff",
        "" | "text" | "txt" | "plain" => return tr("Code").to_string(),
        other => return other.to_string(),
    }
    .to_string()
}

const RUST: &[&str] = &["as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where", "while"];
const PYTHON: &[&str] = &["and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif", "else", "except", "False", "finally", "for", "from", "global", "if", "import", "in", "is", "lambda", "None", "nonlocal", "not", "or", "pass", "raise", "return", "self", "True", "try", "while", "with", "yield"];
const JS: &[&str] = &["async", "await", "break", "case", "catch", "class", "const", "continue", "default", "delete", "do", "else", "export", "extends", "false", "finally", "for", "from", "function", "if", "import", "in", "instanceof", "let", "new", "null", "of", "return", "static", "super", "switch", "this", "throw", "true", "try", "typeof", "undefined", "var", "void", "while", "yield", "interface", "type", "enum", "implements"];
const SHELL: &[&str] = &["if", "then", "else", "elif", "fi", "for", "while", "do", "done", "case", "esac", "in", "function", "return", "local", "export", "sudo", "cd", "echo", "exit"];
const C: &[&str] = &["auto", "break", "case", "char", "const", "continue", "default", "do", "double", "else", "enum", "extern", "float", "for", "goto", "if", "int", "long", "register", "return", "short", "signed", "sizeof", "static", "struct", "switch", "typedef", "union", "unsigned", "void", "volatile", "while", "class", "public", "private", "namespace", "template", "true", "false", "nullptr", "include", "define", "func", "package", "import", "var", "type", "go", "defer", "chan", "map", "range"];

/// What a token is (its colour).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Plain,
    Keyword,
    Text,
    Number,
    Function,
    Type,
    Comment,
}

fn keywords(lang: &str) -> &'static [&'static str] {
    match lang {
        "rs" | "rust" => RUST,
        "py" | "python" => PYTHON,
        "js" | "javascript" | "mjs" | "ts" | "typescript" => JS,
        "sh" | "bash" | "shell" | "zsh" | "console" => SHELL,
        "c" | "h" | "cpp" | "c++" | "cc" | "hpp" | "go" => C,
        _ => &[],
    }
}

fn line_comment(lang: &str) -> &'static [&'static str] {
    match lang {
        "py" | "python" | "sh" | "bash" | "shell" | "zsh" | "console" | "toml" | "yaml" | "yml" => &["#"],
        "sql" => &["--"],
        "" | "text" | "txt" | "plain" | "md" | "markdown" | "json" => &[],
        _ => &["//"],
    }
}

/// A line split into coloured tokens (no state across lines: a block comment colours its own line only).
pub fn tokens(lang: &str, line: &str) -> Vec<(Kind, String)> {
    let kw = keywords(lang);
    let comments = line_comment(lang);
    let rust = matches!(lang, "rs" | "rust");
    let chars: Vec<char> = line.chars().collect();
    let mut out: Vec<(Kind, String)> = Vec::new();
    let push = |out: &mut Vec<(Kind, String)>, k: Kind, s: String| {
        if let Some(last) = out.last_mut().filter(|l| l.0 == k) {
            last.1.push_str(&s);
        } else {
            out.push((k, s));
        }
    };
    let mut i = 0;
    while i < chars.len() {
        let rest: String = chars[i..].iter().collect();
        if comments.iter().any(|c| rest.starts_with(c)) || rest.starts_with("/*") && !comments.is_empty() {
            push(&mut out, Kind::Comment, rest);
            break;
        }
        let c = chars[i];
        // strings ("…", and '…' where it is not a Rust lifetime or char of one letter)
        if c == '"' || c == '`' || (c == '\'' && !rust) {
            let mut j = i + 1;
            while j < chars.len() && chars[j] != c {
                if chars[j] == '\\' {
                    j += 1;
                }
                j += 1;
            }
            let end = (j + 1).min(chars.len());
            push(&mut out, Kind::Text, chars[i..end].iter().collect());
            i = end;
            continue;
        }
        if c.is_ascii_digit() {
            let mut j = i;
            while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '.' || chars[j] == '_') {
                j += 1;
            }
            push(&mut out, Kind::Number, chars[i..j].iter().collect());
            i = j;
            continue;
        }
        if c.is_alphabetic() || c == '_' {
            let mut j = i;
            while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') {
                j += 1;
            }
            let word: String = chars[i..j].iter().collect();
            // (a macro's "!" goes with its name)
            let macro_bang = rust && chars.get(j) == Some(&'!') && chars.get(j + 1) != Some(&'=');
            let next = chars[j..].iter().find(|c| !c.is_whitespace()).copied();
            let kind = if kw.contains(&word.as_str()) {
                Kind::Keyword
            } else if macro_bang {
                Kind::Type
            } else if next == Some('(') && !kw.is_empty() {
                Kind::Function
            } else if word.chars().next().is_some_and(char::is_uppercase) && !kw.is_empty() {
                Kind::Type
            } else {
                Kind::Plain
            };
            push(&mut out, kind, word);
            i = j;
            if macro_bang {
                push(&mut out, Kind::Type, "!".into());
                i += 1;
            }
            continue;
        }
        push(&mut out, Kind::Plain, c.to_string());
        i += 1;
    }
    out
}

/// The block's lines as Pango markup, in the theme's colours now.
fn markup(lang: &str, code: &str) -> String {
    let cor = |k: Kind| -> String {
        crate::tema::token(match k {
            Kind::Plain => "k_text2",
            Kind::Keyword => "k_accent",
            Kind::Text => "k_teal",
            Kind::Number => "k_gold",
            Kind::Function => "k_blue",
            Kind::Type => "k_coral",
            Kind::Comment => "k_dim",
        })
    };
    code.lines()
        .map(|l| {
            tokens(lang, l)
                .into_iter()
                .map(|(k, s)| format!("<span foreground=\"{}\">{}</span>", cor(k), glib::markup_escape_text(&s)))
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The block. `apply`: show "Aplicar no arquivo" (the agent's conversation, a block that names its file); the
/// button runs the widget action `conversa.aplicar` with `{"file", "lang", "code"}`.
pub fn new(info: &str, code: &str, apply: bool) -> gtk::Widget {
    let (lang, file) = parse_info(info);
    let bloco = caixa(Orientation::Vertical, 0, "md-bloco");
    let cab = caixa(Orientation::Horizontal, 8, "md-bloco-cab");
    cab.append(&ui_icon("codigo", "#8E8F99", 16));
    let t = rotulo(&language_name(&lang), "md-bloco-titulo");
    cab.append(&t);
    if let Some(f) = &file {
        let d = rotulo(f, "md-bloco-detalhe");
        d.set_ellipsize(gtk::pango::EllipsizeMode::Start);
        d.set_max_width_chars(1);
        d.set_hexpand(true);
        d.set_xalign(0.0);
        cab.append(&d);
    } else {
        let e = caixa(Orientation::Horizontal, 0, "");
        e.set_hexpand(true);
        cab.append(&e);
    }
    let copiar = gtk::Button::new();
    copiar.add_css_class("md-bloco-botao");
    let cc = caixa(Orientation::Horizontal, 6, "");
    cc.append(&ui_icon("copiar", "#8E8F99", 16));
    let cl = rotulo(tr("Copy"), "");
    cc.append(&cl);
    copiar.set_child(Some(&cc));
    copiar.set_valign(Align::Center);
    let texto = code.to_string();
    let n_linhas = code.lines().count();
    copiar.connect_clicked(move |b| {
        b.clipboard().set_text(&texto);
        cl.set_text(tr("Copied"));
        let l = cl.downgrade();
        glib::timeout_add_local_once(Duration::from_millis(1500), move || {
            if let Some(l) = l.upgrade() {
                l.set_text(tr("Copy"));
            }
        });
        if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
            eprintln!("debug codigo: copiado ({n_linhas} linhas)");
        }
    });
    cab.append(&copiar);
    crate::central::posicao_debug("codigo copiar", copiar.upcast_ref());
    if let (true, Some(f)) = (apply, &file) {
        let aplicar = gtk::Button::with_label(tr("Apply to the file"));
        aplicar.add_css_class("md-bloco-aplicar");
        aplicar.set_valign(Align::Center);
        let p = serde_json::json!({"file": f, "lang": lang, "code": code}).to_string();
        aplicar.connect_clicked(move |b| {
            if b.activate_action("conversa.aplicar", Some(&p.to_variant())).is_err() {
                b.set_sensitive(false);
            }
        });
        crate::central::posicao_debug("codigo aplicar", aplicar.upcast_ref());
        cab.append(&aplicar);
    }
    bloco.append(&cab);
    bloco.append(&caixa(Orientation::Horizontal, 0, "md-bloco-div"));
    let corpo = caixa(Orientation::Horizontal, 14, "md-bloco-corpo");
    let n = code.lines().count().max(1);
    let nums = rotulo(&(1..=n).map(|i| i.to_string()).collect::<Vec<_>>().join("\n"), "md-bloco-num");
    nums.set_xalign(1.0);
    nums.set_valign(Align::Start);
    corpo.append(&nums);
    let linhas = gtk::Label::new(None);
    linhas.add_css_class("md-bloco-codigo");
    linhas.set_xalign(0.0);
    linhas.set_valign(Align::Start);
    linhas.set_selectable(true);
    linhas.set_markup(&markup(&lang, code));
    // (the theme changes: the colours again)
    let (lg, cd) = (lang.clone(), code.to_string());
    crate::tema::registrar(&linhas, move |w| {
        if let Some(l) = w.downcast_ref::<gtk::Label>() {
            l.set_markup(&markup(&lg, &cd));
        }
    });
    corpo.append(&linhas);
    // long lines scroll sideways (the answer's column keeps its width)
    let rol = gtk::ScrolledWindow::new();
    rol.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Never);
    rol.set_propagate_natural_height(true);
    rol.set_child(Some(&corpo));
    rol.add_css_class("md-bloco-rolagem");
    bloco.append(&rol);
    if std::env::var_os("KALANGO_SHELL_DEBUG").is_some() {
        eprintln!("debug codigo: bloco {} ({n} linhas){}", language_name(&lang), file.as_ref().map(|f| format!(" de {f}")).unwrap_or_default());
    }
    bloco.upcast()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_line() {
        assert_eq!(parse_info("rust"), ("rust".into(), None));
        assert_eq!(parse_info("rust src/ui/mod.rs"), ("rust".into(), Some("src/ui/mod.rs".into())));
        assert_eq!(parse_info("rust:src/ui/mod.rs"), ("rust".into(), Some("src/ui/mod.rs".into())));
        assert_eq!(parse_info("src/ui/mod.rs"), ("rs".into(), Some("src/ui/mod.rs".into())));
        assert_eq!(parse_info("python title=\"app.py\""), ("python".into(), Some("app.py".into())));
        assert_eq!(parse_info(""), ("".into(), None));
    }

    #[test]
    fn rust_tokens() {
        let t = tokens("rust", "    let t = x11_get_server_time(&xwin); // hora");
        assert!(t.contains(&(Kind::Keyword, "let".into())));
        assert!(t.contains(&(Kind::Function, "x11_get_server_time".into())));
        assert_eq!(t.last().unwrap(), &(Kind::Comment, "// hora".into()));
        let t = tokens("rust", "println!(\"oi {}\", 42);");
        assert!(t.contains(&(Kind::Type, "println!".into())));
        assert!(t.contains(&(Kind::Text, "\"oi {}\"".into())));
        assert!(t.contains(&(Kind::Number, "42".into())));
        // a lifetime is not a string
        let t = tokens("rust", "fn f<'a>(x: &'a str) -> Option<String>");
        assert!(!t.iter().any(|(k, _)| *k == Kind::Text));
        assert!(t.contains(&(Kind::Type, "Option".into())));
    }

    #[test]
    fn shell_and_plain() {
        let t = tokens("bash", "cargo test --release # tudo");
        assert_eq!(t.last().unwrap(), &(Kind::Comment, "# tudo".into()));
        // plain text: nothing coloured as code
        let t = tokens("", "Olá, mundo (de novo)");
        assert!(t.iter().all(|(k, _)| *k == Kind::Plain || *k == Kind::Number));
    }
}
